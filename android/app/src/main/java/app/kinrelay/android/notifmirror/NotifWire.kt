package app.kinrelay.android.notifmirror

import app.kinrelay.android.protocol.Fields
import app.kinrelay.android.protocol.LinkWire
import app.kinrelay.android.protocol.Utf8
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.util.Base64

/** `notif.*` messages (design 2B.2, section 3). Notification messages are ephemeral: no ack, no de-duplication, never queued. */
object NotifWire {
    /** The `hello` capability: "I understand `notif.dismiss` and `notif.action`, and I can mirror". */
    const val CAPABILITY = "notif"

    const val STATUS = "notif.status"
    const val ICON = "notif.icon"
    const val POST = "notif.post"
    const val REMOVE = "notif.remove"
    const val SYNC = "notif.sync"
    const val DISMISS = "notif.dismiss"
    const val ACTION = "notif.action"

    /** Field limits in code points (the receiver cleans to the same numbers). */
    const val APP_MAX = 40
    const val TITLE_MAX = 150
    const val TEXT_MAX = 500
    const val BIG_TEXT_MAX = 1000
    const val SUB_TEXT_MAX = 80
    const val CONVERSATION_MAX = 100
    const val SENDER_MAX = 60
    const val MESSAGE_TEXT_MAX = 300
    const val LINE_MAX = 150
    const val HINT_MAX = 60
    const val LABEL_MAX = 30
    const val REPLY_TEXT_MAX = 2000
    const val MAX_MESSAGES = 5
    const val MAX_LINES = 5
    const val MAX_ACTIONS = 3
    const val MAX_SYNC_NIDS = 200
    const val MAX_ACTION_INDEX = 31

    /** Senders stay under this; the receiver refuses a frame over 32 KiB with `too_large`. */
    const val SENDER_FRAME_TARGET = 16 * 1024

    /** `notif.icon` PNGs this app sends stay under this, so the base64 form is under 16 KiB (the spec allows 32,768 bytes, 43,691 base64url characters, which fits the 48 KiB limit of a `notif.icon` frame; this app keeps its own frame small). */
    const val MAX_ICON_PNG_BYTES = 12_000

    private val CODE = Regex("[0-9A-Z-]{4,10}")

    /** The `code` rule of the PC: 4 to 10 characters from `0-9A-Z-`. */
    fun isValidCode(code: String): Boolean = CODE.matches(code)

    fun cleanText(input: String?, max: Int): String? = input?.let { LinkWire.cleanText(it, max) }

    /**
     * A hint or a button label in its toast form (protocol.md, `notif.action`; vectors `toastLabel[]`): U+2028, U+2029, U+FFFE and U+FFFF become U+0020,
     * the ends are trimmed (spaces only), nothing else changes. The PC shows and sends this form, so the phone compares in it.
     */
    fun toastForm(input: String): String =
        input.map { if (it == '\u2028' || it == '\u2029' || it == '\uFFFE' || it == '\uFFFF') ' ' else it }.joinToString("").trim(' ')

    /** `clean(toastForm(input), max)`: the label as the PC has it; null when nothing is left (a button with such a label is not offered). */
    fun cleanLabel(input: String?, max: Int): String? = input?.let { cleanText(toastForm(it), max) }

    fun status(id: String, enabled: Boolean, accessGranted: Boolean, apps: Int, otpHidden: Boolean): String = buildJsonObject {
        put("type", STATUS)
        put("id", id)
        put("enabled", enabled)
        put("access", if (accessGranted) "granted" else "missing")
        put("apps", apps)
        put("otpHidden", otpHidden)
    }.toString()

    fun iconHash(png: ByteArray): String = Hashing.sha256Hex(png).substring(0, 32)

    fun icon(id: String, png: ByteArray): String = buildJsonObject {
        put("type", ICON)
        put("id", id)
        put("hash", iconHash(png))
        put("png", Base64.getUrlEncoder().withoutPadding().encodeToString(png))
    }.toString()

    fun remove(id: String, nid: String): String = buildJsonObject {
        put("type", REMOVE)
        put("id", id)
        put("nid", nid)
    }.toString()

    fun sync(id: String, nids: List<String>): String = buildJsonObject {
        put("type", SYNC)
        put("id", id)
        put("nids", buildJsonArray { nids.take(MAX_SYNC_NIDS).forEach { add(JsonPrimitive(it)) } })
    }.toString()

    fun post(id: String, post: WirePost): String = post.toJson(id).toString()

    /** A valid `notif.action` from the PC. [text] is present if and only if the action is a reply. */
    data class ActionRequest(val id: String, val nid: String, val index: Int, val label: String, val text: String?)

    /** `notif.dismiss`: the `nid`, or null when the message is invalid. */
    fun parseDismiss(frame: JsonObject): String? = Fields.string(frame["nid"])?.takeIf(Nid::isValid)

