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

/** What a transfer is. [UP] is phone to PC (the phone created it), [DOWN] is PC to phone (an `offer`). */
object TransferDirection {
    const val UP = "up"
    const val DOWN = "down"
}

object TransferKind {
    const val FILE = "file"
    const val TEXT = "text"
}

object TransferStatus {
    /** Waiting: the PC wasn't reachable, so nothing was sent yet (or the wait for it ran out). Ready to retry. */
    const val QUEUED = "queued"
    const val ACTIVE = "active"
    const val DONE = "done"
    const val FAILED = "failed"
    const val CANCELED = "canceled"

    fun isFinished(status: String) = status == DONE || status == FAILED || status == CANCELED
}

/**
 * A transfer (architecture.md, "Storage"). [id] is the transfer ID of the wire. [errorCode] is a protocol error code
 * (`disk_full`, `too_large`, `internal`, `not_found`, ...) or one of ours; [message] is the sentence shown to the user.
 */
@Entity(tableName = "transfers")
data class TransferEntity(
    @PrimaryKey val id: String,
    val direction: String,
    val peerDeviceId: String,
    val peerName: String,
    val kind: String,
    val status: String,
    val createdAtMillis: Long,
    val finishedAtMillis: Long?,
    val totalBytes: Long,
    val errorCode: String?,
    val message: String?,
    /** Downloads only: the PC acked our `complete`. */
    val completeAcked: Boolean,
)

/**
 * One file of a transfer. [name] is what the sender called it (an upload: the display name we send; a download: the raw name
 * from the offer). [finalName] is the name the file was saved under (a download: the one MediaStore chose). [localUri] is where
 * the bytes come from (an upload: the shared content URI, or the cached copy) or go to (a download: the MediaStore entry).
 */
@Entity(tableName = "transfer_files", primaryKeys = ["transferId", "fileId"])
data class TransferFileEntity(
    val transferId: String,
    val fileId: String,
    val ordinal: Int,
    val name: String,
    val finalName: String?,
    val mime: String,
    val size: Long,
    val bytesDone: Long,
    val localUri: String?,
    /** Uploads: the source's modified time when it was shared, to tell whether it changed before a resume. */
    val sourceModifiedMillis: Long?,
    /** Downloads: saved completely (the entry is no longer pending). */
    val finished: Boolean,
)
