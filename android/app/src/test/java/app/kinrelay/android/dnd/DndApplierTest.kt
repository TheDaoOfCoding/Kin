package app.kinrelay.android.dnd

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [DndApplier] against a fake `NotificationManager`, a fake listener and an in-memory record, with **one ordered log** (design 4.1, 4.2.1, 10.1): the route is the narrowest access that works, `we_hold`
 * is persisted **before** the request, a release asks for `ALL` only when Kin holds (and, on a global filter, only if it is still what Kin set), and a failed call keeps the record.
 */
class DndApplierTest {
    private class Rig {
        val log = ArrayList<String>()
        var policy = true
        var listener = false
        var filter = DndFilter.ALL
        var rule = false
        var throwOnSet = false
        var stored = DndHold.NONE

        val store = object : DndHoldStore {
            override suspend fun load() = stored
            override suspend fun save(hold: DndHold) {
                stored = hold
                log += "store weHold=${hold.weHold}"
            }
        }
        val ports = object : DndPorts {
            override fun policyAccess() = policy
            override fun listenerConnected() = listener
            override fun currentFilter() = filter
            override fun setFilter(filter: Int) {
                if (throwOnSet) throw SecurityException("Notification policy access denied")
                this@Rig.filter = filter
                log += "setFilter $filter"
            }
            override fun requestViaListener(filter: Int): Boolean {
                if (!listener) return false
                this@Rig.filter = filter
                log += "listener $filter"
                return true
            }
            override val ruleModel get() = rule
        }
        val applier = DndApplier(ports, store)
    }

    @Test
    fun theRouteIsTheNarrowestAccessThatWorks() {
        val r = Rig()
        assertEquals(DndApplier.Path.POLICY, r.applier.path())
        r.policy = false
        r.listener = true
        assertEquals("notification access alone: the listener", DndApplier.Path.LISTENER, r.applier.path())
        r.listener = false
        assertEquals(DndApplier.Path.NONE, r.applier.path())
        assertFalse(r.applier.hasAccess)
    }

    @Test
    fun theHoldIsPersistedBeforeTheRequestIsMade() = runBlocking {
        val r = Rig()
        assertTrue(r.applier.request(DndLevel.PRIORITY))
        assertEquals(listOf("store weHold=true", "setFilter 2"), r.log)
        assertEquals(DndHold(weHold = true, heldFilter = DndFilter.PRIORITY, route = DndApplier.ROUTE_GLOBAL), r.stored)
        val ruled = Rig().also { it.rule = true }
        ruled.applier.request(DndLevel.ALARMS)
        assertEquals(DndApplier.ROUTE_RULE, ruled.stored.route)
        assertEquals(DndFilter.ALARMS, ruled.stored.heldFilter)
    }

    @Test
    fun theListenerRouteRequestsThroughTheListenerAndTheSameRecordIsKept() = runBlocking {
        val r = Rig().also { it.policy = false; it.listener = true }
        assertTrue(r.applier.request(DndLevel.PRIORITY))
        assertEquals(listOf("store weHold=true", "listener 2"), r.log)
    }

    @Test
    fun withNoAccessNothingIsRequestedAndNothingIsHeld() = runBlocking {
        val r = Rig().also { it.policy = false }
        assertFalse(r.applier.request(DndLevel.PRIORITY))
        assertTrue("nothing written, nothing asked: ${r.log}", r.log.isEmpty())
        assertFalse(r.stored.weHold)
    }

    @Test
    fun aRefusedRequestClearsTheRecordAgain() = runBlocking {
        val r = Rig().also { it.throwOnSet = true }
        assertFalse(r.applier.request(DndLevel.PRIORITY))
        assertEquals("the record was written first, then cleared: ${r.log}", listOf("store weHold=true", "store weHold=false"), r.log)
        assertFalse(r.stored.weHold)
    }

    @Test
    fun noRequestIsEverMadeForALevelKinCannotAskFor() = runBlocking {
        val r = Rig()
        for (level in listOf(DndLevel.ALL, DndLevel.TOTAL, DndLevel.OTHER)) assertFalse("$level", r.applier.request(level))
        assertTrue(r.log.isEmpty())
    }

    @Test
    fun releaseOnTheRuleModelAsksForAllOnlyToDeactivateKinsOwnRule() = runBlocking {
        val r = Rig().also { it.rule = true }
        r.applier.request(DndLevel.PRIORITY)
        r.log.clear()
        // The user's own alarms-only is on top of Kin's rule: the effective filter is theirs, and Kin's release must still run.
        r.filter = DndFilter.ALARMS
        assertTrue(r.applier.release())
        assertEquals(listOf("setFilter 1", "store weHold=false"), r.log)
    }

