package app.kinrelay.android.media

import app.kinrelay.android.link.Clock
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Opus review round 1 of the Android side of 2B.3: the stop race, the stale card, the unposted notification, the PC volume, the command's session, the reconnect, the slider. */
@OptIn(ExperimentalCoroutinesApi::class)
class MediaReviewRound1Test {
    private val sid = MediaWire.sidOf("com.example.player")

    // ---- 1: a late pass of the reading worker can't re-register a callback after stop

    @Test
    fun aLatePassOfTheWorkerCannotRegisterAfterStop() {
        val gate = GenerationGate()
        val registered = ArrayList<String>()
        val gen = gate.advance { registered.clear() }
        assertEquals("a pass of the current run runs", "ran", gate.ifCurrent(gen) { registered += "a"; "ran" })
        // stop(): everything is let go under the same lock, and the run is over.
        gate.advance { registered.clear() }
        // The worker had already taken its next event: its pass is stale and does nothing.
        assertNull(gate.ifCurrent(gen) { registered += "late"; "ran" })
        assertTrue("nothing was registered again", registered.isEmpty())
    }

    @Test
    fun aNewRunStartsFromNothingAndTheOldRunStaysStale() {
        val gate = GenerationGate()
        val registered = ArrayList<String>()
        val first = gate.advance { }
        gate.ifCurrent(first) { registered += "one" }
        gate.advance { registered.clear() } // stop
        val second = gate.advance { registered.clear() } // start
        assertNull(gate.ifCurrent(first) { registered += "from the first run" })
        gate.ifCurrent(second) { registered += "two" }
        assertEquals(listOf("two"), registered)
    }

    // ---- 2: the source stops while sharing -> the PC's card is cleared

    private class Link : MediaLink {
        val frames = ArrayList<JsonObject>()
        override val isReady get() = true
        override val peerUnderstandsMedia get() = true
        override fun sendEphemeral(frame: String): Boolean {
            frames += Json.parseToJsonElement(frame).jsonObject
            return true
        }
        fun types() = frames.map { it.getValue("type").jsonPrimitive.content }
    }

    private class FakeSource(override var isRunning: Boolean = false, val startOk: Boolean = true) : SourceControl {
        var stops = 0
        override fun start(): Boolean {
            if (startOk) isRunning = true
            return startOk
        }
        override fun stop() {
            isRunning = false
            stops++
        }
    }

    private fun snap(title: String = "t") = MediaSnapshot(
        sid = sid, app = "Player", state = PlayState.PLAYING, title = title, artist = "a", album = null, durationMs = 200_000, position = 1_000, positionUpdatedAt = 0, speed = 1f,
        actions = listOf(MediaAction.PLAY, MediaAction.PAUSE), artHash = null, iconHash = null, volume = null,
    )

    @Test
    fun whenTheReaderStopsWhileSharingTheCardOnThePcIsCleared() = runTest {
        val link = Link()
        val sender = MediaSender(backgroundScope, Clock { testScheduler.currentTime }, { link })
        sender.setOwnShare(true)
        sender.onPeerStatus(MediaStatus(show = true, share = false))
        val source = FakeSource()
        val switch = SourceSwitch({ source }, sender)
        assertEquals(SourceSwitch.Result.STARTED, switch.apply(true))
        sender.update(snap(), emptyList())
        assertEquals(listOf("media.state"), link.types())
        // Notification access was revoked: share is still on, the reader stops.
        assertEquals(SourceSwitch.Result.STOPPED, switch.apply(false))
        assertEquals(1, source.stops)
        advanceTimeBy(1_600)
        runCurrent()
        assertEquals("the PC's card is cleared", listOf("media.state", "media.clear"), link.types())
        assertEquals("nothing happens again", SourceSwitch.Result.UNCHANGED, switch.apply(false))
    }

    @Test
    fun aReaderThatCannotStartIsReportedAndNothingIsCleared() = runTest {
        val link = Link()
        val sender = MediaSender(backgroundScope, Clock { testScheduler.currentTime }, { link })
        val switch = SourceSwitch({ FakeSource(startOk = false) }, sender)
        assertEquals(SourceSwitch.Result.START_FAILED, switch.apply(true))
        advanceTimeBy(2_000)
        runCurrent()
        assertTrue(link.frames.isEmpty())
    }

