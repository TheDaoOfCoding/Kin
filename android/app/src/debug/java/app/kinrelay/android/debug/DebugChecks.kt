package app.kinrelay.android.debug

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.MediaStore
import androidx.core.net.toUri
import app.kinrelay.android.transfer.MediaStoreSink
import java.io.IOException

/**
 * Debug builds only: things the end-to-end scripts and the emulator runs need to look at that normal code never reports.
 * Each function returns log lines for the `CMD` result (logcat tag `KinRelay`).
 */
object DebugChecks {
    /**
     * What is in `Pictures/Kin`, `Movies/Kin` and `Download/Kin` that this app created (scoped storage shows no one else's), with the
     * path MediaStore reports and whether the entry is still pending. Pending entries are included on purpose: that is how a half-done
     * download is seen.
     */
    fun received(context: Context): List<String> {
        val resolver = context.contentResolver
        val collections = listOf(
            "images" to MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
            "video" to MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
            "downloads" to MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
        )
        val lines = mutableListOf<String>()
        for ((label, base) in collections) {
            val projection = arrayOf(
                MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.SIZE, MediaStore.MediaColumns.MIME_TYPE,
                MediaStore.MediaColumns.RELATIVE_PATH, MediaStore.MediaColumns.IS_PENDING,
            )
            val args = Bundle().apply {
                putString(ContentResolver.QUERY_ARG_SQL_SELECTION, "${MediaStore.MediaColumns.RELATIVE_PATH} LIKE ?")
                putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, arrayOf("%/Kin/%"))
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) putInt(MediaStore.QUERY_ARG_MATCH_PENDING, MediaStore.MATCH_INCLUDE)
            }
            // On Android 10 there is no match-pending argument: the pending items are asked for by the URI.
            @Suppress("DEPRECATION") val uri = if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) MediaStore.setIncludePending(base) else base
            try {
                resolver.query(uri, projection, args, null)?.use { cursor ->
                    while (cursor.moveToNext()) {
                        val id = cursor.getLong(0)
                        lines += "$label id=$id name='${cursor.getString(1)}' size=${cursor.getLong(2)} mime=${cursor.getString(3)} " +
                            "path=${cursor.getString(4)} pending=${cursor.getInt(5)} uri=${Uri.withAppendedPath(base, id.toString())}"
                    }
                }
            } catch (e: Exception) {
                lines += "$label query failed: ${e.javaClass.simpleName}: ${e.message}"
            }
        }
        return lines.ifEmpty { listOf("none") }
    }

    /**
     * R7: does appending to a pending MediaStore entry work on this Android? Three ways are tried on one new entry each: reopening with
     * `openOutputStream(uri, "wa")`, with `"w"` (which may truncate: this tells), and the way the app really does it
     * ([MediaStoreSink.openForWrite]: a read-write descriptor, positioned at the end). Each writes 100,000 bytes, closes, reopens and writes 50,000 more.
     */
    fun appendCheck(context: Context): List<String> {
        val resolver = context.contentResolver
        val sink = MediaStoreSink(context)
        val first = pattern(100_000, 1)
        val second = pattern(50_000, 2)
        val expected = first + second
        val lines = mutableListOf<String>()
        for (mode in listOf("wa", "w", "sink")) {
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, "kin-append-check-$mode.bin")
                put(MediaStore.MediaColumns.MIME_TYPE, "application/octet-stream")
                put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Kin")
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }
            val uri = resolver.insert(MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values)
            if (uri == null) {
                lines += "mode=$mode insert failed"
                continue
            }
            try {
                resolver.openOutputStream(uri, "w")!!.use { it.write(first) }
                val afterFirst = sink.length(uri.toString())
                when (mode) {
                    "wa" -> resolver.openOutputStream(uri, "wa")!!.use { it.write(second) }
                    "w" -> resolver.openOutputStream(uri, "w")!!.use { it.write(second) }
                    else -> sink.openForWrite(uri.toString(), afterFirst ?: 0).use { it.write(second) }
                }
                val after = sink.length(uri.toString())
                val content = resolver.openInputStream(uri)!!.use { it.readBytes() }
                lines += "mode=$mode sdk=${Build.VERSION.SDK_INT} afterFirst=$afterFirst afterSecond=$after expected=${expected.size} " +
                    "contentOk=${content.contentEquals(expected)} startsWithFirst=${content.size >= first.size && content.copyOf(first.size).contentEquals(first)}"
            } catch (e: IOException) {
                lines += "mode=$mode failed: ${e.javaClass.simpleName}: ${e.message}"
            } finally {
                runCatching { resolver.delete(uri, null, null) }
            }
        }
        return lines
    }

    /** Deletes every entry this app made in `Pictures/Kin`, `Movies/Kin` and `Download/Kin` (pending ones too): a clean slate for a test run. */
    fun deleteReceived(context: Context): List<String> {
        val resolver = context.contentResolver
        var deleted = 0
        for (line in received(context)) {
            val uri = Regex("uri=(\\S+)").find(line)?.groupValues?.get(1) ?: continue
            deleted += runCatching { resolver.delete(uri.toUri(), null, null) }.getOrDefault(0)
        }
        return listOf("deleted=$deleted")
    }

    /** What can be done with a brand new, empty pending entry: opened for reading, for writing, its size. */
    fun emptyEntryCheck(context: Context): List<String> {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, "kin-empty-check.bin")
            put(MediaStore.MediaColumns.MIME_TYPE, "application/octet-stream")
            put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Kin")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values) ?: return listOf("insert failed")
        val lines = mutableListOf<String>()
        for (mode in listOf("r", "rw", "w")) {
            lines += try {
                resolver.openFileDescriptor(uri, mode).use { "mode=$mode ok statSize=${it?.statSize}" }
            } catch (e: Exception) {
                "mode=$mode failed: ${e.javaClass.simpleName}: ${e.message}"
            }
        }
        runCatching { resolver.delete(uri, null, null) }
        return lines
    }

    /**
     * What a crash leaves behind: a pending entry in `Download/Kin` that no transfer owns, and a cached copy (an hour old) of a transfer that
     * doesn't exist. The start-up sweep ([app.kinrelay.android.transfer.TransferEngine.sweepOrphans]) must delete both.
     */
    fun makeOrphans(context: Context): List<String> {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, "kin-orphan.bin")
            put(MediaStore.MediaColumns.MIME_TYPE, "application/octet-stream")
            put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Kin")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values) ?: return listOf("insert failed")
        resolver.openFileDescriptor(uri, "w")?.close()
        val dir = java.io.File(java.io.File(context.cacheDir, "queue"), "orphan-transfer").also { it.mkdirs() }
        java.io.File(dir, "f1").writeText("left behind")
        dir.setLastModified(System.currentTimeMillis() - 60 * 60 * 1000)
        return listOf("entry=$uri cacheDir=${dir.name}")
    }

    private fun pattern(size: Int, seed: Long): ByteArray = ByteArray(size).also { java.util.Random(seed).nextBytes(it) }
}