    @Test
    fun releaseOnAGlobalFilterRestoresAllOnlyIfItIsStillWhatKinSet() = runBlocking {
        val r = Rig()
        r.applier.request(DndLevel.PRIORITY)
        r.log.clear()
        assertTrue(r.applier.release())
        assertEquals(listOf("setFilter 1", "store weHold=false"), r.log)
        // The user changed it to alarms while Kin held: that is theirs now, Kin asks for nothing and only forgets.
        val changed = Rig()
        changed.applier.request(DndLevel.PRIORITY)
        changed.filter = DndFilter.ALARMS
        changed.log.clear()
        assertTrue(changed.applier.release())
        assertEquals("no call to the system: ${changed.log}", listOf("store weHold=false"), changed.log)
        assertEquals(DndFilter.ALARMS, changed.filter)
    }

    @Test
    fun aReleaseThatCannotBeMadeKeepsTheRecordForTheNextStart() = runBlocking {
        val r = Rig()
        r.applier.request(DndLevel.PRIORITY)
        r.policy = false
        assertFalse("the access is gone", r.applier.release())
        assertTrue("the record stays", r.stored.weHold)
        r.policy = true
        r.throwOnSet = true
        assertFalse("the call is refused", r.applier.release())
        assertTrue(r.stored.weHold)
        r.throwOnSet = false
        assertTrue("the next start works", r.applier.release())
        assertFalse(r.stored.weHold)
    }

    @Test
    fun neverAsksForAllWhileItDoesNotHold() = runBlocking {
        val r = Rig().also { it.filter = DndFilter.PRIORITY }
        assertTrue(r.applier.release())
        r.applier.overridden()
        assertTrue("nothing at all was asked: ${r.log}", r.log.none { it.startsWith("setFilter") || it.startsWith("listener") })
        assertEquals("the user's own priority is untouched", DndFilter.PRIORITY, r.filter)
    }

    @Test
    fun anOverrideOnTheRuleModelDeactivatesKinsRuleAndOnAGlobalFilterTouchesNothing() = runBlocking {
        val rule = Rig().also { it.rule = true }
        rule.applier.request(DndLevel.PRIORITY)
        rule.log.clear()
        rule.applier.overridden()
        assertEquals(listOf("setFilter 1", "store weHold=false"), rule.log)
        val global = Rig()
        global.applier.request(DndLevel.PRIORITY)
        global.filter = DndFilter.NONE
        global.log.clear()
        global.applier.overridden()
        assertEquals(listOf("store weHold=false"), global.log)
        assertEquals(DndFilter.NONE, global.filter)
    }

    @Test
    fun anOverrideOnTheRuleModelThatCannotDeactivateKinsRuleKeepsTheRecord() = runBlocking {
        // Android 15+: Kin's own rule is still on; with the access gone it can't be switched off, so the record stays for the release guard.
        val noAccess = Rig().also { it.rule = true }
        noAccess.applier.request(DndLevel.PRIORITY)
        noAccess.policy = false
        assertFalse(noAccess.applier.overridden())
        assertTrue(noAccess.stored.weHold)
        // A refused call is the same.
        val refused = Rig().also { it.rule = true }
        refused.applier.request(DndLevel.PRIORITY)
        refused.throwOnSet = true
        assertFalse(refused.applier.overridden())
        assertTrue(refused.stored.weHold)
        // With the access back, the same call deactivates the rule and clears the record.
        noAccess.policy = true
        assertTrue(noAccess.applier.overridden())
        assertFalse(noAccess.stored.weHold)
        assertEquals(DndFilter.ALL, noAccess.filter)
    }

    @Test
    fun aGlobalFilterOverrideClearsTheRecordEvenWithNoAccess() = runBlocking {
        val r = Rig()
        r.applier.request(DndLevel.PRIORITY)
        r.policy = false
        assertTrue("the user's change is the filter now; there is nothing of Kin's to switch off", r.applier.overridden())
        assertFalse(r.stored.weHold)
    }

    @Test
    fun theAlarmIsSilencedByTotalSilenceAndByPriorityThatDoesNotAllowAlarms() {
        assertTrue(AlarmAudibility.silenced(DndFilter.NONE, 0))
        assertTrue(AlarmAudibility.silenced(DndFilter.NONE, AlarmAudibility.PRIORITY_CATEGORY_ALARMS))
        assertTrue("priority without alarms", AlarmAudibility.silenced(DndFilter.PRIORITY, 0))
        assertTrue(AlarmAudibility.silenced(DndFilter.PRIORITY, 4 or 8))
        assertFalse("priority with alarms (the default)", AlarmAudibility.silenced(DndFilter.PRIORITY, AlarmAudibility.PRIORITY_CATEGORY_ALARMS or 8))
        assertFalse("alarms only plays alarms", AlarmAudibility.silenced(DndFilter.ALARMS, 0))
        assertFalse(AlarmAudibility.silenced(DndFilter.ALL, 0))
        assertEquals(android.app.NotificationManager.Policy.PRIORITY_CATEGORY_ALARMS, AlarmAudibility.PRIORITY_CATEGORY_ALARMS)
    }
}
