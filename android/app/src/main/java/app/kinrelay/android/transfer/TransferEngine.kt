package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.protocol.Offer
import app.kinrelay.android.protocol.OfferHistory
import app.kinrelay.android.protocol.Utf8
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap

/** A file the user shared: a content URI and what the share said about it. */
data class ShareSource(
    val uri: String,
    val nameHint: String? = null,
    val mimeHint: String? = null,
    /**
     * An auto-sent image (2B.4) is stored, and sent, under a generic name that is this text plus the file's extension ("Screenshot 2026-10-17 14.32.05.png"), never its real name (the real name of a
     * screenshot says which app it came from: design 5.8).
     */
    val genericStem: String? = null,
)

/**
 * Owns the transfers: it creates them (a share, a `.txt` for long text, an `offer`), runs them one at a time per direction on the
 * app's scope, and handles cancel, retry and recovery. Nothing here blocks the WebSocket: an `offer` is acked first, and the
 * download runs on its own coroutine.
 *
 * The queue (1.4): every upload is stored as `queued` before it is tried, with its small files already copied into the cache when it
 * couldn't start ([TransferStatus.QUEUED]). When the link is up again the [app.kinrelay.android.link.OutboxFlusher] starts the queued
 * uploads in creation order ([queuedUploads], [startQueuedUpload]).
 *
 * A transfer belongs to one PC (its `peerDeviceId`). Everything that comes from a PC (an `offer`, a `cancel`) says which PC it came
 * from, and a transfer is only ever started, accepted, cancelled or recorded for that PC: right after a re-pair the old PC's connection
 * is briefly still the current one.
 */
