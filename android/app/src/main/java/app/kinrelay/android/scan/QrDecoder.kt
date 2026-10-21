package app.kinrelay.android.scan

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.LuminanceSource
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader

/**
 * Decodes one QR code from a camera frame, with ZXing only (no Google services, nothing leaves the phone).
 * Plain Kotlin on purpose: the unit tests feed it generated codes without a camera.
 * One instance per thread: the ZXing reader is not documented as thread-safe.
 */
class QrDecoder {
    private val reader = QRCodeReader()
    private val hints = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true,
    )

    /**
     * [luminance] is the Y plane of a camera frame: one byte per pixel, [rowStride] bytes per row (which may be wider than [width]).
     * Returns the text of the code, or null when the frame holds none (the usual case: most frames are not a QR code).
     */
    fun decodeLuminance(luminance: ByteArray, width: Int, height: Int, rowStride: Int = width): String? {
        require(width > 0 && height > 0 && rowStride >= width) { "bad frame ${width}x$height stride $rowStride" }
        if (luminance.size < rowStride * (height - 1) + width) return null
        return decode(PlanarYUVLuminanceSource(luminance, rowStride, height, 0, 0, width, height, false))
    }

    /** A bitmap's pixels (`Bitmap.getPixels`, ARGB). */
    fun decodeRgb(pixels: IntArray, width: Int, height: Int): String? {
        require(width > 0 && height > 0 && pixels.size >= width * height) { "bad image ${width}x$height" }
        return decode(RGBLuminanceSource(width, height, pixels))
    }

    // A light-on-dark code (a dark-mode screen) only reads when inverted, so a frame without a result gets a second try.
    private fun decode(source: LuminanceSource): String? = tryDecode(source) ?: tryDecode(source.invert())

    private fun tryDecode(source: LuminanceSource): String? = try {
        reader.decode(BinaryBitmap(HybridBinarizer(source)), hints).text
    } catch (_: ReaderException) {
        null // NotFoundException, ChecksumException, FormatException: no readable code in this frame
    } finally {
        reader.reset()
    }
}
