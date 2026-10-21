package app.kinrelay.android.protocol

/**
 * Remembers processed message IDs with the response that was sent, so a repeat gets the same
 * response without being processed again. An ID is forgotten only when it is both older than
 * 24 hours and beyond the newest 1,000. [onChange] lets the caller persist additions and removals.
 */
class DedupMemory(
    private val nowMillis: () -> Long,
    initial: List<Entry> = emptyList(),
    private val onChange: (added: Entry?, removedIds: List<String>) -> Unit = { _, _ -> },
) {
    data class Entry(val id: String, val response: String, val atMillis: Long)

    private val entries = LinkedHashMap<String, Entry>().apply { initial.sortedBy { it.atMillis }.forEach { put(it.id, it) } }

    val size: Int @Synchronized get() = entries.size

    @Synchronized
    fun lookup(id: String): String? = entries[id]?.response

    @Synchronized
    fun record(id: String, response: String) {
        val entry = Entry(id, response, nowMillis())
        entries.remove(id)
        entries[id] = entry
        val removed = prune()
        onChange(entry, removed)
    }

    private fun prune(): List<String> {
        val removed = mutableListOf<String>()
        val cutoff = nowMillis() - MIN_AGE_MILLIS
        val iterator = entries.values.iterator()
        while (entries.size > MIN_COUNT && iterator.hasNext()) {
            val oldest = iterator.next()
            if (oldest.atMillis >= cutoff) break
            iterator.remove()
            removed += oldest.id
        }
        return removed
    }

    companion object {
        const val MIN_COUNT = 1_000
        const val MIN_AGE_MILLIS = 24L * 60 * 60 * 1000
    }
}
