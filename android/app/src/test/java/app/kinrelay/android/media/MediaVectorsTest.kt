package app.kinrelay.android.media

import app.kinrelay.android.link.Clock
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import app.kinrelay.android.testing.string
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.double
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
import java.util.Base64

/**
 * Runs `protocol/media.json` (changelog 57) on the phone's own code: every array that is not for one platform only (the `windows*`, `toSmtc` and the Windows rows of `sourcePosition`,
 * `volumeLevel` and `volumeSet` are the PC's). The frames themselves (`media-*` cases of `messages.json`) are in `MessageVectorsTest`.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MediaVectorsTest {
    private val v = Vectors.load("media.json")

    private fun JsonObject.long(name: String): Long? = this[name]?.takeIf { it !is JsonNull }?.jsonPrimitive?.long

    private fun JsonObject.int(name: String): Int? = this[name]?.takeIf { it !is JsonNull }?.jsonPrimitive?.int

    private fun JsonObject.strings(name: String): List<String> = getValue(name).jsonArray.map { it.str() }

    private class ManualClock : Clock {
        var now = 0L
        override fun elapsedMillis(): Long = now
    }

    @Test
    fun theLimitsOfTheSpecAreTheConstantsOfTheCode() {
        val l = v.getValue("limits").jsonObject
        assertEquals(l.int("app"), MediaWire.APP_MAX)
        assertEquals(l.int("title"), MediaWire.TITLE_MAX)
        assertEquals(l.int("artist"), MediaWire.ARTIST_MAX)
        assertEquals(l.int("album"), MediaWire.ALBUM_MAX)
        assertEquals(l.int("actions"), MediaWire.MAX_ACTIONS)
        assertEquals(l.long("durationMs"), MediaWire.MAX_DURATION_MS)
        assertEquals(l.long("positionMs"), MediaWire.MAX_DURATION_MS)
        assertEquals(l.int("rateMilli"), MediaWire.MAX_RATE_MILLI)
        assertEquals(l.long("seekMs"), MediaWire.MAX_SEEK_MS)
        assertEquals(l.int("delta"), MediaWire.MAX_DELTA)
        assertEquals(l.int("stateFrameSend"), MediaWire.STATE_SEND_FRAME_BYTES)
        assertEquals(l.int("stateFrameRecv"), MediaWire.STATE_FRAME_BYTES)
        assertEquals(l.int("imageFrame"), MediaWire.IMAGE_FRAME_BYTES)
        assertEquals(l.int("imageBytes"), MediaWire.IMAGE_MAX_BYTES)
        assertEquals(l.int("artPixels"), MediaWire.ART_MAX_SIDE)
        assertEquals(l.int("iconPixels"), MediaWire.ICON_MAX_SIDE)
        assertEquals(l.int("imageMinPixels"), MediaWire.IMAGE_MIN_SIDE)
        assertEquals(l.long("pausedExpiryMs"), MediaLimits.PAUSED_EXPIRY_MILLIS)
        assertEquals(l.long("clearDebounceMs"), MediaLimits.CLEAR_DEBOUNCE_MILLIS)
        assertEquals(l.long("positionJumpMs"), MediaLimits.POSITION_JUMP_MILLIS)
        assertEquals(l.long("endGraceMs"), MediaLimits.END_GRACE_MILLIS)
        assertEquals(l.long("throttleMs"), MediaLimits.STATE_THROTTLE_MILLIS)
    }

    @Test
    fun sidOfAnIdentity() {
        val cases = v.cases("sid")
        for (c in cases) assertEquals(c.str("name"), c.str("sid"), MediaWire.sidOf(c.str("identity")))
        println("media.json sid[]: ${cases.size} cases")
    }

    private fun JsonObject.str(name: String): String = getValue(name).str()

    @Test
    fun cleanedStrings() {
        val limits = v.getValue("limits").jsonObject
        val cases = v.cases("clean")
        // `inputParts` / `expectedParts` write a long text as parts: a string as it is, `[text, n]` = text repeated n times (protocol/README.md).
        fun parts(array: JsonArray): String = array.joinToString("") { p ->
            if (p is JsonArray) p[0].str().repeat(p[1].jsonPrimitive.int) else p.str()
        }
        for (c in cases) {
            val field = c.str("field")
            val input = c["inputParts"]?.jsonArray?.let(::parts) ?: c.str("input")
            val expected = if (c.containsKey("expectedParts")) c.getValue("expectedParts").jsonArray.let(::parts) else c.string("expected")
            assertEquals(c.str("name"), expected, MediaWire.clean(input, limits.int(field)!!))
        }
        println("media.json clean[]: ${cases.size} cases")
    }

    @Test
    fun imagesAreAcceptedDroppedOrBadRequest() {
        val cases = v.cases("image")
        for (c in cases) {
            val bytes = javaClass.classLoader!!.getResourceAsStream(c.str("dataFile"))!!.readBytes()
            val frame = buildJsonObject {
                put("type", MediaWire.IMAGE)
                put("id", "image-case")
                put("hash", c.string("hash") ?: MediaWire.imageHash(bytes))
                put("kind", c.str("kind"))
                put("data", Base64.getUrlEncoder().withoutPadding().encodeToString(bytes))
            }
            val got = when (val r = MediaInbound.parse(MediaWire.IMAGE, "image-case", frame)) {
                is MediaInbound.Result.Ok -> "accepted"
                is MediaInbound.Result.Bad -> "bad_request"
                MediaInbound.Result.Dropped -> "dropped"
            }
            assertEquals(c.str("name"), c.str("expect"), got)
        }
        println("media.json image[]: ${cases.size} cases")
    }

    @Test
    fun whatTheViewerShowsAfterSomeTime() {
        val cases = v.cases("position")
        for (c in cases) {
            val shown = MediaPosition.shown(PlayState.fromWire(c.str("state"))!!, c.long("positionMs"), c.int("rateMilli"), c.long("durationMs"), c.long("elapsedMs")!!)
            val expect = c.getValue("expect").jsonObject
            assertEquals("${c.str("name")}: position", expect.long("positionMs"), shown.positionMs)
            assertEquals("${c.str("name")}: state", expect.str("state"), shown.state.wire)
        }
        println("media.json position[]: ${cases.size} cases")
    }

    @Test
    fun theSourcesOwnPositionOnAndroid() {
        var run = 0
        for (c in v.cases("sourcePosition")) {
            if (c.str("platform") != "android") continue
            val got = MediaPosition.sourceAndroid(
                PlayState.fromWire(c.str("state"))!!, c.long("positionMs")!!, c.long("updateTimeMs")!!, c.getValue("speed").jsonPrimitive.double.toFloat(), c.long("nowMs")!!, c.long("durationMs"),
            )
            assertEquals(c.str("name"), c.long("expect"), got)
            run++
        }
        assertTrue(run >= 9)
        println("media.json sourcePosition[] (android): $run cases")
    }

    @Test
    fun theActiveSessionIsChosenByTheFixedRule() {
        val cases = v.cases("pick")
        for (c in cases) {
            val sessions = c.getValue("sessions").jsonArray.map { it.jsonObject }.map {
                PickSession(it.str("sid"), own = it["own"]?.jsonPrimitive?.boolean ?: false, title = it.getValue("title").jsonPrimitive.boolean, state = PlayState.fromWire(it.str("state")), pausedMs = it.long("pausedMs") ?: 0L)
            }
            assertEquals(c.str("name"), c.string("expect"), MediaPick.choose(sessions, c.string("current")))
        }
        println("media.json pick[]: ${cases.size} cases")
    }

    // ---- the cases that need time: the sender on a virtual clock (the test scheduler's)

    private class Link : MediaLink {
        var ready = true
        var media = true
        val frames = ArrayList<Pair<Long, JsonObject>>()
        var clockMillis: () -> Long = { 0L }
        override val isReady get() = ready
        override val peerUnderstandsMedia get() = media
        override fun sendEphemeral(frame: String): Boolean {
            frames += clockMillis() to Json.parseToJsonElement(frame).jsonObject
            return true
        }
    }

    private fun snapshot(title: String = "t", state: PlayState = PlayState.PLAYING) = MediaSnapshot(
        sid = MediaWire.sidOf("com.example.player"), app = "Player", state = state, title = title, artist = null, album = null, durationMs = 200_000, position = 1000,
        positionUpdatedAt = 0, speed = 1f, actions = listOf(MediaAction.PLAY), artHash = null, iconHash = null, volume = null,
    )

    @Test
    fun aSessionThatIsGoneIsClearedAfter1500Ms() = runTest {
        val cases = v.cases("clearDebounce")
        for (c in cases) {
            val link = Link()
            val scheduler = testScheduler
            val base = scheduler.currentTime
            link.clockMillis = { scheduler.currentTime - base }
            val sender = MediaSender(backgroundScope, Clock { scheduler.currentTime }, { link }, newId = MessageIds::newId)
            sender.setOwnShare(true)
            sender.onPeerStatus(MediaStatus(show = true, share = false))
            // "A session is shown when the run starts."
            sender.update(snapshot(), emptyList())
            for (event in c.getValue("events").jsonArray.map { it.jsonObject }) {
                val at = event.long("t")!!
                val wait = base + at - scheduler.currentTime
                if (wait > 0) advanceTimeBy(wait)
                runCurrent()
                if (event.getValue("present").jsonPrimitive.boolean) sender.update(snapshot(), emptyList()) else sender.clear()
            }
            val clears = link.frames.filter { it.second.getValue("type").str() == MediaWire.CLEAR }.map { it.first }
            assertEquals(c.str("name"), c.getValue("expect").jsonArray.map { it.jsonPrimitive.long }, clears)
            // Each case starts a fresh sender; its jobs must not leak into the next.
            advanceTimeBy(10_000)
            runCurrent()
        }
        println("media.json clearDebounce[]: ${cases.size} cases")
    }

    @Test
    fun theTrailingThrottle() = runTest {
        val cases = v.cases("throttle")
        for (c in cases) {
            val scheduler = testScheduler
            val base = scheduler.currentTime
            val sent = ArrayList<Pair<Long, String>>()
            val throttle = ValueThrottle<String>(MediaLimits.STATE_THROTTLE_MILLIS, Clock { scheduler.currentTime }, backgroundScope) { sent += (scheduler.currentTime - base) to it }
            for (event in c.getValue("events").jsonArray.map { it.jsonObject }) {
                val wait = base + event.long("t")!! - scheduler.currentTime
                if (wait > 0) advanceTimeBy(wait)
                runCurrent()
                throttle.offer(event.str("value"))
            }
            advanceTimeBy(1_000)
            runCurrent()
            val expect = c.getValue("expect").jsonArray.map { it.jsonObject }.map { it.long("t")!! to it.str("value") }
            assertEquals(c.str("name"), expect, sent)
        }
        println("media.json throttle[]: ${cases.size} cases")
    }

    @Test
    fun theFiveRateLimits() {
        val limiters = v.getValue("limiters").jsonObject
        val cases = v.cases("rate")
        for (c in cases) {
            val clock = ManualClock()
            val name = c.str("limiter")
            val spec = limiters.getValue(name).jsonObject
            val take: () -> Boolean = when (name) {
                "sourceState" -> MediaLimits.sourceState(clock)::tryTake
                "viewerState" -> MediaLimits.viewerState(clock)::tryTake
                "sourceCmd" -> MediaLimits.sourceCmd(clock)::tryTake
                "viewerCmd" -> MediaLimits.viewerCmd(clock)::tryTake
                "image" -> MediaLimits.image(clock)::tryTake
                else -> error(name)
            }
            val spacer = spec.long("errorSpacingMs")?.let { ErrorSpacer(it, clock) }
            val runs = ArrayList<Pair<String, Int>>()
            val errors = ArrayList<Long>()
            for (pair in c.getValue("events").jsonArray) {
                val (t, count) = pair.jsonArray.map { it.jsonPrimitive.long }
                clock.now = t
                repeat(count.toInt()) {
                    val outcome = if (take()) "accepted" else "dropped"
                    if (outcome == "dropped" && spacer?.mayAnswer() == true) errors += t
                    if (runs.isNotEmpty() && runs.last().first == outcome) runs[runs.lastIndex] = outcome to runs.last().second + 1 else runs += outcome to 1
                }
            }
            val expect = c.getValue("expect").jsonArray.map { it.jsonArray }.map { it[0].str() to it[1].jsonPrimitive.int }
            assertEquals("${c.str("name")}: outcomes", expect, runs)
            assertEquals("${c.str("name")}: errors", c.getValue("errorsAt").jsonArray.map { it.jsonPrimitive.long }, errors)
        }
        println("media.json rate[]: ${cases.size} cases")
    }

    // ---- the maps and the volume

    @Test
    fun playbackStateToTheWire() {
        val cases = v.cases("androidMap")
        for (c in cases) {
            val state = MediaMapping.playStateOf(c.getValue("state").jsonPrimitive.int)
            val expect = c.getValue("expect").jsonObject
            assertEquals("${c.str("name")}: candidate", expect.getValue("candidate").jsonPrimitive.boolean, state != null)
            expect.string("state")?.let { assertEquals("${c.str("name")}: state", it, state?.wire) }
            val actions = MediaMapping.actionsOf(c.getValue("actions").jsonPrimitive.long, c.getValue("hasDuration").jsonPrimitive.boolean).map { it.wire }
            assertEquals("${c.str("name")}: actions", expect.strings("actions"), actions)
            assertEquals("${c.str("name")}: rateMilli", expect.int("rateMilli"), MediaMapping.rateMilliOf(c.getValue("speed").jsonPrimitive.double.toFloat()))
        }
        println("media.json androidMap[]: ${cases.size} cases")
    }

    @Test
    fun theWireToPlaybackState() {
        val cases = v.cases("toAndroid")
        for (c in cases) {
            val expect = c.getValue("expect").jsonObject
            val actions = c.strings("actions").map { MediaAction.fromWire(it)!! }
            assertEquals("${c.str("name")}: state", expect.int("state"), MediaMapping.androidStateOf(PlayState.fromWire(c.str("state"))!!))
            assertEquals("${c.str("name")}: actions", expect.long("actions"), MediaMapping.maskOf(actions))
        }
        println("media.json toAndroid[]: ${cases.size} cases")
    }

    @Test
    fun volumeLevelsAndSteps() {
        var level = 0
        for (c in v.cases("volumeLevel")) {
            if (c.str("platform") != "android") continue
            assertEquals(c.str("name"), c.int("level"), MediaVolumeMath.levelOf(c.int("index")!!, c.int("max")!!))
            level++
        }
        var set = 0
        for (c in v.cases("volumeSet")) {
            if (c.str("platform") != "android") continue
            val max = c.int("max")!!
            val index = c.int("index")!!
            val op = c.getValue("op").jsonObject
            val expect = c.getValue("expect").jsonObject
            when {
                op["level"] != null -> assertEquals(c.str("name"), expect.int("index"), MediaVolumeMath.indexOfLevel(op.int("level")!!, max))
                op["delta"] != null -> assertEquals(c.str("name"), expect.int("index"), MediaVolumeMath.indexAfterDelta(index, op.int("delta")!!, max))
                else -> assertEquals(c.str("name"), expect.getValue("muted").jsonPrimitive.boolean, op.getValue("mute").jsonPrimitive.boolean)
            }
            set++
        }
        assertTrue(level >= 5 && set >= 8)
        println("media.json volumeLevel[]/volumeSet[] (android): $level + $set cases")
    }

    // ---- who may send what, and what a source answers

    @Test
    fun whoMaySendWhat() {
        val cases = v.cases("sendRule")
        for (c in cases) {
            val hellos = c.getValue("hellos").jsonArray.map { h -> h.jsonArray.map { it.str() } }
            val listed = hellos.lastOrNull()?.contains(MediaWire.CAPABILITY) == true
            val status = c["status"]?.takeIf { it !is JsonNull }?.jsonObject?.let { MediaStatus(it.getValue("show").jsonPrimitive.boolean, it.getValue("share").jsonPrimitive.boolean) }
            val ownShare = c.getValue("ownShare").jsonPrimitive.boolean
            val turnedOff = c["shareJustTurnedOff"]?.jsonPrimitive?.boolean ?: false
            val send = when (val kind = c.str("kind")) {
                "state" -> MediaGate.mayPush(PushKind.STATE, ownShare, listed, status)
                "image" -> MediaGate.mayPush(PushKind.IMAGE, ownShare, listed, status)
                "volume" -> MediaGate.mayPush(PushKind.VOLUME, ownShare, listed, status)
                "clear" -> MediaGate.mayPush(PushKind.CLEAR, ownShare, listed, status, turnedOff)
                "cmd" -> MediaGate.mayCommand(listed, status)
                "status" -> MediaGate.mayStatus(listed)
                else -> error(kind)
            }
            assertEquals(c.str("name"), c.str("expected"), if (send) "send" else "nothing")
        }
        println("media.json sendRule[]: ${cases.size} cases")
    }

    @Test
    fun aViewerThatDoesNotShowDropsSilently() {
        val cases = v.cases("viewerRule")
        for (c in cases) {
            val accepted = MediaGate.viewerAccepts("media." + c.str("kind"), c.getValue("ownShow").jsonPrimitive.boolean)
            assertEquals(c.str("name"), c.str("expected"), if (accepted) "accept" else "drop")
        }
        println("media.json viewerRule[]: ${cases.size} cases")
    }

    private class Transport(val ok: Boolean) : MediaTransport {
        private fun go() { if (!ok) throw IllegalStateException("refused") }
        override fun play() = go()
        override fun pause() = go()
        override fun next() = go()
        override fun prev() = go()
        override fun seekTo(positionMs: Long) = go()
    }

    private class Volume(val ok: Boolean) : MediaVolumeControl {
        override fun setLevel(level: Int) = ok
        override fun adjust(delta: Int) = ok
        override fun setMute(mute: Boolean) = ok
        override fun read(): MediaVolume? = MediaVolume(50, false)
    }

    @Test
    fun whatASourceAnswersToACommand() {
        val cases = v.cases("cmdResult")
        for (c in cases) {
            val clock = ManualClock()
            var share = true
            val systemOk = c.getValue("systemOk").jsonPrimitive.boolean
            val activeSpec = c["active"]?.takeIf { it !is JsonNull }?.jsonObject
            val active = activeSpec?.let { ActiveSession(it.str("sid"), it.strings("actions").map { a -> MediaAction.fromWire(a)!! }.toSet(), PlayState.PAUSED, Transport(systemOk)) }
            val runner = MediaCommandRunner({ active }, Volume(systemOk), { share }, clock)
            val spec = c.getValue("cmd").jsonObject
            val kind = MediaCmdKind.fromWire(spec.str("cmd"))!!
            val cmd = MediaCmd(kind, sid = spec.string("sid"), seekMs = if (kind == MediaCmdKind.SEEK) 0L else null, level = if (kind == MediaCmdKind.VOLUME) 50 else null)
            if (!c.getValue("rateOk").jsonPrimitive.boolean) repeat(20) { runner.run(cmd) }
            share = c.getValue("ownShare").jsonPrimitive.boolean
            val got = when (val outcome = runner.run(cmd)) {
                CmdOutcome.Done -> "ok"
                CmdOutcome.Silent -> "silent"
                is CmdOutcome.Error -> outcome.code
            }
            assertEquals(c.str("name"), c.str("expected"), got)
        }
        println("media.json cmdResult[]: ${cases.size} cases")
    }

    @Test
    fun theStateFrameIsUnderTheSenderLimitAndEverythingThatIsNotSetIsLeftOut() {
        val frame = MediaWire.state("id", snapshot().toWire(0))!!
        val obj = Json.parseToJsonElement(frame).jsonObject
        assertEquals(setOf("type", "id", "sid", "app", "state", "title", "durationMs", "positionMs", "actions"), obj.keys)
        assertNull("a rate of 1000 is the default and is not sent", obj["rateMilli"])
        assertTrue(JsonArray(listOf(JsonPrimitive("play"))) == obj["actions"])
    }
}
