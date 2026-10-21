package app.kinrelay.android.data

import androidx.room3.useWriterConnection
import app.kinrelay.android.log.KinLog

/**
 * Durable writes for the device list (design 5.7, risk R38). The database runs in WAL mode; with `synchronous = NORMAL` a commit is not synced to disk until a checkpoint, so a power cut
 * or an OS crash right after "Unlink" could bring the removed PC back (or lose a new pairing). After every write to `devices` that matters ([KinDatabase] `devices().upsert` at pairing,
 * `delete` at unpairing) the app calls [flush], which runs `PRAGMA wal_checkpoint(FULL)`: the log is copied into the database file and synced, whatever `synchronous` says on this phone.
 */
object Durable {
    /** Checkpoints the write-ahead log (blocks until it is on disk). Never throws: a failure is logged, the write itself already happened. */
    suspend fun flush(db: KinDatabase, log: KinLog) {
        try {
            db.useWriterConnection { connection ->
                connection.usePrepared("PRAGMA wal_checkpoint(FULL)") { statement -> statement.step() }
            }
        } catch (e: Exception) {
            log.w("data", "wal checkpoint failed (${e.javaClass.simpleName}): ${e.message}")
        }
    }

    /** `PRAGMA synchronous` as the database really has it (0 OFF, 1 NORMAL, 2 FULL, 3 EXTRA), or null. For the debug state and the results file, not for the app's logic. */
    suspend fun synchronousMode(db: KinDatabase): Int? = try {
        db.useWriterConnection { connection ->
            connection.usePrepared("PRAGMA synchronous") { statement -> if (statement.step()) statement.getLong(0).toInt() else null }
        }
    } catch (_: Exception) {
        null
    }

    /** `PRAGMA journal_mode` (`wal` expected). */
    suspend fun journalMode(db: KinDatabase): String? = try {
        db.useWriterConnection { connection ->
            connection.usePrepared("PRAGMA journal_mode") { statement -> if (statement.step()) statement.getText(0) else null }
        }
    } catch (_: Exception) {
        null
    }
}
