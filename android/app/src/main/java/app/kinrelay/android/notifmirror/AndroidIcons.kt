package app.kinrelay.android.notifmirror

import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffColorFilter
import android.graphics.drawable.Drawable
import android.service.notification.StatusBarNotification
import androidx.core.graphics.createBitmap
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * App icons for `notif.icon` (design 4.6): the launcher icon drawn to a 96 x 96 PNG (64, then 48, if the PNG is over [NotifWire.MAX_ICON_PNG_BYTES]),
 * cached in memory and in `cacheDir/mirror-icons/` under a name made from the package and its version code, so an app update draws it again. When
 * the app is not visible to Kin (no launcher entry: `<queries>` shows only apps with one) the notification's small icon is drawn white on a circle of
 * the notification's colour. Contact photos (the large icon) are never read. Nothing here is a setting or content.
 */
class AndroidIcons(context: Context) {
    private val context = context.applicationContext
    private val packages: PackageManager = this.context.packageManager
    private val dir = File(this.context.cacheDir, "mirror-icons")

    private class Entry(val version: Long, val icon: IconData?)

    private val memory = HashMap<String, Entry>()

    /** [disk] false (media, 2B.3): the icon is drawn and kept in memory only, nothing is read from or written to `cache/mirror-icons`. */
    @Synchronized
    fun forPackage(packageName: String, sbn: StatusBarNotification?, disk: Boolean = true): IconData? {
        val version = versionOf(packageName)
        memory[packageName]?.takeIf { it.version == version }?.let { return it.icon }
        // Drawing can fail in many ways (a broken adaptive icon, an out-of-memory bitmap): then the post goes out without an icon.
        val icon = (if (disk) fromDisk(packageName, version) else null) ?: try {
            draw(packageName, sbn)?.also { if (disk) toDisk(packageName, version, it) }
        } catch (_: Exception) {
            null
        } catch (_: OutOfMemoryError) {
            null
        }
        memory[packageName] = Entry(version, icon)
        return icon
    }

    private fun versionOf(packageName: String): Long = try {
        packages.getPackageInfo(packageName, 0).longVersionCode
    } catch (_: PackageManager.NameNotFoundException) {
        -1L
    } catch (_: SecurityException) {
        -1L
    }

    private fun draw(packageName: String, sbn: StatusBarNotification?): IconData? {
        val drawable: Drawable? = try {
            packages.getApplicationIcon(packageName)
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }
        if (drawable != null) return encode { size -> render(drawable, size) }
        // Not visible to Kin: the notification's own small icon, a white glyph on a circle of the notification's colour (grey when it has none).
        val notification = sbn?.notification ?: return null
        val glyph = runCatching { notification.smallIcon?.loadDrawable(context) }.getOrNull() ?: return null
        val color = notification.color.takeIf { it != 0 } ?: Color.GRAY
        return encode { size -> renderGlyph(glyph, color, size) }
    }

    private fun encode(render: (Int) -> Bitmap): IconData? {
        for (size in SIZES) {
            val bitmap = render(size)
            val out = ByteArrayOutputStream()
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
            bitmap.recycle()
            val png = out.toByteArray()
            if (png.size <= NotifWire.MAX_ICON_PNG_BYTES) return IconData(NotifWire.iconHash(png), png)
        }
        return null
    }

    private fun render(drawable: Drawable, size: Int): Bitmap {
        val bitmap = createBitmap(size, size)
        val d = drawable.constantState?.newDrawable()?.mutate() ?: drawable
        d.setBounds(0, 0, size, size)
        d.draw(Canvas(bitmap))
        return bitmap
    }

    private fun renderGlyph(glyph: Drawable, color: Int, size: Int): Bitmap {
        val bitmap = createBitmap(size, size)
        val canvas = Canvas(bitmap)
        canvas.drawCircle(size / 2f, size / 2f, size / 2f, Paint(Paint.ANTI_ALIAS_FLAG).apply { this.color = color })
        val d = glyph.constantState?.newDrawable()?.mutate() ?: glyph
        d.colorFilter = PorterDuffColorFilter(Color.WHITE, PorterDuff.Mode.SRC_IN)
        val inset = size / 5
        d.setBounds(inset, inset, size - inset, size - inset)
        d.draw(canvas)
        return bitmap
    }

    private fun fileFor(packageName: String, version: Long) = File(dir, Hashing.sha256Hex("$packageName@$version".toByteArray()).substring(0, 24) + ".png")

    private fun fromDisk(packageName: String, version: Long): IconData? {
        if (version < 0) return null
        val file = fileFor(packageName, version)
        val png = runCatching { file.takeIf { it.isFile && it.length() <= NotifWire.MAX_ICON_PNG_BYTES }?.readBytes() }.getOrNull() ?: return null
        return IconData(NotifWire.iconHash(png), png)
    }

    private fun toDisk(packageName: String, version: Long, icon: IconData) {
        if (version < 0) return
        runCatching {
            dir.mkdirs()
            fileFor(packageName, version).writeBytes(icon.png)
            // A small cache: the oldest files go beyond [MAX_FILES].
            val files = dir.listFiles()?.sortedBy { it.lastModified() }.orEmpty()
            files.take((files.size - MAX_FILES).coerceAtLeast(0)).forEach { it.delete() }
        }
    }

    private companion object {
        val SIZES = listOf(96, 64, 48)
        const val MAX_FILES = 100
    }
}
