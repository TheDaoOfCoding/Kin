package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.log.KinLog
import java.io.InputStream
import java.io.OutputStream

/** The PC this phone is paired with. */
data class PeerInfo(val deviceId: String, val name: String)

/** How the transfers reach the PC: a ready connection (its WebSocket and the HTTPS endpoint beside it). */
interface TransferLink {
    /**
     * A connection that has accepted the PC's `hello`: now, or one that appears within [waitMillis]. [kick] says the user
     * did something (the loop tries to connect now); a transfer that is only waiting for the PC to come back doesn't kick.
     * Only a connection to the PC [peerDeviceId] counts: right after a re-pair the old PC's connection is briefly still the current one,
     * and a transfer must never be sent to (or asked of) a PC it was not made for.
     */
    suspend fun connection(waitMillis: Long, kick: Boolean, peerDeviceId: String): LinkConnection?

    /** Why [connection] found nothing, in words for the user. */
    fun unreachableReason(): String

    suspend fun peer(): PeerInfo?
}

/** What is known about a shared file right now. */
data class SourceInfo(val size: Long, val modifiedMillis: Long?, val displayName: String?, val mime: String?)

/** The shared files of an upload. The real one reads content URIs; tests use plain files. */
interface SourceFiles {
    /** Null when the file can't be read now (gone, or access lost). */
    fun stat(uri: String): SourceInfo?

    /** A stream positioned at [offset]. Throws `IOException` or `SecurityException` when access is lost. */
    fun open(uri: String, offset: Long): InputStream

    /**
     * Copies the file into the app's cache so access survives (android.md). Returns the new URI, or null if it failed. [cacheKey] is the
     * transfer's stable local ID ([app.kinrelay.android.data.TransferEntity.localId]), not its transfer ID: that one can change.
     */
    fun cacheCopy(uri: String, cacheKey: String, fileId: String): String?

    /** Writes [text] (UTF-8) as a cached file and returns its URI. */
    fun cacheText(text: String, cacheKey: String, fileId: String): String

    /** Deletes everything cached under [cacheKey]. */
    fun deleteCache(cacheKey: String)

    /**
     * Deletes the cached copies whose cache key isn't in [owned] (a crash left them behind), except those newer than
     * [minAgeMillis] (a share that is being set up right now has no database row yet). Returns how many were deleted.
     */
    fun deleteCacheExcept(owned: Set<String>, minAgeMillis: Long): Int
}

enum class SaveFolder { PICTURES, MOVIES, DOWNLOADS }

/** A file that was created in the gallery or the downloads collection: where it is and the name it really got. */
data class SavedEntry(val uri: String, val finalName: String)

/** A new entry can't have this name because a file the app can't see already has it (a file of an earlier install, or of another app). */
class NameTakenException(message: String, cause: Throwable? = null) : java.io.IOException(message, cause)

/** Writes received files. The real one is MediaStore; tests write into a temporary folder. */
interface SaveSink {
    /** Names this app already has in [folder] (scoped storage shows only its own files). */
    fun existingNames(folder: SaveFolder): Set<String>

    /**
     * Creates a new, pending entry. The name that comes back is the one the system chose: keep it. Throws [NameTakenException] when
     * the name is taken by a file this app can't see (protocol.md: "already exists" is a collision, so the next suffix is tried),
     * and `IOException` for anything else.
     */
    fun create(folder: SaveFolder, name: String, mime: String): SavedEntry

    /** The bytes written so far, or null when the entry is gone (the user deleted it). */
    fun length(uri: String): Long?

    /** A stream that writes at byte [at], and cuts off anything after it ([at] 0 starts the file over). */
    fun openForWrite(uri: String, at: Long): OutputStream

    /** Makes the file visible: it is complete. */
    fun finish(uri: String)

    fun delete(uri: String)

    /** Every entry this app made that is still pending (a crash can leave one that no transfer owns). */
    fun pendingEntries(): List<String>

    /** Free space for new files, in bytes. */
    fun freeBytes(): Long
}

/** Keeps the phone awake only while bytes move. Calls nest: the lock is held until the last [release]. */
interface WakeLockPort {
    fun acquire()
    fun release()
}

