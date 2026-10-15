package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDao
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity

/** Where transfers are kept. The real one is Room ([RoomTransferStore]); the unit tests use an in-memory one. */
interface TransferStore {
    /** Stores the transfer and its files, both or neither. */
    suspend fun create(transfer: TransferEntity, files: List<TransferFileEntity>)
    suspend fun transfer(id: String): TransferEntity?
    suspend fun files(id: String): List<TransferFileEntity>

    /** Replaces one file's row (it is found by transfer ID and file ID). */
    suspend fun updateFile(file: TransferFileEntity)
    suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long)
    suspend fun setStatus(id: String, status: String, errorCode: String? = null, message: String? = null, finishedAt: Long? = null)
    suspend fun setCompleteAcked(id: String, acked: Boolean)
    suspend fun setTotalBytes(id: String, totalBytes: Long)

    /** Gives a transfer (and its files) another ID: the phone starts over with a new one when a shared file changed. */
    suspend fun rename(oldId: String, newId: String)

    /** Transfers that are `queued` or `active`, oldest first. */
    suspend fun unfinished(): List<TransferEntity>
    suspend fun recent(limit: Int): List<TransferEntity>

    /** Names that unfinished downloads have already claimed (they count as existing for the next name). */
    suspend fun claimedNames(): Set<String>

    /** Bytes that unfinished downloads still have to write. */
    suspend fun pendingDownloadBytes(): Long

    /** The entries (URIs) that unfinished downloads own. */
    suspend fun ownedPartialUris(): Set<String>

    /** IDs of the transfers whose cached copies must be kept: queued, active, or failed (a failed upload can be retried). */
    suspend fun liveIds(): Set<String>
    suspend fun delete(id: String)
}

class RoomTransferStore(private val dao: TransferDao) : TransferStore {
    override suspend fun create(transfer: TransferEntity, files: List<TransferFileEntity>) = dao.insertWithFiles(transfer, files)

    override suspend fun transfer(id: String) = dao.get(id)
    override suspend fun files(id: String) = dao.files(id)
    override suspend fun updateFile(file: TransferFileEntity) = dao.insertFiles(listOf(file))
    override suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long) = dao.setBytesDone(id, fileId, bytesDone)

    override suspend fun setStatus(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long?) =
        dao.setStatus(id, status, errorCode, message, finishedAt)

    override suspend fun setCompleteAcked(id: String, acked: Boolean) = dao.setCompleteAcked(id, acked)
    override suspend fun setTotalBytes(id: String, totalBytes: Long) = dao.setTotalBytes(id, totalBytes)

    override suspend fun rename(oldId: String, newId: String) {
        dao.renameTransfer(oldId, newId)
        dao.renameTransferFiles(oldId, newId)
    }

    override suspend fun unfinished() = dao.unfinished()
    override suspend fun recent(limit: Int) = dao.recent(limit)
    override suspend fun claimedNames() = dao.claimedNames().toSet()
    override suspend fun pendingDownloadBytes() = dao.pendingDownloadBytes()
    override suspend fun ownedPartialUris() = dao.ownedPartialUris().toSet()
    override suspend fun liveIds() = dao.liveIds().toSet()

    override suspend fun delete(id: String) {
        dao.deleteFiles(id)
        dao.delete(id)
    }
}
