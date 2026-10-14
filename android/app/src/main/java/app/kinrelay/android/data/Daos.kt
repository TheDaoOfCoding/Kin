package app.kinrelay.android.data

import androidx.room3.Dao
import androidx.room3.Insert
import androidx.room3.OnConflictStrategy
import androidx.room3.Query
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
