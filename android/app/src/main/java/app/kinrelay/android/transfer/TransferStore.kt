package app.kinrelay.android.transfer

import app.kinrelay.android.data.OutcomeRow
import app.kinrelay.android.data.PendingFile
import app.kinrelay.android.data.TransferDao
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import java.util.concurrent.ConcurrentHashMap

/** Where transfers are kept. The real one is Room ([RoomTransferStore]); the unit tests use an in-memory one. */
interface TransferStore {
    /** Stores the transfer and its files, both or neither. */
    suspend fun create(transfer: TransferEntity, files: List<TransferFileEntity>)
    suspend fun transfer(id: String): TransferEntity?
    suspend fun files(id: String): List<TransferFileEntity>

    /** Replaces one file's row (it is found by transfer ID and file ID). */
    suspend fun updateFile(file: TransferFileEntity)
    suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long)

    /**
     * Sets the status of a transfer that is **not over yet** (`queued` or `active`); a transfer that is `done`, `failed` or `canceled` is
     * left alone, so a late `cancel` or a racing failure can never rewrite it. Returns whether the row changed.
     */
    suspend fun setStatus(
        id: String,
        status: String,
        errorCode: String? = null,
        message: String? = null,
        finishedAt: Long? = null,
        /** A download that ends owes the PC this message (`complete`, or `cancel:<reason>`) under [owedId], in the same write as its status. */
        owed: String? = null,
        owedId: String? = null,
    ): Boolean

    /**
     * Retry: `queued` again, from `failed` (or from `queued`). A transfer that is done, canceled or already retried stays. Returns whether
     * it changed. [userRetry] marks the next run as the user's, until it reaches its `POST` ([clearUserRetry]). The creation time stays.
     */
    suspend fun requeue(id: String, userRetry: Boolean = false): Boolean
    suspend fun clearUserRetry(id: String)

    /** The PC has answered a `POST` for this upload: it is started (the 7 days of the queue no longer apply to it). */
    suspend fun markStarted(id: String)

    /** The downloads of the PC [peerDeviceId] that owe it a `complete` or `cancel`, oldest first. */
    suspend fun owing(peerDeviceId: String): List<TransferEntity>

    /** The owed message was answered (acked, or refused with an `error`: it is not sent again). A `complete` also becomes `completeAcked`. */
    suspend fun clearOwed(id: String)

    /** The PC's own `cancel` for a transfer that failed with `not_found`: it becomes canceled (`peer_canceled`). Never changes anything else. */
    suspend fun markCanceledByPeer(id: String, message: String): Boolean
    suspend fun setCompleteAcked(id: String, acked: Boolean)
    suspend fun setTotalBytes(id: String, totalBytes: Long)

    /** Counts one more try to send the transfer. */
    suspend fun addAttempt(id: String)

    /** Gives a transfer (and its files) another ID, in one transaction: the phone starts over with a new one when a shared file changed. */
    suspend fun rename(oldId: String, newId: String)

    /**
     * A user Retry found the old transfer ID used up: the old transfer ends as failed (`not_found`, [message]), [transfer] (the files that
     * never got a `200`) is stored and the old one points at it, in one transaction.
     */
    suspend fun supersede(oldId: String, transfer: TransferEntity, files: List<TransferFileEntity>, message: String, finishedAt: Long)

    /** Transfers that are `queued` or `active`, oldest first. */
    suspend fun unfinished(): List<TransferEntity>
    suspend fun recent(limit: Int): List<TransferEntity>

    /** Names that unfinished downloads have already claimed (they count as existing for the next name). */
    suspend fun claimedNames(): Set<String>

    /** The files unfinished downloads still have to write, as the store knows them (the free-space check corrects them with [LiveProgress]). */
    suspend fun pendingDownloadFiles(): List<PendingFile>

    /** The entries (URIs) that unfinished downloads own. */
    suspend fun ownedPartialUris(): Set<String>

    /** Cache keys ([TransferEntity.localId]) of the transfers whose cached copies must be kept: queued, active, or failed and not retried. */
    suspend fun liveLocalIds(): Set<String>

    /** How the outgoing transfers ended (for the first-try success rate). */
    suspend fun outgoingOutcomes(): List<OutcomeRow>

    /** A transfer and its files, in one transaction. */
    suspend fun delete(id: String)

    /**
     * The history shows the newest [keep] transfers: the file rows of older finished ones are dropped. **The transfer rows stay for ever**:
     * a transfer ID is never used twice, so it has to be remembered. A transfer that still owes the PC a message keeps its file rows.
     */
    suspend fun trim(keep: Int)
}

