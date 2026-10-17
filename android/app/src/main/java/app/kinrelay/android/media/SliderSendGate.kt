package app.kinrelay.android.media

/**
 * Spaces the intermediate values of a slider drag: the first goes at once, the next only after [gapMillis]. The value where the finger lifts is sent by the caller regardless
 * ([reset] then lets the next drag start fresh), so a long drag can't lose its final value to the sender's 20 commands per 10 s.
 */
class SliderSendGate(private val gapMillis: Long) {
    private var last = Long.MIN_VALUE

    fun mayCommit(now: Long): Boolean {
        if (last != Long.MIN_VALUE && now - last < gapMillis) return false
        last = now
        return true
    }

    fun reset() {
        last = Long.MIN_VALUE
    }
}
