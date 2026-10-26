package app.kinrelay.android.media

import android.media.session.PlaybackState
import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.notifmirror.FilterDecision
import app.kinrelay.android.notifmirror.MirrorFilter
import app.kinrelay.android.notifmirror.MirrorPolicy
import app.kinrelay.android.notifmirror.NotifSnapshot
import app.kinrelay.android.notifmirror.SkipReason
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.StrictJson
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pure parts of 2B.3 that the shared vectors don't pin: the viewer's model, the art caps, the framework constants, the words, the guards and the no-content rule. */
@OptIn(ExperimentalCoroutinesApi::class)
class MediaLogicTest {
    private val sid = MediaWire.sidOf("com.example.player")

    private fun state(title: String? = "Song", artist: String? = "Band", state: PlayState = PlayState.PLAYING, position: Long? = 10_000, duration: Long? = 100_000, rate: Int = 1000, art: String? = null) =
        MediaState(sid, "Player", state, title, artist, null, duration, position, rate, listOf(MediaAction.PLAY, MediaAction.PAUSE), art, null, MediaVolume(40, false))

    // ---- the viewer's model

    @Test
    fun theViewerMovesThePositionFromTheAnchorAndShowsTheArtTheSourceSent() {
        val model = PcMediaModel()
        val art = byteArrayOf(1, 2, 3)
        val hash = MediaWire.imageHash(art)
        model.onImage(MediaImage(hash, MediaWire.KIND_ART, art))
        model.onState(state(art = hash), now = 1_000)
        val view = model.view("Desk PC", now = 4_000)!!
        assertEquals(13_000L, view.positionMs)
        assertEquals(PlayState.PLAYING, view.state)
        assertTrue(view.art!!.contentEquals(art))
        assertEquals("Band · Desk PC", view.deviceLine)
        assertEquals("Player on Desk PC", view.subText)
        assertEquals("Song", view.titleLine)
    }

    @Test
    fun theTitleLineFallsBackToTheAppAndTheDeviceLineToTheDeviceName() {
        val model = PcMediaModel()
        model.onState(state(title = null, artist = null), now = 0)
        val view = model.view("Desk PC", 0)!!
        assertEquals("Player", view.titleLine)
        assertEquals("Desk PC", view.deviceLine)
    }

    @Test
    fun aNewStateReplacesTheOneBeforeAndAClearRemovesIt() {
        val model = PcMediaModel()
        model.onState(state(title = "One"), 0)
        model.onState(state(title = "Two", state = PlayState.PAUSED), 100)
        assertEquals("Two", model.view("PC", 100)!!.titleLine)
        model.clear()
        assertNull(model.view("PC", 100))
    }

    @Test
    fun theLastEightImagesAreKeptAndNoMore() {
        val model = PcMediaModel()
        val hashes = (1..9).map { i ->
            val bytes = byteArrayOf(i.toByte())
            MediaImage(MediaWire.imageHash(bytes), MediaWire.KIND_ART, bytes).also(model::onImage)
        }.map { it.hash }
        model.onState(state(art = hashes.first()), 0)
        assertNull("the oldest of nine is gone", model.view("PC", 0)!!.art)
        model.onState(state(art = hashes.last()), 0)
        assertNotNull(model.view("PC", 0)!!.art)
    }

    @Test
    fun aMediaVolumeUpdatesOnlyTheVolume() {
        val model = PcMediaModel()
        model.onState(state(), 0)
        model.onVolume(MediaVolume(77, true))
        assertEquals(MediaVolume(77, true), model.view("PC", 0)!!.volume)
        assertEquals("Song", model.view("PC", 0)!!.titleLine)
    }

    @Test
    fun aPlayingStateIsShownPausedAtTheEndAfterFiveSecondsWithNothingNew() {
        val model = PcMediaModel()
        model.onState(state(position = 90_000, duration = 100_000), now = 0)
        // 90,000 + t > 105,000 at t = 15,001: one delay is armed for it.
        assertEquals(15_001L, model.millisUntilEnd(0))
        assertEquals(PlayState.PLAYING, model.view("PC", 15_000)!!.state)
        assertEquals(PlayState.PAUSED, model.view("PC", 15_001)!!.state)
        assertEquals(100_000L, model.view("PC", 60_000)!!.positionMs)
        model.onState(state(state = PlayState.PAUSED), 0)
        assertNull("a paused state has no end to wait for", model.millisUntilEnd(0))
        model.onState(state(duration = null), 0)
        assertNull("no duration, no end", model.millisUntilEnd(0))
    }

