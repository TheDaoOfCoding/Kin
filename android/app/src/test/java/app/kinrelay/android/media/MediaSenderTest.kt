package app.kinrelay.android.media

import app.kinrelay.android.link.Clock
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
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

/** [MediaSender]: what goes on the wire and in which order (design 3.1, 3.6, 3.7), on the test scheduler's clock. */
@OptIn(ExperimentalCoroutinesApi::class)
class MediaSenderTest {
    private class Link(var ready: Boolean = true, var media: Boolean = true) : MediaLink {
        val frames = ArrayList<JsonObject>()
        override val isReady get() = ready
        override val peerUnderstandsMedia get() = media
        override fun sendEphemeral(frame: String): Boolean {
            frames += Json.parseToJsonElement(frame).jsonObject
            return true
        }
        fun types() = frames.map { it.getValue("type").jsonPrimitive.content }
        fun ofType(type: String) = frames.filter { it.getValue("type").jsonPrimitive.content == type }
    }

    private class Rig(val scope: TestScope, val link: Link = Link()) {
        var current: MediaLink? = link
        val sender = MediaSender(scope.backgroundScope, Clock { scope.testScheduler.currentTime }, { current })

        /** Share on, the PC shows: the gate is open. */
        fun open(show: Boolean = true) {
            sender.setOwnShare(true)
            sender.onPeerStatus(MediaStatus(show = show, share = false))
        }
    }

    private val sid = MediaWire.sidOf("com.example.player")

    private fun snap(title: String = "t", state: PlayState = PlayState.PLAYING, art: String? = null, position: Long = 1_000, speed: Float = 1f, volume: MediaVolume? = null) = MediaSnapshot(
        sid = sid, app = "Player", state = state, title = title, artist = "a", album = null, durationMs = 200_000, position = position, positionUpdatedAt = 0, speed = speed,
        actions = listOf(MediaAction.PLAY, MediaAction.PAUSE), artHash = art, iconHash = null, volume = volume,
    )

    private fun image(seed: Int): MediaImageData {
        val bytes = byteArrayOf(seed.toByte(), 2, 3)
        return MediaImageData(MediaWire.imageHash(bytes), MediaWire.KIND_ART, bytes)
    }

    @Test
    fun nothingGoesBeforeThePeersStatusAndNothingWhileItDoesNotShow() = runTest {
        val rig = Rig(this)
        rig.sender.setOwnShare(true)
        rig.sender.update(snap(), emptyList())
        assertTrue("no status yet", rig.link.frames.isEmpty())
        assertEquals(1, rig.sender.stats.dropped(MediaStats.PEER_OFF))
        rig.sender.onPeerStatus(MediaStatus(show = false, share = true))
        rig.sender.update(snap("u"), emptyList())
        assertTrue("the PC does not show", rig.link.frames.isEmpty())
        // It starts showing: the session the sender holds goes out at once.
        rig.sender.onPeerStatus(MediaStatus(show = true, share = true))
        assertEquals(listOf("media.state"), rig.link.types())
        assertEquals("u", rig.link.frames.single().getValue("title").jsonPrimitive.content)
    }

    @Test
    fun ownShareOffSendsNothing() = runTest {
        val rig = Rig(this)
        rig.sender.onPeerStatus(MediaStatus(show = true, share = true))
        rig.sender.update(snap(), emptyList())
        assertTrue(rig.link.frames.isEmpty())
        assertEquals(1, rig.sender.stats.dropped(MediaStats.SHARE_OFF))
        assertFalse(rig.sender.isOpen())
    }

