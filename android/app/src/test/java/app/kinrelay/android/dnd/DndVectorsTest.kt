package app.kinrelay.android.dnd

import app.kinrelay.android.presence.PresenceSnapshot
import app.kinrelay.android.presence.PresenceWire
import app.kinrelay.android.presence.WirePause
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.MessageReceiver
import app.kinrelay.android.protocol.ReceiverRole
import app.kinrelay.android.protocol.StrictJson
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Runs the Do Not Disturb part of `protocol/alerts.json` (changelog 59) on the phone's own code: `limits`, `dndMap[]`, `dndRequest[]`, `dndFollow[]` (the real [DndFollower] over every timeline,
 * including the 60 s grace and the start/unlink/pause/access/override cases), and `presenceDnd[]` of `presence.json` (through the real [MessageReceiver] and [PresenceWire]).
 */
class DndVectorsTest {
    private val v = Vectors.load("alerts.json")
    private val presence = Vectors.load("presence.json")

    private fun JsonObject.bool(name: String): Boolean = this[name]?.takeIf { it !is JsonNull }?.jsonPrimitive?.boolean ?: false

    private fun JsonObject.word(name: String): String? = this[name]?.takeIf { it !is JsonNull }?.jsonPrimitive?.content

    @Test
    fun theLimitsOfTheSpecAreTheConstantsOfTheCode() {
        val l = v.getValue("limits").jsonObject
        assertEquals(l.getValue("dndGraceMs").jsonPrimitive.long, DndFollower.GRACE_MILLIS)
    }

    @Test
    fun theFilterMapIsTheVectorsMap() {
        var run = 0
        for (case in v.cases("dndMap")) {
            val name = case.getValue("name").str()
            val filter = case.getValue("filter").jsonPrimitive.int
            assertEquals("$name: wire word", case.word("wire"), DndWire.wire(filter))
            run++
        }
        assertTrue("ran $run", run >= 7)
        // The platform constants are the numbers the vectors name.
        assertEquals(1, DndFilter.ALL)
        assertEquals(2, DndFilter.PRIORITY)
        assertEquals(3, DndFilter.NONE)
        assertEquals(4, DndFilter.ALARMS)
        assertEquals(android.app.NotificationManager.INTERRUPTION_FILTER_PRIORITY, DndFilter.PRIORITY)
        assertEquals(android.app.NotificationManager.INTERRUPTION_FILTER_ALARMS, DndFilter.ALARMS)
        assertEquals(android.app.NotificationManager.INTERRUPTION_FILTER_NONE, DndFilter.NONE)
        assertEquals(android.app.NotificationManager.INTERRUPTION_FILTER_ALL, DndFilter.ALL)
    }

    @Test
    fun theRequestIsNeverTotalSilence() {
        var run = 0
        for (case in v.cases("dndRequest")) {
            val name = case.getValue("name").str()
            assertEquals("$name: the filter Kin requests", case.getValue("filter").jsonPrimitive.int, DndWire.requestFor(case.word("setting")))
            run++
        }
        assertTrue("ran $run", run >= 4)
        for (setting in listOf(null, "", "total", "none", "all", "priority", "alarms", "garbage")) {
            val filter = DndWire.requestFor(setting)
            assertTrue("$setting -> $filter", filter == DndFilter.PRIORITY || filter == DndFilter.ALARMS)
        }
    }

    // ------------------------------------------------------------------ dndFollow[]

