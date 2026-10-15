package app.kinrelay.android.data

/** How long the queue keeps things, and how much history there is. */
object QueuePolicy {
    /** A queued text or upload that has waited this long expires: it is shown as failed ("expired") and never goes out (milestone 1.4). */
    const val EXPIRY_MILLIS = 7L * 24 * 60 * 60 * 1000

    /** The history shows, and the database keeps, the newest this many items of each kind. */
    const val HISTORY_LIMIT = 500

    /** Expired at exactly [EXPIRY_MILLIS] (the same edge as the PC's 24 hours: the window is open while the age is below it). */
    fun isExpired(createdAtMillis: Long, nowMillis: Long): Boolean = nowMillis - createdAtMillis >= EXPIRY_MILLIS
}