class TransferEngine(
    private val scope: CoroutineScope,
    private val d: TransferDeps,
    private val timings: TransferTimings = TransferTimings(),
    private val mimeForExtension: (String) -> String? = { null },
    private val httpFor: (app.kinrelay.android.link.HttpEndpoint) -> TransferHttp = ::TransferHttp,
) {
    private val owedMessages = OwedMessages(d, timings.ackTimeoutMillis)
    private val uploadLock = Mutex()
    private val downloadLock = Mutex()
    private val jobs = ConcurrentHashMap<String, Job>()

    /** Uploads whose row is being written and whose job is not registered yet: nothing may judge them "unfinished and not running" (see [createUpload]). */
    private val creating: MutableSet<String> = ConcurrentHashMap.newKeySet()

    /** Old transfer ID to new, for a transfer that started over under a new ID (a changed source, or a user Retry after `404`). */
    private val renamed = ConcurrentHashMap<String, String>()

    // ---------------------------------------------------------------- phone -> PC

    /**
     * Creates an upload for [sources] and starts it. Returns the transfer ID, or null when nothing could be sent (no paired PC,
     * no readable file, too many files). The user is told in the null cases.
     */
    suspend fun enqueueUpload(sources: List<ShareSource>, origin: String? = null): String? {
        val peer = d.link.peer() ?: run {
            d.notifier.message("Pair a PC first.")
            return null
        }
        if (sources.size > MAX_FILES) {
            d.notifier.message("Too many files at once (the limit is $MAX_FILES).")
            return null
        }
        val id = d.newId()
        val files = ArrayList<TransferFileEntity>()
        // Each look-up is a round trip to the app that owns the file (a gallery): 50 photos one after the other took seconds, so they are
        // looked at several at a time. Only the cheap, read-only `stat` runs in parallel; what follows (a cache copy, the file IDs) is in order.
        val looked = sources.mapParallel(timings.statParallelism) { d.sources.stat(it.uri) }
        for ((index, source) in sources.withIndex()) {
            val fileId = "f${files.size + 1}"
            var uri = source.uri
            var info = looked[index]
            if (info == null) {
                // No size to report (a stream, or a provider that doesn't say): a copy in the cache has one.
                val cached = runCatching { d.sources.cacheCopy(uri, id, fileId) }.getOrNull()
                info = cached?.let { d.sources.stat(it) }
                if (info != null) uri = cached!!
            }
            if (info == null) {
                d.log.w("transfer", "share: a file can't be read and was skipped (${source.uri.substringBefore(':')})")
                continue
            }
            val realName = (info.displayName ?: source.nameHint)?.takeIf { it.isNotBlank() }
            val mime = (info.mime ?: source.mimeHint)?.takeIf { it.isNotBlank() && !it.contains('*') } ?: MediaKinds.DEFAULT_MIME
            val name = source.genericStem?.let { it + genericExtension(realName, mime) } ?: realName ?: fallbackName(source.uri)
            files += TransferFileEntity(id, fileId, files.size, name, null, mime, info.size, 0, uri, info.modifiedMillis, finished = false)
        }
        if (files.isEmpty()) {
            d.notifier.message("Kin couldn't read what you shared.")
            return null
        }
        createUpload(id, peer, files, origin)
        return id
    }

    /** Long text (over [app.kinrelay.android.protocol.MAX_TEXT_BYTES]) goes to the PC as a `.txt` file. [text] is already in wire form. */
    suspend fun enqueueText(text: String, fileName: String): String? {
        val peer = d.link.peer() ?: run {
            d.notifier.message("Pair a PC first.")
            return null
        }
        val id = d.newId()
        val uri = d.sources.cacheText(text, id, "f1")
        val info = d.sources.stat(uri) ?: return null
        val file = TransferFileEntity(id, "f1", 0, fileName, null, "text/plain", info.size, 0, uri, info.modifiedMillis, finished = false)
        createUpload(id, peer, listOf(file))
        return id
    }

    /** Stored as `queued` first, then tried at once. With no connection it stays queued and goes out when the link is up again. */
    private suspend fun createUpload(id: String, peer: PeerInfo, files: List<TransferFileEntity>, origin: String? = null) {
        val transfer = TransferEntity(
            id, TransferDirection.UP, peer.deviceId, peer.name, TransferKind.FILE, TransferStatus.QUEUED,
            d.nowMillis(), null, files.sumOf { it.size }, null, null, completeAcked = false, origin = origin,
        )
        // Between the row and its job there was a gap (the 2B.4 review): a connection coming up in it ran [endAutoSentUnfinished], saw an unfinished auto-sent upload that nothing ran, and ended it
        // as "not sent". The id is marked as being created from before the row exists until the job is registered.
        creating += id
        try {
            d.store.create(transfer, files)
            d.log.i("transfer", "upload $id created: ${files.size} files, ${transfer.totalBytes} bytes")
            // The files were looked at a moment ago (that is how the transfer was made): the first attempt doesn't look at all of them again.
            launchUpload(id, sourcesJustChecked = true)
        } finally {
            creating -= id
        }
    }

    private fun fallbackName(uri: String): String = uri.substringBefore('?').substringAfterLast('/').ifBlank { "file" }

    /** The extension (with its dot) of the real name, else one from the MIME type, else none: all that is kept of a real name when a generic one is used. */
    private fun genericExtension(realName: String?, mime: String): String {
        val own = realName?.substringAfterLast('.', "")?.takeIf { it.length in 1..5 && it.all { c -> c.isLetterOrDigit() } }
        val ext = own ?: when (mime.lowercase()) {
            "image/png" -> "png"
            "image/jpeg" -> "jpg"
            "image/webp" -> "webp"
            "image/gif" -> "gif"
            "image/heic" -> "heic"
            else -> null
        }
        return if (ext == null) "" else ".${ext.lowercase()}"
    }

    private fun launchUpload(id: String, userRetry: Boolean = false, sourcesJustChecked: Boolean = false) = launch(id, uploadLock) {
        UploadRunner(d, timings, httpFor, onRenamed = { old, new -> rename(old, new) }, userRetry = userRetry, sourcesJustChecked = sourcesJustChecked).run(id)
    }

    private fun rename(old: String, new: String) {
        renamed[old] = new
        jobs[old]?.let { job -> jobs[new] = job }
    }

    // ---------------------------------------------------------------- PC -> phone

    /**
     * The gate for an `offer`, run before the ack. It returns null to accept, or the code of the `error` the phone sends *instead of*
     * the ack (protocol.md, Files: PC -> phone), which ends the offer's life:
     * - `too_large`: more than [MAX_OFFER_FILES] files, or sizes that overflow a 64-bit sum;
     * - `bad_request`: a negative size, an offer from a PC that isn't the paired one (only possible in the moment after a re-pair), or a
     *   transfer ID that is already used by something else (an upload, a transfer of another PC, a finished one). A transfer ID is never
     *   reused; only a re-offer of an unfinished download from the same PC resumes it;
     * - `disk_full`: the files don't fit in the free space once what the unfinished downloads still have to write, and what offers
     *   accepted a moment ago have claimed, is taken away. What a download still has to write is counted from the bytes it has really
     *   written ([LiveProgress]), because the free space already shrank by them.
     *
     * [fromPeerDeviceId] is the PC whose connection the offer arrived on. It runs on the socket's thread, not the main thread: the store
     * lookups are short and are awaited here.
     */
    fun acceptOffer(offer: Offer, fromPeerDeviceId: String): String? = runBlocking { gate(offer, fromPeerDeviceId) }

    /**
     * What the phone knows about the ID of an `offer` (protocol.md, Files: PC -> phone, item 52). **The same paired PC repeating an ID it
     * offered before** is a [OfferHistory.REPEAT], whatever state the download is in: the phone answers the first response again (the ack)
     * and downloads nothing a second time ([onOfferRepeated] does the rest). **Any other reuse** is [OfferHistory.REUSED] (`bad_request`):
     * the ID of an upload of ours, of a transfer of another PC, an offer from a PC that isn't the paired one, or an offer whose file list
     * (file IDs and sizes) differs from the first one. An ID the transfer store has never seen is [OfferHistory.NEW] (the receiver still
     * checks its memory of message IDs: a `text` may have used it).
     */
    fun offerHistory(offer: Offer, fromPeerDeviceId: String): OfferHistory = runBlocking {
        val existing = d.store.transfer(offer.id) ?: return@runBlocking OfferHistory.NEW
        val peer = d.link.peer()
        if (peer == null || peer.deviceId != fromPeerDeviceId) return@runBlocking OfferHistory.REUSED
        if (existing.direction != TransferDirection.DOWN || existing.peerDeviceId != peer.deviceId) return@runBlocking OfferHistory.REUSED
        // The file rows of a transfer that is long over may have been dropped (storage): then there is nothing to compare, and it is a repeat.
        val stored = d.store.files(offer.id)
        if (stored.isNotEmpty() && !sameFiles(stored, offer)) return@runBlocking OfferHistory.REUSED
        OfferHistory.REPEAT
    }

    /** The file IDs and sizes of an offer are what the first offer said. */
    private fun sameFiles(stored: List<TransferFileEntity>, offer: Offer): Boolean =
        stored.size == offer.files.size && stored.associate { it.fileId to it.size } == offer.files.associate { it.fileId to it.size }

    private suspend fun gate(offer: Offer, fromPeerDeviceId: String): String? {
        if (offer.files.size > MAX_OFFER_FILES) return "too_large"
        if (offer.files.any { it.size < 0 }) return "bad_request"
        var needed = 0L
        for (file in offer.files) needed = addOrNull(needed, file.size) ?: return "too_large"
        val peer = d.link.peer()
        if (peer == null || peer.deviceId != fromPeerDeviceId) return "bad_request"
        // A repeat or a reuse never reaches the gate (the receiver asked [offerHistory] first); an ID the store knows is refused anyway.
        if (d.store.transfer(offer.id) != null) return "bad_request"
        val claimed = synchronized(reservations) {
            val now = d.nowMillis()
            reservations.values.removeAll { now - it.atMillis > RESERVATION_MILLIS }
            reservations.values.fold(0L) { sum, r -> addOrNull(sum, r.bytes) ?: Long.MAX_VALUE }
        }
        val written = d.progress.stillToWrite(d.store.pendingDownloadFiles()) ?: return "too_large"
        val stillToWrite = addOrNull(written, claimed) ?: return "too_large"
        val required = addOrNull(needed, stillToWrite) ?: return "too_large"
        if (required > d.sink.freeBytes()) return "disk_full"
        synchronized(reservations) { reservations[offer.id] = Reservation(needed, d.nowMillis()) }
        return null
    }

    private fun addOrNull(a: Long, b: Long): Long? = try {
        Math.addExact(a, b)
    } catch (_: ArithmeticException) {
        null
    }

    /** An `offer` the gate accepted, and the bytes it will need until its rows are in the store (then the store counts them). */
    private class Reservation(val bytes: Long, val atMillis: Long)

    private val reservations = HashMap<String, Reservation>()

    /**
     * A valid `offer` that was acked: remember it and download it. Returns at once; the download runs on its own coroutine. It is recorded
     * for the PC it came from ([fromPeerDeviceId]), and only if that PC is the paired one.
     */
    suspend fun onOffer(fromPeerDeviceId: String, peerName: String, offer: Offer) {
        try {
            val peer = d.link.peer()
            if (peer == null || peer.deviceId != fromPeerDeviceId) {
                d.log.w("transfer", "offer ${offer.id} from '$peerName' ignored: that PC is not the paired one")
                return
            }
            if (d.store.transfer(offer.id) != null) {
                // A transfer ID is never reused (protocol.md). The rows are not touched: they may be an upload of ours.
                d.log.w("transfer", "offer ${offer.id} reuses the ID of another transfer; ignored")
                return
            }
            val files = offer.files.mapIndexed { i, f ->
                TransferFileEntity(offer.id, f.fileId, i, f.name, null, f.mime, f.size, 0, null, null, finished = false)
            }
            val transfer = TransferEntity(
                offer.id, TransferDirection.DOWN, peer.deviceId, peer.name, TransferKind.FILE,
                TransferStatus.QUEUED, d.nowMillis(), null, files.sumOf { it.size }, null, null, completeAcked = false,
            )
            d.store.create(transfer, files)
            d.log.i("transfer", "offer ${offer.id} accepted: ${files.size} files, ${transfer.totalBytes} bytes")
            launchDownload(offer.id)
        } finally {
            synchronized(reservations) { reservations.remove(offer.id) }
        }
    }

    /**
     * A valid `offer` repeated by the paired PC (the receiver acked it again, see [offerHistory]); nothing is downloaded a second time.
     * An **unfinished** download carries on (resumes with `Range` if it isn't running). A download that is **done** sends its `complete`
     * again if the PC never acked it, and one that was **canceled or failed** sends its `cancel` again (both are owed until acked).
     */
    suspend fun onOfferRepeated(fromPeerDeviceId: String, offer: Offer) {
        val peer = d.link.peer()
        if (peer == null || peer.deviceId != fromPeerDeviceId) return
        val transfer = d.store.transfer(offer.id) ?: return
        if (transfer.direction != TransferDirection.DOWN || transfer.peerDeviceId != peer.deviceId) return
        if (!TransferStatus.isFinished(transfer.status)) {
            d.log.i("transfer", "offer ${offer.id} is repeated; the download carries on")
            launchDownload(offer.id)
            return
        }
        if (transfer.owed == null) return
        d.log.i("transfer", "offer ${offer.id} is repeated; its ${transfer.owed} was not acked, so it is sent again")
        owedMessages.send(d.connectionFor(transfer, timings.connectWaitMillis, kick = false), transfer.id)
    }

    private fun launchDownload(id: String) = launch(id, downloadLock) {
        DownloadRunner(d, timings, httpFor, mimeForExtension).run(id)
    }

    // ---------------------------------------------------------------- cancel, retry, recovery

    /** The user cancelled (the notification's Cancel, or Cancel in the history). An upload sends `DELETE`; a download sends a `cancel` message. */
    fun cancel(transferId: String) {
        scope.launch(Dispatchers.IO) {
            val id = resolve(transferId)
            jobs[id]?.cancelAndJoin()
            val transfer = d.store.transfer(id) ?: return@launch
            if (TransferStatus.isFinished(transfer.status)) {
                d.notifier.clear(id)
                return@launch
            }
            d.log.i("transfer", "$id cancelled by the user")
            if (transfer.direction == TransferDirection.UP) cancelUpload(transfer) else cancelDownload(transfer)
        }
    }

    private suspend fun cancelUpload(transfer: TransferEntity) {
        // A queued upload that a flush started a moment ago is stopped by the first line of `cancel`; this status change is the one that counts.
        if (!d.store.setStatus(transfer.id, TransferStatus.CANCELED, "user", "Canceled.", d.nowMillis())) return
        d.notifier.clear(transfer.id)
        d.sources.deleteCache(transfer.localId)
        // The PC drops its partial files. A retried DELETE that gets 404 counts as done (the transfer is already gone).
        repeat(DELETE_ATTEMPTS) { attempt ->
            val endpoint = d.connectionFor(transfer, if (attempt == 0) timings.connectWaitMillis else 1_000, kick = attempt == 0)?.httpEndpoint
            if (endpoint != null) {
                try {
                    if (httpFor(endpoint).delete(transfer.id) is ApiResult.Ok) return
                } catch (e: CancellationException) {
                    throw e
                } catch (e: IOException) {
                    d.log.w("transfer", "DELETE ${transfer.id}: ${e.javaClass.simpleName}: ${e.message}")
                }
            }
            delay(timings.retryDelayMillis)
        }
        d.log.w("transfer", "the PC wasn't told about the cancel of ${transfer.id}; it drops the partial files after 24 hours")
    }

    private suspend fun cancelDownload(transfer: TransferEntity) {
        // The `cancel` is owed to the PC from this write on: sent now if there is a connection, else after the next `hello`, until acked.
        val owed = OwedMessages.cancel("user")
        if (!d.store.setStatus(transfer.id, TransferStatus.CANCELED, "user", "Canceled.", d.nowMillis(), owed, OwedMessages.newMessageId())) return
        DownloadRunner.discardPartials(d, transfer.id)
        d.notifier.clear(transfer.id)
        owedMessages.send(d.connectionFor(transfer, timings.connectWaitMillis, kick = false), transfer.id)
    }

    /**
     * The PC sent `cancel`. Unknown or finished transfers are ignored (the message was acked already), and so is a `cancel` from a PC
     * that the transfer doesn't belong to. The status is read again after the job stopped: a transfer that finished meanwhile stays
     * `done` (a late `cancel` must not turn a received file into a canceled one). **One exception:** a transfer that **failed with
     * `not_found`** (the phone only found out through a `404`) is turned into "canceled on <PC>" by the `cancel` of the PC that owns it: that
     * is what the PC says, and the history then says the same (protocol.md, item 52). It can never change `done`.
     */
    fun onPeerCancel(fromPeerDeviceId: String, transferId: String, reason: String) {
        scope.launch(Dispatchers.IO) {
            val id = resolve(transferId)
            val first = d.store.transfer(id) ?: return@launch
            if (first.peerDeviceId != fromPeerDeviceId) {
                d.log.w("transfer", "a cancel for $id came from a PC the transfer doesn't belong to; ignored")
                return@launch
            }
            if (TransferStatus.isFinished(first.status)) {
                if (first.status == TransferStatus.FAILED && first.errorCode == "not_found" &&
                    d.store.markCanceledByPeer(id, "Canceled on ${first.peerName}.")
                ) {
                    d.log.i("transfer", "$id: the PC's cancel turns a failed (not_found) transfer into a canceled one")
                    d.notifier.clear(id)
                }
                return@launch
            }
            jobs[id]?.cancelAndJoin()
            val transfer = d.store.transfer(id) ?: return@launch
            if (TransferStatus.isFinished(transfer.status)) return@launch
            val message = "Canceled on ${transfer.peerName}."
            // The status change is guarded: if it doesn't apply, the transfer ended in the meantime and nothing else here may touch it.
            if (!d.store.setStatus(id, TransferStatus.CANCELED, "peer_canceled", message, d.nowMillis())) return@launch
            d.log.i("transfer", "$id cancelled by the PC (reason '$reason')")
            if (transfer.direction == TransferDirection.DOWN) DownloadRunner.discardPartials(d, id) else d.sources.deleteCache(transfer.localId)
            d.notifier.clear(id)
            d.notifier.failed(transfer.copy(status = TransferStatus.CANCELED), message)
        }
    }

    /**
     * Starts a transfer again because the **user** asked (the Retry button): a queued one, or a failed upload. A download can only be
     * resumed while it is queued. An upload first `POST`s its old transfer ID, so one the PC finished within 24 hours is recognised; on
     * `404` it goes on under a new ID with only the files that never got a `200` ([UploadRunner]). Nothing else may do that: an
     * automatic resume that gets `404` fails the transfer and sends nothing.
     */
    fun retry(transferId: String) {
        scope.launch(Dispatchers.IO) {
            val id = resolve(transferId)
            val transfer = d.store.transfer(id) ?: return@launch
            val retryable = transfer.status == TransferStatus.QUEUED ||
                (transfer.direction == TransferDirection.UP && transfer.status == TransferStatus.FAILED)
            if (!retryable || transfer.retriedAs != null) return@launch
            // The run that follows is the user's, even if it has to wait for the PC and the queue flush starts it later: it is stored.
            if (!d.store.requeue(id, userRetry = transfer.direction == TransferDirection.UP)) return@launch
            if (transfer.direction == TransferDirection.UP) launchUpload(id, userRetry = true) else launchDownload(id)
        }
    }

    /** The queued uploads, oldest first (the queue flush merges them with the queued texts by creation time). */
    suspend fun queuedUploads(): List<TransferEntity> =
        d.store.unfinished().filter { it.direction == TransferDirection.UP && it.status == TransferStatus.QUEUED && it.origin == null }

    /**
     * Auto-sent images (2B.4) are **never queued and never resumed later**: one that is still unfinished when a connection comes up and that nothing is running (`queued`: the process died between
     * its creation and the first `POST`, or no connection came; `active`: the connection or the process was lost mid-upload) is ended as "not sent" instead of going out hours later and landing on
     * the PC clipboard. (The PC drops what it received of it after its 24 hours without progress.)
     */
    suspend fun endAutoSentUnfinished() {
        val now = d.nowMillis()
        for (transfer in d.store.unfinished()) {
            if (transfer.direction != TransferDirection.UP || transfer.origin == null) continue
            if (transfer.status != TransferStatus.QUEUED && transfer.status != TransferStatus.ACTIVE) continue
            if (jobs[transfer.id]?.isActive == true || transfer.id in creating) continue
            if (d.store.setStatus(transfer.id, TransferStatus.FAILED, "offline", "Not sent: the PC was away. Screenshots are never queued or resumed.", now)) {
                d.sources.deleteCache(transfer.localId)
                d.notifier.clear(transfer.id)
            }
        }
    }

    /**
     * Auto-send (2B.4): sends [sources] with [origin] and waits for the PC's answer to the first `POST` (up to [waitMillis]). Returns true if the PC took it (the bytes follow on their own coroutine),
     * false if it didn't answer in time or the transfer could not be made: the transfer is then cancelled, never queued. The caller has checked that a ready connection exists.
     */
    suspend fun sendNow(sources: List<ShareSource>, origin: String, waitMillis: Long): Boolean {
        val id = enqueueUpload(sources, origin) ?: return false
        val deadline = System.nanoTime() + waitMillis * 1_000_000
        while (true) {
            val transfer = d.store.transfer(id)
            if (transfer == null) return false
            if (transfer.started || transfer.status == TransferStatus.DONE) return true
            if (TransferStatus.isFinished(transfer.status)) return false
            if (System.nanoTime() >= deadline) break
            delay(START_POLL_MILLIS)
        }
        cancel(id)
        return false
    }

    /** The queue flush starts a queued upload. Automatic: a `404` to its `POST` fails it, it never goes out under a new ID. */
    fun startQueuedUpload(id: String) = launchUpload(id)

    /** Starts every queued upload, oldest first. */
    suspend fun retryQueuedUploads() {
        for (transfer in queuedUploads()) launchUpload(transfer.id)
    }

    /**
     * Queued uploads older than [maxAgeMillis] have expired: they fail with code `expired` and their cached copies go. (Texts have the
     * same rule in the text queue.) **Only an upload that never started expires** (the spec's 7 days are for the queue); one the PC has
     * answered follows the PC's `.part` rule (24 hours without progress), which the phone learns from a `404` when it resumes. Returns how
     * many expired.
     */
    suspend fun expireQueued(maxAgeMillis: Long): Int {
        val now = d.nowMillis()
        var expired = 0
        for (transfer in queuedUploads()) {
            if (transfer.started || now - transfer.createdAtMillis < maxAgeMillis || jobs[transfer.id]?.isActive == true) continue
            if (d.store.setStatus(transfer.id, TransferStatus.FAILED, EXPIRED, "Expired: it waited in the queue for too long.", now)) {
                d.sources.deleteCache(transfer.localId)
                d.notifier.clear(transfer.id)
                d.log.w("transfer", "${transfer.id} expired in the queue")
                expired++
            }
        }
        return expired
    }

    /**
     * The link to the PC [fromPeerDeviceId] is up (or the service just started): bring back what an earlier run left behind. An `active`
     * upload resumes (its source access is checked first), and a download resumes whether it is `active` or waiting for the PC. Only
     * the transfers of that PC are touched. Queued uploads are started by the queue flush. Safe to call at every connect.
     *
     * **First** the `complete` and `cancel` messages that nobody acked are sent again on [connection] (protocol.md, Queue: kept until acked,
     * re-sent after every `hello`, before the queue). Then the unfinished transfers resume.
     */
    suspend fun onLinkReady(fromPeerDeviceId: String, connection: LinkConnection? = null) {
        dropTransfersOfOtherPcs()
        endAutoSentUnfinished()
        val link = connection ?: d.link.connection(0, false, fromPeerDeviceId)
        if (link != null && link.peerDeviceId == fromPeerDeviceId) owedMessages.resendAll(link)
        for (transfer in d.store.unfinished()) {
            if (jobs[transfer.id]?.isActive == true || transfer.peerDeviceId != fromPeerDeviceId) continue
            when {
                transfer.direction == TransferDirection.DOWN -> launchDownload(transfer.id)
                transfer.status == TransferStatus.ACTIVE -> launchUpload(transfer.id)
            }
        }
    }

    // ---------------------------------------------------------------- unpair, orphans

    /**
     * The PC [deviceId] is not paired any more (unpaired from either side, or replaced by another PC): every unfinished transfer
     * with it is cancelled and its partial files and cached copies are deleted. Nothing is sent to anyone: the transfer's PC is gone,
     * and the PC that is paired now must never see it. Files that were saved completely stay.
     */
    suspend fun onUnpaired(deviceId: String) {
        for (transfer in d.store.unfinished()) {
            if (transfer.peerDeviceId == deviceId) dropUnpaired(transfer)
        }
    }

    /** The same for every unfinished transfer whose PC is not the paired one (a pairing replaced, or rows from before the PC was recorded). */
    private suspend fun dropTransfersOfOtherPcs() {
        val paired = d.link.peer()?.deviceId
        for (transfer in d.store.unfinished()) {
            if (transfer.peerDeviceId != paired) dropUnpaired(transfer)
        }
    }

    private suspend fun dropUnpaired(transfer: TransferEntity) {
        val id = resolve(transfer.id)
        jobs[id]?.cancelAndJoin()
        val current = d.store.transfer(id) ?: return
        if (TransferStatus.isFinished(current.status)) return
        d.log.i("transfer", "$id cancelled: ${current.peerName} is not paired any more")
        if (!d.store.setStatus(id, TransferStatus.CANCELED, UNPAIRED, "${current.peerName} was unpaired.", d.nowMillis())) return
        runCatching { if (current.direction == TransferDirection.DOWN) DownloadRunner.discardPartials(d, id) else d.sources.deleteCache(current.localId) }
        d.notifier.clear(id)
    }

    /**
     * Run once when the app starts: deletes what a crash left behind that no transfer owns. Pending MediaStore entries that no unfinished
     * download has a row for, and cached copies of transfers that are over (copies younger than ten minutes are left: a share is being
     * set up). Holds the download lock, so it can't meet a download that has just made an entry and not yet stored it.
     */
    suspend fun sweepOrphans() {
        val removedEntries = downloadLock.withLock {
            val owned = d.store.ownedPartialUris()
            val orphans = d.sink.pendingEntries().filter { it !in owned }
            orphans.forEach { runCatching { d.sink.delete(it) } }
            orphans.size
        }
        // Cached copies are keyed by the transfer's stable local ID, which survives a new transfer ID.
        val removedCaches = d.sources.deleteCacheExcept(d.store.liveLocalIds(), CACHE_SWEEP_MIN_AGE_MILLIS)
        if (removedEntries > 0 || removedCaches > 0) {
            d.log.w("transfer", "start-up sweep: deleted $removedEntries unfinished entries and $removedCaches cached copies that no transfer owned")
        }
    }

    /** Whether the transfer has a job running right now. */
    fun isRunning(transferId: String): Boolean = jobs[resolve(transferId)]?.isActive == true

    /** One line per recent transfer, for the debug state dump. */
    suspend fun summary(limit: Int = 10): List<String> = d.store.recent(limit).map { t ->
        val files = d.store.files(t.id)
        val done = files.sumOf { if (it.finished) it.size else it.bytesDone }
        "${t.id} ${t.direction} ${t.status} ${done}/${t.totalBytes} files=${files.size} " +
            "names=${files.joinToString("|") { it.finalName ?: it.name }} err=${t.errorCode} retriedAs=${t.retriedAs} " +
            "userRetry=${t.userRetry} started=${t.started} owed=${t.owed} completeAcked=${t.completeAcked}"
    }

    private fun resolve(id: String): String {
        var current = id
        while (true) current = renamed[current] ?: return current
    }

    private fun launch(id: String, lock: Mutex, block: suspend () -> RunResult) {
        synchronized(jobs) {
            if (jobs[id]?.isActive == true) return
            val job = scope.launch(Dispatchers.IO, start = CoroutineStart.LAZY) {
                try {
                    lock.withLock { block() }
                } catch (e: CancellationException) {
                    // Stopped on purpose (cancel, unpair) or the app is going down: whoever cancelled decides what to clean up.
                    throw e
                } catch (e: Throwable) {
                    d.log.e("transfer", "$id crashed: ${e.javaClass.simpleName}: ${e.message}")
                    failCrashed(resolve(id))
                    if (e is Error) throw e
                } finally {
                    jobs.remove(id)
                    jobs.remove(resolve(id))
                    renamed.remove(id)
                }
            }
            jobs[id] = job
            job.start()
        }
    }

    /**
     * A runner ended with an exception it didn't expect. The transfer fails with `internal`; a download deletes its partial files and
     * tells the PC (`cancel`, reason `error`, as for any file that can't be saved). An upload keeps its cached copies so Retry still
     * works, like every other failed upload. Every step is tried on its own: one that fails must not stop the others.
     */
    private suspend fun failCrashed(id: String) = withContext(NonCancellable) {
        val message = "Something went wrong inside Kin."
        val transfer = runCatching { d.store.transfer(id) }.getOrNull() ?: return@withContext
        if (TransferStatus.isFinished(transfer.status)) return@withContext
        if (transfer.direction == TransferDirection.DOWN) runCatching { DownloadRunner.discardPartials(d, id) }
        val down = transfer.direction == TransferDirection.DOWN
        val owed = if (down) OwedMessages.cancel("error") else null
        val owedId = if (down) OwedMessages.newMessageId() else null
        val changed = runCatching { d.store.setStatus(id, TransferStatus.FAILED, "internal", message, d.nowMillis(), owed, owedId) }.getOrDefault(false)
        if (!changed) return@withContext
        runCatching { d.notifier.failed(transfer.copy(status = TransferStatus.FAILED), message) }
        if (down) {
            runCatching {
                val connection = withTimeoutOrNull(timings.connectWaitMillis + timings.ackTimeoutMillis) {
                    d.connectionFor(transfer, timings.connectWaitMillis, kick = false)
                }
                owedMessages.send(connection, id)
            }
        }
    }

    companion object {
        const val MAX_FILES = 1000

        /** Files in one `offer`; more is refused with `too_large`. The same limit as the PC's own offers (protocol.md). */
        const val MAX_OFFER_FILES = app.kinrelay.android.protocol.MAX_OFFER_FILES

        /** The error code of a transfer cancelled because its PC was unpaired. */
        const val UNPAIRED = "unpaired"

        /** The error code of a queued item that waited longer than the queue keeps it ([app.kinrelay.android.data.QueuePolicy.EXPIRY_MILLIS]). */
        const val EXPIRED = "expired"

        private const val CACHE_SWEEP_MIN_AGE_MILLIS = 10 * 60_000L

        /** An accepted offer claims its bytes for this long, while its rows are being stored. */
        private const val RESERVATION_MILLIS = 60_000L

        private const val DELETE_ATTEMPTS = 3
        private const val START_POLL_MILLIS = 150L

        /** The name of the `.txt` file a long text becomes, without characters a name can't have. */
        fun textFileName(nowMillis: Long): String {
            val stamp = java.text.SimpleDateFormat("yyyy-MM-dd HH.mm.ss", java.util.Locale.ROOT).format(java.util.Date(nowMillis))
            return "Kin text $stamp.txt"
        }

        /** Over the threshold: sent as a file. */
        fun isLongText(wireText: String): Boolean = Utf8.size(wireText) > app.kinrelay.android.protocol.MAX_TEXT_BYTES
    }
}
