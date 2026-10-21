package app.kinrelay.android.ring

import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.presence.Cancel
import app.kinrelay.android.presence.PresenceWire
import app.kinrelay.android.presence.Timers
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** What the machine needs from the phone. The real one ([AndroidRingPorts]) plays the alarm, vibrates, posts the Stop notification and sends `ring.status`; the tests record the calls in order. */
interface RingPorts {
    /** The device has a fixed volume (the alarm volume can't be changed): the volume is never touched. */
    fun volumeFixed(): Boolean

    fun alarmVolume(): Int

    fun alarmMax(): Int

    fun setAlarmVolume(index: Int)

    /** Starts the alarm sound and the vibration. Throws when nothing could be started (the notification still shows). */
    fun startSound()

    fun stopSound()

    /**
     * Do Not Disturb silences the alarm stream: "total silence", or Priority mode when the user's policy does not allow alarms ([app.kinrelay.android.dnd.AlarmAudibility]). Android plays nothing, so
     * the volume is not touched (it can't be heard, and changing it under total silence may be refused) and the ring says `audible: false`.
     */
    fun totalSilence(): Boolean

    /**
     * Posts the Stop notification (a full-screen intent where Android allows it), or posts it again with a new end when a restart extends the ring ([endsInMillis] from now, for the countdown).
     * False when it couldn't be posted (no notification permission).
     */
    fun showStop(pcName: String, endsInMillis: Long): Boolean

    fun hideStop()

    /** A partial wake lock with a timeout while ringing; false releases it. */
    fun holdAwake(on: Boolean)

    /** The alarm volume index saved before it was raised, or null: durable (a crash mid-ring must not leave the alarm at maximum). */
    fun savedIndex(): Int?

    fun saveIndex(index: Int)

    fun clearSavedIndex()

    /** Sends `ring.status` (if the PC can take it). */
    fun announce(state: String, reason: String?, audible: Boolean?)
}

/** The phone's answer to `ring.start` or `ring.stop`: an `ack`, or an `error` with the `ref` of the request. */
sealed interface RingReply {
    data object Ack : RingReply
    data class Error(val code: String, val message: String) : RingReply
}

/** Counts only (design 5.8): `ring: started N, stopped {timeout: N, user: N, pc: N}`. */
class RingStats {
    val started = AtomicLong()
    val rejected = AtomicLong()
    private val stopped = ConcurrentHashMap<String, AtomicLong>()

    fun stopped(reason: String) {
        stopped.computeIfAbsent(reason) { AtomicLong() }.incrementAndGet()
    }

    fun stoppedCount(reason: String): Long = stopped[reason]?.get() ?: 0L

    fun line(): String =
        "ring: started ${started.get()}, rejected ${rejected.get()}, stopped {timeout: ${stoppedCount("timeout")}, user: ${stoppedCount("user")}, pc: ${stoppedCount("pc")}, limit: ${stoppedCount("limit")}, error: ${stoppedCount("error")}}"
}

/**
 * Ring my phone, the pure part (design 3.8 and 5.4): `idle -> ringing (30 s timer) -> idle`.
 *
 * - `ring.start`: `disabled` when the switch is off, `rate_limited` (with the request's `ref`) beyond **3 starts in 60 s** or while the 60 s lock-out after the cap lasts; otherwise it
 *   starts (or, when already ringing, restarts the 30 s), answers `ack` **once the sound is running** (or the notification posted), then sends `ring.status ringing`.
 * - It stops after **30 s** (`timeout`), on the user's Stop (`user`), on `ring.stop` (`pc`), and at the **2-minute continuous cap** (`limit`, then a 60 s lock-out) so repeated starts can't
 *   ring for ever.
 * - The alarm volume is saved to durable storage **before** it is raised and restored on every stop, and again at every service start while a saved value is still there and nothing rings.
 * - `ring.stop` is always answered with `ack`, and sends `ring.status stopped` only if it stopped something.
 *
 * It ignores Pause (a lost phone is the reason for the feature) and Do Not Disturb except "total silence" (reported as `audible: false`). The timers are one-shots; the service is alive.
 */
