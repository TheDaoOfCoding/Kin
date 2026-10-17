package app.kinrelay.android.data

import androidx.room3.ColumnInfo
import androidx.room3.Entity
import androidx.room3.Index
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
@Entity(
    tableName = "transfers",
    indices = [Index(value = ["status", "createdAtMillis"]), Index(value = ["createdAtMillis"]), Index(value = ["owed"])],
)
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
    /**
     * A stable local key that never changes, even when [id] does (a changed source starts over under a new transfer ID). The cached
     * copies of an upload live in a folder named after it. Equal to [id] when the transfer is created.
     */
    @ColumnInfo(defaultValue = "''") val localId: String = id,
    /** Uploads: the ID of the transfer a user Retry made for the files that never got a `200` (this one stays as it ended). */
    val retriedAs: String? = null,
    /** Uploads: how many times the phone tried to send it (the first-try success rate counts the ones that needed more than one). */
    @ColumnInfo(defaultValue = "0") val attempts: Int = 0,
    /**
     * Uploads: the user asked for the next run (Retry) and it hasn't reached its `POST` yet. It survives a Retry made while offline: the
     * upload parks as queued and the queue flush starts it later, and that run is still the user's (a `404` may then start a new transfer).
     */
    @ColumnInfo(defaultValue = "0") val userRetry: Boolean = false,
    /** Uploads: the PC has answered a `POST` for it (the transfer exists there). A started upload follows the `.part` rule, not the 7-day queue expiry. */
    @ColumnInfo(defaultValue = "0") val started: Boolean = false,
    /**
     * Downloads: a message to the PC that nobody acked yet, kept until it is acked and sent again after every `hello`: `complete`, or
     * `cancel:<reason>`. Null when nothing is owed. [owedId] is its message ID (the same ID every time it is sent).
     */
    val owed: String? = null,
    val owedId: String? = null,
    /**
     * Uploads (2B.4): `screenshot` or `camera` for an image the phone sent by itself (auto-send), null for everything the user sent. It goes in `POST /v1/transfers` as `origin` (on resumes too: the PC
     * keeps the first value), so the PC words its toast and keeps a screenshot out of its clipboard history. An auto-sent image is never queued.
     */
    val origin: String? = null,
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

/** Where a text came from. [AUTO_CLIPBOARD] is never stored (the automatic sync is not history); it exists so a filter can say so. */
object TextSource {
    const val SHARE = "share"
    const val PROCESS_TEXT = "process_text"
    const val CLIPBOARD = "clipboard"
    const val DEBUG = "debug"

    /** A link shared to Kin (the share sheet): sent as a `link`. */
    const val SHARE_LINK = "share_link"

    /** A text the PC sent (a hotkey send): history of the receiving side. */
    const val PC = "pc"
    const val AUTO_CLIPBOARD = "auto_clipboard"
}

/**
 * A text sent to or received from the PC (milestone 1.4): the persistent queue and the history. [id] is the message ID of the wire.
 * [body] is the text in wire form (LF only). It is dropped for a sensitive text as soon as the item is over (delivered, failed or
 * canceled), and a received sensitive text is never stored at all.
 */
@Entity(
    tableName = "texts",
    indices = [Index(value = ["createdAtMillis"]), Index(value = ["direction", "status", "createdAtMillis"])],
)
data class TextItemEntity(
    @PrimaryKey val id: String,
    val direction: String,
    val peerDeviceId: String,
    val peerName: String,
    val status: String,
    val createdAtMillis: Long,
    val finishedAtMillis: Long?,
    val body: String?,
    val byteSize: Long,
    val sensitive: Boolean,
    val isUrl: Boolean,
    val source: String,
    val errorCode: String?,
    val message: String?,
    val attempts: Int,
    /** A failed text that a user Retry sent again as a new message: the ID of the new one (this row stays as it ended). */
    val retriedAs: String? = null,
    /** [TextKind.TEXT] or [TextKind.LINK] (2B.1): a shared web page is queued and shown as a link, and goes out as a `link` (or, to an old PC, as a `text`). */
    @ColumnInfo(defaultValue = "'text'") val kind: String = TextKind.TEXT,
    /** The page title of a link (already cleaned), or null. Never set on a text. */
    val title: String? = null,
)

/** What a row of `texts` is. */
object TextKind {
    const val TEXT = "text"
    const val LINK = "link"
}
