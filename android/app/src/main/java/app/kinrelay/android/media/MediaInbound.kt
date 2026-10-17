package app.kinrelay.android.media

import app.kinrelay.android.protocol.Fields
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.util.Base64

/**
 * What a receiver does with a `media.*` message that passed the capability and frame-size rules (protocol 3.3 and 3.9; vectors `messages.json` `media-*` and `media.json`
 * `clean[]`, `image[]`): field validation and normalisation. The phone is the viewer of the PC's `media.state`, `media.image`, `media.clear` and `media.volume`, the source
 * that gets the PC's `media.cmd`, and the receiver of `media.status` either way, so this is the phone's own code, not a stand-in. **Strings are cleaned and cut here, never
 * rejected for length**; an image that fails a check is dropped, not an error (an image can't start an error storm).
 */
object MediaInbound {
    sealed interface Result {
        class Ok(val message: MediaMsg) : Result
        class Bad(val message: String) : Result

        /** Dropped silently and counted: a `media.image` with a wrong hash, a bad encoding, a wrong format or size. */
        data object Dropped : Result
    }

    fun parse(type: String, id: String, frame: JsonObject): Result = try {
        when (type) {
            MediaWire.STATUS -> status(id, frame)
            MediaWire.STATE -> state(id, frame)
            MediaWire.IMAGE -> image(id, frame)
            MediaWire.CLEAR -> Result.Ok(MediaMsg.Clear(id))
            MediaWire.VOLUME -> Result.Ok(MediaMsg.Volume(id, MediaVolume(level(frame["level"]), bool(frame["muted"], "muted"))))
            MediaWire.CMD -> cmd(id, frame)
            else -> Result.Bad("not a media message")
        }
    } catch (e: Bad) {
        Result.Bad(e.message ?: "invalid")
    }

    private class Bad(message: String) : Exception(message)

    private fun bad(message: String): Nothing = throw Bad(message)

    private fun bool(element: JsonElement?, name: String): Boolean = Fields.boolean(element) ?: bad("$name must be a boolean")

    private fun optionalAbsent(element: JsonElement?): Boolean = element == null || element is JsonNull

    private fun level(element: JsonElement?): Int = Fields.integer(element)?.takeIf { it in 0..100 }?.toInt() ?: bad("level must be an integer from 0 to 100")

    private fun hex32(element: JsonElement?, name: String): String = Fields.string(element)?.takeIf(MediaWire::isHash) ?: bad("$name must be 32 lowercase hex characters")

    private fun optionalText(element: JsonElement?, max: Int, name: String): String? {
        if (optionalAbsent(element)) return null
        return MediaWire.clean(Fields.string(element) ?: bad("$name must be a string"), max)
    }

    private fun status(id: String, frame: JsonObject): Result {
        val access = if (optionalAbsent(frame["access"])) null else Fields.string(frame["access"])?.takeIf { it == "granted" || it == "missing" } ?: bad("access must be \"granted\" or \"missing\"")
        return Result.Ok(MediaMsg.Status(id, MediaStatus(bool(frame["show"], "show"), bool(frame["share"], "share"), access)))
    }

