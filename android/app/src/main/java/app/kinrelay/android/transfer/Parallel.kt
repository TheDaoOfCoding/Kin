package app.kinrelay.android.transfer

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

/**
 * Maps every item with [transform] on IO threads, at most [limit] at a time, and returns the results in the order of the items. Used
 * for the per-file provider look-ups (`stat`) of a share: 50 photos looked at one by one took seconds, because each look-up is a
 * round trip to the gallery app's provider.
 */
internal suspend fun <T, R> List<T>.mapParallel(limit: Int, transform: (T) -> R): List<R> {
    if (size <= 1 || limit <= 1) return map(transform)
    return coroutineScope {
        val gate = Semaphore(limit)
        map { item -> async(Dispatchers.IO) { gate.withPermit { transform(item) } } }.awaitAll()
    }
}
