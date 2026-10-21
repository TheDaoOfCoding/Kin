package app.kinrelay.android.pause

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.presence.WirePause
import app.kinrelay.android.testing.FakeTime
import kotlinx.coroutines.launch
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import java.time.ZonedDateTime

/**
 * Pause (design 3.5): the end of each choice in real zones (08:00 local, 2 hours away at least, a daylight-saving change in between), the 8-day guard, the controller over a memory storage with a
 * virtual clock (persisted, one-shot expiry, the quiet counter, unlinking the last peer), and the words. (The shared tables are in `PresenceVectorsTest`.)
 */
class PauseTest {
    private fun at(zone: String, text: String): Long = ZonedDateTime.parse(text).withZoneSameInstant(ZoneId.of(zone)).toInstant().toEpochMilli()

    private fun local(zone: String, millis: Long): String = Instant.ofEpochMilli(millis).atZone(ZoneId.of(zone)).toLocalDateTime().toString()

    @Test
    fun untilTomorrowIsTheNextEightOClockAtLeastTwoHoursAway() {
        val zone = ZoneId.of("Europe/London")
        fun end(now: String) = local("Europe/London", PauseGate.tomorrowMillis(at("Europe/London", now), zone))
        assertEquals("2026-06-02T08:00", end("2026-06-01T07:30:00+01:00[Europe/London]"))
        assertEquals("2026-06-01T08:00", end("2026-06-01T05:00:00+01:00[Europe/London]"))
        assertEquals("2026-06-01T08:00", end("2026-06-01T06:00:00+01:00[Europe/London]"))
        assertEquals("2026-06-02T08:00", end("2026-06-01T06:00:01+01:00[Europe/London]"))
        assertEquals("2026-06-02T08:00", end("2026-06-01T08:00:00+01:00[Europe/London]"))
        assertEquals("2026-06-02T08:00", end("2026-06-01T23:59:00+01:00[Europe/London]"))
    }

    @Test
    fun theWallClockEightSurvivesADaylightSavingChange() {
        // Berlin springs forward on 2026-03-29 (02:00 -> 03:00): the night is 7 hours of elapsed time, and 08:00 is still 08:00 on the wall.
        val berlin = ZoneId.of("Europe/Berlin")
        val evening = at("Europe/Berlin", "2026-03-28T23:59:00+01:00[Europe/Berlin]")
        val end = PauseGate.tomorrowMillis(evening, berlin)
        assertEquals("2026-03-29T08:00", local("Europe/Berlin", end))
        assertEquals("7 hours and a minute of elapsed time, not 8", 7 * 60 + 1L, (end - evening) / 60_000)
        // Falls back on 2026-10-25: a 9-hour night.
        val fall = at("Europe/Berlin", "2026-10-24T23:59:00+02:00[Europe/Berlin]")
        assertEquals(9 * 60 + 1L, (PauseGate.tomorrowMillis(fall, berlin) - fall) / 60_000)
        assertEquals("2026-10-25T08:00", local("Europe/Berlin", PauseGate.tomorrowMillis(fall, berlin)))
    }

    @Test
    fun oneHourIsOneHourOfElapsedTimeAcrossAChange() {
        val berlin = ZoneId.of("Europe/Berlin")
        val now = at("Europe/Berlin", "2026-03-29T01:30:00+01:00[Europe/Berlin]")
        assertEquals("2026-03-29T03:30", local("Europe/Berlin", PauseGate.stateFor(PauseChoice.HOUR, now, berlin).untilMillis))
        assertTrue(PauseGate.stateFor(PauseChoice.UNTIL_RESUMED, now, berlin).untilResumed)
    }

