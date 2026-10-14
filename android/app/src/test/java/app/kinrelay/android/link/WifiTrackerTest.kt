package app.kinrelay.android.link

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WifiTrackerTest {
    private val tracker = WifiTracker<String>()

    @Test
    fun noNetworkAtTheStart() {
        assertNull(tracker.current)
        assertTrue(tracker.isEmpty)
    }

    @Test
    fun theNewestNetworkIsTheCurrentOne() {
        tracker.available("home")
        assertEquals("home", tracker.current)
        tracker.available("office")
        assertEquals("office", tracker.current)
    }

    @Test
    fun losingTheOnlyNetworkMeansNotOnWifi() {
        tracker.available("home")
        val lost = tracker.lost("home")
        assertTrue(lost.wasCurrent)
        assertNull(lost.next)
        assertNull(tracker.current)
    }

    @Test
    fun losingTheNewerOfTwoMovesToTheOtherInsteadOfSayingNotOnWifi() {
        // The bug: with two Wi-Fi networks, losing the newer one showed "Not on Wi-Fi" while the older was still up.
        tracker.available("home")
        tracker.available("hotspot")
        val lost = tracker.lost("hotspot")
        assertTrue(lost.wasCurrent)
        assertEquals("home", lost.next)
        assertEquals("home", tracker.current)
        assertFalse(tracker.isEmpty)
    }

    @Test
    fun losingTheOlderOfTwoChangesNothing() {
        tracker.available("home")
        tracker.available("hotspot")
        val lost = tracker.lost("home")
        assertFalse(lost.wasCurrent)
        assertEquals("hotspot", lost.next)
        assertEquals("hotspot", tracker.current)
        // and now the hotspot goes: nothing is left
        assertNull(tracker.lost("hotspot").next)
    }

    @Test
    fun withThreeNetworksTheNewestRemainingTakesOver() {
        tracker.available("a")
        tracker.available("b")
        tracker.available("c")
        assertEquals("b", tracker.lost("c").next)
        assertEquals("a", tracker.lost("b").next)
        assertNull(tracker.lost("a").next)
    }

    @Test
    fun aNetworkThatComesBackIsTheNewestAgain() {
        tracker.available("a")
        tracker.available("b")
        tracker.lost("b")
        tracker.available("b")
        assertEquals("b", tracker.current)
        assertEquals("a", tracker.lost("b").next)
    }

    @Test
    fun anUnknownNetworkLostIsIgnored() {
        tracker.available("a")
        val lost = tracker.lost("never-seen")
        assertFalse(lost.wasCurrent)
        assertEquals("a", lost.next)
        assertEquals("a", tracker.current)
    }

    @Test
    fun availableTwiceDoesNotDuplicate() {
        tracker.available("a")
        tracker.available("a")
        assertNull(tracker.lost("a").next)
        assertTrue(tracker.isEmpty)
    }
}
