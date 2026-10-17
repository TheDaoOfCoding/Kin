package app.kinrelay.android.media

import android.graphics.Bitmap
import androidx.core.graphics.scale
import java.io.ByteArrayOutputStream

/**
 * The ladder of design 4.3: the longest side at most 256 px, JPEG at quality 80, 70, 60, 50, then side 192 and 128 the same way, until the result is at most 24,576 bytes. Even 128 px at 50
 * too big, or a picture under 16 px, means no art. Pure ([encode] is told the side and the quality), so the caps are tested on the JVM.
 */
object ArtLadder {
    val SIDES = listOf(MediaWire.ART_MAX_SIDE, 192, 128)
    val QUALITIES = listOf(80, 70, 60, 50)

    /** The first encoding that fits [MediaWire.IMAGE_MAX_BYTES], or null. [width] and [height] are the source's; the sides tried never exceed them (a small picture is not enlarged). */
    fun firstFitting(width: Int, height: Int, encode: (width: Int, height: Int, quality: Int) -> ByteArray?): ByteArray? {
        val longest = maxOf(width, height)
        if (minOf(width, height) < MediaWire.IMAGE_MIN_SIDE) return null
        var lastSide = -1
        for (candidate in SIDES) {
            val side = minOf(candidate, longest)
            if (side == lastSide) continue
            lastSide = side
            val w = maxOf(MediaWire.IMAGE_MIN_SIDE, Math.round(width.toLong() * side / longest.toFloat()))
            val h = maxOf(MediaWire.IMAGE_MIN_SIDE, Math.round(height.toLong() * side / longest.toFloat()))
            for (quality in QUALITIES) {
                val bytes = encode(w, h, quality) ?: continue
                if (bytes.size <= MediaWire.IMAGE_MAX_BYTES) return bytes
            }
        }
        return null
    }
}

/** Bitmap to a `media.image` JPEG for `art` (design 4.3). The source bitmap is never kept; the caller drops its reference. */
class ArtworkEncoder {
    fun encode(bitmap: Bitmap): MediaImageData? {
        if (bitmap.width <= 0 || bitmap.height <= 0) return null
        val bytes = try {
            ArtLadder.firstFitting(bitmap.width, bitmap.height) { w, h, quality -> jpeg(bitmap, w, h, quality) }
        } catch (_: Exception) {
            null
        } catch (_: OutOfMemoryError) {
            null
        } ?: return null
        return MediaImageData(MediaWire.imageHash(bytes), MediaWire.KIND_ART, bytes)
    }

    private fun jpeg(source: Bitmap, width: Int, height: Int, quality: Int): ByteArray {
        val scaled = if (width == source.width && height == source.height) source else source.scale(width, height)
        try {
            val out = ByteArrayOutputStream()
            scaled.compress(Bitmap.CompressFormat.JPEG, quality, out)
            return out.toByteArray()
        } finally {
            if (scaled !== source) scaled.recycle()
        }
    }
}
