package app.kinrelay.android.pause

import app.kinrelay.android.link.Clock
import app.kinrelay.android.presence.PresenceWire
import app.kinrelay.android.presence.WirePause
import java.time.Instant
import java.time.LocalTime
import java.time.ZoneId

/** The three ways to pause (design 3.5): 1 hour, until tomorrow morning, until the user resumes. */
enum class PauseChoice { HOUR, TOMORROW, UNTIL_RESUMED }

/** This device's own pause: [untilMillis] is an epoch time in ms (0 = none), [untilResumed] a pause that has no end. Persisted; expiry is a wall-clock comparison. */
data class PauseState(val untilMillis: Long = 0L, val untilResumed: Boolean = false) {
    companion object {
        val NONE = PauseState()
    }
}

/** What to do with a message that is not an explicit send (the rows of design 3.5). */
enum class GateKind {
    /** The PC's automatic `clipboard`. */
    CLIPBOARD,

    /** `notif.post` (new content). */
    NOTIF_POST,

    /** `notif.remove`, `notif.sync`, `notif.dismiss`: cleanup is not content. */
    NOTIF_CLEANUP,

    /** `media.state`, `media.image`. */
    MEDIA,

    /** An auto-sent screenshot or camera photo. */
    SHOT,

    /** `presence` itself. */
    PRESENCE,

    /** The low-battery nudge and the other toasts of automatic features. */
    TOAST,

    /** Ring my phone: works while paused (decision 5). */
    RING,

    /** `unpair`, `ping`, `ack`, `error`, file transfers already running. */
    CONTROL,

    /** `text`, `link`, an offer, an upload: the user's own act. */
    MANUAL,
}

/** What happens to an incoming message of a [GateKind] while this device is paused. */
enum class Inbound {
    /** Handled as if not paused. */
    PROCESS,

    /** Dropped silently (counted). */
    DROP,

    /** Stored in the history and nothing visible happens by itself: no notification or toast, no clipboard write, no auto-opened link. Files are saved. */
    QUIET,
}

/** The gate of design 3.5, in code. One `if` at each entry; nothing is started or stopped by pausing, and no timer is needed to resume. */
object PauseGate {
    /** [PauseChoice.TOMORROW]: the next 08:00 local time that is at least 2 hours away. One constant, one place. */
    const val TOMORROW_HOUR = 8
    const val TOMORROW_MIN_AHEAD_MILLIS = 2L * 60 * 60 * 1000
    const val HOUR_MILLIS = 60L * 60 * 1000

    /** The state a [choice] made at [nowMillis] gives, in [zone]. */
    fun stateFor(choice: PauseChoice, nowMillis: Long, zone: ZoneId): PauseState = when (choice) {
        PauseChoice.HOUR -> PauseState(untilMillis = nowMillis + HOUR_MILLIS)
        PauseChoice.TOMORROW -> PauseState(untilMillis = tomorrowMillis(nowMillis, zone))
        PauseChoice.UNTIL_RESUMED -> PauseState(untilResumed = true)
    }

    /**
     * The next 08:00 local time at least 2 hours after [nowMillis]: at 07:30 tomorrow 08:00, at 05:00 today 08:00, at 08:00 exactly tomorrow 08:00. Built from the local date and
     * the zone's rules (not "plus 24 hours"), so a daylight-saving change in between still lands on 08:00 on the wall clock.
     */
    fun tomorrowMillis(nowMillis: Long, zone: ZoneId): Long {
        val local = Instant.ofEpochMilli(nowMillis).atZone(zone)
        var day = local.toLocalDate()
        while (true) {
            val candidate = day.atTime(LocalTime.of(TOMORROW_HOUR, 0)).atZone(zone).toInstant().toEpochMilli()
            if (candidate - nowMillis >= TOMORROW_MIN_AHEAD_MILLIS) return candidate
            day = day.plusDays(1)
        }
    }

    /** True while [state] says paused at [nowMillis]. A `pausedUntil` more than 8 days ahead is treated as expired (a clock set backwards must not freeze Kin for a year). */
    fun paused(state: PauseState, nowMillis: Long): Boolean {
        if (state.untilResumed) return true
        val ahead = state.untilMillis - nowMillis
        return ahead in 1..PresenceWire.MAX_PAUSE_MILLIS
    }

    /** The pause as `presence` carries it, or null when not paused. */
    fun wire(state: PauseState, nowMillis: Long): WirePause? = when {
        state.untilResumed -> WirePause.UntilResumed
        paused(state, nowMillis) -> WirePause.Remaining((state.untilMillis - nowMillis).coerceIn(1, PresenceWire.MAX_PAUSE_MILLIS))
        else -> null
    }

    /**
     * May this device **send** a message of [kind] now? Automatic content stops while this device or the peer is paused; cleanup, presence, ring, control and manual sends
     * always go (a manual send is the user's own act: the sender isn't paused by the peer's pause).
     */
    fun maySend(kind: GateKind, selfPaused: Boolean, peerPaused: Boolean): Boolean = when (kind) {
        GateKind.CLIPBOARD, GateKind.NOTIF_POST, GateKind.MEDIA, GateKind.SHOT, GateKind.TOAST -> !selfPaused && !peerPaused
        GateKind.NOTIF_CLEANUP, GateKind.PRESENCE, GateKind.RING, GateKind.CONTROL, GateKind.MANUAL -> true
    }

    /** What happens to an incoming message of [kind] on this device, whether this device is paused. The receiver gates too, so a race or an old peer changes nothing on screen. */
    fun inbound(kind: GateKind, selfPaused: Boolean): Inbound = when {
        !selfPaused -> Inbound.PROCESS
        kind == GateKind.CLIPBOARD || kind == GateKind.NOTIF_POST || kind == GateKind.MEDIA -> Inbound.DROP
        // A manual send is recorded and nothing visible happens; so is an auto-sent image on the PC (a phone never receives one, the rule is the same table).
        kind == GateKind.MANUAL || kind == GateKind.SHOT -> Inbound.QUIET
        else -> Inbound.PROCESS
    }
}

/**
 * What this device knows about its **peer's** pause (design 3.4): `expiry = arrival (monotonic) + remainingMs`. It resumes automatic things at the expiry or when a `presence`
 * without `pause` arrives, whichever is first, and forgets everything when the connection closes. In memory only.
 */
class PeerPauseView(private val clock: Clock) {
    private var expiresAt: Long? = null
    private var untilResumed = false

    @Synchronized
    fun onPresence(pause: WirePause?) {
        when (pause) {
            null -> {
                expiresAt = null
                untilResumed = false
            }
            WirePause.UntilResumed -> {
                expiresAt = null
                untilResumed = true
            }
            is WirePause.Remaining -> {
                expiresAt = clock.elapsedMillis() + pause.millis
                untilResumed = false
            }
        }
    }

    @Synchronized
    fun isPaused(): Boolean = untilResumed || (expiresAt?.let { clock.elapsedMillis() < it } == true)

    /** Milliseconds until the peer's pause ends by itself, or null when it has no end, or isn't paused. */
    @Synchronized
    fun remainingMillis(): Long? = expiresAt?.let { (it - clock.elapsedMillis()).takeIf { left -> left > 0 } }

    @Synchronized
    fun hasNoEnd(): Boolean = untilResumed

    @Synchronized
    fun onClosed() {
        expiresAt = null
        untilResumed = false
    }
}
