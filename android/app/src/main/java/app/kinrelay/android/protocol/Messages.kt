package app.kinrelay.android.protocol

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.security.SecureRandom

const val PROTOCOL_VERSION = 1
const val MAX_FRAME_BYTES = 1_048_576

/** A `hello` the receiver accepted. */
data class Hello(val id: String, val name: String, val platform: String, val appVersion: String)

object ErrorCodes {
    const val BAD_REQUEST = "bad_request"
    const val UNSUPPORTED_TYPE = "unsupported_type"
    const val UNSUPPORTED_VERSION = "unsupported_version"
}

/** Builders for the frames this app sends. Output is compact JSON, as senders must write it. */
object Frames {
    fun hello(id: String, name: String, platform: String, appVersion: String): String = obj {
        put("type", "hello")
        put("id", id)
        put("protocol", PROTOCOL_VERSION)
        put("name", name)
        put("platform", platform)
        put("appVersion", appVersion)
    }

    fun ack(ref: String): String = obj {
        put("type", "ack")
        put("ref", ref)
    }

    fun error(code: String, message: String, ref: String?): String = obj {
        put("type", "error")
        if (ref != null) put("ref", ref)
        put("code", code)
        put("message", message)
    }

    fun ping(id: String, sentAt: Long): String = obj {
        put("type", "ping")
        put("id", id)
        put("sentAt", sentAt)
    }

    fun unpair(id: String): String = obj {
        put("type", "unpair")
        put("id", id)
    }

    private fun obj(block: kotlinx.serialization.json.JsonObjectBuilder.() -> Unit): String {
        val frame: JsonObject = buildJsonObject(block)
        val text = frame.toString()
        check(Utf8.size(text) <= MAX_FRAME_BYTES) { "outgoing frame is over 1 MiB" }
        return text
    }

    /** Senders replace an unpaired surrogate with U+FFFD before serializing. */
    fun sanitize(text: String): String {
        val out = StringBuilder(text.length)
        var i = 0
        while (i < text.length) {
            val c = text[i]
            when {
                Character.isHighSurrogate(c) && i + 1 < text.length && Character.isLowSurrogate(text[i + 1]) -> {
                    out.append(c).append(text[i + 1]); i++
                }
                Character.isSurrogate(c) -> out.append('�')
                else -> out.append(c)
            }
            i++
        }
        return out.toString()
    }
}

object Utf8 {
    /** Size in UTF-8 bytes without encoding the text. An unpaired surrogate counts as 3 (U+FFFD). */
    fun size(text: String): Int {
        var bytes = 0
        var i = 0
        while (i < text.length) {
            val c = text[i]
            bytes += when {
                c.code < 0x80 -> 1
                c.code < 0x800 -> 2
                Character.isHighSurrogate(c) && i + 1 < text.length && Character.isLowSurrogate(text[i + 1]) -> { i++; 4 }
                else -> 3
            }
            i++
        }
        return bytes
    }
}

/** Message IDs: UUIDv7, lowercase, hyphenated. */
object MessageIds {
    private val random = SecureRandom()

    fun newId(nowMillis: Long = System.currentTimeMillis()): String {
        val bytes = ByteArray(16).also(random::nextBytes)
        for (i in 0 until 6) bytes[i] = (nowMillis shr (8 * (5 - i))).toByte()
        bytes[6] = ((bytes[6].toInt() and 0x0F) or 0x70).toByte()
        bytes[8] = ((bytes[8].toInt() and 0x3F) or 0x80).toByte()
        val hex = bytes.joinToString("") { "%02x".format(it) }
        return "${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-${hex.substring(16, 20)}-${hex.substring(20)}"
    }
}
