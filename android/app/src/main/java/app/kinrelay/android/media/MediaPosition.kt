package app.kinrelay.android.media

/** Position and clocks (design 3.4, vectors `position[]` and `sourcePosition[]`): time is never sent as a clock value, only the position as of the moment of sending, and each side extrapolates. */
object MediaPosition {
    /** What a viewer shows: [positionMs] null means "unknown" (no progress bar), [state] may differ from the sent one (rate 0 and a run past the end are shown paused). */
    data class Shown(val positionMs: Long?, val state: PlayState)

    /**
     * What the viewer shows [elapsedMs] after the state arrived (the anchor is its own monotonic clock at arrival). Only a playing state moves; a rate over 4,000 counts as 4,000;
     * a rate of 0 is shown paused; a playing state whose extrapolated position is more than 5 s past the duration is shown paused at the end (a lost message can't leave a bar running).
     */
    fun shown(state: PlayState, positionMs: Long?, rateMilli: Int?, durationMs: Long?, elapsedMs: Long): Shown {
        val rate = (rateMilli ?: MediaWire.DEFAULT_RATE_MILLI).coerceIn(0, MediaWire.MAX_RATE_MILLI)
        val effective = if (state == PlayState.PLAYING && rate == 0) PlayState.PAUSED else state
        if (positionMs == null) return Shown(null, effective)
        if (effective != PlayState.PLAYING) return Shown(clamp(positionMs, durationMs), effective)
        val raw = positionMs + maxOf(0L, elapsedMs) * rate / 1000
        if (durationMs != null && raw > durationMs + MediaLimits.END_GRACE_MILLIS) return Shown(durationMs, PlayState.PAUSED)
        return Shown(clamp(raw, durationMs), PlayState.PLAYING)
    }

    /**
     * The source's own position for "now" on Android (3.4): `PlaybackState.position` is the position at `lastPositionUpdateTime`, both on `SystemClock.elapsedRealtime()`; while the state is
     * playing, `now = position + (now - updateTime) x speed`. [position] -1 (`PLAYBACK_POSITION_UNKNOWN`) is null. A clock that went back adds nothing; the result is cut to 0..[durationMs].
     */
    fun sourceAndroid(state: PlayState, position: Long, updateTimeMs: Long, speed: Float, nowMs: Long, durationMs: Long?): Long? {
        if (position < 0) return null
        if (state != PlayState.PLAYING) return clamp(position, durationMs)
        val elapsed = maxOf(0L, nowMs - updateTimeMs)
        val moved = (elapsed * maxOf(0.0, speed.toDouble())).toLong()
        return clamp(position + moved, durationMs)
    }

    private fun clamp(value: Long, durationMs: Long?): Long = maxOf(0L, if (durationMs != null) minOf(value, durationMs) else value)
}