    @Test
    fun theGateIsAWallClockComparisonWithAnEightDayGuard() {
        val now = 1_760_000_000_000L
        assertTrue(PauseGate.paused(PauseState(untilMillis = now + 1), now))
        assertFalse("at the moment it ends it is over", PauseGate.paused(PauseState(untilMillis = now), now))
        assertFalse(PauseGate.paused(PauseState(untilMillis = now - 1), now))
        assertTrue("exactly 8 days ahead still counts", PauseGate.paused(PauseState(untilMillis = now + 691_200_000L), now))
        assertFalse("8 days and a millisecond is a clock set backwards", PauseGate.paused(PauseState(untilMillis = now + 691_200_001L), now))
        assertTrue(PauseGate.paused(PauseState(untilResumed = true), now))
        assertFalse(PauseGate.paused(PauseState.NONE, now))
        assertEquals(WirePause.Remaining(60_000), PauseGate.wire(PauseState(untilMillis = now + 60_000), now))
        assertEquals(WirePause.UntilResumed, PauseGate.wire(PauseState(untilResumed = true), now))
        assertNull(PauseGate.wire(PauseState(untilMillis = now - 5), now))
    }

    @Test
    fun thePeersPauseResumesAtItsEndOrOnASnapshotWithoutPauseAndIsForgottenOnClose() {
        val time = FakeTime()
        val view = PeerPauseView(time)
        view.onPresence(WirePause.Remaining(10_000))
        assertTrue(view.isPaused())
        assertEquals(10_000L, view.remainingMillis())
        time.advanceBy(9_999)
        assertTrue(view.isPaused())
        time.advanceBy(1)
        assertFalse(view.isPaused())
        view.onPresence(WirePause.UntilResumed)
        assertTrue(view.isPaused() && view.hasNoEnd())
        view.onPresence(null)
        assertFalse(view.isPaused())
        view.onPresence(WirePause.UntilResumed)
        view.onClosed()
        assertFalse("nothing is paused as far as this side knows", view.isPaused())
    }

    // ------------------------------------------------------------------ the controller

    private class Memory(var state: PauseState = PauseState.NONE) : PauseStorage {
        override suspend fun current() = state
        override suspend fun set(state: PauseState) { this.state = state }
    }

    private val berlin = ZoneId.of("Europe/Berlin")

    @Test
    fun aPauseIsPersistedAndSurvivesARestart() = runBlocking {
        val time = FakeTime()
        var wall = at("Europe/Berlin", "2026-06-01T10:00:00+02:00[Europe/Berlin]")
        val memory = Memory()
        val first = PauseController(memory, time, KinLog.None, now = { wall }, zone = { berlin })
        first.load()
        assertFalse(first.paused())
        first.set(PauseChoice.HOUR)
        assertTrue(first.paused())
        assertEquals(wall + 3_600_000, memory.state.untilMillis)
        // A new process reads the same storage.
        val second = PauseController(memory, FakeTime(), KinLog.None, now = { wall }, zone = { berlin })
        second.load()
        assertTrue("still paused after a restart", second.paused())
        wall += 3_600_001
        assertFalse("and over by the clock, with no timer needed", second.paused())
        first.resume()
        assertEquals(PauseState.NONE, memory.state)
        assertFalse(first.paused())
    }

    @Test
    fun theOnlyTimerIsTheOneShotAtTheEndAndItTellsThePeer() = runBlocking {
        val time = FakeTime()
        var wall = 1_000_000L
        val c = PauseController(Memory(), time, KinLog.None, now = { wall }, zone = { berlin })
        c.load()
        assertEquals("nothing runs while idle", 0, time.pending)
        val seen = ArrayList<PauseChange>()
        val job = launchCollector(c, seen)
        c.set(PauseChoice.HOUR)
        assertEquals(1, time.pending)
        assertEquals(listOf(PauseChange.CHANGED), seen)
        wall += 3_600_000
        time.advanceBy(3_600_000)
        assertEquals(listOf(PauseChange.CHANGED, PauseChange.EXPIRED), seen)
        assertEquals(0, time.pending)
        assertFalse(c.paused())
        job.cancel()
    }

