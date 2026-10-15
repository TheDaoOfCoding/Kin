package app.kinrelay.android.scan

import app.kinrelay.android.pairing.PairingParse
import app.kinrelay.android.pairing.PairingUrl
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.common.BitMatrix
import com.google.zxing.oned.Code128Writer
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The scanner's decoder on QR codes made here by the ZXing encoder: a real pairing link, drawn into camera-style frames. */
class QrDecoderTest {
    private val decoder = QrDecoder()

    private val pairingLink =
        "kinrelay://pair?v=1&a=192.168.1.20:47821,10.0.0.5:47821&id=yetOwJHO-3liWauefLv2Zs8_NNIkRYvy7_i5RBG-fP8&tk=AAECAwQFBgcICQoLDA0ODw&n=Desk%20PC"

    /** A grey frame ([width] x [height], one byte per pixel) with the code drawn at [scale] pixels per module. */
    private fun frame(
        text: String, width: Int = 640, height: Int = 480, scale: Int = 5, dark: Int = 0, light: Int = 255, background: Int = 255,
        level: ErrorCorrectionLevel = ErrorCorrectionLevel.M,
    ): ByteArray {
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0, mapOf(EncodeHintType.ERROR_CORRECTION to level, EncodeHintType.MARGIN to 0))
        val pixels = ByteArray(width * height) { background.toByte() }
        val size = matrix.width * scale
        val left = (width - size) / 2
        val top = (height - size) / 2
        check(left >= 0 && top >= 0) { "code ${size}px does not fit ${width}x$height" }
        for (y in 0 until size) for (x in 0 until size) {
            pixels[(top + y) * width + left + x] = (if (matrix.get(x / scale, y / scale)) dark else light).toByte()
        }
        return pixels
    }

    private fun withStride(pixels: ByteArray, width: Int, height: Int, stride: Int): ByteArray {
        val out = ByteArray(stride * height) { 0x55 } // the padding at the end of each row is junk, as on a camera
        for (y in 0 until height) System.arraycopy(pixels, y * width, out, y * stride, width)
        return out
    }

    private fun rotate90(pixels: ByteArray, width: Int, height: Int): ByteArray {
        val out = ByteArray(pixels.size)
        for (y in 0 until height) for (x in 0 until width) out[x * height + (height - 1 - y)] = pixels[y * width + x]
        return out
    }

    @Test
    fun decodesARealPairingLinkFromAFrame() {
        val text = decoder.decodeLuminance(frame(pairingLink), 640, 480)
        assertEquals(pairingLink, text)
        val parsed = PairingUrl.parse(text!!)
        assertTrue("the decoded text is a valid pairing link (got $parsed)", parsed is PairingParse.Ok)
        assertEquals("Desk PC", (parsed as PairingParse.Ok).target.name)
        assertEquals(2, parsed.target.addresses.size)
    }

    @Test
    fun decodesEveryCanonicalPairingUrlOfTheSharedVectors() {
        val cases = Vectors.load("pairing-urls.json").cases("valid").filter {
            it["canonical"]?.toString() == "true"
        }
        assertTrue(cases.size >= 5)
        for (case in cases) {
            val url = case.getValue("url").str()
            val text = decoder.decodeLuminance(frame(url, 800, 800, scale = 6), 800, 800)
            assertEquals(case.getValue("name").str(), url, text)
        }
    }

    @Test
    fun decodesTheHttpsLinkForm() {
        val link = "https://kin.example/pair#v=1&a=192.168.1.20:47821&id=yetOwJHO-3liWauefLv2Zs8_NNIkRYvy7_i5RBG-fP8&tk=AAECAwQFBgcICQoLDA0ODw"
        assertEquals(link, decoder.decodeLuminance(frame(link), 640, 480))
    }

    @Test
    fun decodesFromAFrameWhoseRowsAreWiderThanItsWidth() {
        // Camera Y planes pad each row (rowStride > width); reading them as tightly packed would shear the code.
        val width = 640
        val height = 480
        val padded = withStride(frame(pairingLink, width, height), width, height, stride = 704)
        assertEquals(pairingLink, decoder.decodeLuminance(padded, width, height, rowStride = 704))
    }

    @Test
    fun decodesARotatedFrame() {
        // The sensor of most phones is turned 90 degrees against the screen.
        val rotated = rotate90(frame(pairingLink, 640, 480), 640, 480)
        assertEquals(pairingLink, decoder.decodeLuminance(rotated, 480, 640))
    }

    @Test
    fun decodesAnUpsideDownFrame() {
        val frame = frame(pairingLink, 640, 480)
        assertEquals(pairingLink, decoder.decodeLuminance(frame.reversedArray(), 640, 480))
    }

    @Test
    fun decodesALightCodeOnADarkBackground() {
        // A phone or PC screen in dark mode can show the code inverted.
        val inverted = frame(pairingLink, dark = 255, light = 0, background = 0)
        assertEquals(pairingLink, decoder.decodeLuminance(inverted, 640, 480))
    }

    @Test
    fun decodesALowContrastCodeAndASmallOne() {
        assertEquals(pairingLink, decoder.decodeLuminance(frame(pairingLink, dark = 70, light = 190, background = 190), 640, 480))
        // 3 pixels per module: about what a code at arm's length gives in a 720p frame.
        assertEquals(pairingLink, decoder.decodeLuminance(frame(pairingLink, 1280, 720, scale = 3), 1280, 720))
    }

    @Test
    fun decodesFromBitmapPixels() {
        val grey = frame(pairingLink)
        val argb = IntArray(grey.size) { val g = grey[it].toInt() and 0xFF; (0xFF shl 24) or (g shl 16) or (g shl 8) or g }
        assertEquals(pairingLink, decoder.decodeRgb(argb, 640, 480))
    }

    @Test
    fun anEmptyOrNoisyFrameHoldsNoCode() {
        assertNull(decoder.decodeLuminance(ByteArray(640 * 480) { 128.toByte() }, 640, 480))
        val noise = java.util.Random(7)
        assertNull(decoder.decodeLuminance(ByteArray(640 * 480).also { noise.nextBytes(it) }, 640, 480))
    }

    @Test
    fun aTruncatedBufferIsNotAnError() {
        assertNull(decoder.decodeLuminance(ByteArray(100), 640, 480))
    }

    @Test
    fun aBarcodeThatIsNotAQrCodeIsIgnored() {
        val bars: BitMatrix = Code128Writer().encode("kinrelay", BarcodeFormat.CODE_128, 300, 60)
        val width = 640
        val height = 480
        val pixels = ByteArray(width * height) { 255.toByte() }
        for (y in 0 until 60) for (x in 0 until bars.width) pixels[(100 + y) * width + 100 + x] = (if (bars.get(x, y)) 0 else 255).toByte()
        assertNull(decoder.decodeLuminance(pixels, width, height))
    }

    @Test
    fun theDecoderKeepsWorkingAfterEmptyFrames() {
        // The camera sends many frames without a code, then one with it: nothing may be left over from the misses.
        repeat(20) { assertNull(decoder.decodeLuminance(ByteArray(640 * 480) { 200.toByte() }, 640, 480)) }
        assertEquals(pairingLink, decoder.decodeLuminance(frame(pairingLink), 640, 480))
        assertNull(decoder.decodeLuminance(ByteArray(640 * 480) { 200.toByte() }, 640, 480))
    }
}
