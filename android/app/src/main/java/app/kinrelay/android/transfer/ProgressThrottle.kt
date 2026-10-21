package app.kinrelay.android.transfer

import java.util.concurrent.atomic.AtomicLong

/**
 * At most one progress update per [intervalMillis], with no way to force one. Android rate-limits an app's notification updates, and a
 * limited update can be a result notification that then never appears (a "Receiving..." that stays for ever), so the runners must stay
 * well under the limit and the result goes out under its own ID ([TransferNotifications]).
 */
class ProgressThrottle(private val intervalMillis: Long = DEFAULT_INTERVAL_MILLIS) {
    private val last = AtomicLong(Long.MIN_VALUE)

    /** True when an update may go out now (and counts it as sent). Safe to call from several threads. */
    fun tryReport(nowMillis: Long): Boolean {
        while (true) {
            val before = last.get()
            // A clock that stepped back counts as "long ago": updates must not stop until it catches up.
            if (before != Long.MIN_VALUE && nowMillis >= before && nowMillis - before < intervalMillis) return false
            if (last.compareAndSet(before, nowMillis)) return true
        }
    }

    companion object {
        const val DEFAULT_INTERVAL_MILLIS = 500L
    }
}
