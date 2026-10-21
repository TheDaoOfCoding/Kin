package app.kinrelay.android.presence

import app.kinrelay.android.features.FeatureOverrides
import app.kinrelay.android.features.FeatureResolver
import app.kinrelay.android.pause.GateKind
import app.kinrelay.android.pause.Inbound
import app.kinrelay.android.pause.PauseChoice
import app.kinrelay.android.pause.PauseGate
import app.kinrelay.android.pause.PauseState
import app.kinrelay.android.pause.PeerPauseView
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.MessageReceiver
import app.kinrelay.android.protocol.ReceiverRole
import app.kinrelay.android.protocol.StrictJson
import app.kinrelay.android.ring.RingMachine
import app.kinrelay.android.ring.RingPorts
import app.kinrelay.android.ring.RingReply
import app.kinrelay.android.shots.ShotConfig
import app.kinrelay.android.shots.ShotDecision
import app.kinrelay.android.shots.ShotPicker
import app.kinrelay.android.shots.ShotRow
import app.kinrelay.android.testing.FakeTime
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import app.kinrelay.android.testing.string
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.OffsetDateTime
import java.time.ZoneId

/**
 * Runs `protocol/presence.json` (changelog 58) on the phone's own code: `limits`, `presence[]` (through the real [MessageReceiver]), `pauseUntil[]`, `pauseGate[]`, `pauseExpiry[]`,
 * `lowBattery[]`, `batteryMap[]` (the Android rows), `presenceRate[]` (the real [PresenceSender] on a virtual clock, and [PresenceReceiveLimiter]), `ring[]` (the real [RingMachine] over fake ports),
 * `shot[]` (the real [ShotPicker]) and `features[]` (the real resolver). `origin[]` is the PC's.
 */
class PresenceVectorsTest {
    private val v = Vectors.load("presence.json")

    private fun JsonObject.long(name: String): Long? = this[name]?.takeIf { it !is JsonNull }?.jsonPrimitive?.long

    private fun JsonObject.bool(name: String): Boolean? = this[name]?.takeIf { it !is JsonNull }?.jsonPrimitive?.boolean

    private fun JsonObject.obj(name: String): JsonObject? = (this[name] as? JsonObject)

    private fun millis(iso: String): Long = OffsetDateTime.parse(iso).toInstant().toEpochMilli()

    @Test
    fun theLimitsOfTheSpecAreTheConstantsOfTheCode() {
        val l = v.getValue("limits").jsonObject
        assertEquals(l.long("frame"), PresenceWire.FRAME_BYTES.toLong())
        assertEquals(l.long("ringFrame"), PresenceWire.RING_FRAME_BYTES.toLong())
        assertEquals(l.long("maxPauseMs"), PresenceWire.MAX_PAUSE_MILLIS)
        assertEquals(l.long("batteryEveryMs"), PresenceSender.BATTERY_WINDOW_MILLIS)
        assertEquals(l.long("pauseTrailingMs"), PresenceSender.PAUSE_WINDOW_MILLIS)
        assertEquals(l.long("receiverCapacity"), PresenceReceiveLimiter.CAPACITY.toLong())
        assertEquals(l.long("receiverRefillMs"), PresenceReceiveLimiter.REFILL_MILLIS)
        assertEquals(l.long("receiverErrorSpacingMs"), PresenceReceiveLimiter.ERROR_SPACING_MILLIS)
        assertEquals(l.long("ringTimeoutMs"), RingMachine.TIMEOUT_MILLIS)
        assertEquals(l.long("ringStartsPerWindow"), RingMachine.MAX_STARTS.toLong())
        assertEquals(l.long("ringWindowMs"), RingMachine.START_WINDOW_MILLIS)
        assertEquals(l.long("ringCapMs"), RingMachine.CONTINUOUS_CAP_MILLIS)
        assertEquals(l.long("ringLockoutMs"), RingMachine.LOCKOUT_MILLIS)
        assertEquals(l.long("tomorrowHour"), PauseGate.TOMORROW_HOUR.toLong())
        assertEquals(l.long("tomorrowMinAheadMs"), PauseGate.TOMORROW_MIN_AHEAD_MILLIS)
        assertEquals(l.long("nudgeAt"), app.kinrelay.android.presence.LowBatteryNudge.NUDGE_LEVEL.toLong())
        assertEquals(l.long("nudgeRearmAt"), app.kinrelay.android.presence.LowBatteryNudge.REARM_LEVEL.toLong())
        assertEquals(l.long("shotStaleSec"), ShotPicker.STALE_SECONDS)
        assertEquals(l.long("shotRateCapacity"), app.kinrelay.android.shots.ShotWatcherCore.RATE_CAPACITY.toLong())
        assertEquals(l.long("shotRateRefillMs"), app.kinrelay.android.shots.ShotWatcherCore.RATE_REFILL_MILLIS)
        assertEquals(l.long("shotHoldMs"), app.kinrelay.android.shots.ShotWatcherCore.HOLD_MILLIS)
    }