    private fun state(id: String, frame: JsonObject): Result {
        val sid = hex32(frame["sid"], "sid")
        val app = MediaWire.clean(Fields.string(frame["app"]) ?: bad("app must be a string"), MediaWire.APP_MAX) ?: bad("app is empty")
        val state = Fields.string(frame["state"])?.let(PlayState::fromWire) ?: bad("state must be playing, paused, buffering or stopped")
        val title = optionalText(frame["title"], MediaWire.TITLE_MAX, "title")
        val artist = optionalText(frame["artist"], MediaWire.ARTIST_MAX, "artist")
        val album = optionalText(frame["album"], MediaWire.ALBUM_MAX, "album")
        val duration = if (optionalAbsent(frame["durationMs"])) null else Fields.integer(frame["durationMs"])?.takeIf { it in 1..MediaWire.MAX_DURATION_MS } ?: bad("durationMs must be an integer from 1 to 86,400,000")
        val rawPosition = if (optionalAbsent(frame["positionMs"])) null else Fields.integer(frame["positionMs"])?.takeIf { it in 0..MediaWire.MAX_DURATION_MS } ?: bad("positionMs must be an integer from 0 to 86,400,000")
        val position = if (rawPosition != null && duration != null) minOf(rawPosition, duration) else rawPosition
        val rate = if (optionalAbsent(frame["rateMilli"])) MediaWire.DEFAULT_RATE_MILLI else minOf(Fields.integer(frame["rateMilli"]) ?: bad("rateMilli must be a non-negative integer"), MediaWire.MAX_RATE_MILLI.toLong()).toInt()
        val actions = actions(frame["actions"], hasDuration = duration != null)
        val artHash = if (optionalAbsent(frame["artHash"])) null else hex32(frame["artHash"], "artHash")
        val iconHash = if (optionalAbsent(frame["iconHash"])) null else hex32(frame["iconHash"], "iconHash")
        val volume = when (val raw = frame["volume"]) {
            null, is JsonNull -> null
            is JsonObject -> MediaVolume(level(raw["level"]), bool(raw["muted"], "volume.muted"))
            else -> bad("volume must be an object")
        }
        return Result.Ok(MediaMsg.State(id, MediaState(sid, app, state, title, artist, album, duration, position, rate, actions, artHash, iconHash, volume)))
    }

    /** At most the first 8 entries are looked at, each must be a string; unknown ones are ignored, the result is in the canonical order, and `seek` is believed only with a duration. */
    private fun actions(element: JsonElement?, hasDuration: Boolean): List<MediaAction> {
        if (optionalAbsent(element)) return emptyList()
        val list = element as? JsonArray ?: bad("actions must be an array")
        val first = list.take(MediaWire.MAX_ACTIONS).map { Fields.string(it) ?: bad("an action must be a string") }.toSet()
        return MediaAction.entries.filter { it.wire in first && (hasDuration || it != MediaAction.SEEK) }
    }

    private fun image(id: String, frame: JsonObject): Result {
        val hash = hex32(frame["hash"], "hash")
        val kind = Fields.string(frame["kind"])?.takeIf { it == MediaWire.KIND_ART || it == MediaWire.KIND_ICON } ?: bad("kind must be art or icon")
        val data = Fields.string(frame["data"]) ?: bad("data must be a string")
        val bytes = decodeImage(data) ?: return Result.Dropped
        if (MediaWire.imageHash(bytes) != hash || !MediaImages.acceptable(bytes, kind)) return Result.Dropped
        return Result.Ok(MediaMsg.Image(id, MediaImage(hash, kind, bytes)))
    }

    private val BASE64URL = Regex("[A-Za-z0-9_-]*")

    /** base64url with no padding, at most [MediaWire.IMAGE_MAX_BYTES] bytes; null when it isn't. */
    private fun decodeImage(data: String): ByteArray? {
        if (!BASE64URL.matches(data) || data.length % 4 == 1 || data.length > (MediaWire.IMAGE_MAX_BYTES * 4 + 2) / 3 + 4) return null
        return try {
            Base64.getUrlDecoder().decode(data).takeIf { it.size <= MediaWire.IMAGE_MAX_BYTES }
        } catch (_: IllegalArgumentException) {
            null
        }
    }

    private fun cmd(id: String, frame: JsonObject): Result {
        val kind = Fields.string(frame["cmd"])?.let(MediaCmdKind::fromWire) ?: bad("cmd must be play, pause, toggle, next, prev, seek or volume")
        val sid = if (optionalAbsent(frame["sid"])) {
            if (kind != MediaCmdKind.VOLUME) bad("sid is required")
            null
        } else {
            hex32(frame["sid"], "sid")
        }
        fun present(name: String) = !optionalAbsent(frame[name])
        var seek: Long? = null
        if (kind == MediaCmdKind.SEEK) {
            if (!present("seekMs")) bad("seekMs is required for seek")
            seek = Fields.integer(frame["seekMs"])?.takeIf { it in 0..MediaWire.MAX_SEEK_MS } ?: bad("seekMs must be an integer from 0 to 86,400,000")
        } else if (present("seekMs")) {
            bad("seekMs belongs to seek only")
        }
        var level: Int? = null
        var delta: Int? = null
        var mute: Boolean? = null
        if (kind == MediaCmdKind.VOLUME) {
            val given = listOf("level", "delta", "mute").filter(::present)
            if (given.size != 1) bad("volume needs exactly one of level, delta and mute")
            when (given.single()) {
                "level" -> level = level(frame["level"])
                "delta" -> delta = signed(frame["delta"])?.takeIf { it in -MediaWire.MAX_DELTA..MediaWire.MAX_DELTA && it != 0 } ?: bad("delta must be an integer from -100 to 100, not 0")
                else -> mute = bool(frame["mute"], "mute")
            }
        } else if (present("level") || present("delta") || present("mute")) {
            bad("level, delta and mute belong to volume only")
        }
        return Result.Ok(MediaMsg.Cmd(id, MediaCmd(kind, sid, seek, level, delta, mute)))
    }

