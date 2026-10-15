package app.kinrelay.android.link

import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.data.TextItemEntity
import app.kinrelay.android.log.KinLog
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** An upload that waits in the queue. */
data class QueuedUpload(val id: String, val createdAtMillis: Long, val peerDeviceId: String)

/** The queued file uploads, as the flusher sees them (the transfer engine in the app). */
interface UploadQueue {
    /** Queued uploads, oldest first. */
    suspend fun queued(): List<QueuedUpload>

    /** Starts one (it runs on its own coroutine and returns at once). */
    fun start(id: String)

    /** Fails the queued uploads that waited [maxAgeMillis] or more (`expired`). */
    suspend fun expire(maxAgeMillis: Long): Int
}

/** What one flush did, for tests and the log. */
data class FlushReport(val texts: Int = 0, val uploads: Int = 0, val failed: Int = 0, val expired: Int = 0, val stoppedEarly: Boolean = false)

/**
 * Sends the queue when there is a connection: **after the PC's `hello`**, and also right after a new item was queued while connected.
 * Everything goes out in **creation order**, texts and uploads together:
 * - a text is sent and waits for its ack (5 s) before the next item; an unacked one is re-queued and the flush stops (the connection is
 *   treated as dead, and the next `hello` flushes again);
 * - an upload is only *started* (its bytes move on their own coroutine), so a big file doesn't hold back the texts after it.
 *
 * One flush at a time (a mutex), so the order can't be mixed up by a flush from the connect and one from a send at the same moment.
 * Expired items (older than [QueuePolicy.EXPIRY_MILLIS]) fail first, and never go out.
 */
class OutboxFlusher(
    private val texts: TextOutbox,
    private val uploads: UploadQueue,
    private val pairedPeerDeviceId: suspend () -> String?,
    private val alerts: Alerts,
    private val log: KinLog,
) {
    private val mutex = Mutex()

    private sealed interface Item {
        val createdAtMillis: Long
        val id: String

        class Text(val entity: TextItemEntity) : Item {
            override val createdAtMillis get() = entity.createdAtMillis
            override val id get() = entity.id
        }

        class Upload(val queued: QueuedUpload) : Item {
            override val createdAtMillis get() = queued.createdAtMillis
            override val id get() = queued.id
        }
    }

    /** Fails what expired, without needing a connection (the app start and the history screen call it). */
    suspend fun expire(aheadMillis: Long = 0): Int =
        mutex.withLock { texts.expire(aheadMillis) + uploads.expire((QueuePolicy.EXPIRY_MILLIS - aheadMillis).coerceAtLeast(0)) }

    suspend fun flush(connection: LinkConnection): FlushReport = mutex.withLock {
        var sentTexts = 0
        var started = 0
        var failed = 0
        val expired = texts.expire() + uploads.expire(QueuePolicy.EXPIRY_MILLIS)
        val items = (texts.queued().map { Item.Text(it) } + uploads.queued().map { Item.Upload(it) })
            .sortedWith(compareBy<Item> { it.createdAtMillis }.thenBy { it.id })
        val paired = pairedPeerDeviceId()
        for (item in items) {
            if (!connection.isReady) return@withLock FlushReport(sentTexts, started, failed, expired, stoppedEarly = true)
            when (item) {
                is Item.Upload -> if (item.queued.peerDeviceId == connection.peerDeviceId) {
                    uploads.start(item.id)
                    started++
                }
                is Item.Text -> {
                    val entity = item.entity
                    if (entity.peerDeviceId != connection.peerDeviceId) {
                        // Meant for another PC. If that PC is not the paired one it never gets it, and neither does this one.
                        if (paired != null && entity.peerDeviceId != paired) {
                            texts.failForPeerChange(entity)
                            failed++
                        }
                        continue
                    }
                    when (val result = texts.sendOne(connection, entity)) {
                        is TextSendResult.Sent -> {
                            sentTexts++
                            if (result.wasReportedQueued) alerts.sentLater(entity.peerName)
                        }
                        is TextSendResult.Failed -> failed++
                        TextSendResult.Requeued -> return@withLock FlushReport(sentTexts, started, failed, expired, stoppedEarly = true)
                        TextSendResult.Skipped -> Unit
                    }
                }
            }
        }
        if (sentTexts + started + failed + expired > 0) log.i("queue", "flush: $sentTexts texts sent, $started uploads started, $failed failed, $expired expired")
        FlushReport(sentTexts, started, failed, expired)
    }
}
