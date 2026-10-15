package app.kinrelay.android.testing

import app.kinrelay.android.data.OutcomeRow
import app.kinrelay.android.data.TextItemEntity
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.link.Alerts
import app.kinrelay.android.link.QueuedUpload
import app.kinrelay.android.link.TextStore
import app.kinrelay.android.link.UploadQueue
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList

/** The text store in memory, with the same rules as the Room queries (claim, cancel only while queued, a sensitive body dropped when over). */
class InMemoryTextStore : TextStore {
    val items = ConcurrentHashMap<String, TextItemEntity>()
    private val lock = Any()

    override suspend fun insert(item: TextItemEntity) { items[item.id] = item }
    override suspend fun get(id: String) = items[id]

    override suspend fun queued() = items.values.filter { it.direction == "up" && it.status == TransferStatus.QUEUED }.sortedWith(compareBy({ it.createdAtMillis }, { it.id }))
    override suspend fun recent(limit: Int) = items.values.sortedByDescending { it.createdAtMillis }.take(limit)

    override suspend fun claim(id: String): Boolean = synchronized(lock) {
        val t = items[id] ?: return false
        if (t.status != TransferStatus.QUEUED) return false
        items[id] = t.copy(status = TransferStatus.ACTIVE, attempts = t.attempts + 1)
        true
    }

    override suspend fun release(id: String): Boolean = synchronized(lock) {
        val t = items[id] ?: return false
        if (t.status != TransferStatus.ACTIVE) return false
        items[id] = t.copy(status = TransferStatus.QUEUED)
        true
    }

    override suspend fun finish(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long): Boolean = synchronized(lock) {
        val t = items[id] ?: return false
        if (t.status != TransferStatus.QUEUED && t.status != TransferStatus.ACTIVE) return false
        items[id] = t.copy(status = status, errorCode = errorCode, message = message, finishedAtMillis = finishedAt, body = if (t.sensitive) null else t.body)
        true
    }

    override suspend fun cancelQueued(id: String, finishedAt: Long): Boolean = synchronized(lock) {
        val t = items[id] ?: return false
        if (t.status != TransferStatus.QUEUED) return false
        items[id] = t.copy(status = TransferStatus.CANCELED, finishedAtMillis = finishedAt, errorCode = "user", message = "Canceled.", body = if (t.sensitive) null else t.body)
        true
    }

    override suspend fun cancelQueuedFor(peerDeviceId: String, now: Long): Int = synchronized(lock) {
        var n = 0
        for (t in items.values.toList()) {
            if (t.direction == "up" && t.peerDeviceId == peerDeviceId && (t.status == TransferStatus.QUEUED || t.status == TransferStatus.ACTIVE)) {
                items[t.id] = t.copy(status = TransferStatus.CANCELED, finishedAtMillis = now, errorCode = "unpaired", body = if (t.sensitive) null else t.body)
                n++
            }
        }
        n
    }

    override suspend fun retryAs(oldId: String, newId: String, now: Long): Boolean = synchronized(lock) {
        val t = items[oldId] ?: return false
        if (t.status != TransferStatus.FAILED || t.body == null || t.retriedAs != null) return false
        // A new row; the failed one stays and points at it.
        items[oldId] = t.copy(retriedAs = newId)
        items[newId] = t.copy(id = newId, status = TransferStatus.QUEUED, createdAtMillis = now, finishedAtMillis = null, errorCode = null, message = null, retriedAs = null)
        true
    }

    override suspend fun recoverActive(): Int = synchronized(lock) {
        var n = 0
        for (t in items.values.toList()) if (t.direction == "up" && t.status == TransferStatus.ACTIVE) { items[t.id] = t.copy(status = TransferStatus.QUEUED); n++ }
        n
    }

    override suspend fun outgoingOutcomes() =
        items.values.filter { it.direction == "up" && (it.status == "done" || it.status == "failed") && it.retriedAs == null }.map { OutcomeRow(it.status, it.errorCode, it.attempts) }

    /** The same rule as the Room query: the bodies of finished texts beyond the newest [keep] go, the rows stay. */
    override suspend fun trim(keep: Int) {
        val newest = items.values.sortedByDescending { it.createdAtMillis }.take(keep).map { it.id }.toSet()
        items.values.filter { TransferStatus.isFinished(it.status) && it.id !in newest && it.body != null }.forEach { items[it.id] = it.copy(body = null) }
    }
}

/** Records what the user would have been told. */
class RecordingAlerts : Alerts {
    class TextAlert(val pc: String, val id: String, val text: String?, val openUrl: String?)

    val texts = java.util.concurrent.LinkedBlockingQueue<TextAlert>()
    val sent = CopyOnWriteArrayList<Pair<String?, Long>>()
    val queued = CopyOnWriteArrayList<Triple<String?, String, Boolean>>()
    val sentLater = CopyOnWriteArrayList<String?>()
    val failures = CopyOnWriteArrayList<Pair<String, String>>()
    val failureTitles = java.util.concurrent.LinkedBlockingQueue<String>()
    val messages = CopyOnWriteArrayList<String>()

    override fun textFrom(pcName: String, messageId: String, text: String?, openUrl: String?) { texts.add(TextAlert(pcName, messageId, text, openUrl)) }
    override fun sent(pcName: String?, arrivalMillis: Long) { sent += pcName to arrivalMillis }
    override fun queued(pcName: String?, reason: String, afterNoReply: Boolean) { queued += Triple(pcName, reason, afterNoReply) }
    override fun sentLater(pcName: String?) { sentLater += pcName }
    override fun sendFailed(title: String, reason: String) { failures += title to reason; failureTitles += title }
    override fun message(text: String) { messages += text }
}

/** Queued uploads for the flusher: a list a test fills, and a record of what was started. */
class FakeUploadQueue : UploadQueue {
    val waiting = CopyOnWriteArrayList<QueuedUpload>()
    val started = CopyOnWriteArrayList<String>()
    val expiredCalls = CopyOnWriteArrayList<Long>()

    /** Called when the flusher starts an upload (to see its place among the texts). */
    @Volatile var onStart: ((String) -> Unit)? = null

    override suspend fun queued() = waiting.sortedBy { it.createdAtMillis }
    override fun start(id: String) { started += id; onStart?.invoke(id); waiting.removeAll { it.id == id } }
    override suspend fun expire(maxAgeMillis: Long): Int { expiredCalls += maxAgeMillis; return 0 }
}
