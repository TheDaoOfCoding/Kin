package app.kinrelay.android.link

import app.kinrelay.android.data.OutcomeRow
import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.data.TextDao
import app.kinrelay.android.data.TextItemEntity
import app.kinrelay.android.data.TextSource
import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.Utf8
import app.kinrelay.android.transfer.PeerInfo
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import java.util.concurrent.ConcurrentHashMap

/** Where the texts of the queue and the history are kept. The real one is Room ([RoomTextStore]); the unit tests use an in-memory one. */
interface TextStore {
    suspend fun insert(item: TextItemEntity)
    suspend fun get(id: String): TextItemEntity?

    /** Queued outgoing texts in creation order. */
    suspend fun queued(): List<TextItemEntity>
    suspend fun recent(limit: Int): List<TextItemEntity>

    /** `queued` to `active` (and one more attempt) for the caller that gets true; false when it was canceled, or is already being sent. */
    suspend fun claim(id: String): Boolean
    suspend fun release(id: String): Boolean
    suspend fun finish(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long): Boolean
    suspend fun cancelQueued(id: String, finishedAt: Long): Boolean
    suspend fun cancelQueuedFor(peerDeviceId: String, now: Long): Int

    /** A failed text goes out again as a new row with a new ID; the failed row stays and points at it (`retriedAs`). False if it can't be retried. */
    suspend fun retryAs(oldId: String, newId: String, now: Long): Boolean
    suspend fun recoverActive(): Int
    suspend fun outgoingOutcomes(): List<OutcomeRow>
    suspend fun trim(keep: Int)
}

class RoomTextStore(private val dao: TextDao) : TextStore {
    override suspend fun insert(item: TextItemEntity) = dao.insert(item)
    override suspend fun get(id: String) = dao.get(id)
    override suspend fun queued() = dao.queued()
    override suspend fun recent(limit: Int) = dao.recent(limit)
    override suspend fun claim(id: String) = dao.claim(id) > 0
    override suspend fun release(id: String) = dao.release(id) > 0
    override suspend fun finish(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long) =
        dao.finish(id, status, errorCode, message, finishedAt) > 0

    override suspend fun cancelQueued(id: String, finishedAt: Long) = dao.cancelQueued(id, finishedAt) > 0
    override suspend fun cancelQueuedFor(peerDeviceId: String, now: Long) = dao.cancelQueuedFor(peerDeviceId, now)
    override suspend fun retryAs(oldId: String, newId: String, now: Long) = dao.retryAs(oldId, newId, now)
    override suspend fun recoverActive() = dao.recoverActive()
    override suspend fun outgoingOutcomes() = dao.outgoingOutcomes()
    override suspend fun trim(keep: Int) = dao.trim(keep)
}

/** What one attempt to send a queued text came to. */
sealed interface TextSendResult {
    /** The PC acked it. [wasReportedQueued]: the user was told it was queued, so the user should be told it went out. */
    data class Sent(val wasReportedQueued: Boolean) : TextSendResult

    /** The PC answered with an `error`: the text failed and is not tried again. */
    data class Failed(val code: String) : TextSendResult

    /** No ack in time, or the connection went: the text is back in the queue, and goes out again with the same ID after the next `hello`. */
    data object Requeued : TextSendResult

    /** Canceled meanwhile, being sent by another caller, or meant for a PC this connection isn't to. */
    data object Skipped : TextSendResult
}

/**
 * The text queue (milestone 1.4, architecture.md "Queue and resume"). Every explicit text send (share, "Send to PC", the clipboard tile and
 * notification button) is stored here as `queued` **before** it is attempted, so it survives a dead connection and a killed process. After
 * the PC's `hello` the [OutboxFlusher] sends the queue in creation order, one message at a time, each waiting for its `ack` (5 s). An
 * unacked send is **re-queued, not failed** (it may or may not have arrived: the PC's de-duplication answers a repeated ID). A queued text
 * expires after [QueuePolicy.EXPIRY_MILLIS]; a user cancel of a queued text means it never goes out.
 *
 * The automatic clipboard sync is not an explicit send and is never queued. A sensitive text is kept only while it waits: its body is
 * dropped as soon as it is over, and history never shows it.
 */