    @Test
    fun theDeviceNameIsCleanedAndHasAFallback() {
        assertEquals("Desk PC", PcMediaModel.deviceName("  Desk\nPC "))
        assertEquals("PC", PcMediaModel.deviceName(null))
        assertEquals("PC", PcMediaModel.deviceName("  "))
        assertEquals(40, PcMediaModel.deviceName("x".repeat(100)).length)
    }

    // ---- art

    @Test
    fun theArtLadderStopsAtTheFirstEncodingUnder24576Bytes() {
        val tried = ArrayList<Triple<Int, Int, Int>>()
        val bytes = ArtLadder.firstFitting(1000, 1000) { w, h, q ->
            tried += Triple(w, h, q)
            ByteArray(if (w == 256 && q > 60) 30_000 else 20_000)
        }!!
        assertEquals(20_000, bytes.size)
        assertEquals(listOf(Triple(256, 256, 80), Triple(256, 256, 70), Triple(256, 256, 60)), tried)
    }

    @Test
    fun theArtLadderStepsDownTo192And128PixelsAndThenGivesUp() {
        val sides = ArrayList<Int>()
        assertNull(ArtLadder.firstFitting(1000, 1000) { w, _, _ -> sides += w; ByteArray(40_000) })
        assertEquals(listOf(256, 256, 256, 256, 192, 192, 192, 192, 128, 128, 128, 128), sides)
        assertNotNull("the third rung can fit", ArtLadder.firstFitting(1000, 1000) { w, _, _ -> ByteArray(if (w <= 128) 1_000 else 40_000) })
    }

    @Test
    fun theArtLadderKeepsTheAspectRatioNeverEnlargesAndRefusesUnder16Pixels() {
        val first = ArrayList<Pair<Int, Int>>()
        ArtLadder.firstFitting(1000, 500) { w, h, _ -> first += w to h; ByteArray(100) }
        assertEquals(256 to 128, first.single())
        val small = ArrayList<Pair<Int, Int>>()
        ArtLadder.firstFitting(100, 80) { w, h, _ -> small += w to h; ByteArray(100) }
        assertEquals("a picture under 256 px is not enlarged", 100 to 80, small.single())
        assertNull("a picture under 16 px is no art", ArtLadder.firstFitting(15, 400) { _, _, _ -> ByteArray(10) })
        assertNull(ArtLadder.firstFitting(400, 8) { _, _, _ -> ByteArray(10) })
    }

    // ---- the framework's numbers, pinned against the SDK stubs

    @Test
    fun theWrittenOutFrameworkConstantsAreTheSdkOnes() {
        assertEquals(PlaybackState.STATE_NONE, MediaMapping.STATE_NONE)
        assertEquals(PlaybackState.STATE_STOPPED, MediaMapping.STATE_STOPPED)
        assertEquals(PlaybackState.STATE_PAUSED, MediaMapping.STATE_PAUSED)
        assertEquals(PlaybackState.STATE_PLAYING, MediaMapping.STATE_PLAYING)
        assertEquals(PlaybackState.STATE_FAST_FORWARDING, MediaMapping.STATE_FAST_FORWARDING)
        assertEquals(PlaybackState.STATE_REWINDING, MediaMapping.STATE_REWINDING)
        assertEquals(PlaybackState.STATE_BUFFERING, MediaMapping.STATE_BUFFERING)
        assertEquals(PlaybackState.STATE_ERROR, MediaMapping.STATE_ERROR)
        assertEquals(PlaybackState.STATE_CONNECTING, MediaMapping.STATE_CONNECTING)
        assertEquals(PlaybackState.STATE_SKIPPING_TO_PREVIOUS, MediaMapping.STATE_SKIPPING_TO_PREVIOUS)
        assertEquals(PlaybackState.STATE_SKIPPING_TO_NEXT, MediaMapping.STATE_SKIPPING_TO_NEXT)
        assertEquals(PlaybackState.STATE_SKIPPING_TO_QUEUE_ITEM, MediaMapping.STATE_SKIPPING_TO_QUEUE_ITEM)
        assertEquals(PlaybackState.ACTION_PAUSE, MediaMapping.ACTION_PAUSE)
        assertEquals(PlaybackState.ACTION_PLAY, MediaMapping.ACTION_PLAY)
        assertEquals(PlaybackState.ACTION_SKIP_TO_PREVIOUS, MediaMapping.ACTION_SKIP_TO_PREVIOUS)
        assertEquals(PlaybackState.ACTION_SKIP_TO_NEXT, MediaMapping.ACTION_SKIP_TO_NEXT)
        assertEquals(PlaybackState.ACTION_SEEK_TO, MediaMapping.ACTION_SEEK_TO)
        assertEquals(PlaybackState.ACTION_PLAY_PAUSE, MediaMapping.ACTION_PLAY_PAUSE)
        assertEquals(PlaybackState.PLAYBACK_POSITION_UNKNOWN, MediaMapping.PLAYBACK_POSITION_UNKNOWN)
    }