    @Test
    fun theFollowerDoesWhatEveryTimelineSays() {
        var run = 0
        for (case in v.cases("dndFollow")) {
            val name = case.getValue("name").str()
            val init = case.getValue("init").jsonObject
            val f = DndFollower(
                switchOn = init.bool("switch"), access = init.bool("access"), paused = init.bool("paused"),
                follow = DndLevel.fromWord(init.word("follow") ?: "priority"), filter = DndLevel.fromWord(init.word("filter") ?: "all"), weHold = init.bool("weHold"),
            )
            // Never `Request` unless the filter was `ALL` and Kin did not hold (T4 of the design): checked at every step from the follower's own view.
            for ((i, step) in case.getValue("steps").jsonArray.withIndex()) {
                val s = step.jsonObject
                val t = s.getValue("t").jsonPrimitive.long
                val filterBefore = f.currentFilter
                val heldBefore = f.weHold
                val actions = when (val ev = s.getValue("ev").str()) {
                    "connect" -> f.onConnect(t, s.bool("presence"))
                    "snapshot" -> f.onSnapshot(t, s.bool("focus"))
                    "close" -> f.onClose(t)
                    "pause" -> f.onPause(t, s.bool("on"))
                    "switch" -> f.onSwitch(t, s.bool("on"))
                    "access" -> f.onAccess(t, s.bool("granted"))
                    "filter" -> f.onFilter(t, DndLevel.fromWord(s.getValue("value").str()))
                    "unlink" -> f.onUnlink(t)
                    "start" -> f.onStart(t)
                    "tick" -> f.onTick(t)
                    else -> error("$name#$i: unknown event $ev")
                }
                assertEquals("$name#$i (${s.getValue("ev").str()} at $t): actions", s.getValue("do").jsonArray.map { it.str() }, actions.map { it.word })
                assertEquals("$name#$i: weHold", s.bool("weHold"), f.weHold)
                for (action in actions) {
                    if (action is DndAction.Request) {
                        assertTrue("$name#$i: a request needs the filter to have been ALL", filterBefore == DndLevel.ALL || s.getValue("ev").str() == "filter")
                        assertTrue("$name#$i: a request is never made while Kin holds", !heldBefore)
                        assertTrue("$name#$i: never total silence", action.level == DndLevel.PRIORITY || action.level == DndLevel.ALARMS)
                    }
                }
            }
            run++
        }
        assertTrue("ran $run", run >= 30)
    }

    @Test
    fun theGraceDeadlineIsSixtySecondsAfterTheCloseAndAClosedOnePausedFollowerHasNone() {
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        f.onSnapshot(10, true)
        assertNull(f.graceDeadline())
        f.onClose(1_000)
        assertEquals(61_000L, f.graceDeadline())
        f.onSnapshot(2_000, true)
        assertNull("a snapshot clears the grace timer", f.graceDeadline())
        f.onClose(5_000)
        assertEquals(65_000L, f.graceDeadline())
        assertEquals("a second close does not extend it", 65_000L, f.also { it.onClose(9_000) }.graceDeadline())
        assertEquals(listOf("release"), f.onTick(65_000).map { it.word })
        assertNull(f.graceDeadline())
    }

    @Test
    fun aRequestTheSystemRefusedHoldsNothing() {
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        assertEquals(listOf("request:priority"), f.onSnapshot(10, true).map { it.word })
        assertTrue(f.weHold)
        f.onRequestFailed()
        assertTrue(!f.weHold)
        assertEquals("the edge is handled: no retry loop", emptyList<String>(), f.onSnapshot(20, true).map { it.word })
    }

    @Test
    fun releaseNowLetsGoAndDoesNotAskAgainUntilTheNextEdge() {
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        f.onSnapshot(10, true)
        assertEquals(listOf("release"), f.onReleaseNow().map { it.word })
        assertTrue(!f.weHold)
        assertEquals(emptyList<String>(), f.onSnapshot(20, true).map { it.word })
        assertEquals(emptyList<String>(), f.onSnapshot(30, false).map { it.word })
        assertEquals(listOf("request:priority"), f.onSnapshot(40, true).map { it.word })
        assertEquals(emptyList<String>(), DndFollower().onReleaseNow().map { it.word })
    }

