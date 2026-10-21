package app.kinrelay.android.debug

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.net.Uri
import androidx.core.content.FileProvider
import androidx.core.graphics.createBitmap
import androidx.core.graphics.set
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.Random

/**
 * Debug builds only: files for the share hooks. They live in `files/debug-share/` and are handed out through [DebugFileProvider]
 * (authority `app.kinrelay.android.debugfiles`), so a share intent can carry real `content://` URIs without any storage permission.
 *
 * A file spec is `name:size` (deterministic pseudo-random bytes, so the PC side can verify them) or `name@WxH` (a valid JPEG or PNG
 * picture of that size, with some noise so it isn't tiny).
 */
object DebugFiles {
    const val AUTHORITY = "app.kinrelay.android.debugfiles"

    fun dir(context: Context): File = File(context.filesDir, "debug-share").also { it.mkdirs() }

    fun uriOf(context: Context, name: String): Uri = FileProvider.getUriForFile(context, AUTHORITY, File(dir(context), fileName(name)))

    /**
     * A name given on the command line, with `%uXXXX;` standing for one code point (adb can't carry bidi controls or CJK reliably
     * through a Windows console): `photo%u202E;gpj.exe` is `photo` + U+202E + `gpj.exe`.
     */
    fun fileName(raw: String): String = Regex("%u([0-9A-Fa-f]{4,6});").replace(raw) { String(Character.toChars(it.groupValues[1].toInt(16))) }

    /** The bytes of a `name:size` file: the same every time for the same size and seed, so a receiver can check them. */
    fun pattern(size: Int, seed: Long = 1): ByteArray = ByteArray(size).also { Random(seed).nextBytes(it) }

    /** Creates the files of [spec] (comma separated). Returns `name size sha256-prefix` lines for each. */
    fun make(context: Context, spec: String): List<String> = spec.split(',').map { it.trim() }.filter { it.isNotEmpty() }.map { item ->
        val picture = item.substringAfter('@', "")
        if (picture.isNotEmpty()) {
            val name = fileName(item.substringBefore('@'))
            val (w, h) = picture.split('x').map { it.toInt() }
            val file = File(dir(context), name)
            val bitmap = createBitmap(w, h)
            val random = Random(name.hashCode().toLong())
            for (y in 0 until h step 4) for (x in 0 until w step 4) bitmap[x, y] = Color.rgb(random.nextInt(256), random.nextInt(256), random.nextInt(256))
            FileOutputStream(file).use { bitmap.compress(if (name.endsWith(".png", true)) Bitmap.CompressFormat.PNG else Bitmap.CompressFormat.JPEG, 90, it) }
            describe(file)
        } else {
            val name = fileName(item.substringBefore(':'))
            val size = item.substringAfter(':', "0").toInt()
            val file = File(dir(context), name)
            file.writeBytes(pattern(size, name.hashCode().toLong()))
            describe(file)
        }
    }

    fun describe(file: File): String = "${file.name} ${file.length()} ${sha256(file.readBytes()).take(16)}"

    fun sha256(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}

/** A `FileProvider` that is only in debug builds. Not exported: access comes with the grant on the share intent. */
class DebugFileProvider : FileProvider()
