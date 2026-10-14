package app.kinrelay.android.data

import androidx.room3.Entity
import androidx.room3.PrimaryKey

/** A paired PC (architecture.md, "Storage"). [lastAddresses] is a comma-separated `host:port` list, best first. */
@Entity(tableName = "devices")
data class DeviceEntity(
    @PrimaryKey val deviceId: String,
    val name: String,
    val platform: String,
    val pairedAtMillis: Long,
    val lastSeenMillis: Long?,
    val lastAddresses: String,
)

/** A processed message ID and the response sent for it (de-duplication memory). */
@Entity(tableName = "seen_messages")
data class SeenMessageEntity(
    @PrimaryKey val id: String,
    val response: String,
    val atMillis: Long,
)
