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

/** What an unfinished download still has to write for one file: [size] is the whole file, [bytesDone] what the store knows is written. */
data class PendingFile(val transferId: String, val fileId: String, val size: Long, val bytesDone: Long)

/** The facts the first-try success rate is made of (outgoing items that ended). */
data class OutcomeRow(val status: String, val errorCode: String?, val attempts: Int)

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

    /**
     * A user Retry that found its transfer ID used up, in one transaction: the old transfer ends as failed (`not_found`, [message]), the new
     * transfer (the files without a `200`) is stored, and the old one points at it.
     */
    @Transaction
    suspend fun supersede(oldId: String, transfer: TransferEntity, files: List<TransferFileEntity>, message: String, finishedAt: Long) {
        setStatus(oldId, "failed", "not_found", message, finishedAt, null, null)
        insert(transfer)
        insertFiles(files)
        markRetried(oldId, transfer.id)
    }

    @Query("SELECT * FROM transfers WHERE id = :id")
    suspend fun get(id: String): TransferEntity?

    @Query("SELECT * FROM transfer_files WHERE transferId = :id ORDER BY ordinal")
    suspend fun files(id: String): List<TransferFileEntity>

    @Query("SELECT * FROM transfers WHERE status IN ('queued', 'active') ORDER BY createdAtMillis")
    suspend fun unfinished(): List<TransferEntity>

    @Query("SELECT * FROM transfers ORDER BY createdAtMillis DESC LIMIT :limit")
    suspend fun recent(limit: Int): List<TransferEntity>

    @Query("SELECT * FROM transfers ORDER BY createdAtMillis DESC LIMIT :limit")
    fun observeRecent(limit: Int): Flow<List<TransferEntity>>

    @Query("SELECT * FROM transfer_files WHERE transferId IN (SELECT id FROM transfers ORDER BY createdAtMillis DESC LIMIT :limit) ORDER BY transferId, ordinal")
    fun observeRecentFiles(limit: Int): Flow<List<TransferFileEntity>>

    /**
     * Only an unfinished transfer changes: a late `cancel` or a racing failure must never rewrite a transfer that is already over.
     * Returns the rows changed (0 when the transfer was already over).
     */
    @Query(
        "UPDATE transfers SET status = :status, errorCode = :errorCode, message = :message, finishedAtMillis = :finishedAt, " +
            "owed = :owed, owedId = :owedId WHERE id = :id AND status IN ('queued', 'active')"
    )
    suspend fun setStatus(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long?, owed: String?, owedId: String?): Int

    /**
     * Retry: a failed or queued transfer is queued again, and the next run is the user's ([userRetry]) until it reaches its `POST`. Its
     * creation time stays: the 7 days of the queue run from the moment it was made, not from the moment the user pressed Retry. Anything
     * else (done, canceled, already retried) stays as it is.
     */
    @Query(
        "UPDATE transfers SET status = 'queued', errorCode = NULL, message = NULL, finishedAtMillis = NULL, userRetry = :userRetry " +
            "WHERE id = :id AND status IN ('queued', 'failed') AND retriedAs IS NULL"
    )
    suspend fun requeue(id: String, userRetry: Boolean): Int

    @Query("UPDATE transfers SET userRetry = 0 WHERE id = :id")
    suspend fun clearUserRetry(id: String)

    @Query("UPDATE transfers SET started = 1 WHERE id = :id")
    suspend fun markStarted(id: String)

    /** The downloads of [peerDeviceId] that owe the PC a `complete` or `cancel`, oldest first. */
    @Query("SELECT * FROM transfers WHERE owed IS NOT NULL AND peerDeviceId = :peerDeviceId ORDER BY finishedAtMillis, id")
    suspend fun owing(peerDeviceId: String): List<TransferEntity>

    /** The PC answered the owed message: nothing is owed any more (and a `complete` is acked). */
    @Query("UPDATE transfers SET completeAcked = CASE WHEN owed = 'complete' THEN 1 ELSE completeAcked END, owed = NULL, owedId = NULL WHERE id = :id")
    suspend fun clearOwed(id: String)

    /** The PC's own `cancel` says what a `404` could not: the transfer was canceled there. Only a failed `not_found` changes; `done` never does. */
    @Query(
        "UPDATE transfers SET status = 'canceled', errorCode = 'peer_canceled', message = :message " +
            "WHERE id = :id AND status = 'failed' AND errorCode = 'not_found'"
    )
    suspend fun markCanceledByPeer(id: String, message: String): Int

    @Query("UPDATE transfers SET completeAcked = :acked WHERE id = :id")
    suspend fun setCompleteAcked(id: String, acked: Boolean)

    @Query("UPDATE transfers SET totalBytes = :totalBytes WHERE id = :id")
    suspend fun setTotalBytes(id: String, totalBytes: Long)

    @Query("UPDATE transfers SET attempts = attempts + 1 WHERE id = :id")
    suspend fun addAttempt(id: String)

    @Query("UPDATE transfers SET retriedAs = :newId WHERE id = :oldId")
    suspend fun markRetried(oldId: String, newId: String)

    @Query("UPDATE transfer_files SET bytesDone = :bytesDone WHERE transferId = :id AND fileId = :fileId")
    suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long)

    @Query("UPDATE transfers SET id = :newId WHERE id = :oldId")
    suspend fun renameTransfer(oldId: String, newId: String)

    @Query("UPDATE transfer_files SET transferId = :newId WHERE transferId = :oldId")
    suspend fun renameTransferFiles(oldId: String, newId: String)

    /** A new ID for a transfer and its files: both rows move together or neither does. */
    @Transaction
    suspend fun rename(oldId: String, newId: String) {
        renameTransfer(oldId, newId)
        renameTransferFiles(oldId, newId)
    }

    @Query("SELECT finalName FROM transfer_files WHERE finished = 0 AND finalName IS NOT NULL")
    suspend fun claimedNames(): List<String>

    /** The files that active or waiting downloads still have to write (what the free-space check of a new `offer` must leave alone). */
    @Query(
        "SELECT f.transferId AS transferId, f.fileId AS fileId, f.size AS size, f.bytesDone AS bytesDone FROM transfer_files f " +
            "JOIN transfers t ON t.id = f.transferId " +
            "WHERE t.direction = 'down' AND t.status IN ('queued', 'active') AND f.finished = 0"
    )
    suspend fun pendingDownloadFiles(): List<PendingFile>

    /** The MediaStore entries that unfinished downloads own. A pending entry that isn't listed here is an orphan. */
    @Query(
        "SELECT f.localUri FROM transfer_files f JOIN transfers t ON t.id = f.transferId " +
            "WHERE t.direction = 'down' AND t.status IN ('queued', 'active') AND f.finished = 0 AND f.localUri IS NOT NULL"
    )
    suspend fun ownedPartialUris(): List<String>

    /** The cache keys of transfers whose cached copies may still be needed (a failed upload can be retried; a retried one cannot). */
    @Query(QuerySql.LIVE_LOCAL_IDS)
    suspend fun liveLocalIds(): List<String>

    @Query(QuerySql.OUTGOING_TRANSFER_OUTCOMES)
    suspend fun outgoingOutcomes(): List<OutcomeRow>

    @Query("DELETE FROM transfers WHERE id = :id")
    suspend fun delete(id: String)

    @Query("DELETE FROM transfer_files WHERE transferId = :id")
    suspend fun deleteFiles(id: String)

    /** A transfer and its files go together. */
    @Transaction
    suspend fun deleteWithFiles(id: String) {
        deleteFiles(id)
        delete(id)
    }

    /**
     * The history shows the newest [keep] transfers. The rows of older ones stay (a transfer ID is never used twice, so it is remembered for
     * ever), but their file rows go, except while a `complete` or `cancel` is still owed.
     */
    @Query(QuerySql.DROP_OLD_TRANSFER_FILES)
    suspend fun trim(keep: Int)
}

