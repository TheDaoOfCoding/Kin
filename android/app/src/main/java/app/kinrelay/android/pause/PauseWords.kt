package app.kinrelay.android.pause

import java.time.Instant
import java.time.ZoneId

/** The words for a pause end, as the tile, Home and the notification say them. */
object PauseWords {
    /**
     * "14:30" for an end today, "tomorrow 08:00" for tomorrow, "Sat 08:00" for a later day. [clock] writes a time of day ("14:30" or "2:30 PM": the app uses the phone's setting). A pause with no end
     * says "until resumed".
     */
    fun until(state: PauseState, nowMillis: Long, zone: ZoneId, clock: (Long) -> String, dayName: (Long) -> String): String {
        if (state.untilResumed) return "until resumed"
        val end = Instant.ofEpochMilli(state.untilMillis).atZone(zone)
        val today = Instant.ofEpochMilli(nowMillis).atZone(zone).toLocalDate()
        val time = clock(state.untilMillis)
        return when {
            end.toLocalDate() == today -> time
            end.toLocalDate() == today.plusDays(1) -> "tomorrow $time"
            else -> "${dayName(state.untilMillis)} $time"
        }
    }

    /** "Until 14:30", "Until tomorrow 08:00", "Until resumed". */
    fun untilSentence(state: PauseState, nowMillis: Long, zone: ZoneId, clock: (Long) -> String, dayName: (Long) -> String): String {
        val words = until(state, nowMillis, zone, clock, dayName)
        return "Until " + if (words == "until resumed") "resumed" else words
    }

    /** "Paused until 18:30", "Paused until resumed": the one-row Pause card while a pause is on, from [untilSentence]'s "Until 18:30". */
    fun pausedLine(untilSentence: String): String = "Paused " + untilSentence.replaceFirstChar(Char::lowercase)

    fun choiceLabel(choice: PauseChoice): String = when (choice) {
        PauseChoice.HOUR -> "1 hour"
        PauseChoice.TOMORROW -> "Until tomorrow"
        PauseChoice.UNTIL_RESUMED -> "Until I resume"
    }

    /** "3 received while paused" (the quiet arrivals, in memory); null for none. */
    fun receivedWhilePaused(count: Int): String? = if (count <= 0) null else "$count received while paused"
}
