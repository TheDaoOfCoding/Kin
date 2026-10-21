package app.kinrelay.android.ui

/**
 * The tab row of [KinRoot] has 3 tabs (Kin, History, Settings) with a PC paired and 2 (Kin, Debug) without. The selected index is kept across
 * rotation and process death and is set by a notification tap, so it can be 2 while the row has 2 tabs: a PC that was unlinked from the Settings
 * tab, or a screen restored before the paired PC is read from the database. `PrimaryTabRow(selectedTabIndex = 2)` with 2 tabs throws
 * `IndexOutOfBoundsException` in its indicator (0.2.0 crash, fixed in 0.2.1). Everything that hands an index to the row goes through here.
 */
object TabSelection {
    const val PAIRED_TABS = 3
    const val UNPAIRED_TABS = 2

    fun count(paired: Boolean): Int = if (paired) PAIRED_TABS else UNPAIRED_TABS

    /** [selected] if the row has such a tab, else the last one (never below the first). */
    fun clamp(selected: Int, count: Int): Int = selected.coerceIn(0, (count - 1).coerceAtLeast(0))

    /**
     * The selection to keep once the paired state is known to have changed to [paired]: with no PC the old index means nothing (2 was
     * Settings, 1 was History, but 1 is Debug here), so a row without a PC starts again at the first tab. With a PC any valid index is kept.
     */
    fun afterPairingKnown(selected: Int, paired: Boolean): Int = if (paired) clamp(selected, PAIRED_TABS) else 0
}
