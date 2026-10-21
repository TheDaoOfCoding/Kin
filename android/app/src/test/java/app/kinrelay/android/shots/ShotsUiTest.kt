package app.kinrelay.android.shots

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The permission paths of auto-send (design 5.6): Android 14's "Select photos" is a session-only grant that does not work for a background watcher, and Kin does not declare
 * `READ_MEDIA_VISUAL_USER_SELECTED`, so the system's answer at the point of use is the only truth. A switch that is on without it says "Needs access to all photos".
 */
class ShotsUiTest {
    @Test
    fun aSwitchOnWithoutAccessNeedsAccessAndOneOffNeverDoes() {
        assertTrue(ShotsUi(screenshots = true, permissionGranted = false).needsAccess)
        assertTrue(ShotsUi(camera = true, permissionGranted = false).needsAccess)
        assertFalse(ShotsUi(screenshots = true, permissionGranted = true).needsAccess)
        assertFalse("nothing asked, nothing missing", ShotsUi(permissionGranted = false).needsAccess)
        assertFalse(ShotsUi().anyOn)
    }

    @Test
    fun aSwitchOnWithoutAVisibleHoldNeedsNotificationsAndNothingAskedNeedsNothing() {
        assertTrue(ShotsUi(screenshots = true, notificationsOk = false).needsNotifications)
        assertTrue(ShotsUi(camera = true, notificationsOk = false).needsNotifications)
        assertFalse(ShotsUi(screenshots = true, notificationsOk = true).needsNotifications)
        assertFalse("nothing asked, nothing missing", ShotsUi(notificationsOk = false).needsNotifications)
        assertFalse("the default says nothing is missing", ShotsUi(screenshots = true).needsNotifications)
    }
}
