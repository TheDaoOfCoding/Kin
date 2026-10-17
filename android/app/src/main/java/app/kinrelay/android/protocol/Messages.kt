package app.kinrelay.android.protocol

import app.kinrelay.android.media.MediaWire
import app.kinrelay.android.notifmirror.NotifWire
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.security.SecureRandom

const val PROTOCOL_VERSION = 1
const val MAX_FRAME_BYTES = 1_048_576

/** Files in one `offer`; a bigger one is answered with `too_large`. The PC splits its sends at the same number (protocol.md, Files: PC -> phone). */
const val MAX_OFFER_FILES = 800

/** A text larger than this (UTF-8, wire form) is sent as a `.txt` file instead (the sender size threshold). */
const val MAX_TEXT_BYTES = 128 * 1024

/**
 * A `hello` the receiver accepted. [capabilities] are the extra features the peer says it understands (`link` from 0.2.0); empty for a 0.1.x
 * peer and for a `capabilities` that is not an array. They are the peer's for this connection only.
 */
data class Hello(val id: String, val name: String, val platform: String, val appVersion: String, val capabilities: Set<String> = emptySet()) {
    /** A `link` may go to this peer; to any other peer the same item goes as a `text` (protocol.md, "Versioning"). */
    val understandsLink: Boolean get() = LinkWire.CAPABILITY_LINK in capabilities

    /** The peer's latest `hello` lists `notif` (2B.2): it understands the `notif.*` messages. The PC lists it only while "Show phone notifications" is on. */
    val understandsNotif: Boolean get() = NotifWire.CAPABILITY in capabilities

    /** The peer's latest `hello` lists `media` (2B.3): it understands the `media.*` messages. Every 0.2.0 build lists it, whatever its switches say (they travel in `media.status`). */
    val understandsMedia: Boolean get() = MediaWire.CAPABILITY in capabilities
}

object ErrorCodes {
    const val BAD_REQUEST = "bad_request"
    const val UNSUPPORTED_TYPE = "unsupported_type"
    const val UNSUPPORTED_VERSION = "unsupported_version"
    const val DISK_FULL = "disk_full"
    const val TOO_LARGE = "too_large"
}

/** Builders for the frames this app sends. Output is compact JSON, as senders must write it. */
object Frames {
    /** [capabilities]: what this app understands beyond protocol 1; `link` since 2B.1 (the sender of a `link` asks the peer's `hello` for it), `notif` since 2B.2 and `media` since 2B.3 (always listed on the phone). */
    fun hello(id: String, name: String, platform: String, appVersion: String, capabilities: List<String> = listOf(LinkWire.CAPABILITY_LINK, NotifWire.CAPABILITY, MediaWire.CAPABILITY)): String = obj {
        put("type", "hello")
        put("id", id)
        put("protocol", PROTOCOL_VERSION)
        put("name", name)
        put("platform", platform)
        put("appVersion", appVersion)
        if (capabilities.isNotEmpty()) put("capabilities", buildJsonArray { capabilities.forEach { add(JsonPrimitive(it)) } })
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

    /**
     * An explicit send. The text goes out in its wire form ([wireText]). [sensitive] is written only when true (an optional
     * field is left out when absent). Returns null when the serialized frame is over 1 MiB: the sender switches to a `.txt`
     * file long before that ([MAX_TEXT_BYTES]), so this is only a guard.
     */
    fun text(id: String, text: String, isUrl: Boolean, sensitive: Boolean = false): String? {
        val frame = buildJsonObject {
            put("type", "text")
            put("id", id)
            put("text", wireText(text))
            put("isUrl", isUrl)
            if (sensitive) put("sensitive", true)
        }.toString()
        return frame.takeIf { Utf8.size(it) <= MAX_FRAME_BYTES }
    }

    /**
     * An explicit send of a web address (a shared page). The caller has checked [LinkWire.isLinkUrl]; [title] is cleaned and cut here, and left out when
     * empty. Returns null when the serialized frame is over 1 MiB (it can't be: a URL is at most 8 KiB and a title 300 characters).
     */
    fun link(id: String, url: String, title: String?, sourceApp: String? = null): String? {
        val frame = buildJsonObject {
            put("type", "link")
            put("id", id)
            put("url", url)
            title?.let(LinkWire::cleanTitle)?.let { put("title", it) }
            sourceApp?.let(LinkWire::cleanSourceApp)?.let { put("sourceApp", it) }
        }.toString()
        return frame.takeIf { Utf8.size(it) <= MAX_FRAME_BYTES }
    }

    /** The phone tells the PC every file of the transfer is saved. */
    fun complete(id: String, transferId: String): String = obj {
        put("type", "complete")
        put("id", id)
        put("transferId", transferId)
    }

    fun cancel(id: String, transferId: String, reason: String): String = obj {
        put("type", "cancel")
        put("id", id)
        put("transferId", transferId)
        put("reason", reason)
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

    /**
     * What a text looks like on the wire: the wire uses LF only, so CRLF becomes LF and a lone CR is left alone, and an
     * unpaired surrogate becomes U+FFFD. Loop-guard hashes are taken on this form.
     */
    fun wireText(text: String): String = sanitize(text.replace("\r\n", "\n"))

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
