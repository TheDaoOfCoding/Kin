package app.kinrelay.android.media

import app.kinrelay.android.link.Clock

/**
 * A token bucket in whole milliseconds: [capacity] tokens, one more every [refillMillis]. Integer arithmetic on purpose: a token that is due at exactly 250 ms must be
 * there at 250 ms (a floating-point bucket can be a hair short). Vectors `media.json` `rate[]` (`sourceState`, `viewerState`, `sourceCmd`).
 */
class MediaBucket(private val capacity: Int, private val refillMillis: Long, private val clock: Clock) {
    private var credit = capacity * refillMillis
    private var at = clock.elapsedMillis()

    @Synchronized
    fun tryTake(): Boolean {
        val now = clock.elapsedMillis()
        credit = minOf(capacity * refillMillis, credit + maxOf(0L, now - at))
        at = now
        if (credit < refillMillis) return false
        credit -= refillMillis
        return true
    }
}

/** At most [max] events in any window of [windowMillis] (the viewer's `media.cmd` limit and the image limit): an event is accepted when fewer than [max] happened in the last window. */
class SlidingWindow(private val max: Int, private val windowMillis: Long, private val clock: Clock) {
    private val times = ArrayDeque<Long>()

    @Synchronized
    fun tryTake(): Boolean {
        val now = clock.elapsedMillis()
        while (times.isNotEmpty() && now - times.first() >= windowMillis) times.removeFirst()
        if (times.size >= max) return false
        times.addLast(now)
        return true
    }
}

/** "At most one error answer in [spacingMillis]" (`rate_limited` answers): true for the first, then again once the spacing has passed. */
class ErrorSpacer(private val spacingMillis: Long, private val clock: Clock) {
    private var last = Long.MIN_VALUE

    @Synchronized
    fun mayAnswer(): Boolean {
        val now = clock.elapsedMillis()
        if (last != Long.MIN_VALUE && now - last < spacingMillis) return false
        last = now
        return true
    }
}

/** The five limits of design 3.6 as they are built for one connection. */
object MediaLimits {
    const val STATE_THROTTLE_MILLIS = 250L
    const val CLEAR_DEBOUNCE_MILLIS = 1_500L
    const val PAUSED_EXPIRY_MILLIS = 30L * 60 * 1000
    const val POSITION_JUMP_MILLIS = 1_500L
    const val END_GRACE_MILLIS = 5_000L
    const val ERROR_SPACING_MILLIS = 10_000L
    const val STATUS_WINDOW_MILLIS = 5_000L
    const val COMMAND_COALESCE_MILLIS = 150L

    fun sourceState(clock: Clock) = MediaBucket(10, 250, clock)
    fun viewerState(clock: Clock) = MediaBucket(20, 200, clock)
    fun sourceCmd(clock: Clock) = MediaBucket(20, 500, clock)
    fun viewerCmd(clock: Clock) = SlidingWindow(20, 10_000, clock)
    fun image(clock: Clock) = SlidingWindow(20, 60_000, clock)
}
