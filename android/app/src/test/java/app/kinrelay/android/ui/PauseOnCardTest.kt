package app.kinrelay.android.ui

import app.kinrelay.android.link.LinkState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PauseOnCardTest {
    @Test
    fun aLinkProblemWinsOverPause() {
        listOf(LinkState.PairAgain, LinkState.UpdateApp, LinkState.Replaced, LinkState.NotOnWifi, LinkState.PermissionNeeded).forEach {
            assertNull("$it", pauseOnCard(it, "Until you resume"))
        }
    }

    @Test
    fun pauseIsShownWhileTheLinkIsFineOrLooking() {
        assertEquals("Until you resume", pauseOnCard(LinkState.Connecting(), "Until you resume"))
        assertEquals("Until you resume", pauseOnCard(LinkState.Stopped, "Until you resume"))
    }

    @Test
    fun noPauseMeansNothingToShow() {
        assertNull(pauseOnCard(LinkState.Connecting(), null))
    }
}
