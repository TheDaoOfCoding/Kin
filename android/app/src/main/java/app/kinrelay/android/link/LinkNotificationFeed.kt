package app.kinrelay.android.link

import app.kinrelay.android.notify.TrailingThrottle

/**
 * What reaches the persistent link notification, and the one place that decides it may still be posted (2B.4 and its review).
 *
 * A change of **state** (the structural text: connected, looking, a pause) is posted at once; a **battery-only** change goes through the 30 s trailing throttle. After [destroy] nothing is posted any
 * more, from any thread: the service's collector runs off the main thread and the throttle posts from a handler, so a callback that was already queued, or a collector that was in the middle of a
 * change, must not bring the notification back after the unlink. The check and the post are one step under a lock, so [destroy] cannot slip in between them.
 */
class LinkNotificationFeed(
    intervalMillis: Long,
    now: () -> Long,
    private val schedule: (delayMillis: Long, task: () -> Unit) -> Unit,
    private val post: (String) -> Unit,
) {
    private val lock = Any()
    private var destroyed = false
    private var latest = ""
    private var lastFull: String? = null
    private var lastStructural: String? = null

    private val throttle = TrailingThrottle(
        intervalMillis = intervalMillis,
        now = now,
        schedule = { delay, task -> if (!isDestroyed()) schedule(delay, task) },
        action = { postLatest() },
    )

    /** The notification text as it is now: [structural] without the PC's battery, [full] with it. */
    fun submit(structural: String, full: String) {
        synchronized(lock) {
            if (destroyed || full == lastFull) return
            lastFull = full
            latest = full
            if (structural != lastStructural) {
                lastStructural = structural
                post(full)
                return
            }
        }
        throttle.request()
    }

    /** The service is going away: no post from now on, whatever is still queued. */
    fun destroy() {
        synchronized(lock) { destroyed = true }
    }

    private fun isDestroyed(): Boolean = synchronized(lock) { destroyed }

    private fun postLatest() {
        synchronized(lock) {
            if (!destroyed) post(latest)
        }
    }
}
