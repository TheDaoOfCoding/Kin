package app.kinrelay.android.transfer

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteConstraintException
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.StatFs
import android.provider.MediaStore
import androidx.core.net.toUri
import app.kinrelay.android.DebugHooks
import java.io.FileNotFoundException
import java.io.FileOutputStream
import java.io.IOException
import java.io.OutputStream

/**
 * Saves received files through MediaStore: photos to `Pictures/Kin`, videos to `Movies/Kin`, everything else to `Download/Kin`
 * (android.md). No storage permission is needed for files the app creates itself (API 29+). An entry is written with `IS_PENDING=1`,
 * so half-written files show up nowhere, and the flag is cleared when the file is complete.
 *
 * Resume appends to the pending entry. It is written through a read-write descriptor that is positioned at the end, not through
 * `openOutputStream(uri, "wa")`: the app does not depend on "wa" behaving the same on every phone's MediaStore.
 */
class MediaStoreSink(context: Context) : SaveSink {
    private val appContext: Context = context.applicationContext
    private val resolver: ContentResolver = appContext.contentResolver

    private fun collection(folder: SaveFolder): Uri = when (folder) {
        SaveFolder.PICTURES -> MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        SaveFolder.MOVIES -> MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        SaveFolder.DOWNLOADS -> MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
    }

    private fun relativePath(folder: SaveFolder): String = when (folder) {
        SaveFolder.PICTURES -> "${Environment.DIRECTORY_PICTURES}/$FOLDER_NAME"
        SaveFolder.MOVIES -> "${Environment.DIRECTORY_MOVIES}/$FOLDER_NAME"
        SaveFolder.DOWNLOADS -> "${Environment.DIRECTORY_DOWNLOADS}/$FOLDER_NAME"
    }

    override fun existingNames(folder: SaveFolder): Set<String> {
        val path = relativePath(folder)
        val names = HashSet<String>()
        try {
            resolver.query(
                collection(folder), arrayOf(MediaStore.MediaColumns.DISPLAY_NAME),
                "${MediaStore.MediaColumns.RELATIVE_PATH} IN (?, ?)", arrayOf(path, "$path/"), null,
            )?.use { cursor -> while (cursor.moveToNext()) cursor.getString(0)?.let(names::add) }
        } catch (e: SecurityException) {
            // Scoped storage shows only our own files; a refusal means there is nothing to list.
        }
        return names
    }

