package app.kinrelay.android.alerts

/**
 * "N alerts from <PC>": the alerts that arrive from the PC share **one** notification that is updated, like the texts do (`TextArrivals`): a runaway script is one notification and not fifty (Android
 * drops new notifications beyond 50 per app). It keeps the count and the last few entries, and the memory of the latest. Memory only, never stored; the content is private on a lock screen.
 */
class AlertArrivals(private val maxLines: Int = 5) {
    /** One arrival as the notification shows it. */
    data class Entry(val title: String?, val text: String, val waitedMs: Long?)

    data class State(val pcName: String, val count: Int, /** The newest entries, oldest first. */ val entries: List<Entry>)

    private var pcName = ""
    private var count = 0
    private val entries = ArrayDeque<Entry>()

    @Synchronized
    fun add(pcName: String, entry: Entry): State {
        if (this.pcName != pcName) reset()
        this.pcName = pcName
        count++
        entries.addLast(entry)
        while (entries.size > maxLines) entries.removeFirst()
        return state()
    }

    @Synchronized
    fun state(): State = State(pcName, count, entries.toList())

    /** The user opened the app or dismissed the notification: the next alert starts a new one. */
    @Synchronized
    fun reset() {
        count = 0
        entries.clear()
    }

    @Synchronized
    fun isEmpty(): Boolean = count == 0

    companion object {
        /** The heading: the alert's own title (or the PC's name) for one alert, "N alerts from <PC>" for several. */
        fun title(state: State): String =
            if (state.count == 1) state.entries.lastOrNull()?.title ?: state.pcName else "${state.count} alerts from ${state.pcName}"

        /** What the lock screen shows instead of the content (the 2B.1 and 2B.3 rule). */
        fun publicTitle(state: State): String = if (state.count == 1) "Alert from ${state.pcName}" else "${state.count} alerts from ${state.pcName}"

        /** "3 h ago" for an alert that waited more than 2 minutes in the PC's queue, null otherwise (the number is the PC's: clock-independent). */
        fun waitedText(waitedMs: Long?): String? {
            if (waitedMs == null || waitedMs <= AlertWire.WAITED_SHOWN_OVER_MILLIS) return null
            val minutes = waitedMs / 60_000
            return when {
                minutes < 60 -> "$minutes min ago"
                minutes < 60 * 48 -> "${minutes / 60} h ago"
                else -> "${minutes / (60 * 24)} d ago"
            }
        }

        /** The sub text of a single alert: the PC's name, and how long it waited when that is over 2 minutes ("Desk PC · 3 h ago"). */
        fun subText(pcName: String, waitedMs: Long?): String = waitedText(waitedMs)?.let { "$pcName · $it" } ?: pcName

        /** One line of a stack of several: "<title>: <text>" (or just the text), with how long it waited. */
        fun line(entry: Entry): String {
            val body = if (entry.title != null) "${entry.title}: ${entry.text}" else entry.text
            return waitedText(entry.waitedMs)?.let { "$body · $it" } ?: body
        }
    }
}
