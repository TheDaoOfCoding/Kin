package app.kinrelay.android.dnd

/**
 * Android's interruption filter, as the plain numbers of `NotificationManager.INTERRUPTION_FILTER_*` (they are compile-time constants of the platform and cannot change), so the pure
 * classes of 2B.5 and their tests need no Android.
 */
object DndFilter {
    const val UNKNOWN = 0
    const val ALL = 1
    const val PRIORITY = 2
    const val NONE = 3
    const val ALARMS = 4
}

/** The phone's filter as the follower sees it: `ALL` is "no Do Not Disturb"; any filter this code does not know is [OTHER] (it counts as "the user is already in some DND"). */
enum class DndLevel(val word: String) {
    ALL("all"),
    PRIORITY("priority"),
    ALARMS("alarms"),
    TOTAL("total"),
    OTHER("other"),
    ;

    companion object {
        fun fromWord(word: String): DndLevel = entries.firstOrNull { it.word == word } ?: OTHER
    }
}

/**
 * The wire words and the filter maps of 2B.5 (changelog 59; vectors `alerts.json` `dndMap[]`, `dndRequest[]`): the phone's filter as `presence.dnd` (`priority`, `alarms`, `total`; `ALL`, `UNKNOWN` and
 * anything else is absent), and the filter Kin requests for its "How much" setting. **Never total silence**: whatever is stored, the request is Priority or Alarms.
 */
object DndWire {
    const val PRIORITY = "priority"
    const val ALARMS = "alarms"
    const val TOTAL = "total"

    /** The wire `dnd` of an Android filter, or null (the field is left out: off, or a filter this table does not know). */
    fun wire(filter: Int): String? = when (filter) {
        DndFilter.PRIORITY -> PRIORITY
        DndFilter.ALARMS -> ALARMS
        DndFilter.NONE -> TOTAL
        else -> null
    }

    fun levelOf(filter: Int): DndLevel = when (filter) {
        DndFilter.ALL -> DndLevel.ALL
        DndFilter.PRIORITY -> DndLevel.PRIORITY
        DndFilter.ALARMS -> DndLevel.ALARMS
        DndFilter.NONE -> DndLevel.TOTAL
        else -> DndLevel.OTHER
    }

    /** The Android filter of a level Kin can request (Priority or Alarms); null for the others. */
    fun filterOf(level: DndLevel): Int? = when (level) {
        DndLevel.PRIORITY -> DndFilter.PRIORITY
        DndLevel.ALARMS -> DndFilter.ALARMS
        else -> null
    }

    /** What Kin asks for with the stored "How much" setting (`priority` or `alarms`): anything else, a missing value and "total" included, is Priority. */
    fun requestFor(setting: String?): Int = if (setting == ALARMS) DndFilter.ALARMS else DndFilter.PRIORITY

    fun levelFor(setting: String?): DndLevel = if (setting == ALARMS) DndLevel.ALARMS else DndLevel.PRIORITY
}

/**
 * Whether the alarm stream would be silent under the phone's Do Not Disturb (design 3.8 of 2B.4, corrected in the 2B.4 review): "total silence" mutes it, and so does Priority mode when the
 * user's own policy does not allow alarms. Alarms only and Priority with alarms allowed (the default) play it. The ring machine and the follower share this one rule.
 */
object AlarmAudibility {
    /** `NotificationManager.Policy.PRIORITY_CATEGORY_ALARMS`. */
    const val PRIORITY_CATEGORY_ALARMS = 32

    fun silenced(filter: Int, priorityCategories: Int): Boolean = when (filter) {
        DndFilter.NONE -> true
        DndFilter.PRIORITY -> priorityCategories and PRIORITY_CATEGORY_ALARMS == 0
        else -> false
    }
}