    // ---- 3, 4, 8: the PC-media card

    private class FakeDisplay(var canPost: Boolean = true) : PcMediaDisplay {
        var shown = false
        var shows = 0
        var hides = 0
        var volumeKeys: Boolean? = null
        var lastVolume: MediaVolume? = null
        override val isShown get() = shown
        override fun show(view: PcMediaView, showOnLockScreen: Boolean, volumeKeys: Boolean): Boolean {
            shows++
            this.volumeKeys = volumeKeys
            shown = canPost
            return canPost
        }
        override fun updateVolume(volume: MediaVolume?) {
            lastVolume = volume
        }
        override fun hide() {
            hides++
            shown = false
        }
    }

    private class Rig(val display: FakeDisplay = FakeDisplay()) {
        var now = 0L
        val ends = ArrayList<Long?>()
        val viewer = PcMediaViewer(Clock { now }, { display }, { display }, { ends += it }, { "Desk PC" })
        val policy = MediaPolicy(show = true)
    }

    private fun state(title: String = "Song", volume: MediaVolume? = MediaVolume(40, false)) = MediaState(
        sid, "Player", PlayState.PLAYING, title, "Band", null, 100_000L, 10_000L, 1000, listOf(MediaAction.PLAY, MediaAction.PAUSE), null, null, volume,
    )

    @Test
    fun whenTheNotificationCannotBePostedTheSessionIsHiddenToo() {
        val rig = Rig(FakeDisplay(canPost = false))
        rig.viewer.onState(state(), rig.policy)
        assertFalse("no session is left to take the headset and volume keys", rig.display.isShown)
        assertEquals(1, rig.display.hides)
        assertFalse(rig.viewer.notificationsAllowed)
        // The permission comes: the next state shows it.
        rig.display.canPost = true
        rig.viewer.onState(state("Next"), rig.policy)
        assertTrue(rig.display.isShown)
        assertTrue(rig.viewer.notificationsAllowed)
    }

    @Test
    fun theVolumeOfAPcWithNoSessionIsKept() {
        val rig = Rig()
        assertNull(rig.viewer.volume)
        rig.viewer.onVolume(MediaVolume(65, false))
        assertEquals("known without any session", MediaVolume(65, false), rig.viewer.volume)
        assertNull(rig.viewer.current)
        assertFalse("nothing is shown for a volume alone", rig.display.isShown)
        assertEquals("and nothing was created for it", 0, rig.display.shows)
        // A session arrives: its own volume is the PC's now, and a clear doesn't forget it.
        rig.viewer.onState(state(volume = MediaVolume(30, false)), rig.policy)
        assertEquals(MediaVolume(30, false), rig.viewer.volume)
        rig.viewer.onVolume(MediaVolume(55, true))
        assertEquals(MediaVolume(55, true), rig.display.lastVolume)
        rig.viewer.onClear()
        assertNull(rig.viewer.current)
        assertEquals(MediaVolume(55, true), rig.viewer.volume)
        // The connection goes: everything goes.
        rig.viewer.onLinkClosed()
        assertNull(rig.viewer.volume)
    }

    @Test
    fun theVolumeKeysFollowTheSwitchAndTheKnownVolume() {
        val rig = Rig()
        rig.viewer.onState(state(), rig.policy)
        assertEquals(false, rig.display.volumeKeys)
        rig.viewer.render(rig.policy.copy(useVolumeKeys = true))
        assertEquals(true, rig.display.volumeKeys)
        rig.viewer.onState(state(volume = null), rig.policy.copy(useVolumeKeys = true))
        assertEquals("a fixed-volume PC has no keys to give", false, rig.display.volumeKeys)
    }

    @Test
    fun aDifferentConnectionDropsTheOldPcsCardAndModel() {
        val rig = Rig()
        val first = Any()
        assertTrue(rig.viewer.onLinkReady(first))
        rig.viewer.onState(state("From the old PC"), rig.policy)
        rig.viewer.onVolume(MediaVolume(70, false))
        assertTrue(rig.display.isShown)
        assertFalse("a later hello of the same connection changes nothing", rig.viewer.onLinkReady(first))
        assertTrue(rig.display.isShown)
        // A fast reconnect: a new connection object arrives before the old link's close was seen.
        assertTrue(rig.viewer.onLinkReady(Any()))
        assertFalse("the old PC's card is gone", rig.display.isShown)
        assertNull(rig.viewer.current)
        assertNull(rig.viewer.volume)
        assertNull("the end timer was cancelled", rig.ends.last())
    }