    override fun pendingEntries(): List<String> {
        val found = ArrayList<String>()
        val args = Bundle().apply { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) putInt(MediaStore.QUERY_ARG_MATCH_PENDING, MediaStore.MATCH_ONLY) }
        for (folder in SaveFolder.entries) {
            val path = relativePath(folder)
            // On Android 10 the pending flag is not a query argument: pending rows are included on request and the flag is filtered here.
            @Suppress("DEPRECATION") val target = if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) MediaStore.setIncludePending(collection(folder)) else collection(folder)
            args.putString(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION, "${MediaStore.MediaColumns.RELATIVE_PATH} IN (?, ?) AND ${MediaStore.MediaColumns.IS_PENDING} = 1")
            args.putStringArray(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, arrayOf(path, "$path/"))
            try {
                resolver.query(target, arrayOf(MediaStore.MediaColumns._ID), args, null)?.use { cursor ->
                    while (cursor.moveToNext()) found += android.content.ContentUris.withAppendedId(collection(folder), cursor.getLong(0)).toString()
                }
            } catch (e: SecurityException) {
                // Scoped storage shows only our own files; a refusal means there is nothing to list.
            }
        }
        return found
    }

    override fun create(folder: SaveFolder, name: String, mime: String): SavedEntry {
        if (DebugHooks.refuseNewNames(appContext)) throw NameTakenException("$name is refused on purpose (debug switch)")
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath(folder))
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = try {
            resolver.insert(collection(folder), values)
        } catch (e: IllegalArgumentException) {
            // The collection refused the MIME type or the extension: keep the file, in Downloads, as plain data.
            if (folder == SaveFolder.DOWNLOADS) throw IOException("MediaStore refused $name: ${e.message}", e)
            return create(SaveFolder.DOWNLOADS, name, MediaKinds.DEFAULT_MIME)
        } catch (e: SQLiteConstraintException) {
            // A file with this path exists that this app can't see (left by an earlier install, or made by another app): MediaStore's
            // unique index on the path refuses the insert instead of renaming. That is "already exists": the caller tries the next suffix.
            throw NameTakenException("$name is taken by a file this app can't see", e)
        } ?: throw IOException("MediaStore did not create $name")
        // MediaStore may have picked another name (a clash, or a missing extension): that is the real name.
        val finalName = pendingQuery(uri, MediaStore.MediaColumns.DISPLAY_NAME) { it.getString(0) } ?: name
        // A new entry has no file yet: until it is opened for writing, opening it for reading fails with ENOENT (found on Android 17).
        // Opening it once for writing creates the file, so "how many bytes does it have" works from the start.
        try {
            resolver.openFileDescriptor(uri, "w")?.close()
        } catch (e: IOException) {
            runCatching { resolver.delete(uri, null, null) }
            throw IOException("MediaStore created $name but it can't be written", e)
        }
        return SavedEntry(uri.toString(), finalName)
    }

    override fun length(uri: String): Long? = try {
        resolver.openFileDescriptor(uri.toUri(), "r")?.use { it.statSize }
    } catch (e: FileNotFoundException) {
        // No file: either the entry is gone (the user deleted it) or it is a pending one that was never written to, which is empty.
        if (pendingQuery(uri.toUri(), MediaStore.MediaColumns._ID) { it.getLong(0) } != null) 0L else null
    } catch (e: SecurityException) {
        null
    }

    /** One value of one entry, pending entries included. Null if the entry doesn't exist. */
    private fun <T> pendingQuery(uri: Uri, column: String, read: (android.database.Cursor) -> T): T? {
        val args = Bundle().apply { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) putInt(MediaStore.QUERY_ARG_MATCH_PENDING, MediaStore.MATCH_INCLUDE) }
        @Suppress("DEPRECATION") val target = if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) MediaStore.setIncludePending(uri) else uri
        return try {
            resolver.query(target, arrayOf(column), args, null)?.use { if (it.moveToFirst()) read(it) else null }
        } catch (e: SecurityException) {
            null
        }
    }

    override fun openForWrite(uri: String, at: Long): OutputStream {
        val descriptor = resolver.openFileDescriptor(uri.toUri(), "rw") ?: throw FileNotFoundException(uri)
        val stream = FileOutputStream(descriptor.fileDescriptor)
        try {
            val channel = stream.channel
            // Anything after `at` is cut off: a restart (at == 0) or a longer entry than the bytes we know are good.
            if (channel.size() > at) channel.truncate(at)
            channel.position(at)
        } catch (e: IOException) {
            stream.close()
            descriptor.close()
            throw e
        }
        return object : OutputStream() {
            override fun write(b: Int) = stream.write(b)
            override fun write(b: ByteArray, off: Int, len: Int) = stream.write(b, off, len)
            override fun flush() = stream.flush()
            override fun close() {
                try {
                    stream.close()
                } finally {
                    descriptor.close()
                }
            }
        }
    }

    override fun finish(uri: String) {
        val values = ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }
        resolver.update(uri.toUri(), values, null, null)
    }

    override fun delete(uri: String) {
        try {
            resolver.delete(uri.toUri(), null, null)
        } catch (e: SecurityException) {
            // Not ours any more: nothing to delete.
        }
    }

    override fun freeBytes(): Long = StatFs(Environment.getExternalStorageDirectory().path).availableBytes

    companion object {
        const val FOLDER_NAME = "Kin"
    }
}
