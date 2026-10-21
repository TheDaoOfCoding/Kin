package app.kinrelay.android.dnd

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 2B.5 review round 2, risk R44: a release that failed for lack of access is owed until it works, and is paid when the access comes back, at the next alarm and at the start of the service. The
 * follower's side of the same hole (an access memory gone stale) is in the last tests.
 */
class DndReleaseGuardTest {
    private class Rig {
        var policy = true

        /** Access is there but the system refuses the call (the listener route that keeps saying no). */
        var refuse = false
        var filter = DndFilter.ALL
        var rule = false
        var stored = DndHold.NONE
        var setCalls = 0
        val changes = ArrayList<Boolean>()

        val store = object : DndHoldStore {
            override suspend fun load() = stored
            override suspend fun save(hold: DndHold) {
                stored = hold
            }
        }
        val ports = object : DndPorts {
            override fun policyAccess() = policy
            override fun listenerConnected() = false
            override fun currentFilter() = filter
            override fun setFilter(filter: Int) {
                if (!policy || refuse) throw SecurityException("Notification policy access denied")
                setCalls++
                this@Rig.filter = filter
            }
            override fun requestViaListener(filter: Int) = false
            override val ruleModel get() = rule
        }
        val applier = DndApplier(ports, store)
        lateinit var guard: DndReleaseGuard

        init {
            guard = DndReleaseGuard(release = { applier.release() }, onChange = { changes += guard.pending }, userChanged = { applier.userChangedWhileOwed(it) }, persist = { applier.setOwed(it) })
        }

        /** A new process around the same record: a fresh guard that has only what was persisted. */
        fun restarted(): DndReleaseGuard {
            val g = DndReleaseGuard(release = { applier.release() }, userChanged = { applier.userChangedWhileOwed(it) }, persist = { applier.setOwed(it) })
            g.restore(stored.weHold && stored.releaseOwed)
            return g
        }
    }

    private fun held(r: Rig) = runBlocking {
        assertTrue(r.applier.request(DndLevel.PRIORITY))
        assertEquals(DndFilter.PRIORITY, r.filter)
    }

    @Test
    fun aReleaseThatFailedForLackOfAccessIsOwedAndPaidWhenTheAccessReturns() = runBlocking {
        val r = Rig()
        held(r)
        r.policy = false // the user revoked Do Not Disturb access while Kin held
        assertFalse(r.guard.attempt())
        assertTrue("the record stays", r.stored.weHold)
        assertTrue(r.guard.pending)
        assertEquals("the phone is still silent", DndFilter.PRIORITY, r.filter)

        r.policy = true // the access-changed broadcast: refreshAccess retries first
        assertTrue(r.guard.retry())
        assertFalse(r.stored.weHold)
        assertEquals("the silence is given back", DndFilter.ALL, r.filter)
        assertFalse(r.guard.pending)
        assertEquals("the controller hears owed, then paid (receiver and alarm follow)", listOf(true, false), r.changes)
    }

    @Test
    fun everyRetryWithoutAccessKeepsTheDebtAndTheNextOneWithAccessPaysIt() = runBlocking {
        val r = Rig()
        held(r)
        r.policy = false
        r.guard.attempt()
        assertFalse(r.guard.retry()) // an alarm with no access yet
        assertFalse(r.guard.retry()) // the service starts, still no access
        assertTrue(r.guard.pending)
        assertEquals("owed once, not announced again at each failure", listOf(true), r.changes)
        r.policy = true
        assertTrue(r.guard.retry()) // the next alarm
        assertEquals(DndFilter.ALL, r.filter)
        assertFalse(r.stored.weHold)
    }

    @Test
    fun withNothingOwedARetryDoesNothing() = runBlocking {
        val r = Rig()
        assertTrue(r.guard.retry())
        assertEquals(0, r.setCalls)
        assertTrue(r.changes.isEmpty())
        // Even with a record in the store: a retry pays only a debt that a failed release created (a live hold is the follower's to release).
        held(r)
        val calls = r.setCalls
        assertTrue(r.guard.retry())
        assertEquals(calls, r.setCalls)
        assertTrue(r.stored.weHold)
    }

    @Test
    fun aNewHoldOrTheUsersChangeSettlesTheDebt() = runBlocking {
        val r = Rig()
        held(r)
        r.policy = false
        r.guard.attempt()
        assertTrue(r.guard.pending)
        r.guard.settled() // a new request replaced the record, or the user's change cleared it
        assertFalse(r.guard.pending)
        assertEquals(listOf(true, false), r.changes)
    }