    /** `notif.action`: the request, or null when a field is missing or has a wrong type. Over-long strings are cut, never refused. */
    fun parseAction(id: String, frame: JsonObject): ActionRequest? {
        val nid = Fields.string(frame["nid"])?.takeIf(Nid::isValid) ?: return null
        val index = Fields.integer(frame["i"])?.takeIf { it in 0..MAX_ACTION_INDEX }?.toInt() ?: return null
        val label = Fields.string(frame["label"]) ?: return null
        val rawText = Fields.present(frame["text"])
        val text = if (rawText == null) null else (Fields.string(rawText) ?: return null)
        val cleanedText = text?.let { replyText(it) ?: return null }
        return ActionRequest(id, nid, index, cleanLabel(label, LABEL_MAX).orEmpty(), cleanedText)
    }

    /**
     * A reply as the receiver takes it (protocol.md, `notif.action`): trimmed with the Unicode `White_Space` set (inner line breaks stay), and 1 to 2,000
     * code points. Empty after trimming, or longer, is not a reply: null (the receiver answers `bad_request`). It is **not cut**.
     */
    fun replyText(raw: String): String? {
        val trimmed = raw.trim { app.kinrelay.android.protocol.UrlRules.isWhitespace(it) }
        if (trimmed.isEmpty()) return null
        return trimmed.takeIf { it.codePointCount(0, it.length) <= REPLY_TEXT_MAX }
    }
}

/** One MessagingStyle message on the wire. [sender] null means "You" (the user's own message). */
data class WireMessage(val sender: String?, val text: String)

data class WireReply(val index: Int, val hint: String?)

data class WireAction(val index: Int, val label: String)

/**
 * The fields of one `notif.post` (design 3.3), already cleaned and cut. [toJson] writes only what is present; [contentHash] says whether two posts
 * show the same thing to the user (so an identical one is not sent again).
 */
data class WirePost(
    val nid: String,
    val app: String,
    val iconHash: String? = null,
    val title: String? = null,
    val text: String? = null,
    val bigText: String? = null,
    val subText: String? = null,
    val conversation: String? = null,
    val group: Boolean = false,
    val messages: List<WireMessage> = emptyList(),
    val lines: List<String> = emptyList(),
    val importance: Int = 3,
    val silent: Boolean = false,
    val sensitive: Boolean = false,
    val redacted: Boolean = false,
    val code: String? = null,
    val reply: WireReply? = null,
    val actions: List<WireAction> = emptyList(),
) {
    fun toJson(id: String): JsonObject = buildJsonObject {
        put("type", NotifWire.POST)
        put("id", id)
        put("nid", nid)
        put("app", app)
        iconHash?.let { put("iconHash", it) }
        title?.let { put("title", it) }
        text?.let { put("text", it) }
        bigText?.let { put("bigText", it) }
        subText?.let { put("subText", it) }
        conversation?.let { put("conversation", it) }
        if (group) put("group", true)
        if (messages.isNotEmpty()) put("messages", buildJsonArray {
            messages.forEach { m -> add(buildJsonObject { m.sender?.let { s -> put("sender", s) }; put("text", m.text) }) }
        })
        if (lines.isNotEmpty()) put("lines", buildJsonArray { lines.forEach { add(JsonPrimitive(it)) } })
        if (importance != 3) put("importance", importance)
        if (silent) put("silent", true)
        if (sensitive) put("sensitive", true)
        if (redacted) put("redacted", true)
        code?.let { put("code", it) }
        reply?.let { r -> put("reply", buildJsonObject { put("i", r.index); r.hint?.let { h -> put("hint", h) } }) }
        if (actions.isNotEmpty()) put("actions", buildJsonArray {
            actions.forEach { a -> add(buildJsonObject { put("i", a.index); put("label", a.label) }) }
        })
    }

    /** The post a user would see as unchanged: everything but `nid` and the icon. A hash, so no content is kept in memory beyond the post itself. */
    fun contentHash(): String = Hashing.sha256Hex(copy(nid = "", iconHash = null).toJson("x").toString().toByteArray(Charsets.UTF_8)).substring(0, 16)

    fun frameSize(id: String = "00000000-0000-7000-8000-000000000000"): Int = Utf8.size(toJson(id).toString())

    /**
     * The same post, made smaller until its frame is at most [limit] bytes: the big text goes first, then the inbox lines from the oldest, then the
     * oldest messages, then the body text is cut. A post can't reach 32 KiB with these limits; this keeps senders under 16 KiB (design 3.3).
     */
    fun fit(limit: Int = NotifWire.SENDER_FRAME_TARGET): WirePost {
        var post = this
        while (post.frameSize() > limit) {
            post = when {
                post.bigText != null -> post.copy(bigText = null)
                post.lines.size > 1 -> post.copy(lines = post.lines.drop(1))
                post.messages.size > 1 -> post.copy(messages = post.messages.drop(1))
                post.text != null && post.text.length > 100 -> post.copy(text = post.text.take(post.text.length / 2).let { if (it.last().isHighSurrogate()) it.dropLast(1) else it })
                else -> return post
            }
        }
        return post
    }
}