@Dao
interface TextDao {
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(item: TextItemEntity)

    @Query("SELECT * FROM texts WHERE id = :id")
    suspend fun get(id: String): TextItemEntity?

    /** The queue, in creation order. */
    @Query("SELECT * FROM texts WHERE direction = 'up' AND status = 'queued' ORDER BY createdAtMillis, id")
    suspend fun queued(): List<TextItemEntity>

    @Query("SELECT * FROM texts ORDER BY createdAtMillis DESC LIMIT :limit")
    suspend fun recent(limit: Int): List<TextItemEntity>

    @Query("SELECT * FROM texts ORDER BY createdAtMillis DESC LIMIT :limit")
    fun observeRecent(limit: Int): Flow<List<TextItemEntity>>

    /** `queued` to `active` for exactly one caller: a cancel that came first wins, and nothing goes out twice at once. */
    @Query("UPDATE texts SET status = 'active', attempts = attempts + 1 WHERE id = :id AND status = 'queued'")
    suspend fun claim(id: String): Int

    /** Back in the queue after a send that wasn't acked. */
    @Query("UPDATE texts SET status = 'queued' WHERE id = :id AND status = 'active'")
    suspend fun release(id: String): Int

    /** An item that ended. [body] is kept for history unless the text is sensitive. */
    @Query(
        "UPDATE texts SET status = :status, errorCode = :errorCode, message = :message, finishedAtMillis = :finishedAt, " +
            "body = CASE WHEN sensitive = 1 THEN NULL ELSE body END WHERE id = :id AND status IN ('queued', 'active')"
    )
    suspend fun finish(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long): Int