    @Test
    fun theOwedReleaseIsRetriedOnTheRuleModelToo() = runBlocking {
        val r = Rig().also { it.rule = true }
        held(r)
        r.policy = false
        assertFalse(r.guard.attempt())
        r.policy = true
        assertTrue(r.guard.retry())
        assertEquals("the rule model asks for ALL: it only deactivates Kin's own rule", DndFilter.ALL, r.filter)
        assertFalse(r.stored.weHold)
    }

    @Test
    fun theWholeWayFromTheFollowerToTheRetry() = runBlocking {
        val r = Rig()
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        // The PC enters Focus: the follower asks and the controller does it (the request settles any old debt).
        assertEquals(listOf("request:priority"), f.onSnapshot(10, true).map { it.word })
        assertTrue(r.applier.request(DndLevel.PRIORITY))
        r.guard.settled()
        f.setFilter(DndLevel.PRIORITY)
        // Access is revoked while holding; the PC leaves Focus: the follower says release, and it fails.
        r.policy = false
        f.setAccess(r.applier.hasAccess)
        assertEquals(listOf("release"), f.onSnapshot(20, false).map { it.word })
        assertFalse(r.guard.attempt())
        assertFalse("the follower believes it is done", f.weHold)
        assertTrue("but the record and the debt remain", r.stored.weHold && r.guard.pending)
        // The access comes back: refreshAccess retries before anything else.
        r.policy = true
        assertTrue(r.guard.retry())
        assertEquals(DndFilter.ALL, r.filter)
        assertFalse(r.stored.weHold)
    }

    // ---------------------------------------------------------------- round 3: the user's own Do Not Disturb while a release is owed (R44)

    @Test
    fun aFailedReleaseNeverSwitchesOffTheUsersOwnDoNotDisturbLater() = runBlocking {
        val r = Rig()
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        assertEquals(listOf("request:priority"), f.onSnapshot(10, true).map { it.word })
        assertTrue(r.applier.request(DndLevel.PRIORITY))
        r.guard.settled()
        f.setFilter(DndLevel.PRIORITY)
        // The user revokes access; the PC leaves Focus; the release fails. The follower has let go, the record and the debt stay.
        r.policy = false
        f.setAccess(false)
        assertEquals(listOf("release"), f.onSnapshot(20, false).map { it.word })
        assertFalse(r.guard.attempt())
        assertTrue(r.stored.weHold && r.guard.pending)
        // The user turns Do Not Disturb off: the follower (weHold = false) does nothing, the guard sees it and cancels the debt.
        r.filter = DndFilter.ALL
        assertEquals(emptyList<String>(), f.onFilter(30, DndLevel.ALL).map { it.word })
        r.guard.onFilter(DndFilter.ALL)
        assertFalse("the record is cleared", r.stored.weHold)
        assertFalse("the debt is cancelled (the alarm and the receiver go)", r.guard.pending)
        // Later the user turns Priority on themselves, and the access comes back.
        val callsBefore = r.setCalls
        r.filter = DndFilter.PRIORITY
        f.onFilter(40, DndLevel.PRIORITY)
        r.guard.onFilter(DndFilter.PRIORITY)
        r.policy = true
        assertTrue(r.guard.retry())
        assertEquals("nothing was asked of the system", callsBefore, r.setCalls)
        assertEquals("the user's own Priority is untouched", DndFilter.PRIORITY, r.filter)
        assertEquals(listOf(true, false), r.changes)
    }

    @Test
    fun theSameFilterAsKinsKeepsTheDebt() = runBlocking {
        val r = Rig()
        held(r)
        r.policy = false
        r.guard.attempt()
        r.guard.onFilter(DndFilter.PRIORITY) // a report of what Kin set: Kin's silence is still on and still owed
        assertTrue(r.guard.pending)
        assertTrue(r.stored.weHold)
        r.policy = true
        assertTrue(r.guard.retry())
        assertEquals(DndFilter.ALL, r.filter)
    }

    @Test
    fun aFilterReportWithNothingOwedDoesNothing() = runBlocking {
        val r = Rig()
        held(r) // a live hold is the follower's, not the guard's
        r.guard.onFilter(DndFilter.ALL)
        assertTrue(r.stored.weHold)
        assertTrue(r.changes.isEmpty())
    }

