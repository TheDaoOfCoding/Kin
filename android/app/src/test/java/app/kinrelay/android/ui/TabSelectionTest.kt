package app.kinrelay.android.ui

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** 0.2.1: a tab index that the row does not have crashed the app (`PrimaryTabRow(selectedTabIndex = 2)` with 2 tabs). The row is now the floating bottom nav. */
class TabSelectionTest {
    @Test
    fun theNavHasThreeTabsWithAPcAndNoneWithout() {
        assertEquals(3, TabSelection.count(paired = true))
        assertEquals(1, TabSelection.count(paired = false))
    }

    @Test
    fun aSelectionIsNeverOutsideTheRow() {
        for (paired in listOf(true, false)) {
            val count = TabSelection.count(paired)
            for (selected in -3..7) {
                val shown = TabSelection.clamp(selected, count)
                assertTrue("selected $selected of $count gave $shown", shown in 0 until count)
            }
        }
        assertEquals("Settings with 3 tabs stays Settings", 2, TabSelection.clamp(2, 3))
        assertEquals("the crash: index 2 with 2 tabs", 1, TabSelection.clamp(2, 2))
        assertEquals("index 2 with no PC: the only place there is", 0, TabSelection.clamp(2, TabSelection.count(paired = false)))
        assertEquals(0, TabSelection.clamp(-1, 3))
        assertEquals("an empty row can't crash either", 0, TabSelection.clamp(5, 0))
    }

    @Test
    fun unlinkingFromSettingsLandsOnTheFirstTab() {
        assertEquals("Settings, then the PC is unlinked", 0, TabSelection.afterPairingKnown(2, paired = false))
        assertEquals("History, then unlinked", 0, TabSelection.afterPairingKnown(1, paired = false))
    }

    @Test
    fun aRestoredTabIsKeptWhileAPcIsPaired() {
        assertEquals(2, TabSelection.afterPairingKnown(2, paired = true))
        assertEquals(1, TabSelection.afterPairingKnown(1, paired = true))
        assertEquals("a stale index is brought back into the row", 2, TabSelection.afterPairingKnown(9, paired = true))
    }

    /** The nav in `KinRoot` must be fed through [TabSelection] (a guard against a later edit that hands it `tab` again). */
    @Test
    fun theRootGivesTheNavOnlyAValidIndex() {
        val root = File("src/main/java/app/kinrelay/android/ui/KinRoot.kt").readText()
        assertTrue(root.contains("KinBottomNav("))
        assertTrue(root.contains("selected = TabSelection.clamp(tab, TabSelection.count(true))"))
        assertTrue(root.contains("TabSelection.clamp(tab, TabSelection.count(pc != null))"))
    }
}