    /** A user cancel: only an item that is still waiting is canceled (one that is being sent right now is not). */
    @Query(
        "UPDATE texts SET status = 'canceled', finishedAtMillis = :finishedAt, errorCode = 'user', message = 'Canceled.', " +
            "body = CASE WHEN sensitive = 1 THEN NULL ELSE body END WHERE id = :id AND status = 'queued'"
    )
    suspend fun cancelQueued(id: String, finishedAt: Long): Int

    @Query("UPDATE texts SET retriedAs = :newId WHERE id = :oldId AND status = 'failed' AND body IS NOT NULL AND retriedAs IS NULL")
    suspend fun markRetried(oldId: String, newId: String): Int

    /**
     * A user Retry of a failed text: the same text goes out again as a **new** message (a new ID: the PC remembers the old one and would
     * repeat its first answer), created now, in a **new row**. The failed row stays as it ended and points at the new one
     * ([TextItemEntity.retriedAs]), as for a transfer. The attempts already made stay counted, so a retried text is not a first-try
     * success. A sensitive text has no body any more and can't be retried; a text can be retried once. Returns whether the new row was made.
     */
    @Transaction
    suspend fun retryAs(oldId: String, newId: String, now: Long): Boolean {
        val old = get(oldId) ?: return false
        if (old.status != "failed" || old.body == null || old.retriedAs != null) return false
        insert(old.copy(id = newId, status = "queued", createdAtMillis = now, finishedAtMillis = null, errorCode = null, message = null, retriedAs = null))
        return markRetried(oldId, newId) > 0
    }

    /** The PC is not paired any more: what waited for it is canceled (it must never go to another PC). */
    @Query(
        "UPDATE texts SET status = 'canceled', finishedAtMillis = :now, errorCode = 'unpaired', message = 'The PC was unpaired.', " +
            "body = CASE WHEN sensitive = 1 THEN NULL ELSE body END WHERE direction = 'up' AND peerDeviceId = :peerDeviceId AND status IN ('queued', 'active')"
    )
    suspend fun cancelQueuedFor(peerDeviceId: String, now: Long): Int

    /** After a process kill nothing is being sent: whatever was `active` goes back to the queue. */
    @Query("UPDATE texts SET status = 'queued' WHERE direction = 'up' AND status = 'active'")
    suspend fun recoverActive(): Int

    @Query(QuerySql.OUTGOING_TEXT_OUTCOMES)
    suspend fun outgoingOutcomes(): List<OutcomeRow>

    /** The history shows the newest [keep] texts: the bodies of older finished ones go (the rows stay, a message ID is remembered). */
    @Query(QuerySql.DROP_OLD_TEXT_BODIES)
    suspend fun trim(keep: Int)
}
