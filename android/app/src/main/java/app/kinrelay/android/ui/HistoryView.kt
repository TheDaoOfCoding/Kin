package app.kinrelay.android.ui

import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus
import app.kinrelay.android.history.HistoryType
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** How the Filter orders the list. Newest is the default. */
enum class HistorySort(val label: String) { NEWEST("Newest"), OLDEST("Oldest"), NAME("Name"), SIZE("Size") }

/** Which way an item went. Failed and waiting items count in their direction. */
enum class HistoryDirection(val label: String) { BOTH("Both"), SENT("Sent"), RECEIVED("Received") }

/**
 * What the Filter sheet has chosen (plan 01, contract C3). The four groups combine with AND; several [types] combine with OR. The default shows
 * everything, newest first. It lives in `rememberSaveable` (see `historyQuerySaver`): it survives a rotation, never a restart.
 */
data class HistoryQuery(
    val search: String = "",
    val types: Set<HistoryType> = emptySet(),
    val sort: HistorySort = HistorySort.NEWEST,
    val direction: HistoryDirection = HistoryDirection.BOTH,
) {
    val isDefault: Boolean get() = this == HistoryQuery()
}

/** One removable chip under the title: its words, and what the query becomes when its x is tapped. */
class HistoryChip(val label: String, val remove: (HistoryQuery) -> HistoryQuery)

/** One day of the history: "Today", "Yesterday" or a date, with that day's items, newest first. */
class HistoryGroup(val label: String, val items: List<HistoryItem>)

/** Pure helpers for the history screen, so the tests can check them without a screen. */
object HistoryView {
    /** Splits a list in time order (newest first or oldest first) into days, in the same order. */
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

    /** The longest search words a chip shows; the rest is cut. */
    private const val CHIP_SEARCH_MAX = 24

    private fun newestFirst(): Comparator<HistoryItem> = compareByDescending<HistoryItem> { it.timeMillis }.thenBy { it.key }

    /** What a text row shows (a link's title, else its address, as [HistoryItem.title] is built), or the first file name of a transfer. */
    private fun nameOf(item: HistoryItem): String = if (item.isText) item.title else item.names.firstOrNull() ?: item.title

    private fun matchesSearch(item: HistoryItem, words: String): Boolean =
        item.names.any { it.contains(words, ignoreCase = true) } ||
            item.text?.contains(words, ignoreCase = true) == true ||
            item.title.contains(words, ignoreCase = true)

    /**
     * What [query] leaves of [items], in the order it asks for. A transfer matches a type if any of its files is of that type; type, direction and
     * search combine with AND. Name: case-insensitive and locale-aware, ties newest first. Size: largest first, texts and links after all files.
     */
    fun apply(items: List<HistoryItem>, query: HistoryQuery, locale: Locale = Locale.getDefault()): List<HistoryItem> {
        val words = query.search.trim()
        val kept = items.filter { item ->
            (query.types.isEmpty() || item.types.any { it in query.types }) &&
                when (query.direction) {
                    HistoryDirection.BOTH -> true
                    HistoryDirection.SENT -> item.outgoing
                    HistoryDirection.RECEIVED -> !item.outgoing
                } &&
                (words.isEmpty() || matchesSearch(item, words))
        }
        val newest = newestFirst()
        return when (query.sort) {
            HistorySort.NEWEST -> kept.sortedWith(newest)
            HistorySort.OLDEST -> kept.sortedWith(newest).reversed()
            HistorySort.NAME -> {
                val collator = java.text.Collator.getInstance(locale).apply { strength = java.text.Collator.PRIMARY }
                kept.sortedWith(Comparator<HistoryItem> { a, b -> collator.compare(nameOf(a), nameOf(b)) }.then(newest))
            }
            HistorySort.SIZE -> kept.sortedWith(compareBy<HistoryItem> { it.isText }.thenByDescending { if (it.isText) 0L else it.sizeBytes }.then(newest))
        }
    }

    /** The chips for what [query] has set: a type each, a direction other than Both, a sort other than Newest, the search words, in that order. */
    fun chips(query: HistoryQuery): List<HistoryChip> {
        val out = ArrayList<HistoryChip>()
        val words = query.search.trim()
        for (type in HistoryType.entries) if (type in query.types) out += HistoryChip(type.label) { it.copy(types = it.types - type) }
        if (query.direction != HistoryDirection.BOTH) out += HistoryChip(query.direction.label) { it.copy(direction = HistoryDirection.BOTH) }
        if (query.sort != HistorySort.NEWEST) out += HistoryChip(query.sort.label) { it.copy(sort = HistorySort.NEWEST) }
        if (words.isNotEmpty()) {
            val label = if (words.length > CHIP_SEARCH_MAX) words.take(CHIP_SEARCH_MAX).trimEnd() + "…" else words
            out += HistoryChip(label) { it.copy(search = "") }
        }
        return out
    }

    /** Days only make sense when the list is in time order; sorted by name or size it is one list without day labels. */
    fun byDay(sort: HistorySort): Boolean = sort == HistorySort.NEWEST || sort == HistorySort.OLDEST

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
