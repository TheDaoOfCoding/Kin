package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.Offer
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
data class ShareSource(val uri: String, val nameHint: String? = null, val mimeHint: String? = null)

/**
 * Owns the transfers: it creates them (a share, a `.txt` for long text, an `offer`), runs them one at a time per direction on the
 * app's scope, and handles cancel, retry and recovery. Nothing here blocks the WebSocket: an `offer` is acked first, and the
 * download runs on its own coroutine.
 *
 * The persistent queue of 1.4 builds on this: a transfer that couldn't start is `queued` with its small files already copied into the
 * cache ([TransferStatus.QUEUED]); [retry] and [retryQueuedUploads] start it, and 1.4 only has to call the second one when the PC
 * comes back.
 */
class TransferEngine(
    private val scope: CoroutineScope,
    private val d: TransferDeps,
    private val timings: TransferTimings = TransferTimings(),
    private val mimeForExtension: (String) -> String? = { null },
    private val httpFor: (app.kinrelay.android.link.HttpEndpoint) -> TransferHttp = ::TransferHttp,
) {
    private val uploadLock = Mutex()
    private val downloadLock = Mutex()
    private val jobs = ConcurrentHashMap<String, Job>()

    /** Old transfer ID to new, for a transfer that started over after a shared file changed. */
    private val renamed = ConcurrentHashMap<String, String>()

    // ---------------------------------------------------------------- phone -> PC

    /**
     * Creates an upload for [sources] and starts it. Returns the transfer ID, or null when nothing could be sent (no paired PC,
     * no readable file, too many files). The user is told in the null cases.
     */
    suspend fun enqueueUpload(sources: List<ShareSource>): String? {
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
        for (source in sources) {
            val fileId = "f${files.size + 1}"
            var uri = source.uri
            var info = d.sources.stat(uri)
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
            val name = (info.displayName ?: source.nameHint)?.takeIf { it.isNotBlank() } ?: fallbackName(source.uri)
            val mime = (info.mime ?: source.mimeHint)?.takeIf { it.isNotBlank() && !it.contains('*') } ?: MediaKinds.DEFAULT_MIME
            files += TransferFileEntity(id, fileId, files.size, name, null, mime, info.size, 0, uri, info.modifiedMillis, finished = false)
        }
        if (files.isEmpty()) {
            d.notifier.message("Kin couldn't read what you shared.")
            return null
        }
        createUpload(id, peer, files)
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

    private suspend fun createUpload(id: String, peer: PeerInfo, files: List<TransferFileEntity>) {
        val transfer = TransferEntity(
            id, TransferDirection.UP, peer.deviceId, peer.name, TransferKind.FILE, TransferStatus.QUEUED,
            d.nowMillis(), null, files.sumOf { it.size }, null, null, completeAcked = false,
        )
        d.store.create(transfer, files)
        d.log.i("transfer", "upload $id created: ${files.size} files, ${transfer.totalBytes} bytes")
        launchUpload(id)
    }

    private fun fallbackName(uri: String): String = uri.substringBefore('?').substringAfterLast('/').ifBlank { "file" }

    private fun launchUpload(id: String) = launch(id, uploadLock) {
        UploadRunner(d, timings, httpFor, onRenamed = { old, new -> rename(old, new) }).run(id)
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
     * - `bad_request`: a negative size, or a transfer ID that is already used by something else (an upload, a transfer of another PC, a
     *   finished one). A transfer ID is never reused; only a re-offer of an unfinished download from the same PC resumes it;
     * - `disk_full`: the files don't fit in the free space once what the unfinished downloads still have to write, and what offers
     *   accepted a moment ago have claimed, is taken away.
     *
     * It runs on the socket's thread, not the main thread: the store lookups are short and are awaited here.
     */
    fun acceptOffer(offer: Offer): String? = runBlocking { gate(offer) }

    private suspend fun gate(offer: Offer): String? {
        if (offer.files.size > MAX_OFFER_FILES) return "too_large"
        if (offer.files.any { it.size < 0 }) return "bad_request"
        var needed = 0L
        for (file in offer.files) needed = addOrNull(needed, file.size) ?: return "too_large"
        val existing = d.store.transfer(offer.id)
        if (existing != null) {
            val peer = d.link.peer() ?: return "bad_request"
            // The same PC re-offers a download that isn't finished: it resumes with what is stored, so there is nothing new to fit.
            return if (canResume(existing, peer)) null else "bad_request"
        }
        val claimed = synchronized(reservations) {
            val now = d.nowMillis()
            reservations.values.removeAll { now - it.atMillis > RESERVATION_MILLIS }
            reservations.values.fold(0L) { sum, r -> addOrNull(sum, r.bytes) ?: Long.MAX_VALUE }
        }
        val stillToWrite = addOrNull(d.store.pendingDownloadBytes(), claimed) ?: return "too_large"
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

    /** Only an unfinished download from the PC that is paired now can be started again by an `offer` with its ID. */
    private fun canResume(existing: TransferEntity, peer: PeerInfo): Boolean =
        existing.direction == TransferDirection.DOWN && existing.peerDeviceId == peer.deviceId && !TransferStatus.isFinished(existing.status)

    /** A valid `offer` that was acked: remember it and download it. Returns at once; the download runs on its own coroutine. */
    suspend fun onOffer(peerName: String, offer: Offer) {
        try {
            val peer = d.link.peer()
            if (peer == null) {
                d.log.w("transfer", "offer ${offer.id} from '$peerName' ignored: no PC is paired")
                return
            }
            val existing = d.store.transfer(offer.id)
            if (existing != null) {
                if (canResume(existing, peer)) {
                    d.log.i("transfer", "offer ${offer.id} is already known; not starting it twice")
                    launchDownload(offer.id)
                } else {
                    // A transfer ID is never reused (protocol.md). The rows are not touched: they may be an upload of ours.
                    d.log.w("transfer", "offer ${offer.id} reuses the ID of another transfer; ignored")
                }
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

    private fun launchDownload(id: String) = launch(id, downloadLock) {
        DownloadRunner(d, timings, httpFor, mimeForExtension).run(id)
    }

    // ---------------------------------------------------------------- cancel, retry, recovery

    /** The user cancelled (the notification's Cancel). An upload sends `DELETE`; a download sends a `cancel` message. */
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
        d.store.setStatus(transfer.id, TransferStatus.CANCELED, "user", "Canceled.", d.nowMillis())
        d.notifier.clear(transfer.id)
        d.sources.deleteCache(transfer.id)
        // The PC drops its partial files. A retried DELETE that gets 404 counts as done (the transfer is already gone).
        repeat(DELETE_ATTEMPTS) { attempt ->
            val endpoint = d.link.connection(if (attempt == 0) timings.connectWaitMillis else 1_000, kick = attempt == 0)?.httpEndpoint
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
        DownloadRunner.discardPartials(d, transfer.id)
        d.store.setStatus(transfer.id, TransferStatus.CANCELED, "user", "Canceled.", d.nowMillis())
        d.notifier.clear(transfer.id)
        val messageId = MessageIds.newId()
        val connection = d.link.connection(timings.connectWaitMillis, kick = false)
        connection?.sendAcked(messageId, Frames.cancel(messageId, transfer.id, "user"), timings.ackTimeoutMillis)
    }

    /** The PC sent `cancel`. Unknown or finished transfers are ignored (the message was acked already). */
    fun onPeerCancel(transferId: String, reason: String) {
        scope.launch(Dispatchers.IO) {
            val id = resolve(transferId)
            val transfer = d.store.transfer(id) ?: return@launch
            if (TransferStatus.isFinished(transfer.status)) return@launch
            jobs[id]?.cancelAndJoin()
            d.log.i("transfer", "$id cancelled by the PC (reason '$reason')")
            if (transfer.direction == TransferDirection.DOWN) DownloadRunner.discardPartials(d, id) else d.sources.deleteCache(id)
            d.store.setStatus(id, TransferStatus.CANCELED, "peer_canceled", "${transfer.peerName} canceled the transfer.", d.nowMillis())
            d.notifier.clear(id)
            d.notifier.failed(transfer.copy(status = TransferStatus.CANCELED), "${transfer.peerName} canceled the transfer.")
        }
    }

    /** Starts a transfer again: a queued upload (the Retry action), or a failed one. A download can only be resumed while it is queued. */
    fun retry(transferId: String) {
        scope.launch(Dispatchers.IO) {
            val id = resolve(transferId)
            val transfer = d.store.transfer(id) ?: return@launch
            val retryable = transfer.status == TransferStatus.QUEUED ||
                (transfer.direction == TransferDirection.UP && transfer.status == TransferStatus.FAILED)
            if (!retryable) return@launch
            d.store.setStatus(id, TransferStatus.QUEUED)
            if (transfer.direction == TransferDirection.UP) launchUpload(id) else launchDownload(id)
        }
    }

    /** Starts every queued upload, oldest first. The persistent queue (1.4) calls this when the PC comes back. */
    suspend fun retryQueuedUploads() {
        for (transfer in d.store.unfinished()) {
            if (transfer.direction == TransferDirection.UP && transfer.status == TransferStatus.QUEUED) launchUpload(transfer.id)
        }
    }

    /**
     * The link is up (or the service just started): bring back what an earlier run left behind. An `active` upload resumes (its
     * source access is checked first), and a download resumes whether it is `active` or waiting for the PC. Queued uploads stay
     * queued until [retry]. Safe to call at every connect.
     */
    suspend fun onLinkReady() {
        dropTransfersOfOtherPcs()
        for (transfer in d.store.unfinished()) {
            if (jobs[transfer.id]?.isActive == true) continue
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
        runCatching { if (current.direction == TransferDirection.DOWN) DownloadRunner.discardPartials(d, id) else d.sources.deleteCache(id) }
        d.store.setStatus(id, TransferStatus.CANCELED, UNPAIRED, "${current.peerName} was unpaired.", d.nowMillis())
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
        val removedCaches = d.sources.deleteCacheExcept(d.store.liveIds(), CACHE_SWEEP_MIN_AGE_MILLIS)
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
            "names=${files.joinToString("|") { it.finalName ?: it.name }} err=${t.errorCode}"
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
        runCatching { d.store.setStatus(id, TransferStatus.FAILED, "internal", message, d.nowMillis()) }
        runCatching { d.notifier.failed(transfer.copy(status = TransferStatus.FAILED), message) }
        if (transfer.direction == TransferDirection.DOWN) {
            runCatching {
                val messageId = MessageIds.newId()
                val connection = withTimeoutOrNull(timings.connectWaitMillis + timings.ackTimeoutMillis) {
                    d.link.connection(timings.connectWaitMillis, kick = false)
                }
                connection?.sendAcked(messageId, Frames.cancel(messageId, id, "error"), timings.ackTimeoutMillis)
            }
        }
    }

    companion object {
        const val MAX_FILES = 1000

        /** Files in one `offer`; more is refused with `too_large`. The same limit as the PC's own offers (protocol.md). */
        const val MAX_OFFER_FILES = app.kinrelay.android.protocol.MAX_OFFER_FILES

        /** The error code of a transfer cancelled because its PC was unpaired. */
        const val UNPAIRED = "unpaired"

        private const val CACHE_SWEEP_MIN_AGE_MILLIS = 10 * 60_000L

        /** An accepted offer claims its bytes for this long, while its rows are being stored. */
        private const val RESERVATION_MILLIS = 60_000L

        private const val DELETE_ATTEMPTS = 3

        /** The name of the `.txt` file a long text becomes, without characters a name can't have. */
        fun textFileName(nowMillis: Long): String {
            val stamp = java.text.SimpleDateFormat("yyyy-MM-dd HH.mm.ss", java.util.Locale.ROOT).format(java.util.Date(nowMillis))
            return "Kin text $stamp.txt"
        }

        /** Over the threshold: sent as a file. */
        fun isLongText(wireText: String): Boolean = Utf8.size(wireText) > app.kinrelay.android.protocol.MAX_TEXT_BYTES
    }
}
