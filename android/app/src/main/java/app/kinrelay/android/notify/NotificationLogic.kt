package app.kinrelay.android.notify

/**
 * Which notifications of one group are up, oldest first, with a cap. Android drops **new** notifications of an app that already has 50
 * (and a phone with a long burst of arrivals reached that: the 1.3 review), so every group keeps at most [maxSlots] of them: a new one
 * beyond the cap takes the place of the oldest, which is cancelled. One slot is one transfer (its progress notification and, later, its
 * result); a slot keeps its place when it is refreshed.
 *
 * The values are the titles shown in the group's summary notification.
 *
 * The ledger is the **record of what this app posted**, with the time of each post. `NotificationManager.activeNotifications` lags a `notify`
 * that was just made (a burst of 15 files showed a summary of "3 files" with 10 results on screen, and slots that were forgotten too early
 * left their notifications on screen beyond the cap), so it is never trusted for something posted a moment ago: [retainOnly] forgets a slot
 * only when its notification is missing from the system's list **and** the slot was last posted more than [graceMillis] ago.
 */
class NotificationLedger(val maxSlots: Int, private val now: () -> Long = System::currentTimeMillis) {
    private val slots = LinkedHashMap<String, String>()
    private val postedAt = HashMap<String, Long>()

    val size: Int get() = slots.size

    /** The slot keys, oldest first. */
    val keys: List<String> get() = slots.keys.toList()

    /** The titles, oldest first. */
    val titles: List<String> get() = slots.values.toList()

    /** Takes (or refreshes) the slot [key]. Returns the keys of the slots that were pushed out to make room, oldest first: cancel them. */
    fun claim(key: String, title: String): List<String> {
        val known = slots.containsKey(key)
        slots[key] = title
        postedAt[key] = now()
        if (known) return emptyList()
        val evicted = ArrayList<String>()
        while (slots.size > maxSlots) {
            val oldest = slots.keys.first()
            if (oldest == key) break
            slots.remove(oldest)
            postedAt.remove(oldest)
            evicted += oldest
        }
        return evicted
    }

    fun release(key: String): Boolean {
        postedAt.remove(key)
        return slots.remove(key) != null
    }

    /**
     * Forgets the slots whose notification is gone (dismissed, timed out): a slot stays while its key is in [live], and also while it was
     * posted less than [graceMillis] ago (the system's list may not show it yet). Returns how many were forgotten.
     */
    fun retainOnly(live: Set<String>, graceMillis: Long = MISSING_GRACE_MILLIS): Int {
        val at = now()
        val gone = slots.keys.filter { it !in live && at - (postedAt[it] ?: Long.MIN_VALUE / 2) > graceMillis }
        gone.forEach { release(it) }
        return gone.size
    }

    operator fun contains(key: String) = slots.containsKey(key)

    companion object {
        /** How long a notification may be missing from the system's list before its slot is forgotten. */
        const val MISSING_GRACE_MILLIS = 2_000L
    }
}

/**
 * "N texts from <PC>": the texts that arrive from the PC share **one** notification that is updated, so a burst of 60 texts is one
 * notification and not 60. It keeps the count and the last few previews; a sensitive text counts but shows no preview ([null]).
 */
class TextArrivals(private val maxLines: Int = 5) {
    data class State(
        val pcName: String,
        val count: Int,
        /** The newest previews, oldest first. Null is a sensitive text: no preview, ever. */
        val lines: List<String?>,
        /** The newest text's link, if it is one that may be opened (`http` or `https`); null otherwise. */
        val openUrl: String?,
    )

    private var pcName = ""
    private var count = 0
    private var lines = ArrayDeque<String?>()
    private var openUrl: String? = null

    /** Adds one arrival and returns the state to show. [preview] is null for a sensitive text. */
    @Synchronized
    fun add(pcName: String, preview: String?, openUrl: String?): State {
        if (this.pcName != pcName) reset()
        this.pcName = pcName
        count++
        lines.addLast(preview)
        while (lines.size > maxLines) lines.removeFirst()
        this.openUrl = openUrl
        return state()
    }

    @Synchronized
    fun state(): State = State(pcName, count, lines.toList(), openUrl)

    /** The user saw or dismissed the notification: the next text starts a new one. */
    @Synchronized
    fun reset() {
        count = 0
        lines.clear()
        openUrl = null
    }

    @Synchronized
    fun isEmpty(): Boolean = count == 0

    companion object {
        fun title(state: State): String = if (state.count == 1) "Text from ${state.pcName}" else "${state.count} texts from ${state.pcName}"
        const val SENSITIVE_LINE = "Sensitive text"
    }
}

/**
 * Runs an action at most once per [intervalMillis], and always runs the **last** request: when requests come faster, the later ones are
 * merged into one run at the end of the interval. Android drops updates of a notification that is updated too fast, and the update that
 * is dropped could be the final count; with this the final state is always posted.
 */
class TrailingThrottle(
    private val intervalMillis: Long,
    private val now: () -> Long,
    private val schedule: (delayMillis: Long, task: () -> Unit) -> Unit,
    private val action: () -> Unit,
) {
    private var lastRun = Long.MIN_VALUE / 2
    private var scheduled = false

    fun request() {
        val wait: Long
        synchronized(this) {
            if (scheduled) return
            val since = now() - lastRun
            if (since >= intervalMillis) {
                lastRun = now()
                wait = -1
            } else {
                scheduled = true
                wait = intervalMillis - since
            }
        }
        if (wait < 0) {
            action()
        } else {
            schedule(wait) {
                synchronized(this) {
                    scheduled = false
                    lastRun = now()
                }
                action()
            }
        }
    }
}