    @Test
    fun aViewerSessionMaskHasPlayAndPauseWhenOnlyToggleIsOffered() {
        assertEquals("the vectors' mask is one bit per action", MediaMapping.ACTION_PLAY_PAUSE, MediaMapping.maskOf(listOf(MediaAction.TOGGLE)))
        val mask = MediaMapping.sessionMaskOf(listOf(MediaAction.TOGGLE))
        assertTrue(mask and MediaMapping.ACTION_PLAY != 0L && mask and MediaMapping.ACTION_PAUSE != 0L)
        assertEquals(MediaCmdKind.TOGGLE, MediaMapping.playOrToggle(setOf(MediaAction.TOGGLE)))
        assertEquals(MediaCmdKind.PLAY, MediaMapping.playOrToggle(setOf(MediaAction.TOGGLE, MediaAction.PLAY)))
        assertNull(MediaMapping.playOrToggle(setOf(MediaAction.NEXT)))
        assertEquals(MediaCmdKind.PAUSE, MediaMapping.pauseOrToggle(setOf(MediaAction.PAUSE)))
    }

    @Test
    fun aPlayingSessionAtRateZeroIsPausedOnAndroid() {
        assertEquals(PlaybackState.STATE_PAUSED, MediaMapping.androidStateOf(PlayState.PLAYING, 0))
        assertEquals(PlaybackState.STATE_PLAYING, MediaMapping.androidStateOf(PlayState.PLAYING, 1000))
    }

    // ---- the guards and the commands beyond the vectors

    private class Recorder : MediaTransport {
        val calls = ArrayList<String>()
        override fun play() { calls += "play" }
        override fun pause() { calls += "pause" }
        override fun next() { calls += "next" }
        override fun prev() { calls += "prev" }
        override fun seekTo(positionMs: Long) { calls += "seek $positionMs" }
    }

    private class FakeVolume : MediaVolumeControl {
        var level = 50
        var muted = false
        var fail = false
        override fun setLevel(level: Int): Boolean { if (fail) throw SecurityException("dnd"); this.level = level; return true }
        override fun adjust(delta: Int): Boolean { level += delta; return true }
        override fun setMute(mute: Boolean): Boolean { muted = mute; return true }
        override fun read(): MediaVolume? = MediaVolume(level, muted)
    }

