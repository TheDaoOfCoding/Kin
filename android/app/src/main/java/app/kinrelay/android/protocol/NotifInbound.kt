package app.kinrelay.android.protocol

import app.kinrelay.android.notifmirror.Hashing
import app.kinrelay.android.notifmirror.Nid
import app.kinrelay.android.notifmirror.NotifWire
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * What the **PC** does with the phone's `notif.*` messages (design 3.3, 3.9; protocol.md "Notifications"): field validation and normalisation.
 * The real receiver is the desktop's. This copy stands for the PC in the shared vectors (`ReceiverRole.PC`, as for `complete`), so the Android tests
 * run the same `post[]` cases and the same sequences as the desktop does, and so the frames this app builds are proven acceptable to a receiver
 * that follows the spec. It is not used by the app itself.
 */
internal object NotifInbound {
    sealed interface Result {
        data class Ok(val data: JsonObject) : Result
        data class Bad(val message: String) : Result

        /** Dropped silently and counted (a bad icon: so that a bad icon can't start an error storm). */
        data object Dropped : Result
    }

    private val HEX32 = Regex("[0-9a-f]{32}")

    fun status(frame: JsonObject): Result {
        val enabled = Fields.boolean(frame["enabled"]) ?: return bad("enabled must be a boolean")
        val access = Fields.string(frame["access"])?.takeIf { it == "granted" || it == "missing" } ?: return bad("access must be \"granted\" or \"missing\"")
        val apps = Fields.integer(frame["apps"]) ?: return bad("apps must be a non-negative integer")
        val otp = optionalBoolean(frame["otpHidden"]) ?: return bad("otpHidden must be a boolean")
        return Result.Ok(buildJsonObject { put("enabled", enabled); put("access", access); put("apps", apps); put("otpHidden", otp.orFalse()) })
    }

    fun icon(frame: JsonObject): Result {
        val hash = Fields.string(frame["hash"])?.takeIf { HEX32.matches(it) } ?: return bad("hash must be 32 lowercase hex characters")
        val png = Fields.string(frame["png"]) ?: return bad("png must be a string")
        // The project's one encoding is base64url with no padding: a '=' is never written and is invalid when read.
        if ('=' in png) return Result.Dropped
        val bytes = try {
            java.util.Base64.getUrlDecoder().decode(png)
        } catch (_: IllegalArgumentException) {
            return Result.Dropped
        }
        if (bytes.size > MAX_ICON_BYTES || Hashing.sha256Hex(bytes).substring(0, 32) != hash) return Result.Dropped
        val side = pngSquareSide(bytes) ?: return Result.Dropped
        if (side !in 24..128) return Result.Dropped
        return Result.Ok(buildJsonObject { put("hash", hash); put("bytes", bytes.size) })
    }

    fun remove(frame: JsonObject): Result {
        val nid = Fields.string(frame["nid"])?.takeIf(Nid::isValid) ?: return bad("nid must be 32 lowercase hex characters")
        return Result.Ok(buildJsonObject { put("nid", nid) })
    }

    fun sync(frame: JsonObject): Result {
        val list = frame["nids"] as? JsonArray ?: return bad("nids must be an array")
        if (list.size > NotifWire.MAX_SYNC_NIDS) return bad("at most ${NotifWire.MAX_SYNC_NIDS} nids")
        val nids = list.map { Fields.string(it)?.takeIf(Nid::isValid) ?: return bad("every nid must be 32 lowercase hex characters") }
        return Result.Ok(buildJsonObject { put("nids", buildJsonArray { nids.forEach { add(JsonPrimitive(it)) } }) })
    }

