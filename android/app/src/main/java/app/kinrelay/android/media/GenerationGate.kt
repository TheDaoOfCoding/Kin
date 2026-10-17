package app.kinrelay.android.media

/**
 * One lock and a counter: work for an old run of a worker can never touch state that a later `start()` or `stop()` already took over. [MediaSource] does all of its registering and
 * unregistering of session callbacks through it, so a late pass of the reading worker can't re-register a callback after `stop()` let go of every controller.
 */
class GenerationGate {
    private var generation = 0

    /** Runs [block] under the lock and makes every earlier generation stale. Returns the new generation. */
    @Synchronized
    fun advance(block: () -> Unit): Int {
        generation++
        block()
        return generation
    }

    /** Runs [block] under the lock only while [gen] is still the current generation; null (and [block] not run) when it is stale. */
    @Synchronized
    fun <T> ifCurrent(gen: Int, block: () -> T): T? = if (gen == generation) block() else null
}