class TextOutbox(
    private val store: TextStore,
    private val log: KinLog,
    private val nowMillis: () -> Long = System::currentTimeMillis,
    private val newId: () -> String = { MessageIds.newId() },
    private val ackTimeoutMillis: Long = OutgoingSender.ACK_TIMEOUT_MILLIS,
) {
    /** Texts the user was told were queued (so that the user is told when they finally go out). Only in memory: a restart forgets it. */
    private val reportedQueued: MutableSet<String> = ConcurrentHashMap.newKeySet()

    /** Stores the text as `queued` for [peer]. [text] is as the user had it: the wire form (LF only) is made when it is sent ([Frames.text]), once. */
    suspend fun enqueue(text: String, isUrl: Boolean, sensitive: Boolean, source: String, peer: PeerInfo): TextItemEntity {
        val item = TextItemEntity(
            id = newId(), direction = TransferDirection.UP, peerDeviceId = peer.deviceId, peerName = peer.name, status = TransferStatus.QUEUED,
            createdAtMillis = nowMillis(), finishedAtMillis = null, body = Frames.sanitize(text), byteSize = Utf8.size(Frames.wireText(text)).toLong(),
            sensitive = sensitive, isUrl = isUrl, source = source, errorCode = null, message = null, attempts = 0,
        )
        store.insert(item)
        store.trim(QueuePolicy.HISTORY_LIMIT)
        log.i("queue", "text ${item.id} queued (${item.source}${if (sensitive) ", sensitive" else ", ${item.byteSize} bytes"})")
        return item
    }

    suspend fun get(id: String): TextItemEntity? = store.get(id)

    suspend fun queued(): List<TextItemEntity> = store.queued()

    fun markReportedQueued(id: String) {
        reportedQueued += id
    }

    /**
     * Sends one queued text on [connection] and waits for its ack. The claim (`queued` to `active`) is what makes a cancel safe: a text that
     * the user canceled first is never claimed, so it never goes out.
     */
    suspend fun sendOne(connection: LinkConnection, item: TextItemEntity): TextSendResult {
        if (item.peerDeviceId != connection.peerDeviceId) return TextSendResult.Skipped
        if (!store.claim(item.id)) return TextSendResult.Skipped
        val body = item.body
        val frame = body?.let { Frames.text(item.id, it, item.isUrl, item.sensitive) }
        if (frame == null) {
            store.finish(item.id, TransferStatus.FAILED, "too_large", "That text can't be sent.", nowMillis())
            return TextSendResult.Failed("too_large")
        }
        // The text is `active` from the claim until one of the three outcomes is written. If this coroutine is cancelled in between (the
        // service's scope ends while the ack is awaited), the text must not stay `active` for ever: it goes back to the queue, and the
        // same ID goes out again later (the PC's memory of IDs makes a repeat harmless).
        var settled = false
        try {
            val outcome = when (val result = connection.sendAcked(item.id, frame, ackTimeoutMillis)) {
                AckResult.Acked -> {
                    store.finish(item.id, TransferStatus.DONE, null, null, nowMillis())
                    log.i("queue", "text ${item.id} delivered (attempt ${item.attempts + 1})")
                    TextSendResult.Sent(reportedQueued.remove(item.id))
                }
                is AckResult.Refused -> {
                    store.finish(item.id, TransferStatus.FAILED, result.code, "The PC answered with an error (${result.code}).", nowMillis())
                    log.w("queue", "text ${item.id} refused: ${result.code}")
                    TextSendResult.Failed(result.code)
                }
                AckResult.NoReply, AckResult.NoConnection -> {
                    store.release(item.id)
                    log.w("queue", "text ${item.id} not acked; back in the queue")
                    TextSendResult.Requeued
                }
            }
            settled = true
            return outcome
        } finally {
            if (!settled) {
                withContext(NonCancellable) { runCatching { store.release(item.id) } }
                log.w("queue", "text ${item.id}: the send was interrupted; back in the queue")
            }
        }
    }

    /** A user cancel. Only a text that still waits is canceled; one that is being sent right now is not (it may already be on the PC). */
    suspend fun cancel(id: String): Boolean = store.cancelQueued(id, nowMillis()).also { if (it) log.i("queue", "text $id canceled before it went out") }

    /** A user Retry of a failed text: it goes out again as a new message in a new row; the failed one stays in the history and points at it. */
    suspend fun retry(id: String): Boolean = store.retryAs(id, newId(), nowMillis())

    /** Keeps the database small: the bodies of finished texts beyond the history's 500 go. Called on every connect and once a day. */
    suspend fun prune() = store.trim(QueuePolicy.HISTORY_LIMIT)

    /**
     * Queued texts that waited [QueuePolicy.EXPIRY_MILLIS] or more fail with code `expired`. Returns how many. [aheadMillis] looks at the
     * queue as if the clock were that far ahead (the debug `EXPIRE` command; zero in the app).
     */
    suspend fun expire(aheadMillis: Long = 0): Int {
        val now = nowMillis()
        var expired = 0
        for (item in store.queued()) {
            if (QueuePolicy.isExpired(item.createdAtMillis, now + aheadMillis) &&
                store.finish(item.id, TransferStatus.FAILED, EXPIRED, "Expired: it waited in the queue for too long.", now)
            ) {
                log.w("queue", "text ${item.id} expired")
                expired++
            }
        }
        return expired
    }

    /** A queued text for a PC that is not paired any more fails: it must never go to another PC. */
    suspend fun failForPeerChange(item: TextItemEntity) {
        store.finish(item.id, TransferStatus.FAILED, "peer_changed", "It was meant for ${item.peerName}, which is no longer paired.", nowMillis())
    }

    suspend fun onUnpaired(peerDeviceId: String) {
        val n = store.cancelQueuedFor(peerDeviceId, nowMillis())
        if (n > 0) log.i("queue", "$n queued texts for an unpaired PC were canceled")
    }

    /** After a process start: nothing is being sent, so whatever was `active` is queued again. */
    suspend fun recoverAtStart() {
        val n = store.recoverActive()
        if (n > 0) log.i("queue", "$n texts that were being sent when the app stopped are queued again")
    }

    /**
     * A `text` the PC sent (explicit, from its hotkey) goes into the history of this phone. A sensitive text is never stored. (The automatic
     * `clipboard` message never gets here.)
     */
    suspend fun recordIncoming(peerDeviceId: String, peerName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean) {
        if (sensitive) return
        val now = nowMillis()
        store.insert(
            TextItemEntity(
                id = id, direction = TransferDirection.DOWN, peerDeviceId = peerDeviceId, peerName = peerName, status = TransferStatus.DONE,
                createdAtMillis = now, finishedAtMillis = now, body = text, byteSize = Utf8.size(text).toLong(), sensitive = false, isUrl = isUrl,
                source = TextSource.PC, errorCode = null, message = null, attempts = 0,
            )
        )
        store.trim(QueuePolicy.HISTORY_LIMIT)
    }

    companion object {
        const val EXPIRED = "expired"
    }
}