    private val SIGNED = Regex("-?(0|[1-9][0-9]{0,15})")

    /** A JSON integer that may be negative (`delta`, the one that may): no quotes, fraction or exponent. */
    private fun signed(element: JsonElement?): Int? {
        val primitive = element as? JsonPrimitive ?: return null
        if (primitive is JsonNull || primitive.isString || !SIGNED.matches(primitive.content) || primitive.content == "-0") return null
        return primitive.content.toLongOrNull()?.takeIf { it in -1_000_000L..1_000_000L }?.toInt()
    }
}

/** The wire's image rules (design 3.3): PNG or JPEG, at most 24,576 bytes, at least 16 px, longest side at most 256 (`art`) or 128 (`icon`). Reads the headers only, decodes nothing. */
object MediaImages {
    fun acceptable(bytes: ByteArray, kind: String): Boolean {
        if (bytes.size > MediaWire.IMAGE_MAX_BYTES) return false
        val size = sizeOf(bytes) ?: return false
        if (size.first < MediaWire.IMAGE_MIN_SIDE || size.second < MediaWire.IMAGE_MIN_SIDE) return false
        return maxOf(size.first, size.second) <= (if (kind == MediaWire.KIND_ART) MediaWire.ART_MAX_SIDE else MediaWire.ICON_MAX_SIDE)
    }

    /** (width, height) of a PNG or a JPEG from its header, or null. */
    fun sizeOf(b: ByteArray): Pair<Int, Int>? = pngSize(b) ?: jpegSize(b)

    private fun u8(b: ByteArray, i: Int) = b[i].toInt() and 0xFF

    private fun u32(b: ByteArray, o: Int): Long = (u8(b, o).toLong() shl 24) or (u8(b, o + 1).toLong() shl 16) or (u8(b, o + 2).toLong() shl 8) or u8(b, o + 3).toLong()

    private val PNG_SIGNATURE = intArrayOf(0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

    private fun pngSize(b: ByteArray): Pair<Int, Int>? {
        if (b.size < 33 || PNG_SIGNATURE.indices.any { u8(b, it) != PNG_SIGNATURE[it] }) return null
        if (String(b, 12, 4, Charsets.ISO_8859_1) != "IHDR" || u32(b, 8) != 13L) return null
        val w = u32(b, 16)
        val h = u32(b, 20)
        return if (w > Int.MAX_VALUE || h > Int.MAX_VALUE) null else w.toInt() to h.toInt()
    }

    private fun jpegSize(b: ByteArray): Pair<Int, Int>? {
        if (b.size < 4 || u8(b, 0) != 0xFF || u8(b, 1) != 0xD8) return null
        var pos = 2
        while (pos + 4 <= b.size) {
            if (u8(b, pos) != 0xFF) return null
            while (pos < b.size && u8(b, pos) == 0xFF) pos++
            if (pos >= b.size) return null
            val marker = u8(b, pos)
            pos++
            if (marker == 0xD9 || marker == 0xDA) return null
            if (marker == 0x01 || marker in 0xD0..0xD8) continue
            if (pos + 2 > b.size) return null
            val len = (u8(b, pos) shl 8) or u8(b, pos + 1)
            if (len < 2) return null
            val sof = marker in 0xC0..0xCF && marker != 0xC4 && marker != 0xC8 && marker != 0xCC
            if (sof) {
                if (pos + 7 > b.size) return null
                return ((u8(b, pos + 5) shl 8) or u8(b, pos + 6)) to ((u8(b, pos + 3) shl 8) or u8(b, pos + 4))
            }
            pos += len
        }
        return null
    }
}