class RingMachine(
    private val ports: RingPorts,
    private val clock: Clock,
    private val timers: Timers,
    /** "Let my PC ring this phone" (through the feature resolver). */
    private val allowed: () -> Boolean,
    /** "Ring volume: Loud (maximum)" (default) or "Use my alarm volume". */
    private val loud: () -> Boolean,
    private val pcName: () -> String,
    private val log: KinLog = KinLog.None,
    val stats: RingStats = RingStats(),
) {
    private val lock = Any()
    private var ringing = false
    private var continuousSince = 0L
    private var timeout: Cancel? = null

    /** Counts the timers that were set: a timer that was replaced (a restart) or called off, but already waiting for the lock, finds a newer number and does nothing. */
    private var timerGeneration = 0L
    private val starts = ArrayDeque<Long>()
    private var lockedUntil = NEVER
    private var lastAudible = true
    private var lastReason: String? = null

    val isRinging: Boolean get() = synchronized(lock) { ringing }

    val lastStopReason: String? get() = synchronized(lock) { lastReason }

    val audible: Boolean get() = synchronized(lock) { lastAudible }

    /** The user's switch went off while ringing: it stops (a phone whose owner said no must not keep ringing). */
    fun allowedChanged() {
        synchronized(lock) {
            if (ringing && !allowed()) stop(PresenceWire.REASON_USER, announce = true)
        }
    }

    /**
     * A `ring.start`. [reply] is called exactly once with the answer (the caller puts it on the wire and remembers it for the de-duplication); the `ring.status` that follows an accepted start is sent
     * after it.
     */
    fun start(reply: (RingReply) -> Unit) {
        var announceAudible: Boolean? = null
        synchronized(lock) {
            val now = clock.elapsedMillis()
            if (!allowed()) {
                stats.rejected.incrementAndGet()
                reply(RingReply.Error("disabled", "Ringing is turned off on this phone."))
                return
            }
            if (lockedUntil != NEVER && now < lockedUntil) {
                stats.rejected.incrementAndGet()
                reply(RingReply.Error("rate_limited", "Wait a minute, then try again."))
                return
            }
            while (starts.isNotEmpty() && now - starts.first() >= START_WINDOW_MILLIS) starts.removeFirst()
            if (starts.size >= MAX_STARTS) {
                stats.rejected.incrementAndGet()
                reply(RingReply.Error("rate_limited", "Wait a minute, then try again."))
                return
            }
            if (ringing && now - continuousSince >= CONTINUOUS_CAP_MILLIS) {
                stop(PresenceWire.REASON_LIMIT, announce = true)
                lockedUntil = now + LOCKOUT_MILLIS
                stats.rejected.incrementAndGet()
                reply(RingReply.Error("rate_limited", "Wait a minute, then try again."))
                return
            }
            starts.addLast(now)
            stats.started.incrementAndGet()
            var soundOk = true
            val first = !ringing
            if (first) continuousSince = now
            val capLeft = CONTINUOUS_CAP_MILLIS - (now - continuousSince)
            val wait = minOf(TIMEOUT_MILLIS, capLeft)
            val reason = if (capLeft < TIMEOUT_MILLIS) PresenceWire.REASON_LIMIT else PresenceWire.REASON_TIMEOUT
            if (first) {
                raiseVolume()
                soundOk = try {
                    ports.startSound()
                    true
                } catch (e: Exception) {
                    log.w("ring", "ring: the sound could not start (${e.javaClass.simpleName})")
                    false
                }
                ports.holdAwake(true)
                if (!ports.showStop(pcName(), wait)) log.w("ring", "ring: the Stop notification could not be posted")
                ringing = true
            } else {
                ports.holdAwake(true)
                // A restart extends the ring: the Stop notification is posted again (same id, new end) so it, and the Stop screen's countdown, last as long as the sound does.
                if (!ports.showStop(pcName(), wait)) log.w("ring", "ring: the Stop notification could not be posted")
            }
            timeout?.cancel()
            val generation = ++timerGeneration
            timeout = timers.after(wait) { onTimer(generation, reason) }
            lastAudible = soundOk && !ports.totalSilence()
            announceAudible = lastAudible
            reply(RingReply.Ack)
        }
        // After the ack went out (the caller sends it inside `reply`), outside the lock.
        announceAudible?.let { ports.announce(PresenceWire.STATE_RINGING, null, it) }
    }

    /** A `ring.stop`: always `ack`; `ring.status stopped` only if something was ringing. */
    fun stopFromPc(reply: (RingReply) -> Unit) {
        val stopped = synchronized(lock) { stop(PresenceWire.REASON_PC, announce = false) }
        reply(RingReply.Ack)
        if (stopped) ports.announce(PresenceWire.STATE_STOPPED, PresenceWire.REASON_PC, null)
    }

    /**
     * The Stop button (the notification, the full-screen screen, the app). The Stop notification is **always** taken down, also when nothing rings: after the process died in the middle of a ring the
     * notification outlives the sound, and its Stop button must clear it (2B.4 review).
     */
    fun stopByUser() {
        synchronized(lock) {
            if (!stop(PresenceWire.REASON_USER, announce = true)) ports.hideStop()
        }
    }

    /** Unlinking: no ring, no volume left raised, and nobody to tell (the peer is gone). */
    fun onUnlink() {
        synchronized(lock) {
            stop(PresenceWire.REASON_ERROR, announce = false)
            starts.clear()
            lockedUntil = NEVER
            restoreVolume()
        }
    }

    /**
     * At every service start: a crash mid-ring must not leave the alarm volume at maximum, so a saved index that is still there is put back when nothing is ringing. True when it
     * restored one.
     */
    fun restoreAtStart(): Boolean = synchronized(lock) {
        if (ringing) return false
        // A Stop notification of a ring that died with its process has nothing behind it: it goes now, and so does the Stop screen's state.
        ports.hideStop()
        val saved = ports.savedIndex() ?: return false
        if (!restoreVolume()) return false
        log.i("ring", "ring: the alarm volume of an interrupted ring was put back")
        saved >= 0
    }

    private fun onTimer(generation: Long, reason: String) {
        synchronized(lock) {
            if (generation != timerGeneration) return
            timeout = null
            if (!ringing) return
            stop(reason, announce = true)
            if (reason == PresenceWire.REASON_LIMIT) lockedUntil = clock.elapsedMillis() + LOCKOUT_MILLIS
        }
    }

    /** Stops what rings. Returns true if something was ringing. [announce] sends `ring.status stopped` now (a `ring.stop` answers first and sends it itself). */
    private fun stop(reason: String, announce: Boolean): Boolean {
        if (!ringing) return false
        ringing = false
        timeout?.cancel()
        timeout = null
        timerGeneration++
        try {
            ports.stopSound()
        } catch (_: Exception) {
        }
        ports.hideStop()
        ports.holdAwake(false)
        restoreVolume()
        lastReason = reason
        stats.stopped(reason)
        if (announce) ports.announce(PresenceWire.STATE_STOPPED, reason, null)
        return true
    }

    /**
     * Save first, durably, and only then raise. Not when the device has a fixed volume, nor when the user asked for "Use my alarm volume", nor **under a Do Not Disturb that silences the alarm**
     * (nothing could be heard, and the platform may refuse the change: nothing is saved either). Never twice (the second would save the raised value).
     */
    private fun raiseVolume() {
        if (!loud() || ports.volumeFixed() || ports.savedIndex() != null || ports.totalSilence()) return
        val max = ports.alarmMax()
        val current = ports.alarmVolume()
        if (current >= max) return
        ports.saveIndex(current)
        ports.setAlarmVolume(max)
    }

    /**
     * Puts the user's alarm volume back and forgets the saved value **only if the volume really changed back** (read again after the call). A refusal (Do Not Disturb that blocks the change), an
     * exception or a volume that did not move keeps the saved value, and the next service start tries again. True when nothing is left to restore.
     */
    private fun restoreVolume(): Boolean {
        val saved = ports.savedIndex() ?: return true
        if (ports.volumeFixed()) {
            ports.clearSavedIndex()
            return true
        }
        // Kin raised the volume to the maximum. Anything else now is a volume the user set by hand after that (or one Kin never raised): theirs wins, and the saved value is dropped, not restored over it.
        if (ports.alarmVolume() != ports.alarmMax()) {
            log.i("ring", "ring: the alarm volume is not the one Kin raised; the saved value is dropped")
            ports.clearSavedIndex()
            return true
        }
        val target = saved.coerceIn(0, ports.alarmMax())
        try {
            if (ports.alarmVolume() != target) ports.setAlarmVolume(target)
        } catch (e: Exception) {
            log.w("ring", "ring: the alarm volume could not be put back (${e.javaClass.simpleName}); it is tried again at the next start")
            return false
        }
        if (ports.alarmVolume() != target) {
            log.w("ring", "ring: the alarm volume did not change back; it is tried again at the next start")
            return false
        }
        ports.clearSavedIndex()
        return true
    }

    companion object {
        const val TIMEOUT_MILLIS = 30_000L
        const val CONTINUOUS_CAP_MILLIS = 120_000L
        const val LOCKOUT_MILLIS = 60_000L
        const val START_WINDOW_MILLIS = 60_000L
        const val MAX_STARTS = 3
        private const val NEVER = Long.MIN_VALUE
    }
}