    @Test
    fun theCapabilityIsCheckedForEveryMessage() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap("a"), emptyList())
        rig.link.media = false // a new hello without media
        advanceTimeBy(1_000)
        rig.sender.update(snap("b"), emptyList())
        rig.link.media = true
        assertEquals(listOf("media.state"), rig.link.types())
        assertEquals(1, rig.sender.stats.dropped(MediaStats.OFFLINE))
    }

    @Test
    fun aBurstOfStatesIsTheFirstAndTheLastEvery250Ms() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap("a"), emptyList())
        for (title in listOf("b", "c", "d")) { advanceTimeBy(50); rig.sender.update(snap(title), emptyList()) }
        assertEquals(1, rig.link.frames.size)
        advanceTimeBy(300)
        runCurrent()
        assertEquals(listOf("a", "d"), rig.link.ofType("media.state").map { it.getValue("title").jsonPrimitive.content })
    }

    @Test
    fun anIdenticalStateIsNotSentAndASeekIs() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap(position = 1_000), emptyList())
        advanceTimeBy(5_000)
        // The player says the same thing again (the extrapolation explains the position): nothing goes out.
        rig.sender.update(snap(position = 1_000), emptyList())
        assertEquals(1, rig.link.frames.size)
        // A position the extrapolation does not explain (more than 1,500 ms away) is a seek.
        rig.sender.update(snap(position = 90_000), emptyList())
        assertEquals(2, rig.link.frames.size)
        // 90,000 at time 0 and now 5,000: the position of the moment of sending is 95,000.
        assertEquals(95_000L, rig.link.frames.last().getValue("positionMs").jsonPrimitive.content.toLong())
    }

    @Test
    fun thePositionSentIsTheOneOfTheMomentOfSending() = runTest {
        val rig = Rig(this)
        rig.open()
        advanceTimeBy(4_000)
        rig.sender.update(snap(position = 1_000), emptyList())
        // position 1,000 at time 0, now 4,000, speed 1: 5,000.
        assertEquals(5_000L, rig.link.frames.single().getValue("positionMs").jsonPrimitive.content.toLong())
    }

    @Test
    fun anImageGoesOutOnceBeforeTheFirstStateThatUsesItAndAgainOnANewConnection() = runTest {
        val rig = Rig(this)
        rig.open()
        val art = image(1)
        rig.sender.update(snap("a", art = art.hash), listOf(art))
        advanceTimeBy(1_000)
        rig.sender.update(snap("b", art = art.hash), listOf(art))
        assertEquals(listOf("media.image", "media.state", "media.state"), rig.link.types())
        assertEquals(art.hash, rig.link.frames[1].getValue("artHash").jsonPrimitive.content)
        // A new connection sends it again (nothing about a peer is remembered across connections), after the new peer's status.
        val second = Link()
        rig.current = second
        rig.sender.onPeerStatus(MediaStatus(show = true, share = false))
        rig.sender.update(snap("c", art = art.hash), listOf(art))
        assertEquals(listOf("media.image", "media.state"), second.types())
    }

    @Test
    fun aStateWhoseImageCouldNotBeSentGoesWithoutItsHash() = runTest {
        val rig = Rig(this)
        rig.open()
        // 20 images in a minute is the budget: the 21st state's image is not sent and its hash is left out.
        for (i in 1..21) {
            val art = image(i)
            rig.sender.update(snap("t$i", art = art.hash), listOf(art))
            advanceTimeBy(300)
        }
        assertEquals(20, rig.link.ofType("media.image").size)
        assertNull(rig.link.ofType("media.state").last()["artHash"])
    }

    @Test
    fun aClearWaits1500MsAndAReturningSessionCancelsIt() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap(), emptyList())
        rig.sender.clear()
        advanceTimeBy(1_000)
        rig.sender.update(snap(), emptyList())
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals(listOf("media.state"), rig.link.types())
        rig.sender.clear()
        advanceTimeBy(1_501)
        runCurrent()
        assertEquals(listOf("media.state", "media.clear"), rig.link.types())
    }

    @Test
    fun theImagesGoAgainWithTheNextSessionAfterAClear() = runTest {
        // A viewer may drop its pictures with the session, so after a media.clear the art is sent again (still before the state that uses it).
        val rig = Rig(this)
        rig.open()
        val art = image(3)
        rig.sender.update(snap("a", art = art.hash), listOf(art))
        rig.sender.clear()
        advanceTimeBy(1_600)
        runCurrent()
        rig.sender.update(snap("b", art = art.hash), listOf(art))
        assertEquals(listOf("media.image", "media.state", "media.clear", "media.image", "media.state"), rig.link.types())
    }

    @Test
    fun turningShareOffSendsOneClearAndThenNothing() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap(), emptyList())
        rig.sender.setOwnShare(false)
        rig.sender.update(snap("x"), emptyList())
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals(listOf("media.state", "media.clear"), rig.link.types())
    }

    @Test
    fun aClearIsNotSentToAViewerThatDoesNotShow() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap(), emptyList())
        rig.sender.onPeerStatus(MediaStatus(show = false, share = false))
        rig.sender.setOwnShare(false)
        assertEquals(listOf("media.state"), rig.link.types())
    }

    @Test
    fun theVolumeIsCoalescedAndAnIdenticalOneIsNotSent() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.volume(MediaVolume(10, false))
        for (level in listOf(20, 30, 40)) { advanceTimeBy(50); rig.sender.volume(MediaVolume(level, false)) }
        advanceTimeBy(300)
        runCurrent()
        rig.sender.volume(MediaVolume(40, false))
        assertEquals(listOf(10, 40), rig.link.ofType("media.volume").map { it.getValue("level").jsonPrimitive.content.toInt() })
    }

    @Test
    fun twelveStatesInOneInstantAreTheFirstAndTheLast() = runTest {
        // The 250 ms throttle is in front of the budget of 10 (the budget itself is proved by `rate[]` of media.json): a burst never even reaches it.
        val rig = Rig(this)
        rig.open()
        for (i in 1..12) rig.sender.update(snap("t$i"), emptyList())
        assertEquals(1, rig.link.frames.size)
        advanceTimeBy(300)
        runCurrent()
        assertEquals(listOf("t1", "t12"), rig.link.ofType("media.state").map { it.getValue("title").jsonPrimitive.content })
        assertEquals(0, rig.sender.stats.dropped(MediaStats.RATE))
    }

    @Test
    fun aCommandGoesOnlyToAPeerThatSharesAndSeekAndVolumeAreCoalesced() = runTest {
        val rig = Rig(this)
        val play = MediaCmd(MediaCmdKind.PLAY, sid = sid)
        assertEquals(MediaSender.CmdResult.DROPPED, rig.sender.command(play))
        rig.sender.onPeerStatus(MediaStatus(show = true, share = false))
        assertEquals("the PC does not share", MediaSender.CmdResult.DROPPED, rig.sender.command(play))
        rig.sender.onPeerStatus(MediaStatus(show = true, share = true))
        assertEquals(MediaSender.CmdResult.SENT, rig.sender.command(play))
        // A seek bar drag: the first goes at once, the rest are one command, the last value.
        assertEquals(MediaSender.CmdResult.SENT, rig.sender.command(MediaCmd(MediaCmdKind.SEEK, sid = sid, seekMs = 1_000)))
        for (ms in listOf(2_000L, 3_000L, 4_000L)) { advanceTimeBy(30); assertEquals(MediaSender.CmdResult.HELD, rig.sender.command(MediaCmd(MediaCmdKind.SEEK, sid = sid, seekMs = ms))) }
        advanceTimeBy(200)
        runCurrent()
        assertEquals(listOf(1_000L, 4_000L), rig.link.ofType("media.cmd").filter { it.getValue("cmd").jsonPrimitive.content == "seek" }.map { it.getValue("seekMs").jsonPrimitive.content.toLong() })
        assertEquals("the two commands before the PC shared", 2, rig.sender.stats.dropped(MediaStats.NO_SHARE))
    }

    @Test
    fun twentyCommandsInTenSecondsIsTheViewersLimit() = runTest {
        val rig = Rig(this)
        rig.sender.onPeerStatus(MediaStatus(show = true, share = true))
        var sent = 0
        repeat(25) { if (rig.sender.command(MediaCmd(MediaCmdKind.NEXT, sid = sid)) == MediaSender.CmdResult.SENT) sent++ }
        assertEquals(20, sent)
        assertEquals(5, rig.sender.stats.dropped(MediaStats.RATE))
    }

    @Test
    fun theStatusGoesAtOnceAndChangesAreSpacedBy5Seconds() = runTest {
        val rig = Rig(this)
        rig.sender.onConnected(MediaStatus(show = false, share = false, access = "missing"))
        rig.sender.statusChanged(MediaStatus(show = true, share = false, access = "missing"))
        rig.sender.statusChanged(MediaStatus(show = true, share = true, access = "granted"))
        assertEquals("one status so far", 1, rig.link.ofType("media.status").size)
        advanceTimeBy(5_001)
        runCurrent()
        val statuses = rig.link.ofType("media.status")
        assertEquals(2, statuses.size)
        assertEquals("the last value of the window", "granted", statuses.last().getValue("access").jsonPrimitive.content)
    }

    @Test
    fun aPeerWithoutMediaGetsNoStatus() = runTest {
        val rig = Rig(this, Link(media = false))
        rig.sender.onConnected(MediaStatus(show = true, share = true))
        assertTrue(rig.link.frames.isEmpty())
    }

    @Test
    fun offlineIsDroppedAndNothingIsKept() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.link.ready = false
        rig.sender.update(snap(), emptyList())
        assertTrue(rig.link.frames.isEmpty())
        assertEquals(1, rig.sender.stats.dropped(MediaStats.OFFLINE))
        rig.current = null
        rig.sender.update(snap("x"), emptyList())
        assertEquals(2, rig.sender.stats.dropped(MediaStats.OFFLINE))
    }

    @Test
    fun aSessionHeldWhileThePcStoppedShowingIsNotSentWhenItShowsAgain() = runTest {
        // Review round 1, item 7: the source stops while the PC doesn't show, so the session the sender held then may be long gone.
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap("old"), emptyList())
        assertEquals(listOf("media.state"), rig.link.types())
        rig.sender.onPeerStatus(MediaStatus(show = false, share = false))
        rig.sender.onPeerStatus(MediaStatus(show = true, share = false))
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals("the old session is not sent again", listOf("media.state"), rig.link.types())
        // A fresh read after the next show goes out normally.
        rig.sender.update(snap("new"), emptyList())
        assertEquals(listOf("old", "new"), rig.link.ofType("media.state").map { it.getValue("title").jsonPrimitive.content })
    }

    @Test
    fun aSessionHeldWhenTheSourceStoppedIsDroppedByTheClear() = runTest {
        val rig = Rig(this)
        rig.open()
        rig.sender.update(snap("old"), emptyList())
        rig.sender.clear()
        advanceTimeBy(1_600)
        runCurrent()
        // The PC turns show off and on: nothing is left to send from before.
        rig.sender.onPeerStatus(MediaStatus(show = false, share = false))
        rig.sender.onPeerStatus(MediaStatus(show = true, share = false))
        assertEquals(listOf("media.state", "media.clear"), rig.link.types())
    }
}
