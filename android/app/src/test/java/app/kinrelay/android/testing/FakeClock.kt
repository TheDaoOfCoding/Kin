package app.kinrelay.android.testing

/**
 * The PC's clock in a sequence of `protocol/http.json`: a step `{ "clock": { "advanceHours": N } }` sends no request and moves it
 * forward by N hours (the 24 hours a finished transfer's ID answers for). Starts at 0.
 */
class FakeClock {
    var nowMillis: Long = 0
        private set

    fun advanceHours(hours: Long) {
        require(hours > 0) { "a clock only moves forward: $hours" }
        nowMillis += hours * HOUR_MILLIS
    }

    companion object {
        const val HOUR_MILLIS = 3_600_000L
    }
}
