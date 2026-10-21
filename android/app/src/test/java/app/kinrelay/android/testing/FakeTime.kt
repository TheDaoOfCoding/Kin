package app.kinrelay.android.testing

import app.kinrelay.android.link.Clock
import app.kinrelay.android.presence.Cancel
import app.kinrelay.android.presence.Timers

/**
 * A virtual clock and its one-shot timers, for the pure classes of 2B.4 ([app.kinrelay.android.presence.PresenceSender], [app.kinrelay.android.ring.RingMachine]): time only moves when a test
 * says so, and a timer runs at the moment it is due (in the order it was set when two are due together).
 */
class FakeTime(start: Long = 0L) : Clock, Timers {
    var now: Long = start
        private set

    private class Task(val at: Long, val seq: Int, val run: () -> Unit) {
        var cancelled = false
    }

    private val tasks = ArrayList<Task>()
    private var seq = 0

    override fun elapsedMillis(): Long = now

    override fun after(delayMillis: Long, task: () -> Unit): Cancel {
        val t = Task(now + delayMillis, seq++, task)
        tasks += t
        return Cancel { t.cancelled = true }
    }

    /** The number of timers that are set and not cancelled or run (a class that has no periodic timer has none when idle). */
    val pending: Int get() = tasks.count { !it.cancelled }

    /** Runs every timer due **at or before** [time], in order, then sets the clock to [time]. */
    fun advanceTo(time: Long) {
        while (true) {
            val next = tasks.filter { !it.cancelled && it.at <= time }.minWithOrNull(compareBy<Task>({ it.at }, { it.seq })) ?: break
            tasks.remove(next)
            now = maxOf(now, next.at)
            next.run()
        }
        now = maxOf(now, time)
    }

    fun advanceBy(millis: Long) = advanceTo(now + millis)
}
