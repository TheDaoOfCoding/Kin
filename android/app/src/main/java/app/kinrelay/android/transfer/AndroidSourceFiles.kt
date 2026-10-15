package app.kinrelay.android.transfer

import android.content.ContentProviderClient
import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.os.RemoteException
import android.provider.OpenableColumns
import androidx.core.net.toUri
import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.io.InputStream

/**
 * Reads the files a share handed over (content URIs) and keeps copies in the app's cache for transfers that had to wait.
 * `file:` URIs only ever come from the cache here: [app.kinrelay.android.share.ShareIntents] refuses them from an intent.
 */
class AndroidSourceFiles(context: Context) : SourceFiles {
    private val resolver = context.applicationContext.contentResolver
    private val root = File(context.applicationContext.cacheDir, "queue")

    override fun stat(uri: String): SourceInfo? {
        val parsed = uri.toUri()
        if (parsed.scheme == "file") {
            val file = File(parsed.path ?: return null)
            return if (file.isFile) SourceInfo(file.length(), file.lastModified(), file.name, null) else null
        }
        return try {
            var name: String? = null
            var size = -1L
            var modified: Long? = null
            var mime: String? = null
            // An *unstable* client: if the app that owns the file dies, this one gets an exception. A normal (stable) reference would make
            // Android kill Kin together with it, and with Kin the transfers of every other file.
            unstableClient(parsed).use { client ->
                // All columns, then look up the ones that exist: providers disagree on which columns they have.
                client.query(parsed, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 }?.let { name = cursor.getString(it) }
                        cursor.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 }?.let { if (!cursor.isNull(it)) size = cursor.getLong(it) }
                        cursor.getColumnIndex(LAST_MODIFIED_MILLIS).takeIf { it >= 0 }?.let { if (!cursor.isNull(it)) modified = cursor.getLong(it) }
                        if (modified == null) {
                            // MediaStore keeps seconds.
                            cursor.getColumnIndex(DATE_MODIFIED_SECONDS).takeIf { it >= 0 }?.let { if (!cursor.isNull(it)) modified = cursor.getLong(it) * 1000 }
                        }
                    }
                }
                mime = client.getType(parsed)
                if (size < 0) {
                    client.openFile(parsed, "r")?.use { size = it.statSize }
                }
            }
            if (size < 0) null else SourceInfo(size, modified, name, mime)
        } catch (e: SecurityException) {
            null
        } catch (e: IOException) {
            null
        } catch (e: RemoteException) {
            null
        } catch (e: IllegalArgumentException) {
            null
        }
    }

    /** An unstable client for the provider of [uri]; closing it lets go of the connection (an open file descriptor stays valid). */
    private fun unstableClient(uri: Uri): ContentProviderClient =
        resolver.acquireUnstableContentProviderClient(uri) ?: throw IOException("no provider for $uri")

    override fun open(uri: String, offset: Long): InputStream {
        val parsed = uri.toUri()
        if (parsed.scheme == "file") return FileInputStream(File(parsed.path ?: throw IOException("no path"))).also { seek(it, offset) }
        // The descriptor comes from an unstable client, which is closed at once: nothing ties Kin's process to the file's owner while it
        // reads. (`ContentResolver.openFileDescriptor` keeps a stable reference until the descriptor is closed, so when the gallery app
        // dies mid-upload, Android kills Kin too. Seen on the emulator in the 1.3 R5 run.)
        val descriptor = try {
            unstableClient(parsed).use { client -> client.openFile(parsed, "r") } ?: throw IOException("the file can't be opened")
        } catch (e: RemoteException) {
            throw IOException("the app that owns the file is gone", e)
        }
        val stream = ParcelFileDescriptor.AutoCloseInputStream(descriptor)
        try {
            seek(stream, offset)
        } catch (e: Throwable) {
            stream.close()
            throw e
        }
        return stream
    }

    /** Seeks the descriptor when it can; a stream that can't be sought (a pipe) is read up to [offset] and thrown away. */
    private fun seek(stream: FileInputStream, offset: Long) {
        if (offset <= 0) return
        try {
            stream.channel.position(offset)
        } catch (e: IOException) {
            var skipped = 0L
            while (skipped < offset) {
                val n = stream.skip(offset - skipped)
                if (n <= 0) throw IOException("can't skip to $offset")
                skipped += n
            }
        }
    }

    override fun cacheCopy(uri: String, transferId: String, fileId: String): String? = try {
        val target = File(File(root, transferId).also { it.mkdirs() }, fileId)
        open(uri, 0).use { input -> target.outputStream().use { output -> input.copyTo(output, COPY_BUFFER_BYTES) } }
        Uri.fromFile(target).toString()
    } catch (e: IOException) {
        null
    } catch (e: SecurityException) {
        null
    }

    override fun cacheText(text: String, transferId: String, fileId: String): String {
        val target = File(File(root, transferId).also { it.mkdirs() }, fileId)
        target.writeBytes(text.toByteArray(Charsets.UTF_8))
        return Uri.fromFile(target).toString()
    }

    override fun deleteCache(transferId: String) {
        File(root, transferId).deleteRecursively()
    }

    override fun deleteCacheExcept(owned: Set<String>, minAgeMillis: Long): Int {
        val now = System.currentTimeMillis()
        val orphans = root.listFiles()?.filter { it.name !in owned && now - it.lastModified() >= minAgeMillis }.orEmpty()
        orphans.forEach { it.deleteRecursively() }
        return orphans.size
    }

    /** Removes cached copies of transfers older than [maxAgeMillis] (a failed or forgotten transfer must not fill the cache). */
    fun deleteCacheOlderThan(maxAgeMillis: Long, nowMillis: Long = System.currentTimeMillis()) {
        root.listFiles()?.filter { nowMillis - it.lastModified() > maxAgeMillis }?.forEach { it.deleteRecursively() }
    }

    private companion object {
        const val LAST_MODIFIED_MILLIS = "last_modified"
        const val DATE_MODIFIED_SECONDS = "date_modified"
        const val COPY_BUFFER_BYTES = 256 * 1024
    }
}
