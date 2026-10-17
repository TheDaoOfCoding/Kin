package app.kinrelay.android.media

/** Volume mapping on Android (design 4.4, vectors `volumeLevel[]` and `volumeSet[]`): native index <-> `level` 0..100, a `delta` that moves at least one native step, and a floor of index 1. */
object MediaVolumeMath {
    /** `level = round(index x 100 / max)`. */
    fun levelOf(index: Int, max: Int): Int = if (max <= 0) 0 else Math.round(index * 100f / max).coerceIn(0, 100)

    /** `index = round(level x max / 100)`, but never 0: Kin sets the lowest level to index 1 (only `mute` silences), so a `SecurityException` for "volume to zero" (Do Not Disturb) can't be hit by a level or delta. */
    fun indexOfLevel(level: Int, max: Int): Int = floor(Math.round(level.coerceIn(0, 100) * max / 100f), max)

    /** A `delta` of percentage points moves the index by `round(|delta| x max / 100)` steps, at least one, in its direction; never below the floor or above [max]. */
    fun indexAfterDelta(index: Int, delta: Int, max: Int): Int {
        val steps = maxOf(1, Math.round(Math.abs(delta) * max / 100f))
        val moved = if (delta > 0) index + steps else index - steps
        return floor(moved, max)
    }

    private fun floor(index: Int, max: Int): Int = index.coerceIn(minOf(1, max), max)
}
