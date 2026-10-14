package app.kinrelay.android.identity

import java.io.ByteArrayOutputStream

/** The few DER encodings an X.509 certificate needs. */
internal object Der {
    const val TAG_INTEGER = 0x02
    const val TAG_BIT_STRING = 0x03
    const val TAG_OID = 0x06
    const val TAG_UTF8_STRING = 0x0C
    const val TAG_UTC_TIME = 0x17
    const val TAG_GENERALIZED_TIME = 0x18
    const val TAG_SEQUENCE = 0x30
    const val TAG_SET = 0x31
    const val TAG_CONTEXT_0 = 0xA0

    fun tlv(tag: Int, content: ByteArray): ByteArray {
        val out = ByteArrayOutputStream()
        out.write(tag)
        writeLength(out, content.size)
        out.write(content)
        return out.toByteArray()
    }

    fun sequence(vararg parts: ByteArray) = tlv(TAG_SEQUENCE, concat(parts))

    fun set(vararg parts: ByteArray) = tlv(TAG_SET, concat(parts))

    fun explicit0(content: ByteArray) = tlv(TAG_CONTEXT_0, content)

    fun smallInteger(value: Int): ByteArray {
        require(value in 0..127)
        return tlv(TAG_INTEGER, byteArrayOf(value.toByte()))
    }

    /** A positive INTEGER from big-endian magnitude bytes: minimal length, never negative. */
    fun positiveInteger(magnitude: ByteArray): ByteArray {
        var start = 0
        while (start < magnitude.size - 1 && magnitude[start] == 0.toByte()) start++
        val trimmed = magnitude.copyOfRange(start, magnitude.size)
        val content = if (trimmed[0].toInt() and 0x80 != 0) byteArrayOf(0) + trimmed else trimmed
        return tlv(TAG_INTEGER, content)
    }

    fun oid(dotted: String): ByteArray {
        val arcs = dotted.split('.').map { it.toLong() }
        val out = ByteArrayOutputStream()
        out.write((arcs[0] * 40 + arcs[1]).toInt())
        for (arc in arcs.drop(2)) writeBase128(out, arc)
        return tlv(TAG_OID, out.toByteArray())
    }

    fun utf8String(text: String) = tlv(TAG_UTF8_STRING, text.toByteArray(Charsets.UTF_8))

    fun bitString(bytes: ByteArray) = tlv(TAG_BIT_STRING, byteArrayOf(0) + bytes)

    fun concat(parts: Array<out ByteArray>): ByteArray {
        val out = ByteArrayOutputStream()
        for (part in parts) out.write(part)
        return out.toByteArray()
    }

    private fun writeLength(out: ByteArrayOutputStream, length: Int) {
        when {
            length < 0x80 -> out.write(length)
            length < 0x100 -> { out.write(0x81); out.write(length) }
            length < 0x10000 -> { out.write(0x82); out.write(length shr 8); out.write(length and 0xFF) }
            else -> error("DER content too long: $length")
        }
    }

    private fun writeBase128(out: ByteArrayOutputStream, value: Long) {
        var shift = 63 - java.lang.Long.numberOfLeadingZeros(value or 1) // highest set bit
        shift -= shift % 7
        while (shift > 0) {
            out.write(((value shr shift) and 0x7F).toInt() or 0x80)
            shift -= 7
        }
        out.write((value and 0x7F).toInt())
    }
}