class RoomTransferStore(private val dao: TransferDao) : TransferStore {
    override suspend fun create(transfer: TransferEntity, files: List<TransferFileEntity>) = dao.insertWithFiles(transfer, files)

    override suspend fun transfer(id: String) = dao.get(id)
    override suspend fun files(id: String) = dao.files(id)
    override suspend fun updateFile(file: TransferFileEntity) = dao.insertFiles(listOf(file))
    override suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long) = dao.setBytesDone(id, fileId, bytesDone)

    override suspend fun setStatus(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long?, owed: String?, owedId: String?): Boolean =
        dao.setStatus(id, status, errorCode, message, finishedAt, owed, owedId) > 0

    override suspend fun requeue(id: String, userRetry: Boolean): Boolean = dao.requeue(id, userRetry) > 0
    override suspend fun clearUserRetry(id: String) = dao.clearUserRetry(id)
    override suspend fun markStarted(id: String) = dao.markStarted(id)
    override suspend fun owing(peerDeviceId: String) = dao.owing(peerDeviceId)
    override suspend fun clearOwed(id: String) = dao.clearOwed(id)
    override suspend fun markCanceledByPeer(id: String, message: String): Boolean = dao.markCanceledByPeer(id, message) > 0
    override suspend fun setCompleteAcked(id: String, acked: Boolean) = dao.setCompleteAcked(id, acked)
    override suspend fun setTotalBytes(id: String, totalBytes: Long) = dao.setTotalBytes(id, totalBytes)
    override suspend fun addAttempt(id: String) = dao.addAttempt(id)

    override suspend fun rename(oldId: String, newId: String) = dao.rename(oldId, newId)
    override suspend fun supersede(oldId: String, transfer: TransferEntity, files: List<TransferFileEntity>, message: String, finishedAt: Long) =
        dao.supersede(oldId, transfer, files, message, finishedAt)

    override suspend fun unfinished() = dao.unfinished()
    override suspend fun recent(limit: Int) = dao.recent(limit)
    override suspend fun claimedNames() = dao.claimedNames().toSet()
    override suspend fun pendingDownloadFiles() = dao.pendingDownloadFiles()
    override suspend fun ownedPartialUris() = dao.ownedPartialUris().toSet()
    override suspend fun liveLocalIds() = dao.liveLocalIds().toSet()
    override suspend fun outgoingOutcomes() = dao.outgoingOutcomes()

    override suspend fun delete(id: String) = dao.deleteWithFiles(id)
    override suspend fun trim(keep: Int) = dao.trim(keep)
}

/**
 * The bytes a download has really written, as it writes them. The store's `bytesDone` is only saved when a download stops, but the free
 * space of the phone already shrinks with every byte written, so a free-space check that took the stored number would count those bytes
 * twice and refuse an `offer` that fits (the 1.3 review).
 */
class LiveProgress {
    private val written = ConcurrentHashMap<String, Long>()

    private fun key(transferId: String, fileId: String) = "$transferId/$fileId"

    /** [bytes] of the file are in its entry now. */
    fun set(transferId: String, fileId: String, bytes: Long) {
        written[key(transferId, fileId)] = bytes
    }

    fun clear(transferId: String) {
        written.keys.removeAll { it.startsWith("$transferId/") }
    }

    fun get(transferId: String, fileId: String): Long? = written[key(transferId, fileId)]

    /** What [files] still have to write: the live number where there is one, the stored one otherwise. Null if the sum overflows. */
    fun stillToWrite(files: List<PendingFile>): Long? {
        var sum = 0L
        for (f in files) {
            val have = written[key(f.transferId, f.fileId)] ?: f.bytesDone
            val left = maxOf(f.size - have, 0L)
            sum = try {
                Math.addExact(sum, left)
            } catch (_: ArithmeticException) {
                return null
            }
        }
        return sum
    }
}