    @Test
    fun onTheRuleModelAChangeWithNoAccessKeepsKinsOwnRuleOwedUntilItCanBeSwitchedOff() = runBlocking {
        val r = Rig().also { it.rule = true }
        held(r)
        r.policy = false
        r.guard.attempt()
        r.guard.onFilter(DndFilter.NONE) // the user's own, stronger rule; Kin's rule is still on and cannot be reached
        assertTrue("the record stays: Kin's own rule is still Kin's to take off", r.stored.weHold)
        assertTrue(r.guard.pending)
        r.policy = true
        assertTrue(r.guard.retry())
        assertFalse(r.stored.weHold)
        assertEquals(DndFilter.ALL, r.filter)
    }

    // ---------------------------------------------------------------- the alarm

    @Test
    fun anOwedReleaseSetsARetryAlarmEvenWithNoHold() {
        assertEquals(1_000L + DndWatchdog.RETRY_RELEASE_MILLIS, DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = true, now = 1_000))
        assertEquals(null, DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = false, now = 1_000))
        // A live hold keeps its own wake (the grace or the probe).
        assertEquals(61_000L, DndWatchdog.nextWake(holding = true, graceEnd = 61_000, lastAliveAt = 0, releaseOwed = true, now = 1_000))
    }

    @Test
    fun theRetryAlarmIsNotRearmedWhileTheAccessIsMissing() = runBlocking {
        val r = Rig()
        held(r)
        r.policy = false
        r.guard.attempt()
        // Alarm after alarm with no access: each retry fails and no further alarm is set (the access-changed broadcast and the service start pay the debt instead).
        repeat(5) {
            assertFalse(r.guard.retry())
            assertNull(DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = r.guard.pending, now = it * 300_000L, accessAvailable = r.applier.hasAccess))
        }
        assertTrue("still owed", r.guard.pending)
        // With access but a refused call (not a missing access) a retry is still scheduled: a phone must not stay silent.
        r.policy = true
        assertEquals(300_000L, DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = true, now = 0, accessAvailable = r.applier.hasAccess))
    }

    // ---------------------------------------------------------------- 2B.6: the owed flag survives a process death

    @Test
    fun theOwedFlagIsPersistedWithTheRecordAndAFreshProcessStillOwesTheRelease() = runBlocking {
        val r = Rig()
        held(r)
        assertFalse("a fresh hold owes nothing", r.stored.releaseOwed)
        r.policy = false
        assertFalse(r.guard.attempt())
        assertTrue("the flag is written next to we_hold", r.stored.weHold && r.stored.releaseOwed)

        // The process dies; the next one reads the record and the flag.
        val g = r.restarted()
        assertTrue("still owed after the restart", g.pending)
        r.policy = true
        assertTrue(g.retry())
        assertFalse(g.pending)
        assertEquals("the owed release was paid", DndFilter.ALL, r.filter)
        assertEquals(DndHold.NONE, r.stored)
    }

    @Test
    fun anOwedFlagWithNoRecordIsNeverRestoredAndASettledDebtClearsTheStoredFlag() = runBlocking {
        val r = Rig()
        r.stored = DndHold(weHold = false, releaseOwed = true) // can't happen through the store, but a stale flag must not matter
        assertFalse(r.restarted().pending)
        held(r)
        r.policy = false
        r.guard.attempt()
        assertTrue(r.stored.releaseOwed)
        r.guard.settled()
        assertTrue("the record stays (the new owner clears it), the flag does not", r.stored.weHold && !r.stored.releaseOwed)
        assertFalse(r.restarted().pending)
    }

    @Test
    fun aRecordWrittenBeforeTheFlagExistedReadsAsNothingOwed() {
        // The 0.2.0-pre stores had no release_owed key: the data class default is the migration.
        assertFalse(DndHold(weHold = true, heldFilter = DndFilter.PRIORITY, route = DndApplier.ROUTE_GLOBAL).releaseOwed)
    }

    // ---------------------------------------------------------------- 2B.6: the retry alarm backs off

    @Test
    fun theRetryAlarmBacksOffFiveTenTwentyFortyMinutesThenStops() = runBlocking {
        val r = Rig()
        held(r)
        r.refuse = true // access is there, the call keeps being refused
        assertFalse(r.guard.attempt()) // the release that created the debt (not an alarm)
        val minute = 60_000L
        val delays = ArrayList<Long?>()
        repeat(6) {
            delays += DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = r.guard.pending, now = 0, accessAvailable = r.applier.hasAccess, retryStep = r.guard.retryStep)
            r.guard.retry(fromAlarm = true) // the alarm that was set goes off and fails
        }
        assertEquals(listOf(5 * minute, 10 * minute, 20 * minute, 40 * minute, null, null), delays)
        assertTrue("still owed: the other chances remain", r.guard.pending && r.stored.releaseOwed)
    }

    @Test
    fun aChanceThatIsNotTheAlarmAndASuccessStartTheLadderAgain() = runBlocking {
        val r = Rig()
        held(r)
        r.refuse = true
        r.guard.attempt()
        repeat(DndWatchdog.RETRY_STEPS) { r.guard.retry(fromAlarm = true) }
        assertNull(DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = true, now = 0, retryStep = r.guard.retryStep))
        // The access-changed broadcast, a service start, an app start or "Release now": it fails again, but the alarm is back at 5 minutes.
        assertFalse(r.guard.retry())
        assertEquals(DndWatchdog.RETRY_RELEASE_MILLIS, DndWatchdog.nextWake(holding = false, graceEnd = null, lastAliveAt = 0, releaseOwed = true, now = 0, retryStep = r.guard.retryStep))
        // Then the call works.
        r.refuse = false
        assertTrue(r.guard.retry(fromAlarm = true))
        assertEquals(0, r.guard.retryStep)
        assertFalse(r.guard.pending)
        assertEquals(DndFilter.ALL, r.filter)
    }

    @Test
    fun theRetryDelaysAreTheDocumentedLadder() {
        assertEquals(listOf<Long?>(300_000, 600_000, 1_200_000, 2_400_000, null), (0..4).map { DndWatchdog.retryDelay(it) })
        assertNull(DndWatchdog.retryDelay(-1))
    }

    // ---------------------------------------------------------------- 2B.6: a rebuilt follower expects the filter Kin saved

    @Test
    fun aFollowerRebuiltAroundAHoldExpectsTheSavedFilterNotTheCurrentHowMuchSetting() {
        // Kin held Alarms only; the user changed "How much" to Priority since. The phone still reports Alarms: that is Kin's own silence, not an override.
        val f = DndFollower(switchOn = true, access = true, follow = DndLevel.PRIORITY, filter = DndLevel.ALARMS, weHold = true, heldLevel = DndLevel.ALARMS)
        assertEquals(emptyList<String>(), f.onFilter(10, DndLevel.ALARMS).map { it.word })
        assertTrue(f.weHold)
        // Priority is now what differs from what Kin saved: the user's.
        assertEquals(listOf("override"), f.onFilter(20, DndLevel.PRIORITY).map { it.word })
        assertFalse(f.weHold)
    }

    @Test
    fun aFollowerRebuiltWithNoSavedLevelStillFallsBackToTheSetting() {
        val f = DndFollower(switchOn = true, access = true, follow = DndLevel.ALARMS, filter = DndLevel.ALARMS, weHold = true)
        assertEquals(emptyList<String>(), f.onFilter(10, DndLevel.ALARMS).map { it.word })
    }

    // ---------------------------------------------------------------- the follower's access memory

    @Test
    fun anAccessRevokedWhileNoReceiverListenedDoesNotSpendTheRisingEdge() {
        // Both switches were off, the follower remembered "access", and the user revoked it with nobody listening. The controller reads the access again before every event.
        val f = DndFollower(switchOn = false, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        f.onSnapshot(10, true) // the PC is in Focus; the switch is off
        f.setAccess(false) // the real access, read at evaluation time
        assertEquals("no request that can't be made, and the edge is not used up", emptyList<String>(), f.onSwitch(20, true).map { it.word })
        assertFalse(f.weHold)
        // Granting later starts the hold (the access-changed broadcast, or any next event).
        f.setAccess(true)
        assertEquals(listOf("request:priority"), f.onAccess(30, true).map { it.word })
        assertTrue(f.weHold)
    }

    @Test
    fun aFreshReadOfAnAccessThatWasMissingStartsTheHoldAtTheNextEvent() {
        val f = DndFollower(switchOn = true, access = false, filter = DndLevel.ALL)
        f.onConnect(0, true)
        assertEquals(emptyList<String>(), f.onSnapshot(10, true).map { it.word })
        f.setAccess(true) // granted while the receiver was gone: the next event sees it
        assertEquals(listOf("request:priority"), f.onSnapshot(20, true).map { it.word })
    }
}