    // ------------------------------------------------------------------ presence[]

    private fun snapshotJson(s: PresenceSnapshot): JsonObject = buildJsonObject {
        s.battery?.let { b -> put("battery", buildJsonObject { put("level", b.level); put("charging", b.charging); b.saver?.let { put("saver", it) } }) }
        when (val p = s.pause) {
            is WirePause.Remaining -> put("pause", buildJsonObject { put("remainingMs", p.millis) })
            WirePause.UntilResumed -> put("pause", buildJsonObject { put("untilResumed", true) })
            null -> Unit
        }
    }

    @Test
    fun presenceBodiesAreAcceptedOrBadRequestAsTheVectorsSay() {
        var run = 0
        for (case in v.cases("presence")) {
            val name = case.getValue("name").str()
            val receiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PC)
            receiver.ownCapabilities = setOf("presence")
            receiver.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Peer","platform":"android","appVersion":"0.2.0","capabilities":["presence"]}""")
            val text = case["frame"]?.toString() ?: case.getValue("raw").str()
            val outcome = receiver.onText(text)
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
        assertTrue("ran $run", run >= 45)
    }

    // ------------------------------------------------------------------ pauseUntil[], pauseGate[], pauseExpiry[]

    @Test
    fun pauseEndsAreTheVectorsWallClockTimes() {
        var run = 0
        for (case in v.cases("pauseUntil")) {
            val name = case.getValue("name").str()
            val choice = when (val c = case.getValue("choice").str()) {
                "1h" -> PauseChoice.HOUR
                "tomorrow" -> PauseChoice.TOMORROW
                else -> error("$name: unknown choice $c")
            }
            val state = PauseGate.stateFor(choice, millis(case.getValue("now").str()), ZoneId.of(case.getValue("zone").str()))
            assertEquals(name, millis(case.getValue("expect").str()), state.untilMillis)
            run++
        }
        assertTrue("ran $run", run > 15)
    }

    private fun gateKind(kind: String): GateKind = when (kind) {
        "clipboard" -> GateKind.CLIPBOARD
        "notif.post" -> GateKind.NOTIF_POST
        "notif.remove", "notif.sync", "notif.dismiss", "media.clear" -> GateKind.NOTIF_CLEANUP
        "media.state", "media.image", "media.volume" -> GateKind.MEDIA
        "screenshot" -> GateKind.SHOT
        "presence" -> GateKind.PRESENCE
        "ring.start", "ring.stop", "ring.status" -> GateKind.RING
        "unpair", "ping", "transfer-in-flight" -> GateKind.CONTROL
        "text", "link", "offer", "file" -> GateKind.MANUAL
        else -> error("unknown gate kind $kind")
    }

    @Test
    fun theGateDoesWhatTheTableOfTheSpecSays() {
        var run = 0
        for (case in v.cases("pauseGate")) {
            val name = case.getValue("name").str()
            val kind = gateKind(case.getValue("kind").str())
            val self = case.bool("selfPaused")!!
            val peer = case.bool("peerPaused")!!
            val expected = case.getValue("expected").str()
            val got = when (case.getValue("at").str()) {
                "send" -> when {
                    !PauseGate.maySend(kind, self, peer) -> "drop"
                    kind == GateKind.CLIPBOARD || kind == GateKind.NOTIF_POST || kind == GateKind.MEDIA || kind == GateKind.SHOT || kind == GateKind.MANUAL -> "send"
                    else -> "flows"
                }
                "receive" -> when (PauseGate.inbound(kind, self)) {
                    Inbound.DROP -> "drop"
                    Inbound.QUIET -> "quiet"
                    Inbound.PROCESS -> "flows"
                }
                else -> error("$name: at?")
            }
            assertEquals(name, expected, got)
            run++
        }
        assertTrue("ran $run", run >= 45)
    }

    @Test
    fun aPausesEndIsOnTheClockAndThePeersIsMonotonic() {
        var run = 0
        for (case in v.cases("pauseExpiry")) {
            val name = case.getValue("name").str()
            val expected = case.bool("expected")!!
            when (case.getValue("side").str()) {
                "self" -> {
                    val state = PauseState(untilMillis = case.long("pausedUntil") ?: 0L, untilResumed = case.bool("untilResumed") == true)
                    assertEquals(name, expected, PauseGate.paused(state, case.long("now")!!))
                }
                "peer" -> {
                    val time = FakeTime()
                    val view = PeerPauseView(time)
                    time.advanceTo(case.long("arrivalAt")!!)
                    view.onPresence(if (case.bool("untilResumed") == true) WirePause.UntilResumed else WirePause.Remaining(case.long("remainingMs")!!))
                    val nowAt = case.long("nowAt")!!
                    val later = case.long("laterPresenceWithoutPauseAt")
                    if (later != null && later <= nowAt) {
                        time.advanceTo(later)
                        view.onPresence(null)
                    }
                    time.advanceTo(nowAt)
                    assertEquals(name, expected, view.isPaused())
                }
                else -> error("$name: side?")
            }
            run++
        }
        assertTrue("ran $run", run > 10)
    }

    // ------------------------------------------------------------------ lowBattery[], batteryMap[]

    @Test
    fun theLowBatteryNudgeFiresOncePerEpisode() {
        var run = 0
        for (case in v.cases("lowBattery")) {
            val name = case.getValue("name").str()
            val machine = LowBatteryNudge()
            val fired = ArrayList<Long>()
            for (step in case.getValue("steps").jsonArray) {
                val s = step.jsonObject
                if (s.bool("none") == true) continue
                if (machine.onBattery(s.getValue("level").jsonPrimitive.int, s.bool("charging")!!, suppressed = s.bool("paused") == true)) fired += s.long("t")!!
            }
            assertEquals(name, case.getValue("expect").jsonArray.map { it.jsonPrimitive.long }, fired)
            run++
        }
        assertTrue("ran $run", run > 10)
    }

    @Test
    fun androidBatteryExtrasMapToTheWireBattery() {
        var run = 0
        for (case in v.cases("batteryMap")) {
            if (case.getValue("platform").str() != "android") continue
            val name = case.getValue("name").str()
            val got = BatteryMap.fromExtras(case.getValue("level").jsonPrimitive.int, case.getValue("scale").jsonPrimitive.int, case.getValue("plugged").jsonPrimitive.int, saver = null)
            val expect = case["expect"]
            if (expect == null || expect is JsonNull) {
                assertNull(name, got)
            } else {
                val e = expect.jsonObject
                assertEquals("$name level", e.getValue("level").jsonPrimitive.int, got?.level)
                assertEquals("$name charging", e.bool("charging"), got?.charging)
            }
            run++
        }
        assertTrue("ran $run", run > 8)
    }

    // ------------------------------------------------------------------ presenceRate[]

    private class FakeLink : PresenceLink {
        var ready = false
        val frames = ArrayList<JsonObject>()
        override val isReady: Boolean get() = ready
        override val peerListsPresence: Boolean get() = true
        override fun sendEphemeral(frame: String): Boolean {
            frames += StrictJson.parse(frame).jsonObject
            return true
        }
    }

    @Test
    fun theSenderFollowsItsRulesOnAVirtualClock() {
        var run = 0
        for (case in v.cases("presenceRate")) {
            if (case.getValue("side").str() != "sender") continue
            val name = case.getValue("name").str()
            val time = FakeTime()
            val link = FakeLink()
            var battery: Battery? = null
            var pauseUntil: Long? = null
            var untilResumed = false
            var expiry: Cancel? = null
            val sentAt = ArrayList<Long>()
            val recording = object : PresenceLink {
                override val isReady get() = link.isReady
                override val peerListsPresence get() = true
                override fun sendEphemeral(frame: String): Boolean {
                    sentAt += time.now
                    return link.sendEphemeral(frame)
                }
            }
            val withRecording = PresenceSender(
                time, time, link = { recording }, battery = { battery },
                pause = {
                    when {
                        untilResumed -> WirePause.UntilResumed
                        pauseUntil != null && pauseUntil!! > time.now -> WirePause.Remaining(pauseUntil!! - time.now)
                        else -> null
                    }
                },
                pauseKey = { if (untilResumed) "resumed" else pauseUntil?.takeIf { it > time.now } },
            )
            for (event in case.getValue("events").jsonArray) {
                val e = event.jsonObject
                time.advanceTo(e.long("t")!!)
                when (e.getValue("op").str()) {
                    "battery" -> {
                        battery = if (e.bool("none") == true) null else Battery(
                            e.getValue("level").jsonPrimitive.int, e.bool("charging")!!, e["saver"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.boolean,
                        )
                        withRecording.onBatteryChanged()
                    }
                    "connected" -> {
                        link.ready = true
                        withRecording.onReady()
                    }
                    "pause" -> {
                        expiry?.cancel()
                        expiry = null
                        untilResumed = e.bool("untilResumed") == true
                        pauseUntil = e.long("until")
                        // The pausing side's one-shot at the moment the pause ends.
                        pauseUntil?.let { until -> expiry = time.after(until - time.now) { withRecording.onPauseExpired() } }
                        withRecording.onPauseChanged()
                    }
                    "end" -> Unit
                    else -> error("$name: op?")
                }
            }
            // Nothing else is sent after the last event.
            val last = case.getValue("events").jsonArray.last().jsonObject.long("t")!!
            time.advanceTo(last)
            val expected = case.getValue("expect").jsonArray.map { it.jsonObject }
            assertEquals("$name: how many snapshots, sent at $sentAt", expected.size, link.frames.size)
            for ((i, want) in expected.withIndex()) {
                val got = link.frames[i]
                assertEquals("$name #$i: time", want.long("t"), sentAt[i])
                val wantBattery = want.obj("battery")
                val gotBattery = got["battery"] as? JsonObject
                assertEquals("$name #$i: battery", wantBattery, gotBattery)
                val wantPause = want.obj("pause")
                val gotPause = got["pause"] as? JsonObject
                assertEquals("$name #$i: pause", wantPause, gotPause)
            }
            run++
        }
        assertTrue("ran $run", run > 15)
    }

    @Test
    fun theReceiverLetsTenInAndRefillsOneEveryThreeSeconds() {
        var run = 0
        for (case in v.cases("presenceRate")) {
            if (case.getValue("side").str() != "receiver") continue
            val name = case.getValue("name").str()
            val time = FakeTime()
            val limiter = PresenceReceiveLimiter(time)
            val verdicts = ArrayList<String>()
            val errorsAt = ArrayList<Long>()
            for (event in case.getValue("events").jsonArray) {
                val (t, count) = event.jsonArray.map { it.jsonPrimitive.long }
                time.advanceTo(t)
                repeat(count.toInt()) {
                    val verdict = limiter.onPresence()
                    verdicts += if (verdict.accepted) "accepted" else "dropped"
                    if (verdict.sendError) errorsAt += t
                }
            }
            val runs = ArrayList<Pair<String, Int>>()
            for (verdict in verdicts) if (runs.isNotEmpty() && runs.last().first == verdict) runs[runs.size - 1] = verdict to runs.last().second + 1 else runs += verdict to 1
            assertEquals(name, case.getValue("expect").jsonArray.map { it.jsonArray[0].str() to it.jsonArray[1].jsonPrimitive.int }, runs)
            assertEquals("$name: errors", case.getValue("errorsAt").jsonArray.map { it.jsonPrimitive.long }, errorsAt)
            run++
        }
        assertTrue("ran $run", run >= 7)
    }

    // ------------------------------------------------------------------ ring[]

    private class RingRig(val time: FakeTime) {
        var allowed = true
        var silence = false
        val out = ArrayList<List<Any>>()
        var volume = 2
        var saved: Int? = null

        val ports = object : RingPorts {
            override fun volumeFixed() = false
            override fun alarmVolume() = volume
            override fun alarmMax() = 7
            override fun setAlarmVolume(index: Int) { volume = index }
            override fun startSound() = Unit
            override fun stopSound() = Unit
            override fun totalSilence() = silence
            override fun showStop(pcName: String, endsInMillis: Long) = true
            override fun hideStop() = Unit
            override fun holdAwake(on: Boolean) = Unit
            override fun savedIndex() = saved
            override fun saveIndex(index: Int) { saved = index }
            override fun clearSavedIndex() { saved = null }
            override fun announce(state: String, reason: String?, audible: Boolean?) {
                out += if (state == "ringing") listOf(time.now, "status", "ringing", audible ?: true) else listOf(time.now, "status", "stopped", reason ?: "")
            }
        }
        val machine = RingMachine(ports, time, time, allowed = { allowed }, loud = { true }, pcName = { "Desk PC" })

        fun reply(r: RingReply) {
            out += when (r) {
                RingReply.Ack -> listOf(time.now, "ack")
                is RingReply.Error -> listOf(time.now, "error", r.code)
            }
        }
    }

    @Test
    fun theRingMachineAnswersAndStopsAsTheTimelinesSay() {
        var run = 0
        for (case in v.cases("ring")) {
            val name = case.getValue("name").str()
            val time = FakeTime()
            val rig = RingRig(time)
            for (event in case.getValue("events").jsonArray) {
                val e = event.jsonObject
                time.advanceTo(e.long("t")!!)
                when (e.getValue("ev").str()) {
                    "start" -> {
                        rig.silence = e.bool("audible") == false
                        rig.machine.start(rig::reply)
                    }
                    "stop" -> rig.machine.stopFromPc(rig::reply)
                    "user_stop" -> rig.machine.stopByUser()
                    "allow" -> {
                        rig.allowed = e.bool("value")!!
                        rig.machine.allowedChanged()
                    }
                    "end" -> Unit
                    else -> error("$name: ev?")
                }
            }
            val last = case.getValue("events").jsonArray.last().jsonObject.long("t")!!
            time.advanceTo(last)
            val expected = case.getValue("expect").jsonArray.map { row -> row.jsonArray.map { cell -> (cell as JsonPrimitive).let { p -> if (p.isString) p.content else if (p.content == "true" || p.content == "false") p.boolean else p.long } } }
            assertEquals(name, expected, rig.out)
            run++
        }
        assertTrue("ran $run", run > 12)
    }

    // ------------------------------------------------------------------ shot[]

    @Test
    fun theScreenshotPickerDecidesAsTheVectorsSay() {
        var run = 0
        for (case in v.cases("shot")) {
            val name = case.getValue("name").str()
            val r = case.getValue("row").jsonObject
            val row = ShotRow(
                r.long("id")!!, r.string("relativePath"), r.string("mime"), r.bool("isPending")!!, r.long("dateAdded")!!, r.long("size")!!, r.string("ownerPackage"),
            )
            val baseline = case.long("baselineId")!!
            val config = ShotConfig(
                case.bool("screenshots")!!, case.bool("camera")!!, case.bool("paused")!!, baseline, case.getValue("handled").jsonArray.map { it.jsonPrimitive.long }.toSet(),
            )
            val decision = ShotPicker.decide(row, case.long("now")!!, config)
            // The rate limit (rule 10) is the caller's: the vector says whether the bucket had a token.
            val send = (decision as? ShotDecision.Send)?.takeIf { case.bool("rateOk") == true }?.kind?.wire
            val reason = when {
                decision is ShotDecision.Skip -> decision.reason
                send == null -> ShotPicker.SKIP_RATE
                else -> null
            }
            val expect = case.getValue("expect").jsonObject
            assertEquals("$name: send", expect.string("send"), send)
            if (send == null) assertEquals("$name: reason", expect.string("reason"), reason)
            // A skip while paused moves the baseline up to that image; nothing else moves it in the picker.
            val baselineAfter = if (reason == ShotPicker.SKIP_PAUSED) maxOf(baseline, row.id) else baseline
            assertEquals("$name: baseline", expect.long("baselineAfter"), baselineAfter)
            run++
        }
        assertTrue("ran $run", run > 25)
    }

    // ------------------------------------------------------------------ features[]

    @Test
    fun theResolverUsesTheOverrideThenTheDefault() {
        var run = 0
        for (case in v.cases("features")) {
            val name = case.getValue("name").str()
            val feature = case.getValue("feature").str()
            val peer = case.string("peer")
            val defaults = case.getValue("defaults").jsonObject
            val default = defaults[feature]?.takeIf { it is JsonPrimitive && !it.isString }?.jsonPrimitive?.boolean ?: false
            val overrides = FeatureOverrides.parse(case.getValue("overrides").toString())
            assertEquals(name, case.bool("expected"), FeatureResolver.enabled(overrides, feature, peer, default))
            run++
        }
        assertTrue("ran $run", run >= 13)
        // The PC's rule for `origin` is the desktop's own test.
        assertFalse(v.cases("origin").isEmpty())
        assertTrue(v.getValue("limits") is JsonObject && v["pauseUntil"] is JsonArray)
    }
}