    fun post(frame: JsonObject): Result {
        val nid = Fields.string(frame["nid"])?.takeIf(Nid::isValid) ?: return bad("nid must be 32 lowercase hex characters")
        val app = (Fields.string(frame["app"]) ?: return bad("app must be a string")).let { NotifWire.cleanText(it, NotifWire.APP_MAX) } ?: return bad("app is empty")
        val iconHash = when (val raw = Fields.present(frame["iconHash"])) {
            null -> null
            else -> Fields.string(raw)?.takeIf { HEX32.matches(it) } ?: return bad("iconHash must be 32 lowercase hex characters")
        }
        val title = text(frame["title"], NotifWire.TITLE_MAX) ?: return bad("title must be a string")
        val body = text(frame["text"], NotifWire.TEXT_MAX) ?: return bad("text must be a string")
        val bigText = text(frame["bigText"], NotifWire.BIG_TEXT_MAX) ?: return bad("bigText must be a string")
        val subText = text(frame["subText"], NotifWire.SUB_TEXT_MAX) ?: return bad("subText must be a string")
        val conversation = text(frame["conversation"], NotifWire.CONVERSATION_MAX) ?: return bad("conversation must be a string")
        val group = optionalBoolean(frame["group"]) ?: return bad("group must be a boolean")
        val silent = optionalBoolean(frame["silent"]) ?: return bad("silent must be a boolean")
        val sensitive = optionalBoolean(frame["sensitive"]) ?: return bad("sensitive must be a boolean")
        val redacted = optionalBoolean(frame["redacted"]) ?: return bad("redacted must be a boolean")
        val importance = when (val raw = Fields.present(frame["importance"])) {
            null -> 3
            else -> Fields.integer(raw)?.takeIf { it in 2..5 }?.toInt() ?: return bad("importance must be an integer from 2 to 5")
        }
        val messages: List<Pair<String?, String>>? = when (val raw = Fields.present(frame["messages"])) {
            null -> emptyList()
            is JsonArray -> {
                val cleaned = raw.map { entry ->
                    val obj = entry as? JsonObject ?: return bad("a message must be an object")
                    val messageText = Fields.string(obj["text"]) ?: return bad("a message needs a text string")
                    val sender = text(obj["sender"], NotifWire.SENDER_MAX) ?: return bad("a sender must be a string")
                    sender.value to NotifWire.cleanText(messageText, NotifWire.MESSAGE_TEXT_MAX)
                }
                cleaned.filter { it.second != null }.map { it.first to it.second!! }.takeLast(NotifWire.MAX_MESSAGES)
            }
            else -> return bad("messages must be an array")
        }
        val lines: List<String> = when (val raw = Fields.present(frame["lines"])) {
            null -> emptyList()
            is JsonArray -> raw.map { entry -> Fields.string(entry) ?: return bad("a line must be a string") }
                .mapNotNull { NotifWire.cleanText(it, NotifWire.LINE_MAX) }.takeLast(NotifWire.MAX_LINES)
            else -> return bad("lines must be an array")
        }
        // `code` is checked exactly as sent, of any type; anything else is ignored and never an error.
        val code = (frame["code"] as? JsonPrimitive)?.takeIf { it.isString }?.content?.takeIf(NotifWire::isValidCode)
        val reply: Pair<Int, String?>? = when (val raw = Fields.present(frame["reply"])) {
            null -> null
            is JsonObject -> {
                val i = actionIndex(raw["i"]) ?: return bad("reply.i must be an integer from 0 to 31")
                val hint = text(raw["hint"], NotifWire.HINT_MAX) ?: return bad("reply.hint must be a string")
                i to hint.value
            }
            else -> return bad("reply must be an object")
        }
        val actions: List<Pair<Int, String>> = when (val raw = Fields.present(frame["actions"])) {
            null -> emptyList()
            is JsonArray -> raw.map { entry ->
                val obj = entry as? JsonObject ?: return bad("an action must be an object")
                val i = actionIndex(obj["i"]) ?: return bad("action.i must be an integer from 0 to 31")
                val label = Fields.string(obj["label"]) ?: return bad("an action needs a label string")
                i to NotifWire.cleanText(label, NotifWire.LABEL_MAX)
            }.filter { it.second != null }.map { it.first to it.second!! }.take(NotifWire.MAX_ACTIONS)
            else -> return bad("actions must be an array")
        }
        val hasContent = title.value != null || body.value != null || bigText.value != null || messages.orEmpty().isNotEmpty() || lines.isNotEmpty()
        if (!hasContent && !redacted.orFalse()) return bad("a notification needs a title, text, bigText, messages or lines (or redacted)")
        return Result.Ok(buildJsonObject {
            put("group", group.orFalse()); put("importance", importance); put("silent", silent.orFalse())
            put("sensitive", sensitive.orFalse()); put("redacted", redacted.orFalse())
            put("nid", nid); put("app", app)
            iconHash?.let { put("iconHash", it) }
            title.value?.let { put("title", it) }; body.value?.let { put("text", it) }; bigText.value?.let { put("bigText", it) }
            subText.value?.let { put("subText", it) }; conversation.value?.let { put("conversation", it) }
            if (!messages.isNullOrEmpty()) put("messages", buildJsonArray {
                messages.forEach { (sender, t) -> add(buildJsonObject { sender?.let { put("sender", it) }; put("text", t) }) }
            })
            if (lines.isNotEmpty()) put("lines", buildJsonArray { lines.forEach { add(JsonPrimitive(it)) } })
            code?.let { put("code", it) }
            reply?.let { (i, hint) -> put("reply", buildJsonObject { put("i", i); hint?.let { put("hint", it) } }) }
            if (actions.isNotEmpty()) put("actions", buildJsonArray { actions.forEach { (i, label) -> add(buildJsonObject { put("i", i); put("label", label) }) } })
        })
    }

    /** A cleaned optional string: absent or `null` gives a [Cleaned] with no value, a non-string gives null (the caller says `bad_request`). */
    private class Cleaned(val value: String?)

    private fun text(element: JsonElement?, max: Int): Cleaned? {
        val raw = Fields.present(element) ?: return Cleaned(null)
        val string = Fields.string(raw) ?: return null
        return Cleaned(NotifWire.cleanText(string, max))
    }

    private fun actionIndex(element: JsonElement?): Int? = Fields.integer(element)?.takeIf { it in 0..NotifWire.MAX_ACTION_INDEX }?.toInt()

    /** `null` element is absent (a [Tri] with no value); a non-boolean is invalid (null result). */
    private class Tri(val value: Boolean?) {
        fun orFalse() = value == true
    }

    private fun optionalBoolean(element: JsonElement?): Tri? {
        val raw = Fields.present(element) ?: return Tri(null)
        return Fields.boolean(raw)?.let { Tri(it) }
    }

    private fun bad(message: String): Result.Bad = Result.Bad(message)

    private const val MAX_ICON_BYTES = 32_768

    /** The width of a PNG whose width and height are equal (read from the IHDR chunk), or null when it isn't a PNG or isn't square. */
    private fun pngSquareSide(bytes: ByteArray): Int? {
        if (bytes.size < 24) return null
        val signature = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
        for (i in signature.indices) if (bytes[i] != signature[i]) return null
        if (String(bytes, 12, 4, Charsets.US_ASCII) != "IHDR") return null
        fun int(at: Int) = ((bytes[at].toInt() and 0xFF) shl 24) or ((bytes[at + 1].toInt() and 0xFF) shl 16) or ((bytes[at + 2].toInt() and 0xFF) shl 8) or (bytes[at + 3].toInt() and 0xFF)
        val width = int(16)
        val height = int(20)
        return width.takeIf { it == height }
    }
}
