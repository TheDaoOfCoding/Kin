package app.kinrelay.android.dnd

/**
 * When Kin must look again at a Do Not Disturb it holds, so a phone can never stay silent because a timer slept (2B.5 review of risk R44). Pure: the times are `elapsedRealtime` (which counts
 * the time the phone spends in deep sleep), and the tests move them by hand.
 *
 * Two timers of the design do **not** run while the phone sleeps: the 60 s grace (a coroutine `delay`) and the link's dead-link detection (OkHttp's ping, 240 s and 240 s more for the pong, on
 * the monotonic clock that stops with the CPU). Without more, a sleeping phone could stay silent long after the PC is gone. While Kin holds, an alarm (`AlarmManager`, `ELAPSED_REALTIME_WAKEUP`,
 * `setAndAllowWhileIdle`: no exact-alarm permission) is set at [nextWake]; when it goes off, [due] says what to do:
 *
 * ```
 * not holding                              -> nothing, no alarm
 * a grace is running (the connection closed) -> the alarm is at its end; at that time the follower's own tick releases ([Due.GRACE_END])
 * connected, no sign of life for PROBE_AFTER -> ask the network whether the PC still answers ([Due.PROBE]); a silent PC ends the connection, which starts the grace above
 * not connected, no grace, no sign of life   -> release now ([Due.RELEASE]): a state the follower should not reach, and the cheap safe answer if it does
 * ```
 *
 * "A sign of life" is the connection being made, a `presence` snapshot of the PC, or a probe that found the PC answering. The bound is the design's: 240 s of silence, then the probe (10 s), then
 * the 60 s grace: 310 s as the alarms are set. `setAndAllowWhileIdle` is inexact (up to 0.75 of the delay late), so with the phone asleep the realistic bound is about 9 minutes.
 */
object DndWatchdog {
    /** A connected PC that has not been heard from for this long is asked (the design's 240 s dead-link time). */
    const val PROBE_AFTER_MILLIS = 240_000L

    enum class Due { NOTHING, GRACE_END, PROBE, RELEASE }

    /** A release that failed is first tried again at the alarm after this long (2B.5 review round 2); [retryDelay] doubles it after each failure. */
    const val RETRY_RELEASE_MILLIS = 300_000L

    /** How many failed alarm-driven retries are allowed to move the retry alarm further out: 5, 10, 20 and 40 minutes, then no alarm (2B.6). */
    const val RETRY_STEPS = 4

    /** The delay of the retry alarm after [retryStep] failed retries in a row (5, 10, 20, 40 minutes), or null once the ladder is used up. */
    fun retryDelay(retryStep: Int): Long? = if (retryStep < 0 || retryStep >= RETRY_STEPS) null else RETRY_RELEASE_MILLIS shl retryStep

    /**
     * The `elapsedRealtime` at which the alarm should go off, or null when none is needed. With [releaseOwed] (a release failed and the record is still there) and no live hold, the alarm is a
     * retry [retryDelay] after [now] **only while there is access to retry with**: with no access a retry cannot work and would only wake the phone every 5 minutes for as long as the
     * user leaves it revoked (2B.5 review round 3). The access-changed broadcast, the start of the service and the start of the app pay the debt then. With access but a call that keeps being
     * refused the delay grows with [retryStep] and, after the fourth failure, the alarm stops (2B.6): the same chances, and "Release now", remain.
     */
    fun nextWake(holding: Boolean, graceEnd: Long?, lastAliveAt: Long, releaseOwed: Boolean = false, now: Long = 0L, accessAvailable: Boolean = true, retryStep: Int = 0): Long? {
        if (!holding) return if (releaseOwed && accessAvailable) retryDelay(retryStep)?.let { now + it } else null
        return graceEnd ?: (lastAliveAt + PROBE_AFTER_MILLIS)
    }

    /** What the alarm (or any other look at the state) should do at [now]. */
    fun due(now: Long, holding: Boolean, graceEnd: Long?, connected: Boolean, lastAliveAt: Long): Due {
        if (!holding) return Due.NOTHING
        if (graceEnd != null) return if (now >= graceEnd) Due.GRACE_END else Due.NOTHING
        if (now < lastAliveAt + PROBE_AFTER_MILLIS) return Due.NOTHING
        return if (connected) Due.PROBE else Due.RELEASE
    }
}