    @Test
    fun theRunnerDoesTheButtonOnTheActiveSessionAndAToggleDependsOnTheState() {
        val t = Recorder()
        var playing = PlayState.PLAYING
        val runner = MediaCommandRunner({ ActiveSession(sid, setOf(MediaAction.TOGGLE, MediaAction.NEXT, MediaAction.SEEK), playing, t) }, FakeVolume(), { true }, Clock { 0L })
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.TOGGLE, sid)))
        playing = PlayState.PAUSED
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.TOGGLE, sid)))
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.NEXT, sid)))
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.SEEK, sid, seekMs = 42_000)))
        assertEquals(listOf("pause", "play", "next", "seek 42000"), t.calls)
    }

    @Test
    fun aRefusedCommandIsActionFailedAndNothingElseHappened() {
        val refuse = object : MediaTransport {
            override fun play() = throw IllegalStateException("the player died")
            override fun pause() = Unit
            override fun next() = Unit
            override fun prev() = Unit
            override fun seekTo(positionMs: Long) = Unit
        }
        val runner = MediaCommandRunner({ ActiveSession(sid, setOf(MediaAction.PLAY), PlayState.PAUSED, refuse) }, FakeVolume(), { true }, Clock { 0L })
        val outcome = runner.run(MediaCmd(MediaCmdKind.PLAY, sid)) as CmdOutcome.Error
        assertEquals("action_failed", outcome.code)
    }

    @Test
    fun theVolumeCommandsSetTheLevelNeverBelowTheFloorAndReportTheNewVolume() {
        val volume = FakeVolume()
        val reported = ArrayList<MediaVolume>()
        val runner = MediaCommandRunner({ null }, volume, { true }, Clock { 0L }, volumeChanged = { reported += it })
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.VOLUME, level = 30)))
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.VOLUME, mute = true)))
        assertEquals(listOf(MediaVolume(30, false), MediaVolume(30, true)), reported)
        volume.fail = true
        val failed = runner.run(MediaCmd(MediaCmdKind.VOLUME, level = 5)) as CmdOutcome.Error
        assertEquals("a SecurityException is action_failed", "action_failed", failed.code)
        assertEquals("nothing was reported for it", 2, reported.size)
    }

    @Test
    fun theRateLimitErrorIsAnsweredOnceThenSilent() {
        var now = 0L
        val runner = MediaCommandRunner({ null }, FakeVolume(), { true }, Clock { now })
        repeat(20) { runner.run(MediaCmd(MediaCmdKind.VOLUME, level = 10)) }
        assertTrue(runner.run(MediaCmd(MediaCmdKind.VOLUME, level = 10)) is CmdOutcome.Error)
        assertEquals(CmdOutcome.Silent, runner.run(MediaCmd(MediaCmdKind.VOLUME, level = 10)))
        now = 10_000
        assertEquals("the budget came back", CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.VOLUME, level = 10)))
    }

    // ---- hostile input

    @Test
    fun aHostileTitleChangesNothingButTheShownText() {
        val title = "<script>alert(1)</script>\u202E\n" + "A".repeat(100_000)
        val frame = buildJsonObject {
            put("type", "media.state"); put("id", "i1"); put("sid", sid); put("app", "Player"); put("state", "playing"); put("title", title)
        }
        val ok = MediaInbound.parse(MediaWire.STATE, "i1", frame) as MediaInbound.Result.Ok
        val shown = (ok.message as MediaMsg.State).state.title!!
        assertEquals(MediaWire.TITLE_MAX, shown.codePointCount(0, shown.length))
        assertTrue(shown.startsWith("<script>alert(1)</script>_ AAAA"))
        assertFalse(shown.contains('\n') || shown.contains('\u202E'))
    }

    @Test
    fun anImageOf24577BytesA257PixelPictureAWrongHashAndAGifAreDropped() {
        fun frame(bytes: ByteArray, kind: String = "art", hash: String = MediaWire.imageHash(bytes)): JsonObject = buildJsonObject {
            put("type", "media.image"); put("id", "i"); put("hash", hash); put("kind", kind)
            put("data", java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(bytes))
        }
        fun png(w: Int, h: Int, size: Int): ByteArray {
            val b = ByteArray(maxOf(size, 33))
            val signature = intArrayOf(0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
            signature.forEachIndexed { i, v -> b[i] = v.toByte() }
            b[11] = 13
            "IHDR".forEachIndexed { i, c -> b[12 + i] = c.code.toByte() }
            for (i in 0..3) { b[16 + i] = (w shr (24 - 8 * i)).toByte(); b[20 + i] = (h shr (24 - 8 * i)).toByte() }
            return b
        }
        assertTrue(MediaInbound.parse(MediaWire.IMAGE, "i", frame(png(256, 256, 24_576))) is MediaInbound.Result.Ok)
        assertEquals(MediaInbound.Result.Dropped, MediaInbound.parse(MediaWire.IMAGE, "i", frame(png(256, 256, 24_577))))
        assertEquals(MediaInbound.Result.Dropped, MediaInbound.parse(MediaWire.IMAGE, "i", frame(png(257, 100, 100))))
        assertEquals(MediaInbound.Result.Dropped, MediaInbound.parse(MediaWire.IMAGE, "i", frame(png(100, 100, 100), hash = "0".repeat(32))))
        assertEquals(MediaInbound.Result.Dropped, MediaInbound.parse(MediaWire.IMAGE, "i", frame("GIF89a".toByteArray() + ByteArray(40))))
    }

    // ---- the words

    @Test
    fun theMediaCardSaysWhatIsWrong() {
        val on = MediaUi(policy = MediaPolicy(share = true, show = true), connected = true, peerListsMedia = true, pcName = "Desk PC", notificationsAllowed = true)
        // A line appears only for something to act on or wait for: the switch itself says on and off.
        assertNull(MediaUiText.shareLine(MediaUi(), true))
        assertEquals("Paused: Kin doesn't have notification access", MediaUiText.shareLine(on, access = false))
        assertEquals("Waiting for Desk PC", MediaUiText.shareLine(on.copy(connected = false), true))
        assertEquals("Desk PC needs Kin 0.2.0 to show media", MediaUiText.shareLine(on.copy(peerListsMedia = false), true))
        assertEquals("Desk PC isn't showing your media: turn it on in Kin on your PC", MediaUiText.shareLine(on.copy(peerStatus = MediaStatus(show = false, share = false)), true))
        assertNull(MediaUiText.shareLine(on.copy(peerStatus = MediaStatus(show = true, share = false)), true))
        assertNull(MediaUiText.showLine(MediaUi()))
        assertEquals("Desk PC isn't sharing its media: turn it on in Kin on your PC", MediaUiText.showLine(on.copy(peerStatus = MediaStatus(show = false, share = false))))
        assertEquals("Notifications are off for Kin, so Android can't show it", MediaUiText.showLine(on.copy(peerStatus = MediaStatus(show = false, share = true), notificationsAllowed = false)))
        assertEquals("Showing Desk PC's media", MediaUiText.showLine(on.copy(peerStatus = MediaStatus(show = false, share = true), showing = true)))
        assertNull(MediaUiText.showLine(on.copy(peerStatus = MediaStatus(show = false, share = true))))
        assertEquals("Desk PC is playing", MediaUiText.homeLine(on.copy(showing = true, pcPlaying = true), true))
        assertEquals("Desk PC has media paused", MediaUiText.homeLine(on.copy(showing = true, pcPlaying = false), true))
        assertNull(MediaUiText.homeLine(MediaUi(), true))
    }

    @Test
    fun thePlayPauseButtonSendsPauseWhilePlayingElsePlayOrTheSourcesToggle() {
        assertEquals(MediaCmdKind.PAUSE, MediaMapping.playPauseCommand(true, setOf(MediaAction.PLAY, MediaAction.PAUSE)))
        assertEquals(MediaCmdKind.PLAY, MediaMapping.playPauseCommand(false, setOf(MediaAction.PLAY, MediaAction.PAUSE)))
        assertEquals(MediaCmdKind.TOGGLE, MediaMapping.playPauseCommand(true, setOf(MediaAction.TOGGLE)))
        assertEquals(MediaCmdKind.TOGGLE, MediaMapping.playPauseCommand(false, setOf(MediaAction.TOGGLE)))
        assertNull(MediaMapping.playPauseCommand(true, setOf(MediaAction.NEXT)))
        assertNull(MediaMapping.playPauseCommand(false, emptySet()))
    }

    // ---- 4.6: the media notifications are never mirrored as notifications

    @Test
    fun theMediaNotificationsOfKinAndOfAnyAppAreNeverMirrored() {
        val own = "app.kinrelay.android"
        val policy = MirrorPolicy(enabled = true, allowed = setOf(own, "com.example.player"))
        val release = MirrorFilter(own, debugChannel = null)
        val debug = MirrorFilter(own, debugChannel = "kin_debug_mirror")
        fun snapshot(pkg: String, channel: String, media: Boolean) = NotifSnapshot(
            key = "0|$pkg|30|null|10100", packageName = pkg, channelId = channel, currentUser = true, ongoing = false, foregroundService = false, notClearable = false, groupSummary = false,
            localOnly = false, category = "transport", media = media, progressMax = 0, progressIndeterminate = false, importance = 2, suspended = false,
            visibility = NotifSnapshot.VISIBILITY_PRIVATE, lockscreenOverride = NotifSnapshot.VISIBILITY_NO_OVERRIDE, title = "Song", text = "Band", redacted = false,
        )
        // The PC-media notification of Kin: its own package (rule 1), and its channel does not start with the debug prefix, so even a debug build skips it as "own".
        val pc = snapshot(own, "kin_pc_media", media = true)
        assertEquals(SkipReason.OWN, (release.decide(pc, policy) as FilterDecision.Skip).reason)
        assertEquals(SkipReason.OWN, (debug.decide(pc, policy) as FilterDecision.Skip).reason)
        // A debug MediaStyle test notification (the debug channel Kin's mirror test uses) passes rule 1 in a debug build and is stopped by the media rule.
        assertEquals(SkipReason.MEDIA, (debug.decide(snapshot(own, "kin_debug_mirror", media = true), policy) as FilterDecision.Skip).reason)
        // Another app's player notification, allowed by the user for its other notifications: still media.
        assertEquals(SkipReason.MEDIA, (release.decide(snapshot("com.example.player", "playback", media = true), policy) as FilterDecision.Skip).reason)
    }

    // ---- 4.11: nothing of a session is logged or kept in the counters

    private class CollectingLog : KinLog {
        val lines = ArrayList<String>()
        override fun log(level: Char, component: String, message: String) { lines += "$level $component $message" }
    }

    @Test
    fun noTitleArtistAlbumAppSidOrArtIsEverLogged() = runTest {
        val log = CollectingLog()
        val frames = ArrayList<String>()
        val link = object : MediaLink {
            override val isReady = true
            override val peerUnderstandsMedia = true
            override fun sendEphemeral(frame: String): Boolean { frames += frame; return true }
        }
        val sender = MediaSender(backgroundScope, Clock { testScheduler.currentTime }, { link }, log)
        sender.setOwnShare(true)
        sender.onPeerStatus(MediaStatus(show = true, share = true))
        val secret = "ZEBRA-SECRET-TITLE"
        val art = MediaImageData(MediaWire.imageHash(byteArrayOf(9, 9, 9)), MediaWire.KIND_ART, byteArrayOf(9, 9, 9))
        val snapshot = MediaSnapshot(sid, "ZEBRA-APP", PlayState.PLAYING, secret, "ZEBRA-ARTIST", "ZEBRA-ALBUM", 100_000, 0, 0, 1f, listOf(MediaAction.PLAY), art.hash, null, null)
        sender.update(snapshot, listOf(art))
        for (i in 1..30) { advanceTimeBy(300); sender.update(snapshot.copy(position = i * 1_000L), listOf(art)) }
        sender.clear()
        advanceTimeBy(2_000)
        runCurrent()
        sender.command(MediaCmd(MediaCmdKind.NEXT, sid))
        sender.stats.maybeLog(1_000_000, log)
        assertTrue("the test sent frames", frames.any { it.contains(secret) })
        val logged = log.lines.joinToString("\n") + "\n" + sender.stats.line()
        for (needle in listOf(secret, "ZEBRA", sid, art.hash)) assertFalse("log has $needle:\n$logged", logged.contains(needle))
        assertTrue("the counts line is there", logged.contains("media: sent"))
    }

    @Test
    fun theWireFramesParseAsStrictJson() {
        val frame = MediaWire.state("id", MediaState(sid, "Player", PlayState.PAUSED, "x", null, null, null, null, 1000, emptyList()))!!
        assertEquals("media.state", StrictJson.parse(frame).jsonObject.getValue("type").let { (it as kotlinx.serialization.json.JsonPrimitive).content })
        assertTrue(Frames.hello("h", "Pixel", "android", "0.1.1").contains("\"media\""))
    }
}
