package app.kinrelay.android.presence

import app.kinrelay.android.link.Clock
import app.kinrelay.android.media.ErrorSpacer
import app.kinrelay.android.media.MediaBucket
import app.kinrelay.android.protocol.MessageIds
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.abs

/** The part of a connection [PresenceSender] needs: ready, the peer's latest `hello` lists `presence`, and a way to put a frame on the socket with no ack and no queue. */
interface PresenceLink {
    val isReady: Boolean
    val peerListsPresence: Boolean
    fun sendEphemeral(frame: String): Boolean
}

/** Counts only (design 5.8): never a level or a pause time. */
class PresenceStats {
    val sent = AtomicLong()
    val received = AtomicLong()
    val droppedRate = AtomicLong()
    val droppedOffline = AtomicLong()
    val droppedNotListed = AtomicLong()

    fun line(): String = "presence: sent ${sent.get()}, received ${received.get()}, dropped {rate: ${droppedRate.get()}, offline: ${droppedOffline.get()}, not_listed: ${droppedNotListed.get()}}"
}

/** What the receiver does with one `presence` (vectors `presenceRate[]`, side `receiver`): [accept], or drop it, and say so with one `error rate_limited` (no `ref`) per 10 s at most. */
class PresenceReceiveLimiter(clock: Clock) {
    private val bucket = MediaBucket(CAPACITY, REFILL_MILLIS, clock)
    private val errors = ErrorSpacer(ERROR_SPACING_MILLIS, clock)

    /** The verdict for one arriving `presence`. */
    data class Verdict(val accepted: Boolean, val sendError: Boolean)

    fun onPresence(): Verdict {
        if (bucket.tryTake()) return Verdict(accepted = true, sendError = false)
        return Verdict(accepted = false, sendError = errors.mayAnswer())
    }

    companion object {
        const val CAPACITY = 10
        const val REFILL_MILLIS = 3_000L
        const val ERROR_SPACING_MILLIS = 10_000L
    }
}

/**
 * The only writer of `presence` (design 3.4 and 5.1). A snapshot goes out (a) right after both `hello`s are accepted (even `{}`: it tells the peer "not paused"), (b) when something in it
 * changed: **battery-only** changes only if the level moved by at least 2 points from the last **sent** level, or crossed 15, 20 or 100, or `charging` or `saver` flipped (or the battery appeared
 * or went away), and **at most once per 30 s** (trailing: the latest value goes at the end of the window, if it is still a change); a **pause change** goes **1 s after the first change** of a burst
 * (trailing, so a pause that is set and undone within the second sends nothing) and is never held back by the battery limit; and (c) at the **moment a pause expires** ([onPauseExpired], no
 * delay). A snapshot is the whole current state, so one send also carries a battery change that was waiting for its 30 s. **There is no periodic timer**; the only timers are the one-shot trailing
 * ones. The capability is checked per message (a peer whose `hello` doesn't list `presence` gets nothing). It never starts `LinkService`: with no connection it drops and counts.
 */
