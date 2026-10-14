package app.kinrelay.android.link

/** Waits between reconnect attempts: 5 s, 30 s, 2 min, then every 10 min (architecture.md, "Finding each other"). */
class Backoff(private val steps: List<Long> = DEFAULT_STEPS_MILLIS) {
    private var index = 0

    fun next(): Long = steps[minOf(index, steps.lastIndex)].also { index++ }

    fun reset() {
        index = 0
    }

    companion object {
        val DEFAULT_STEPS_MILLIS = listOf(5_000L, 30_000L, 120_000L, 600_000L)
    }
}
