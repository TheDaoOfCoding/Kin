package app.kinrelay.android.data

import androidx.room3.Dao
import androidx.room3.Insert
import androidx.room3.OnConflictStrategy
import androidx.room3.Query
import androidx.room3.Transaction
import kotlinx.coroutines.flow.Flow

@Dao
interface DeviceDao {
    @Query("SELECT * FROM devices ORDER BY pairedAtMillis DESC")
    fun observeAll(): Flow<List<DeviceEntity>>

    @Query("SELECT * FROM devices ORDER BY pairedAtMillis DESC")
    suspend fun all(): List<DeviceEntity>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsert(device: DeviceEntity)

    @Query("UPDATE devices SET lastSeenMillis = :seenAt, lastAddresses = :addresses WHERE deviceId = :deviceId")
    suspend fun updateSeen(deviceId: String, seenAt: Long, addresses: String)

    @Query("UPDATE devices SET name = :name WHERE deviceId = :deviceId")
    suspend fun updateName(deviceId: String, name: String)

    @Query("DELETE FROM devices WHERE deviceId = :deviceId")
    suspend fun delete(deviceId: String)

    @Query("DELETE FROM devices")
    suspend fun deleteAll()
}

@Dao
interface SeenMessageDao {
    @Query("SELECT * FROM seen_messages")
    suspend fun all(): List<SeenMessageEntity>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(entry: SeenMessageEntity)

    @Query("DELETE FROM seen_messages WHERE id IN (:ids)")
    suspend fun delete(ids: List<String>)
}

@Dao
interface TransferDao {
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(transfer: TransferEntity)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertFiles(files: List<TransferFileEntity>)

    /** The transfer and its file rows in one transaction: a crash between the two inserts must not leave a transfer with no files. */
    @Transaction
    suspend fun insertWithFiles(transfer: TransferEntity, files: List<TransferFileEntity>) {
        insert(transfer)
        insertFiles(files)
    }

    @Query("SELECT * FROM transfers WHERE id = :id")
    suspend fun get(id: String): TransferEntity?

    @Query("SELECT * FROM transfer_files WHERE transferId = :id ORDER BY ordinal")
    suspend fun files(id: String): List<TransferFileEntity>

    @Query("SELECT * FROM transfers WHERE status IN ('queued', 'active') ORDER BY createdAtMillis")
    suspend fun unfinished(): List<TransferEntity>

    @Query("SELECT * FROM transfers ORDER BY createdAtMillis DESC LIMIT :limit")
    suspend fun recent(limit: Int): List<TransferEntity>

    @Query("UPDATE transfers SET status = :status, errorCode = :errorCode, message = :message, finishedAtMillis = :finishedAt WHERE id = :id")
    suspend fun setStatus(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long?)

    @Query("UPDATE transfers SET completeAcked = :acked WHERE id = :id")
    suspend fun setCompleteAcked(id: String, acked: Boolean)

    @Query("UPDATE transfers SET totalBytes = :totalBytes WHERE id = :id")
    suspend fun setTotalBytes(id: String, totalBytes: Long)

    @Query("UPDATE transfer_files SET bytesDone = :bytesDone WHERE transferId = :id AND fileId = :fileId")
    suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long)

    @Query("UPDATE transfers SET id = :newId WHERE id = :oldId")
    suspend fun renameTransfer(oldId: String, newId: String)

    @Query("UPDATE transfer_files SET transferId = :newId WHERE transferId = :oldId")
    suspend fun renameTransferFiles(oldId: String, newId: String)

    @Query("SELECT finalName FROM transfer_files WHERE finished = 0 AND finalName IS NOT NULL")
    suspend fun claimedNames(): List<String>

    /** Bytes that active or waiting downloads still have to write (what the free-space check of a new `offer` must leave alone). */
    @Query(
        "SELECT COALESCE(SUM(MAX(f.size - f.bytesDone, 0)), 0) FROM transfer_files f JOIN transfers t ON t.id = f.transferId " +
            "WHERE t.direction = 'down' AND t.status IN ('queued', 'active') AND f.finished = 0"
    )
    suspend fun pendingDownloadBytes(): Long

    /** The MediaStore entries that unfinished downloads own. A pending entry that isn't listed here is an orphan. */
    @Query(
        "SELECT f.localUri FROM transfer_files f JOIN transfers t ON t.id = f.transferId " +
            "WHERE t.direction = 'down' AND t.status IN ('queued', 'active') AND f.finished = 0 AND f.localUri IS NOT NULL"
    )
    suspend fun ownedPartialUris(): List<String>

    /** Transfers whose cached copies may still be needed (a failed upload can be retried). */
    @Query("SELECT id FROM transfers WHERE status IN ('queued', 'active', 'failed')")
    suspend fun liveIds(): List<String>

    @Query("DELETE FROM transfers WHERE id = :id")
    suspend fun delete(id: String)

    @Query("DELETE FROM transfer_files WHERE transferId = :id")
    suspend fun deleteFiles(id: String)
}
