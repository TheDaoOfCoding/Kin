package app.kinrelay.android.dnd

import app.kinrelay.android.dnd.DndWatchdog.Due
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The decision logic of the watchdog alarm over a fake clock (the times are `elapsedRealtime`, which keeps counting while the phone sleeps). */
class DndWatchdogTest {
    private val probe = DndWatchdog.PROBE_AFTER_MILLIS

    @Test
    fun noHoldNoAlarm() {
        assertNull(DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0))
        assertNull(DndWatchdog.nextWake(holding = false, graceEnd = 61_000, lastAliveAt = 0))
        assertEquals(Due.NOTHING, DndWatchdog.due(10_000_000, holding = false, graceEnd = null, connected = true, lastAliveAt = 0))
    }

    @Test
    fun aGraceSetsTheAlarmAtItsEnd() {
        assertEquals(61_000L, DndWatchdog.nextWake(holding = true, graceEnd = 61_000, lastAliveAt = 10))
    }

    @Test
    fun aGraceEndsWhenTheAlarmComesEvenIfTheCoroutineTimerSlept() {
        // Closed at 1_000, the grace ends at 61_000; the phone slept and the first look is at 20 minutes.
        assertEquals(Due.NOTHING, DndWatchdog.due(60_999, true, 61_000, connected = false, lastAliveAt = 10))
        assertEquals(Due.GRACE_END, DndWatchdog.due(61_000, true, 61_000, connected = false, lastAliveAt = 10))
        assertEquals(Due.GRACE_END, DndWatchdog.due(1_200_000, true, 61_000, connected = false, lastAliveAt = 10))
    }

    @Test
    fun aConnectedSilentPcIsProbedAfterTheDeadLinkTime() {
        assertEquals(1_000L + probe, DndWatchdog.nextWake(holding = true, graceEnd = null, lastAliveAt = 1_000))
        assertEquals(Due.NOTHING, DndWatchdog.due(1_000 + probe - 1, true, null, connected = true, lastAliveAt = 1_000))
        assertEquals(Due.PROBE, DndWatchdog.due(1_000 + probe, true, null, connected = true, lastAliveAt = 1_000))
        // The phone slept through the whole thing: the first look is much later and the answer is the same.
        assertEquals(Due.PROBE, DndWatchdog.due(3_600_000, true, null, connected = true, lastAliveAt = 1_000))
    }

    @Test
    fun aSignOfLifeMovesTheProbe() {
        assertEquals(Due.NOTHING, DndWatchdog.due(1_000 + probe, true, null, connected = true, lastAliveAt = 100_000))
        assertEquals(100_000L + probe, DndWatchdog.nextWake(true, null, 100_000))
    }

    @Test
    fun aHoldWithNoConnectionAndNoGraceIsReleased() {
        assertEquals(Due.NOTHING, DndWatchdog.due(1_000 + probe - 1, true, null, connected = false, lastAliveAt = 1_000))
        assertEquals(Due.RELEASE, DndWatchdog.due(1_000 + probe, true, null, connected = false, lastAliveAt = 1_000))
    }

    @Test
    fun theScheduledBoundIs310SecondsAndTheRealisticOneAboutNineMinutes() {
        // Silent from t0: the probe at t0 + 240 s, its answer (10 s at most) closes the connection, the grace runs 60 s more. That is the time the alarms are SET for.
        val t0 = 0L
        val probeAt = DndWatchdog.nextWake(true, null, t0)!!
        val graceStart = probeAt + 10_000
        val releaseAt = DndWatchdog.nextWake(true, graceStart + DndFollower.GRACE_MILLIS, graceStart)!!
        assertEquals(310_000L, releaseAt)
        // `setAndAllowWhileIdle` is inexact: Android may fire an alarm up to 0.75 of its delay late (round 3). Both alarms late: 240 s * 1.75 + 10 s + 60 s * 1.75 = 535 s, about 9 minutes.
        val lateProbe = probeAt + probeAt * 3 / 4
        val lateRelease = lateProbe + 10_000 + DndFollower.GRACE_MILLIS + DndFollower.GRACE_MILLIS * 3 / 4
        assertEquals(535_000L, lateRelease)
    }

    @Test
    fun theFollowerAndTheWatchdogAgreeOverAClose() {
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        f.onSnapshot(10, true)
        f.onClose(5_000)
        val wake = DndWatchdog.nextWake(f.weHold, f.graceDeadline(), lastAliveAt = 10)!!
        assertEquals(65_000L, wake)
        assertEquals(Due.GRACE_END, DndWatchdog.due(wake, f.weHold, f.graceDeadline(), f.isConnected, 10))
        assertEquals(listOf("release"), f.onTick(wake).map { it.word })
        assertNull(DndWatchdog.nextWake(f.weHold, f.graceDeadline(), 10))
    }
}