    private fun launchCollector(c: PauseController, into: MutableList<PauseChange>): kotlinx.coroutines.Job =
        kotlinx.coroutines.CoroutineScope(@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class) UnconfinedTestDispatcher()).launch { c.changes.collect { into += it } }

    @Test
    fun aSecondPauseReplacesTheFirstAndUnlinkingTheLastPeerClearsIt() = runBlocking {
        val time = FakeTime()
        val wall = 5_000_000L
        val memory = Memory()
        val c = PauseController(memory, time, KinLog.None, now = { wall }, zone = { berlin })
        c.load()
        c.set(PauseChoice.HOUR)
        c.set(PauseChoice.UNTIL_RESUMED)
        assertTrue(memory.state.untilResumed)
        assertEquals("the first one's end is gone and an open-ended pause has none", 0, time.pending)
        c.onLastPeerUnlinked()
        assertEquals(PauseState.NONE, memory.state)
        assertFalse(c.paused())
        c.onLastPeerUnlinked()
        assertEquals(PauseState.NONE, memory.state)
    }

    @Test
    fun theQuietCounterCountsWhilePausedAndResetsOnResume() = runBlocking {
        val c = PauseController(Memory(), FakeTime(), KinLog.None, now = { 1_000L }, zone = { berlin })
        c.load()
        c.set(PauseChoice.UNTIL_RESUMED)
        c.noteQuietArrival()
        c.noteQuietArrival()
        assertEquals(2, c.view.value.receivedWhilePaused)
        assertEquals("2 received while paused", PauseWords.receivedWhilePaused(c.view.value.receivedWhilePaused))
        c.resume()
        assertEquals(0, c.view.value.receivedWhilePaused)
        assertNull(PauseWords.receivedWhilePaused(0))
    }

    @Test
    fun aStateInThePastLoadedFromStorageIsNotPausedAndStartsNoTimer() = runBlocking {
        val time = FakeTime()
        val c = PauseController(Memory(PauseState(untilMillis = 500)), time, KinLog.None, now = { 10_000L }, zone = { berlin })
        c.load()
        assertFalse(c.paused())
        assertEquals(0, time.pending)
        assertNull(c.wire())
    }

    @Test
    fun aStoreThatCannotBeReadDoesNotHangTheGatesAndKinStartsNotPaused() = runBlocking {
        val failing = object : PauseStorage {
            override suspend fun current(): PauseState = throw java.io.IOException("datastore is broken")
            override suspend fun set(state: PauseState) = Unit
        }
        val c = PauseController(failing, FakeTime(), KinLog.None, now = { 10_000L }, zone = { berlin })
        // A gate asked before the load waits for it (`runBlocking` inside `stateNow`)...
        val early = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Default).async { c.paused() }
        c.load()
        // ...and must be answered even though the load failed.
        assertFalse(kotlinx.coroutines.withTimeout(5_000) { early.await() })
        assertFalse(c.paused())
        assertFalse(c.pausedIfLoaded())
        assertNull(c.wire())
    }

    @Test
    fun theViewReadsTheWallClockAgainSoAPauseThatEndedInDeepSleepShowsAsEnded() = runBlocking {
        val time = FakeTime()
        var wall = 1_000_000L
        val c = PauseController(Memory(), time, KinLog.None, now = { wall }, zone = { berlin })
        c.load()
        c.set(PauseChoice.HOUR)
        assertTrue(c.viewNow().paused)
        // The phone slept through the end: the one-shot timer has not run yet, so the published view still says paused.
        wall += 3_600_001
        assertTrue("the last published view is stale", c.view.value.paused)
        assertFalse("what the tile, Home and the notification read is not", c.viewNow().paused)
        c.refresh()
        assertFalse(c.view.value.paused)
    }

    @Test
    fun theWordsForAnEndAreTheWallClockOnes() {
        val zone = ZoneId.of("UTC")
        val now = at("UTC", "2026-06-01T10:00:00Z[UTC]")
        val clock = { m: Long -> Instant.ofEpochMilli(m).atZone(zone).toLocalTime().toString() }
        val day = { m: Long -> Instant.ofEpochMilli(m).atZone(zone).dayOfWeek.name.take(3) }
        assertEquals("Until 11:00", PauseWords.untilSentence(PauseState(untilMillis = now + 3_600_000), now, zone, clock, day))
        assertEquals("Until tomorrow 08:00", PauseWords.untilSentence(PauseState(untilMillis = at("UTC", "2026-06-02T08:00:00Z[UTC]")), now, zone, clock, day))
        assertEquals("Until resumed", PauseWords.untilSentence(PauseState(untilResumed = true), now, zone, clock, day))
        assertEquals("until resumed", PauseWords.until(PauseState(untilResumed = true), now, zone, clock, day))
    }
}
