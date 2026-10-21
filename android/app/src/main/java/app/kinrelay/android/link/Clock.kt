package app.kinrelay.android.link

/** A monotonic clock that keeps counting while the phone sleeps (`SystemClock.elapsedRealtime` on Android). */
fun interface Clock {
    fun elapsedMillis(): Long
}