/** What the user sees of a transfer. The real one posts notifications; tests record the calls. */
interface TransferNotifier {
    /** Progress: [bytesDone] of the transfer's [TransferEntity.totalBytes]. [waiting] means it is waiting for the PC to come back. */
    fun progress(transfer: TransferEntity, files: List<TransferFileEntity>, bytesDone: Long, waiting: Boolean)

    /** An upload finished. */
    fun uploaded(transfer: TransferEntity, files: List<TransferFileEntity>)

    /** A download finished: "Photo from <PC>" with Open and Share. */
    fun received(transfer: TransferEntity, files: List<TransferFileEntity>)

    /** The PC wasn't reachable; the transfer is kept ready to retry. */
    fun queued(transfer: TransferEntity, reason: String)

    fun failed(transfer: TransferEntity, message: String)

    /** Takes the progress notification away (cancelled). */
    fun clear(transferId: String)

    /** A short message with no follow-up (a toast). */
    fun message(text: String)
}

/** Everything a runner needs, in one place. */
class TransferDeps(
    val store: TransferStore,
    val link: TransferLink,
    val sources: SourceFiles,
    val sink: SaveSink,
    val notifier: TransferNotifier,
    val wake: WakeLockPort,
    val log: KinLog,
    val progress: LiveProgress = LiveProgress(),
    val nowMillis: () -> Long = System::currentTimeMillis,
    val newId: () -> String,
) {
    /** Whether the PC that is paired now is the one [transfer] belongs to. A transfer never goes to (or comes from) another PC. */
    suspend fun isPairedWith(transfer: TransferEntity): Boolean = link.peer()?.deviceId == transfer.peerDeviceId

    /**
     * A ready connection to the PC [transfer] belongs to. The check is made twice, by [TransferLink] and here on the connection itself: a
     * connection to another PC is never used, whatever the link says.
     */
    suspend fun connectionFor(transfer: TransferEntity, waitMillis: Long, kick: Boolean): LinkConnection? =
        link.connection(waitMillis, kick, transfer.peerDeviceId)?.takeIf { it.peerDeviceId == transfer.peerDeviceId }
}

/** Time limits the runners use. The defaults are for the app; tests make them short. */
data class TransferTimings(
    /** How long a share waits for a connection before it gives up and keeps the files ready (like the text sender). */
    val connectWaitMillis: Long = 4_000,
    /** How long a transfer that is already moving waits for the PC to come back before it goes back to "queued". */
    val reconnectWaitMillis: Long = 15 * 60_000L,
    /** The PC answers an ack within 5 s (protocol.md). */
    val ackTimeoutMillis: Long = 5_000,
    /** Attempts in a row that moved no bytes (on a connection that answers) before a transfer is failed. */
    val maxStalledAttempts: Int = 5,
    /**
     * No byte moved for this long: the PC is asked whether it still answers (StallGuard.kt). A transfer that moves bytes never asks, and
     * an attempt that failed is checked at once ([probeTimeoutMillis]).
     */
    val stallProbeAfterMillis: Long = 15_000,
    /** How long the PC has to answer that check (a new TLS connection) before the connection is treated as dead and replaced. */
    val probeTimeoutMillis: Long = 10_000,
    val maxOffsetMismatches: Int = 5,
    /** Pause before a retry that isn't waiting for a connection. */
    val retryDelayMillis: Long = 500,
    /** Files under this size are copied into the cache when a share can't be sent right away (android.md). */
    val cacheCopyLimitBytes: Long = 50L * 1024 * 1024,
    /**
     * Files of one transfer that are on the wire at the same time, in either direction (protocol.md: a client MAY run up to 4 file requests
     * of a transfer at once). One request per file, a file is never split, so a single big file is not slower. 1 gives the old one-by-one order.
     */
    val maxParallelFiles: Int = MAX_PARALLEL_FILES,
    /** Shared files looked at (`stat`) at the same time when a share is turned into a transfer. */
    val statParallelism: Int = STAT_PARALLELISM,
) {
    companion object {
        /** The most file requests of one transfer that protocol.md allows a client to run at once. */
        const val MAX_PARALLEL_FILES = 4
        const val STAT_PARALLELISM = 8
    }
}

/** What a runner ended with. */
sealed interface RunResult {
    data object Done : RunResult
    data class Failed(val code: String, val message: String) : RunResult
    data object Canceled : RunResult

    /** Not finished and not failed: waiting for the PC. The transfer is `queued`. */
    data object Waiting : RunResult
}