    @Test
    fun theSwipeAndThePolicyStillDecideWhatIsDrawn() {
        val rig = Rig()
        rig.viewer.onState(state(), rig.policy.copy(show = false))
        assertEquals("show is off: nothing is drawn", 0, rig.display.shows)
        rig.viewer.onState(state(), rig.policy)
        assertEquals(1, rig.display.shows)
        rig.viewer.dismiss()
        assertFalse(rig.display.isShown)
        rig.viewer.onState(state().copy(state = PlayState.PAUSED), rig.policy)
        assertFalse("a swiped card stays away while the same track is paused", rig.display.isShown)
        rig.viewer.onState(state(), rig.policy)
        assertTrue("until the PC plays again", rig.display.isShown)
    }

    // ---- 6: the command presses the session it checked

    private class Recorder : MediaTransport {
        val calls = ArrayList<String>()
        override fun play() { calls += "play" }
        override fun pause() { calls += "pause" }
        override fun next() { calls += "next" }
        override fun prev() { calls += "prev" }
        override fun seekTo(positionMs: Long) { calls += "seek" }
    }

    private class NoVolume : MediaVolumeControl {
        override fun setLevel(level: Int) = true
        override fun adjust(delta: Int) = true
        override fun setMute(mute: Boolean) = true
        override fun read(): MediaVolume? = null
    }

    @Test
    fun aCommandReadsTheActiveSessionOnceAndPressesThatSession() {
        val a = Recorder()
        val b = Recorder()
        val sidA = MediaWire.sidOf("com.example.a")
        val sidB = MediaWire.sidOf("com.example.b")
        var reads = 0
        // The worker switches sessions right after the first read: a second read would give B.
        val runner = MediaCommandRunner(
            { if (reads++ == 0) ActiveSession(sidA, setOf(MediaAction.PLAY), PlayState.PAUSED, a) else ActiveSession(sidB, setOf(MediaAction.PLAY), PlayState.PAUSED, b) },
            NoVolume(), { true }, Clock { 0L },
        )
        assertEquals(CmdOutcome.Done, runner.run(MediaCmd(MediaCmdKind.PLAY, sidA)))
        assertEquals("one read for the check and the press", 1, reads)
        assertEquals(listOf("play"), a.calls)
        assertTrue("B was not touched", b.calls.isEmpty())
        // The same command for A now meets B: stale, and B still isn't touched.
        val stale = runner.run(MediaCmd(MediaCmdKind.PLAY, sidA)) as CmdOutcome.Error
        assertEquals("stale", stale.code)
        assertTrue(b.calls.isEmpty())
    }

    // ---- 5: the slider

    @Test
    fun aDragSendsAtMostOneValuePerGapAndTheFinalValueIsAlwaysTheCallersToSend() {
        val gate = SliderSendGate(400)
        assertTrue(gate.mayCommit(0))
        assertFalse(gate.mayCommit(100))
        assertFalse(gate.mayCommit(399))
        assertTrue(gate.mayCommit(400))
        // 60 s of dragging at 16 ms: about 150 values, nowhere near the 20 per 10 s limit.
        var sent = 0
        var t = 1_000L
        repeat(3_750) {
            if (gate.mayCommit(t)) sent++
            t += 16
        }
        assertTrue("$sent values in 60 s", sent <= 150)
        gate.reset()
        assertTrue("the next drag starts at once", gate.mayCommit(t))
    }

    // ---- the drop reason of a hello without media

    @Test
    fun aHelloWithoutMediaIsCountedApartFromABadImage() {
        assertTrue(MediaStats.NOT_LISTED in MediaStats.DROP_REASONS)
        val stats = MediaStats()
        stats.drop(MediaStats.NOT_LISTED)
        assertEquals(1, stats.dropped(MediaStats.NOT_LISTED))
        assertEquals(0, stats.dropped(MediaStats.BAD_IMAGE))
    }
}
