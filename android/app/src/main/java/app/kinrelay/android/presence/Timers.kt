package app.kinrelay.android.presence

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** A one-shot timer that can be called off. */
fun interface Cancel {
    fun cancel()
}

/**
 * One-shot timers for the pure classes of 2B.4 (`PresenceSender`, `RingMachine`, `ShotWatcherCore`). The app's one is a coroutine `delay` ([CoroutineTimers]); the tests use a
 * virtual clock. Nothing here is periodic: every timer is a single shot that something started on purpose.
 */
fun interface Timers {
    fun after(delayMillis: Long, task: () -> Unit): Cancel
}

/** The real [Timers]: a coroutine on [scope]. */
class CoroutineTimers(private val scope: CoroutineScope) : Timers {
    override fun after(delayMillis: Long, task: () -> Unit): Cancel {
        val job = scope.launch {
            delay(delayMillis)
            task()
        }
        return Cancel { job.cancel() }
    }
}