class PresenceSender(
    private val clock: Clock,
    private val timers: Timers,
    private val link: () -> PresenceLink?,
    /** The battery to say now, or null: not shared (the switch is off), or this device has no battery. */
    private val battery: () -> Battery?,
    /** This device's own pause as `presence` carries it, or null when not paused. */
    private val pause: () -> WirePause?,
    /** Identifies the current pause (null when not paused): two calls are the same pause when they are equal, however much time passed (`remainingMs` itself is a moving number). */
    private val pauseKey: () -> Any? = { pause() },
    /**
     * The phone's Do Not Disturb as `presence.dnd` says it (`priority`, `alarms`, `total`), or null: off, not shared, or this phone is paused (a paused device leaves it out, design 3.1 of 2B.5).
     * A change goes out within 1 s like a pause, and is never held back by the battery limit.
     */
    private val dnd: () -> String? = { null },
    private val stats: PresenceStats = PresenceStats(),
    private val newId: () -> String = MessageIds::newId,
) {
    private val lock = Any()
    private var connection: PresenceLink? = null
    private var lastSent: PresenceSnapshot? = null
    private var lastSentPauseKey: Any? = null
    private var lastSentAt = NEVER
    private var batteryTimer: Cancel? = null
    private var pauseTimer: Cancel? = null

    /** The connection that can take a frame now, or null. A new connection starts a clean per-connection state (a snapshot is per connection, never remembered across them). */
    private fun usable(): PresenceLink? {
        val raw = link()
        if (raw !== connection) {
            reset()
            connection = raw
        }
        return raw?.takeIf { it.isReady && it.peerListsPresence }
    }

    private fun reset() {
        lastSent = null
        lastSentPauseKey = null
        lastSentAt = NEVER
        batteryTimer?.cancel()
        batteryTimer = null
        pauseTimer?.cancel()
        pauseTimer = null
    }

    /** After both `hello`s are accepted (the first, or a later one that changed the peer's capabilities): the first snapshot goes out at once. */
    fun onReady() {
        synchronized(lock) {
            val c = usable()
            if (c == null) {
                if (link()?.isReady == true) stats.droppedNotListed.incrementAndGet()
                return
            }
            if (lastSent == null) sendNow(c)
        }
    }

    /** The connection ended: nothing is kept and no timer is left. */
    fun onClosed() {
        synchronized(lock) {
            connection = null
            reset()
        }
    }

    /** The battery (or what it says: the switch, charging, power saving) may have changed. Cheap: nothing is sent unless it matters, and never faster than once per 30 s. */
    fun onBatteryChanged() {
        synchronized(lock) {
            val c = usable() ?: run {
                stats.droppedOffline.incrementAndGet()
                return
            }
            if (lastSent == null) {
                sendNow(c)
                return
            }
            if (!significant(lastSent?.battery, battery())) return
            val wait = BATTERY_WINDOW_MILLIS - (clock.elapsedMillis() - lastSentAt)
            if (wait <= 0) {
                sendNow(c)
            } else if (batteryTimer == null) {
                stats.droppedRate.incrementAndGet()
                batteryTimer = timers.after(wait) { fireBattery() }
            }
        }
    }

    private fun fireBattery() {
        synchronized(lock) {
            batteryTimer = null
            val c = usable() ?: return
            // The latest value decides: it may have moved back since the timer was set.
            if (significant(lastSent?.battery, battery())) sendNow(c)
        }
    }

    /** This device's pause changed (set, resumed): sent 1 s after the first change of a burst (if the pause still differs from what the peer was told), whatever the battery limit says. */
    fun onPauseChanged() {
        synchronized(lock) {
            val c = usable() ?: run {
                stats.droppedOffline.incrementAndGet()
                return
            }
            if (pauseTimer == null && c.isReady) pauseTimer = timers.after(PAUSE_WINDOW_MILLIS) { firePause() }
        }
    }

    /** A pause ended by itself: the one-shot of the pausing side tells the peer at that moment, with no delay. */
    fun onPauseExpired() {
        synchronized(lock) {
            val c = usable() ?: return
            if (pauseKey() != lastSentPauseKey) sendNow(c)
        }
    }

    /** The phone's Do Not Disturb changed, or sharing it was switched, or the pause that hides it changed (2B.5): sent 1 s after the first change of a burst, if it still differs from what the PC was told. */
    fun onDndChanged() {
        synchronized(lock) {
            val c = usable() ?: return
            if (lastSent == null) return
            if (pauseTimer == null && c.isReady) pauseTimer = timers.after(PAUSE_WINDOW_MILLIS) { firePause() }
        }
    }

    private fun firePause() {
        synchronized(lock) {
            pauseTimer = null
            val c = usable() ?: return
            // The latest value decides: a pause, or a Do Not Disturb, that was set and undone within the second sends nothing.
            if (pauseKey() != lastSentPauseKey || dnd() != lastSent?.dnd) sendNow(c)
        }
    }

    private fun sendNow(c: PresenceLink) {
        batteryTimer?.cancel()
        batteryTimer = null
        pauseTimer?.cancel()
        pauseTimer = null
        val snapshot = PresenceSnapshot(battery(), pause(), dnd())
        val key = pauseKey()
        if (c.sendEphemeral(PresenceWire.presence(newId(), snapshot))) {
            stats.sent.incrementAndGet()
            lastSent = snapshot
            lastSentPauseKey = key
            lastSentAt = clock.elapsedMillis()
        } else {
            stats.droppedOffline.incrementAndGet()
        }
    }

    companion object {
        const val BATTERY_WINDOW_MILLIS = 30_000L
        const val PAUSE_WINDOW_MILLIS = 1_000L
        private const val NEVER = Long.MIN_VALUE

        /**
         * The battery-only rule: a change of at least 2 points from the last **sent** level, a crossing of 15, 20 or 100 (the level is at or above the threshold on one side and below it on the
         * other: 15 to 14 and 19 to 20 and 99 to 100 cross, 16 to 15 does not), a flip of `charging` or `saver`, or the battery appearing or going away.
         */
        fun significant(previous: Battery?, now: Battery?): Boolean {
            if (previous == null && now == null) return false
            if (previous == null || now == null) return true
            if (abs(now.level - previous.level) >= 2) return true
            for (threshold in CROSSINGS) if ((previous.level >= threshold) != (now.level >= threshold)) return true
            return previous.charging != now.charging || previous.saver != now.saver
        }

        private val CROSSINGS = intArrayOf(15, 20, 100)
    }
}
