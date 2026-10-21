package app.kinrelay.android.presence

import app.kinrelay.android.protocol.StrictJson
import app.kinrelay.android.testing.FakeTime
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Presence beyond the shared vectors (design 3.4, 5.2, 8): the capability per message, nothing when offline, no periodic timer, the words, the wire builders and the counts-only line. */
class PresenceBehaviourTest {
    private class Link(var ready: Boolean = true, var lists: Boolean = true, var accepts: Boolean = true) : PresenceLink {
        val frames = ArrayList<String>()
        override val isReady get() = ready
        override val peerListsPresence get() = lists
        override fun sendEphemeral(frame: String): Boolean {
            if (!accepts) return false
            frames += frame
            return true
        }
    }

    private class Rig(var link: Link? = Link()) {
        val time = FakeTime()
        var battery: Battery? = Battery(62, true)
        var pause: WirePause? = null
        var dnd: String? = null
        val stats = PresenceStats()
        val sender = PresenceSender(time, time, link = { link }, battery = { battery }, pause = { pause }, dnd = { dnd }, stats = stats)
    }

    @Test
    fun aChangeOfTheDndIsSentWithinOneSecondAndNeverHeldBackByTheBatteryRule() {
        val r = Rig()
        r.sender.onReady()
        assertEquals(1, r.link!!.frames.size)
        // A battery change was sent a moment ago: the 30 s rule would hold the next battery-only snapshot, but a dnd change goes at 1 s.
        r.time.advanceBy(2_000)
        r.dnd = "priority"
        r.sender.onDndChanged()
        r.time.advanceBy(999)
        assertEquals("not before the second is over", 1, r.link!!.frames.size)
        r.time.advanceBy(1)
        assertEquals(2, r.link!!.frames.size)
        assertEquals("priority", StrictJson.parse(r.link!!.frames.last()).jsonObject["dnd"]?.toString()?.trim('"'))
        // Off again: the field is left out, within the second.
        r.dnd = null
        r.sender.onDndChanged()
        r.time.advanceBy(1_000)
        assertEquals(3, r.link!!.frames.size)
        assertFalse(StrictJson.parse(r.link!!.frames.last()).jsonObject.containsKey("dnd"))
    }

    @Test
    fun aDndSetAndUndoneWithinTheSecondSendsNothingAndNoTimerIsLeft() {
        val r = Rig()
        r.sender.onReady()
        r.dnd = "alarms"
        r.sender.onDndChanged()
        r.time.advanceBy(300)
        r.dnd = null
        r.sender.onDndChanged()
        r.time.advanceBy(5_000)
        assertEquals("what the PC was told is still true", 1, r.link!!.frames.size)
        assertEquals(0, r.time.pending)
    }

    @Test
    fun aDndChangeBeforeTheFirstSnapshotOrWithNoConnectionIsNotSentOnItsOwn() {
        val none = Rig(null)
        none.dnd = "priority"
        none.sender.onDndChanged()
        assertEquals(0, none.time.pending)
        val fresh = Rig()
        fresh.dnd = "priority"
        fresh.sender.onDndChanged()
        assertEquals("the first snapshot carries it: nothing is scheduled", 0, fresh.time.pending)
        fresh.sender.onReady()
        assertEquals("priority", StrictJson.parse(fresh.link!!.frames.single()).jsonObject["dnd"]?.toString()?.trim('"'))
    }

    @Test
    fun aPeerThatDoesNotListPresenceGetsNothingAtAnyTime() {
        val r = Rig(Link(lists = false))
        r.sender.onReady()
        r.sender.onBatteryChanged()
        r.sender.onPauseChanged()
        r.time.advanceBy(120_000)
        assertTrue(r.link!!.frames.isEmpty())
        assertEquals(0, r.time.pending)
        // The peer's later hello lists it: the first snapshot goes out then.
        r.link!!.lists = true
        r.sender.onReady()
        assertEquals(1, r.link!!.frames.size)
    }

    @Test
    fun withNoConnectionNothingIsSentAndNothingIsStarted() {
        val r = Rig(null)
        r.sender.onReady()
        r.sender.onBatteryChanged()
        r.sender.onPauseChanged()
        assertEquals(0, r.time.pending)
        assertEquals(2, r.stats.droppedOffline.get())
        assertEquals(0, r.stats.sent.get())
    }

    @Test
    fun anIdleSenderHasNoTimerAndNeverSendsOnItsOwn() {
        val r = Rig()
        r.sender.onReady()
        assertEquals(1, r.link!!.frames.size)
        assertEquals("no periodic timer", 0, r.time.pending)
        r.time.advanceBy(10 * 60_000)
        assertEquals(1, r.link!!.frames.size)
    }

    @Test
    fun aNewConnectionStartsFromNothingAndGetsItsOwnFirstSnapshot() {
        val r = Rig()
        r.sender.onReady()
        val second = Link()
        r.link = second
        r.sender.onReady()
        assertEquals(1, second.frames.size)
        r.sender.onClosed()
        r.link = null
        r.sender.onBatteryChanged()
        assertEquals(1, second.frames.size)
    }

    @Test
    fun aFailedSendIsCountedAndTheSnapshotIsNotRemembered() {
        val r = Rig(Link(accepts = false))
        r.sender.onReady()
        assertEquals(1, r.stats.droppedOffline.get())
        r.link!!.accepts = true
        r.sender.onBatteryChanged()
        assertEquals("nothing was sent before, so the battery goes now", 1, r.link!!.frames.size)
    }

