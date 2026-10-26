package app.kinrelay.android.ui

import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** The four filters above the history list. "Failed" is a send that didn't go or a file that expired; the other three are by direction. */
enum class HistoryFilter(val label: String) {
    ALL("All"), SENT("Sent"), RECEIVED("Received"), FAILED("Failed");

    fun matches(item: HistoryItem): Boolean {
        val failed = item.status == HistoryStatus.FAILED || item.status == HistoryStatus.EXPIRED
        return when (this) {
            ALL -> true
            SENT -> item.outgoing && !failed
            RECEIVED -> !item.outgoing && !failed
            FAILED -> failed
        }
    }
}

/** One day of the history: "Today", "Yesterday" or a date, with that day's items, newest first. */
class HistoryGroup(val label: String, val items: List<HistoryItem>)

/** Pure helpers for the history screen, so the tests can check them without a screen. */
object HistoryView {
    /** Splits a newest-first list into days, in the same order. */
    fun group(items: List<HistoryItem>, nowMillis: Long, zone: ZoneId = ZoneId.systemDefault(), locale: Locale = Locale.getDefault()): List<HistoryGroup> {
        val today = Instant.ofEpochMilli(nowMillis).atZone(zone).toLocalDate()
        val format = DateTimeFormatter.ofPattern("EEE d MMM", locale)
        val byDay = LinkedHashMap<java.time.LocalDate, MutableList<HistoryItem>>()
        for (item in items) byDay.getOrPut(Instant.ofEpochMilli(item.timeMillis).atZone(zone).toLocalDate()) { ArrayList() } += item
        return byDay.map { (day, list) ->
            val label = when (day) {
                today -> "Today"
                today.minusDays(1) -> "Yesterday"
                else -> format.format(day)
            }
            HistoryGroup(label, list)
        }
    }

    /** What the row's picture shows. */
    enum class Kind { TEXT, IMAGE, FILE }

    private val imageExtensions = setOf("jpg", "jpeg", "png", "webp", "gif", "heic", "heif", "bmp", "avif")

    fun kind(item: HistoryItem): Kind = when {
        item.isText -> Kind.TEXT
        item.openMime?.startsWith("image/") == true -> Kind.IMAGE
        item.openMime == null && item.title.substringAfterLast('.', "").lowercase(Locale.ROOT) in imageExtensions -> Kind.IMAGE
        else -> Kind.FILE
    }
}
