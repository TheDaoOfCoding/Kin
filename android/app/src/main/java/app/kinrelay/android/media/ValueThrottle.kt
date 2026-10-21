package app.kinrelay.android.media

import app.kinrelay.android.link.Clock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The trailing throttle of design 3.6 (vectors `throttle[]`): a value goes out at once if the last send was [intervalMillis] or more ago, otherwise the latest value is held and goes
 * at `last + interval`; a value the same as the last one sent is not sent (and cancels a held one that would change it back); the end of a burst flushes what is held. The wait is a
 * coroutine `delay`, not an alarm: nothing runs while idle. [same] is told the time so a state can compare positions after extrapolating.
 */
class ValueThrottle<T : Any>(
    private val intervalMillis: Long,
    private val clock: Clock,
    private val scope: CoroutineScope,
    /** The monitor to use: a sender with several throttles passes its own, so that a timer and a caller never take two locks in opposite orders. */
    private val lock: Any = Any(),
    private val same: (last: T, candidate: T, now: Long) -> Boolean = { a, b, _ -> a == b },
    private val send: (T) -> Unit,
) {
    private var last: T? = null
    private var lastAt = Long.MIN_VALUE
    private var held: T? = null
    private var timer: Job? = null

    fun offer(value: T) = synchronized(lock) {
        val now = clock.elapsedMillis()
        val sent = last
        if (sent != null && same(sent, value, now)) {
            held = null
            return@synchronized
        }
        if (lastAt == Long.MIN_VALUE || now - lastAt >= intervalMillis) {
            held = null
            timer?.cancel()
            timer = null
            deliver(value, now)
            return@synchronized
        }
        held = value
        if (timer == null) {
            val wait = intervalMillis - (now - lastAt)
            timer = scope.launch {
                delay(wait)
                fire()
            }
        }
    }

    private fun fire() = synchronized(lock) {
        timer = null
        val value = held ?: return@synchronized
        held = null
        val now = clock.elapsedMillis()
        val sent = last
        if (sent != null && same(sent, value, now)) return@synchronized
        deliver(value, now)
    }

    private fun deliver(value: T, now: Long) {
        last = value
        lastAt = now
        send(value)
    }

    /** Forgets everything (a new connection, or the session ended): the next value goes at once and is never "the same as the last". */
    fun reset() = synchronized(lock) {
        timer?.cancel()
        timer = null
        held = null
        last = null
        lastAt = Long.MIN_VALUE
    }

    /** The last value was not really delivered (dropped by a budget): forget it, so that the same value offered later is sent. */
    fun forgetLast() = synchronized(lock) { last = null }

    /** True while a value is held for the end of the interval. */
    fun hasHeld(): Boolean = synchronized(lock) { held != null }
}