    @Test
    fun theFollowSettingAppliesAtTheNextEdgeNotUnderAHold() {
        val f = DndFollower(switchOn = true, access = true, filter = DndLevel.ALL)
        f.onConnect(0, true)
        assertEquals(listOf("request:priority"), f.onSnapshot(10, true).map { it.word })
        f.onFollowLevel(DndLevel.ALARMS)
        f.onFilter(20, DndLevel.PRIORITY)
        assertTrue("the hold in progress is not changed", f.weHold)
        f.onSnapshot(30, false)
        f.onFilter(40, DndLevel.ALL)
        assertEquals(listOf("request:alarms"), f.onSnapshot(50, true).map { it.word })
    }

    // ------------------------------------------------------------------ presenceDnd[]

    private fun snapshotJson(s: PresenceSnapshot): JsonObject = buildJsonObject {
        s.battery?.let { b -> put("battery", buildJsonObject { put("level", b.level); put("charging", b.charging); b.saver?.let { put("saver", it) } }) }
        when (val p = s.pause) {
            is WirePause.Remaining -> put("pause", buildJsonObject { put("remainingMs", p.millis) })
            WirePause.UntilResumed -> put("pause", buildJsonObject { put("untilResumed", true) })
            null -> Unit
        }
        s.dnd?.let { put("dnd", it) }
        if (s.focus) put("focus", true)
    }

    @Test
    fun theDndAndFocusFieldsOfAPresenceAreAcceptedOrBadRequestAsTheVectorsSay() {
        var run = 0
        for (case in presence.cases("presenceDnd")) {
            val name = case.getValue("name").str()
            val receiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PC)
            receiver.ownCapabilities = setOf("presence")
            receiver.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Peer","platform":"android","appVersion":"0.2.0","capabilities":["presence"]}""")
            val outcome = receiver.onText(case.getValue("frame").toString())
            when (case.getValue("expect").str()) {
                "accepted" -> {
                    assertTrue("$name: no reply, got ${outcome.replies}", outcome.replies.isEmpty())
                    val action = outcome.action as? Action.Presence ?: error("$name: expected a presence action, got ${outcome.action}")
                    assertEquals("$name: content", case.getValue("content").jsonObject, snapshotJson(action.snapshot))
                }
                "bad_request" -> {
                    assertEquals("$name: one reply, got ${outcome.replies}", 1, outcome.replies.size)
                    assertEquals("$name: code", "bad_request", StrictJson.parse(outcome.replies[0]).jsonObject.getValue("code").str())
                }
                else -> error("$name: unknown expectation")
            }
            run++
        }
        assertTrue("ran $run", run >= 20)
    }

    @Test
    fun aPresenceThePhoneWritesCarriesDndOnlyWhenItIsOnAndKnown() {
        assertEquals("""{"type":"presence","id":"x","dnd":"priority"}""", PresenceWire.presence("x", PresenceSnapshot(dnd = "priority")))
        assertEquals("""{"type":"presence","id":"x","dnd":"total"}""", PresenceWire.presence("x", PresenceSnapshot(dnd = "total")))
        assertEquals("an unknown word is never written", """{"type":"presence","id":"x"}""", PresenceWire.presence("x", PresenceSnapshot(dnd = "silence")))
        assertEquals("""{"type":"presence","id":"x"}""", PresenceWire.presence("x", PresenceSnapshot()))
        assertEquals("""{"type":"presence","id":"x","focus":true}""", PresenceWire.presence("x", PresenceSnapshot(focus = true)))
        // What the phone writes, the receiver (the PC role stands for the PC) reads back.
        val receiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PC).also { it.ownCapabilities = setOf("presence") }
        receiver.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Peer","platform":"android","appVersion":"0.2.0","capabilities":["presence"]}""")
        val back = receiver.onText(PresenceWire.presence("y", PresenceSnapshot(dnd = "alarms"))).action as Action.Presence
        assertEquals("alarms", back.snapshot.dnd)
    }
}
