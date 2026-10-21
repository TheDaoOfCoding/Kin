package app.kinrelay.android.presence

import app.kinrelay.android.protocol.Fields
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** What a device says about its battery (`presence.battery`). [saver] is the power-saving mode, null when not said. */
data class Battery(val level: Int, val charging: Boolean, val saver: Boolean? = null)

/** What a device says about its pause (`presence.pause`): exactly one of the two. */
sealed interface WirePause {
    /** The pause ends in [millis] from the moment the sender wrote the message. */
    data class Remaining(val millis: Long) : WirePause

    data object UntilResumed : WirePause
}

/**
 * A full snapshot of "what is true about me right now" (design 3.4). A field that is absent means "not shared or not applicable". 2B.5 adds [dnd] (the phone's Do Not Disturb filter when it is on:
 * `priority`, `alarms` or `total`; the phone sends it, the PC reads it) and [focus] (true only while the PC is in Focus; the PC sends it, the phone reads it). `focus: false` and a `dnd` this code
 * does not know are absent.
 */
data class PresenceSnapshot(val battery: Battery? = null, val pause: WirePause? = null, val dnd: String? = null, val focus: Boolean = false)

/**
 * `presence`, `ring.*` (2B.4, changelog 58). `presence` and `ring.status` are ephemeral (no ack, no de-duplication); `ring.start` and `ring.stop` are acked, de-duplicated, never queued and
 * never re-sent. The frame limits are 4 KiB (`presence`) and 2 KiB (`ring.*`).
 */
object PresenceWire {
    /** "I understand `presence`": every 0.2.0 build lists it. */
    const val CAPABILITY = "presence"

    /** "I understand `ring.start` and `ring.stop`": only the phone lists it, only while "Let my PC ring this phone" is on. */
    const val RING_CAPABILITY = "ring"

    const val PRESENCE = "presence"
    const val RING_START = "ring.start"
    const val RING_STOP = "ring.stop"
    const val RING_STATUS = "ring.status"

    const val FRAME_BYTES = 4 * 1024
    const val RING_FRAME_BYTES = 2 * 1024

    /** A `remainingMs` over 8 days is cut to 8 days; a `pausedUntil` further than that from the clock is treated as expired. */
    const val MAX_PAUSE_MILLIS = 691_200_000L

    const val STATE_RINGING = "ringing"
    const val STATE_STOPPED = "stopped"

    const val REASON_TIMEOUT = "timeout"
    const val REASON_USER = "user"
    const val REASON_PC = "pc"
    const val REASON_LIMIT = "limit"
    const val REASON_ERROR = "error"

    /** Result of reading a `presence` frame: [Ok] with the snapshot, or [Bad] (`bad_request`, with the reason for the message). */
    sealed interface Parsed {
        data class Ok(val snapshot: PresenceSnapshot) : Parsed
        data class Bad(val message: String) : Parsed
    }

    fun presence(id: String, snapshot: PresenceSnapshot): String = buildJsonObject {
        put("type", PRESENCE)
        put("id", id)
        snapshot.battery?.let { b ->
            put("battery", buildJsonObject {
                put("level", b.level.coerceIn(0, 100))
                put("charging", b.charging)
                b.saver?.let { put("saver", it) }
            })
        }
        when (val p = snapshot.pause) {
            is WirePause.Remaining -> put("pause", buildJsonObject { put("remainingMs", p.millis.coerceIn(1, MAX_PAUSE_MILLIS)) })
            WirePause.UntilResumed -> put("pause", buildJsonObject { put("untilResumed", true) })
            null -> Unit
        }
        // 2B.5: present only when on; an unknown word is never written.
        snapshot.dnd?.takeIf { it in KNOWN_DND }?.let { put("dnd", it) }
        if (snapshot.focus) put("focus", true)
    }.toString()

    /** Reads the fields of a `presence` (the frame is already a valid object with a valid `id`). `null` counts as absent, an unknown field is ignored, a wrong type is `Bad`. */
    fun parse(frame: JsonObject): Parsed {
        val battery = when (val raw = Fields.present(frame["battery"])) {
            null -> null
            is JsonObject -> parseBattery(raw) ?: return Parsed.Bad("battery needs an integer level from 0 to 100 and a boolean charging (saver, if present, a boolean).")
            else -> return Parsed.Bad("battery must be an object.")
        }
        val pause = when (val raw = Fields.present(frame["pause"])) {
            null -> null
            is JsonObject -> parsePause(raw) ?: return Parsed.Bad("pause needs exactly one of remainingMs (1 to 691200000) or untilResumed (true).")
            else -> return Parsed.Bad("pause must be an object.")
        }
        // 2B.5 (changelog 59): `dnd` is a string (an unknown word, an empty one included, is absent: a later Android filter); `focus` is a boolean (false counts as absent).
        val dnd = when (val raw = Fields.present(frame["dnd"])) {
            null -> null
            else -> (Fields.string(raw) ?: return Parsed.Bad("dnd must be a string (priority, alarms or total)."))
                .takeIf { it in KNOWN_DND }
        }
        val focus = when (val raw = Fields.present(frame["focus"])) {
            null -> false
            else -> Fields.boolean(raw) ?: return Parsed.Bad("focus must be a boolean.")
        }
        return Parsed.Ok(PresenceSnapshot(battery, pause, dnd, focus))
    }

    private val KNOWN_DND = setOf("priority", "alarms", "total")

    private fun parseBattery(o: JsonObject): Battery? {
        val level = Fields.integer(o["level"])?.takeIf { it in 0..100 } ?: return null
        val charging = Fields.boolean(o["charging"]) ?: return null
        val rawSaver: JsonElement? = Fields.present(o["saver"])
        val saver = if (rawSaver == null) null else (Fields.boolean(rawSaver) ?: return null)
        return Battery(level.toInt(), charging, saver)
    }

    private fun parsePause(o: JsonObject): WirePause? {
        val rawRemaining = Fields.present(o["remainingMs"])
        val rawResumed = Fields.present(o["untilResumed"])
        if ((rawRemaining == null) == (rawResumed == null)) return null
        if (rawRemaining != null) {
            val ms = Fields.integer(rawRemaining)?.takeIf { it >= 1 } ?: return null
            return WirePause.Remaining(minOf(ms, MAX_PAUSE_MILLIS))
        }
        return if (Fields.boolean(rawResumed) == true) WirePause.UntilResumed else null
    }

    /** `ring.status`: [reason] goes only with `stopped`, [audible] only with `ringing`. */
    fun ringStatus(id: String, state: String, reason: String? = null, audible: Boolean? = null): String = buildJsonObject {
        put("type", RING_STATUS)
        put("id", id)
        put("state", state)
        if (state == STATE_STOPPED && reason != null) put("reason", reason)
        if (state == STATE_RINGING && audible != null) put("audible", audible)
    }.toString()
}
