package app.kinrelay.android.shots

import org.junit.Assert.assertEquals
import org.junit.Test

/** 2B.5 review round 2, B2: the words of the hold. The notification itself is the cancel button, and says so. */
class ShotHoldTextTest {
    @Test
    fun theTitleNamesThePcAndTheSeconds() {
        assertEquals("Sending screenshot to Desk PC in 5 s", ShotHoldText.title(ShotKind.SCREENSHOT, "Desk PC", ShotWatcherCore.HOLD_MILLIS / 1000))
        assertEquals("Sending photo to your PC in 5 s", ShotHoldText.title(ShotKind.CAMERA, "your PC", 5))
    }

    @Test
    fun theLineUnderItSaysATapCancels() {
        assertEquals("Tap to cancel", ShotHoldText.TAP_TO_CANCEL)
        assertEquals("Cancel", ShotHoldText.CANCEL_ACTION)
    }
}