    @Test
    fun theSnapshotCarriesTheWholeCurrentState() {
        val r = Rig()
        r.pause = WirePause.Remaining(60_000)
        r.sender.onReady()
        val frame = StrictJson.parse(r.link!!.frames.single()).jsonObject
        assertEquals("presence", frame.getValue("type").toString().trim('"'))
        assertEquals(62, (frame.getValue("battery") as kotlinx.serialization.json.JsonObject).getValue("level").toString().toInt())
        assertTrue(frame.containsKey("pause"))
        assertTrue(frame.getValue("id").toString().length > 10)
    }

    @Test
    fun theWireBuilderWritesTheDocumentedShapeAndClampsTheNumbers() {
        assertEquals(
            """{"type":"presence","id":"x","battery":{"level":62,"charging":true,"saver":false},"pause":{"remainingMs":3540000}}""",
            PresenceWire.presence("x", PresenceSnapshot(Battery(62, true, false), WirePause.Remaining(3_540_000))),
        )
        assertEquals("""{"type":"presence","id":"x","pause":{"untilResumed":true}}""", PresenceWire.presence("x", PresenceSnapshot(pause = WirePause.UntilResumed)))
        assertEquals("""{"type":"presence","id":"x"}""", PresenceWire.presence("x", PresenceSnapshot()))
        assertTrue(PresenceWire.presence("x", PresenceSnapshot(pause = WirePause.Remaining(9_999_999_999_999))).contains("\"remainingMs\":691200000"))
        assertEquals("""{"type":"ring.status","id":"x","state":"ringing","audible":false}""", PresenceWire.ringStatus("x", "ringing", reason = "ignored", audible = false))
        assertEquals("""{"type":"ring.status","id":"x","state":"stopped","reason":"timeout"}""", PresenceWire.ringStatus("x", "stopped", reason = "timeout", audible = true))
        assertTrue("a presence has no text field at all: ${PresenceWire.presence("x", PresenceSnapshot(Battery(1, false)))}", !PresenceWire.presence("x", PresenceSnapshot(Battery(1, false))).contains("name"))
    }

    @Test
    fun theSendersSignificanceRuleIsTheSpecs() {
        val s = PresenceSender.Companion
        assertFalse(s.significant(null, null))
        assertTrue("appearing", s.significant(null, Battery(50, false)))
        assertTrue("going away", s.significant(Battery(50, false), null))
        assertFalse(s.significant(Battery(50, false), Battery(51, false)))
        assertTrue(s.significant(Battery(50, false), Battery(52, false)))
        assertTrue(s.significant(Battery(50, false), Battery(48, false)))
        assertTrue("15 to 14", s.significant(Battery(15, false), Battery(14, false)))
        assertFalse("16 to 15", s.significant(Battery(16, false), Battery(15, false)))
        assertTrue("19 to 20", s.significant(Battery(19, false), Battery(20, false)))
        assertTrue("99 to 100", s.significant(Battery(99, true), Battery(100, true)))
        assertTrue(s.significant(Battery(50, false), Battery(50, true)))
        assertTrue(s.significant(Battery(50, false, false), Battery(50, false, true)))
    }

    @Test
    fun theWordsOfTheNotificationAreTheSpecsAndTheBatteryNeedsTheSwitch() {
        val battery = Battery(82, true)
        assertEquals("Connected to Desk PC · 82% · charging", PresenceText.connected("Connected to Desk PC", battery, show = true, pause = null))
        assertEquals("Connected to Desk PC · 14%", PresenceText.connected("Connected to Desk PC", Battery(14, false), show = true, pause = null))
        assertEquals("Connected to Desk PC · Kin paused on the PC until 14:30", PresenceText.connected("Connected to Desk PC", null, true, PeerPauseText(false, "14:30")))
        assertEquals("Connected to Desk PC · paused on the PC", PresenceText.connected("Connected to Desk PC", null, true, PeerPauseText(true, null)))
        assertEquals("the switch hides all of it", "Connected to Desk PC", PresenceText.connected("Connected to Desk PC", battery, show = false, pause = PeerPauseText(true, null)))
    }

    @Test
    fun theBatteryMapRoundsAndRefusesWhatItCannotRead() {
        assertEquals(Battery(50, false, null), BatteryMap.fromExtras(128, 255, 0, null))
        assertEquals(Battery(13, false, null), BatteryMap.fromExtras(1, 8, 0, null))
        assertEquals(Battery(100, true, true), BatteryMap.fromExtras(120, 100, 4, true))
        assertNull(BatteryMap.fromExtras(50, 0, 1, null))
        assertNull(BatteryMap.fromExtras(-1, 100, 0, null))
    }

    @Test
    fun theLowBatteryMachineKeepsItsStateAcrossReconnectsAndDoesNotDisarmWhenSuppressed() {
        val nudge = LowBatteryNudge()
        assertTrue(nudge.onBattery(14, false))
        assertFalse("the same episode after a reconnect", nudge.onBattery(14, false))
        assertFalse(nudge.onBattery(19, false))
        assertFalse(nudge.onBattery(10, false))
        assertFalse(nudge.onBattery(20, false))
        assertFalse(nudge.onBattery(21, false))
        assertTrue("re-armed at 20", nudge.onBattery(15, false))
        val paused = LowBatteryNudge()
        assertFalse("suppressed while paused", paused.onBattery(14, false, suppressed = true))
        assertTrue("and still armed afterwards", paused.onBattery(13, false))
    }

    @Test
    fun theCountsLineHasNoLevelAndNoPauseTime() {
        val r = Rig()
        r.battery = Battery(37, false)
        r.sender.onReady()
        val line = r.stats.line()
        assertEquals("presence: sent 1, received 0, dropped {rate: 0, offline: 0, not_listed: 0}", line)
        assertFalse(line.contains("37"))
    }
}
