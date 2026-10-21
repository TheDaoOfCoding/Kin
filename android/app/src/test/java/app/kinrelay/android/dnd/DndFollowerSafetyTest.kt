package app.kinrelay.android.dnd

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The 2B.5 review's two follower cases: no request without a live connection, and a filter read before a resume (risk R44 and T4 of the design). */
class DndFollowerSafetyTest {
    private fun words(actions: List<DndAction>) = actions.map { it.word }

    private fun holding(): DndFollower {
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        assertEquals(listOf("request:priority"), words(f.onSnapshot(10, true)))
        f.setFilter(DndLevel.PRIORITY) // what the system reports after Kin's request
        return f
    }

    @Test
    fun aPauseDuringTheGraceReleasesAndAResumeInsideItDoesNotRequestWithNoConnection() {
        val f = holding()
        assertEquals(emptyList<String>(), words(f.onClose(1_000))) // grace until 61_000
        assertEquals(61_000L, f.graceDeadline())
        assertEquals(listOf("release"), words(f.onPause(2_000, true)))
        assertFalse(f.weHold)
        f.setFilter(DndLevel.ALL) // the release brought the filter back
        // The resume is inside the 60 s: the remembered Focus must not make a rising edge with no connection.
        assertEquals(emptyList<String>(), words(f.onPause(3_000, false)))
        assertFalse(f.weHold)
        assertNull("the grace of a released hold is gone", f.graceDeadline())
        assertEquals(emptyList<String>(), words(f.onTick(70_000)))
        assertEquals(emptyList<String>(), words(f.onSwitch(71_000, false)))
        assertEquals(emptyList<String>(), words(f.onSwitch(72_000, true)))
        assertEquals(emptyList<String>(), words(f.onAccess(73_000, true)))
        assertFalse(f.weHold)
    }

    @Test
    fun theNextRealConnectionIsStillARisingEdge() {
        val f = holding()
        f.onClose(1_000)
        f.onPause(2_000, true)
        f.setFilter(DndLevel.ALL)
        f.onPause(3_000, false)
        assertEquals(emptyList<String>(), words(f.onConnect(80_000, true)))
        assertEquals(listOf("request:priority"), words(f.onSnapshot(80_100, true)))
        assertTrue(f.weHold)
    }

    @Test
    fun aHoldKeptThroughTheGraceIsNotAffectedByTheConnectionGuard() {
        val f = holding()
        f.onClose(1_000)
        // Back inside the grace: nothing is requested again and nothing is released.
        assertEquals(emptyList<String>(), words(f.onConnect(20_000, true)))
        assertEquals(emptyList<String>(), words(f.onSnapshot(20_100, true)))
        assertTrue(f.weHold)
        assertNull(f.graceDeadline())
    }

    @Test
    fun aGraceThatEndsReleasesWithoutAConnection() {
        val f = holding()
        f.onClose(1_000)
        assertEquals(emptyList<String>(), words(f.onTick(60_999)))
        assertEquals(listOf("release"), words(f.onTick(61_000)))
        assertFalse(f.weHold)
        assertFalse(f.isConnected)
    }

    @Test
    fun neverARequestWithoutAConnection() {
        val f = DndFollower(switchOn = false, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        f.onSnapshot(10, true) // the switch is off
        f.onClose(20)
        // The switch is turned on after the connection went away: nothing remembered counts.
        assertEquals(emptyList<String>(), words(f.onSwitch(30, true)))
        assertFalse(f.weHold)
    }

    @Test
    fun aFilterReadBeforeTheResumeKeepsTheUsersOwnDoNotDisturb() {
        // Connected, the PC in Focus, this phone paused (so nothing held); the user turns their own Priority on; then the pause ends.
        val f = DndFollower(switchOn = true, access = true, paused = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        assertEquals(emptyList<String>(), words(f.onSnapshot(10, true)))
        f.setFilter(DndLevel.PRIORITY) // what the controller reads right before the resume is evaluated
        assertEquals(listOf("leave"), words(f.onPause(5_000, false)))
        assertFalse(f.weHold)
    }

    @Test
    fun withTheFilterStillAllTheResumeRequests() {
        val f = DndFollower(switchOn = true, access = true, paused = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        f.onSnapshot(10, true)
        f.setFilter(DndLevel.ALL)
        assertEquals(listOf("request:priority"), words(f.onPause(5_000, false)))
    }
}
