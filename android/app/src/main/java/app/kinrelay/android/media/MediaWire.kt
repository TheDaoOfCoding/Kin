package app.kinrelay.android.media

import app.kinrelay.android.notifmirror.Hashing
import app.kinrelay.android.protocol.LinkWire
import app.kinrelay.android.protocol.Utf8
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.util.Base64

/** `media.*` messages (design 2B.3, section 3). Like `notif.*` they are ephemeral: no ack, no de-duplication, never queued, never re-sent. */
object MediaWire {
    /** The `hello` capability: "I understand `media.*`". Every 0.2.0 build lists it (design decision 4). */
    const val CAPABILITY = "media"

    const val STATUS = "media.status"
    const val STATE = "media.state"
    const val IMAGE = "media.image"
    const val CLEAR = "media.clear"
    const val VOLUME = "media.volume"
    const val CMD = "media.cmd"

    val TYPES = setOf(STATUS, STATE, IMAGE, CLEAR, VOLUME, CMD)

    /** Field limits in code points (the receiver cleans to the same numbers). */
    const val APP_MAX = 40
    const val TITLE_MAX = 150
    const val ARTIST_MAX = 100
    const val ALBUM_MAX = 100

    const val MAX_DURATION_MS = 86_400_000L
    const val DEFAULT_RATE_MILLI = 1000
    const val MAX_RATE_MILLI = 4000
    const val MAX_ACTIONS = 8
    const val MAX_SEEK_MS = 86_400_000L
    const val MAX_DELTA = 100

    /** Images: at most 24,576 bytes, longest side 256 px for `art` and 128 for `icon`, at least 16 px. */
    const val IMAGE_MAX_BYTES = 24_576
    const val ART_MAX_SIDE = 256
    const val ICON_MAX_SIDE = 128
    const val IMAGE_MIN_SIDE = 16
    const val KIND_ART = "art"
    const val KIND_ICON = "icon"

    /** A sender does not send a `media.state` frame over 16 KiB, a receiver refuses one over 32 KiB; `media.image` frames are refused over 48 KiB. */
    const val STATE_SEND_FRAME_BYTES = 16 * 1024
    const val STATE_FRAME_BYTES = 32 * 1024
    const val IMAGE_FRAME_BYTES = 48 * 1024

    private val HEX32 = Regex("[0-9a-f]{32}")

    fun isHash(value: String): Boolean = HEX32.matches(value)

    /** `sid` = the first 32 hex characters of SHA-256 of the UTF-8 bytes of the source app's identity (Android: the package name). The identity itself never goes on the wire. */
    fun sidOf(identity: String): String = Hashing.sha256Hex(identity.toByteArray(Charsets.UTF_8)).substring(0, 32)

    /** `hash` of a `media.image`: the first 32 hex characters of SHA-256 of the image bytes. */
    fun imageHash(bytes: ByteArray): String = Hashing.sha256Hex(bytes).substring(0, 32)

    /** The five cleaning steps of `link.title` (protocol.md): null when nothing is left. */
    fun clean(input: String?, maxCodePoints: Int): String? = input?.let { LinkWire.cleanText(it, maxCodePoints) }

    fun status(id: String, status: MediaStatus): String = buildJsonObject {
        put("type", STATUS)
        put("id", id)
        put("show", status.show)
        put("share", status.share)
        status.access?.let { put("access", it) }
    }.toString()

    /**
     * A `media.state`. A frame over [STATE_SEND_FRAME_BYTES] is made smaller by cutting the album, then the artist, then the title; null when even that is not
     * enough (it can't happen with the field limits, so this is only the guard).
     */
    fun state(id: String, state: MediaState): String? {
        var s = state
        while (true) {
            val frame = stateJson(id, s)
            if (Utf8.size(frame) <= STATE_SEND_FRAME_BYTES) return frame
            s = when {
                s.album != null -> s.copy(album = null)
                s.artist != null -> s.copy(artist = null)
                s.title != null -> s.copy(title = null)
                else -> return null
            }
        }
    }

    private fun stateJson(id: String, s: MediaState): String = buildJsonObject {
        put("type", STATE)
        put("id", id)
        put("sid", s.sid)
        put("app", s.app)
        put("state", s.state.wire)
        s.title?.let { put("title", it) }
        s.artist?.let { put("artist", it) }
        s.album?.let { put("album", it) }
        s.durationMs?.let { put("durationMs", it) }
        s.positionMs?.let { put("positionMs", it) }
        if (s.rateMilli != DEFAULT_RATE_MILLI) put("rateMilli", s.rateMilli)
        if (s.actions.isNotEmpty()) put("actions", buildJsonArray { s.actions.forEach { add(JsonPrimitive(it.wire)) } })
        s.artHash?.let { put("artHash", it) }
        s.iconHash?.let { put("iconHash", it) }
        s.volume?.let { v -> put("volume", buildJsonObject { put("level", v.level); put("muted", v.muted) }) }
    }.toString()

    fun image(id: String, kind: String, bytes: ByteArray): String = buildJsonObject {
        put("type", IMAGE)
        put("id", id)
        put("hash", imageHash(bytes))
        put("kind", kind)
        put("data", Base64.getUrlEncoder().withoutPadding().encodeToString(bytes))
    }.toString()

    fun clear(id: String): String = buildJsonObject {
        put("type", CLEAR)
        put("id", id)
    }.toString()

    fun volume(id: String, volume: MediaVolume): String = buildJsonObject {
        put("type", VOLUME)
        put("id", id)
        put("level", volume.level)
        put("muted", volume.muted)
    }.toString()

    fun cmd(id: String, cmd: MediaCmd): String = buildJsonObject {
        put("type", CMD)
        put("id", id)
        cmd.sid?.let { put("sid", it) }
        put("cmd", cmd.kind.wire)
        cmd.seekMs?.let { put("seekMs", it) }
        cmd.level?.let { put("level", it) }
        cmd.delta?.let { put("delta", it) }
        cmd.mute?.let { put("mute", it) }
    }.toString()
}
