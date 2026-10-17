package app.kinrelay.android.notifmirror

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
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** [MirrorSender]: what goes on the wire and in which order (design 3.5, 3.6). The clock is the test scheduler's, so a second is instant. */
@OptIn(ExperimentalCoroutinesApi::class)
class MirrorSenderTest {
    private class Link(var ready: Boolean = true, var notif: Boolean = true) : MirrorLink {
        val frames = ArrayList<JsonObject>()
        override val isReady get() = ready
        override val peerUnderstandsNotif get() = notif
        override fun sendEphemeral(frame: String): Boolean {
            frames += Json.parseToJsonElement(frame).jsonObject
            return true
        }
        fun types() = frames.map { it.getValue("type").jsonPrimitive.content }
    }

    private class Rig(scope: TestScope, val link: Link = Link()) {
        var current: MirrorLink? = link
        val sender = MirrorSender(scope.backgroundScope, Clock { scope.testScheduler.currentTime }, { current })
    }

    private fun nid(i: Int) = "%032x".format(i)

    private fun request(i: Int = 1, pkg: String = "com.chat", title: String = "t$i", icon: IconData? = null) =
        PostRequest(pkg, "key$i", WirePost(nid(i), "Chat", iconHash = icon?.hash, title = title), icon)

    @Test
    fun aPostIsSentAtOnceAndAnIdenticalOneIsNot() = runTest {
        val rig = Rig(this)
        assertEquals(PostResult.SENT, rig.sender.post(request()))
        advanceTimeBy(5_000)
        assertEquals(PostResult.IDENTICAL, rig.sender.post(request()))
        assertEquals(listOf("notif.post"), rig.link.types())
        assertEquals(1, rig.sender.stats.identical.get())
    }

    @Test
    fun anIconGoesOutOnceBeforeTheFirstPostThatUsesItOnAConnection() = runTest {
        val rig = Rig(this)
        val icon = IconData(NotifWire.iconHash(byteArrayOf(1, 2, 3)), byteArrayOf(1, 2, 3))
        rig.sender.post(request(1, icon = icon))
        rig.sender.post(request(2, icon = icon))
        assertEquals(listOf("notif.icon", "notif.post", "notif.post"), rig.link.types())
        // A new connection sends it again.
        val second = Link()
        rig.current = second
        rig.sender.post(request(3, icon = icon))
        assertEquals(listOf("notif.icon", "notif.post"), second.types())
    }

    @Test
    fun offlineIsDroppedAndCounted() = runTest {
        val rig = Rig(this)
        rig.link.ready = false
        assertEquals(PostResult.DROPPED_OFFLINE, rig.sender.post(request()))
        rig.link.ready = true
        rig.link.notif = false // the PC's hello doesn't list notif
        assertEquals(PostResult.DROPPED_OFFLINE, rig.sender.post(request(2)))
        rig.current = null
        assertEquals(PostResult.DROPPED_OFFLINE, rig.sender.post(request(3)))
        assertFalse(rig.sender.remove(nid(3)))
        assertTrue(rig.link.frames.isEmpty())
        assertEquals(4, rig.sender.stats.dropped("offline"))
        assertFalse(rig.sender.isOnline())
    }

    @Test
    fun theCapabilityIsCheckedForEveryMessageNotOncePerConnection() = runTest {
        val rig = Rig(this)
        assertEquals(PostResult.SENT, rig.sender.post(request(1)))
        rig.link.notif = false // the PC turned the setting off: a new hello without notif
        assertEquals(PostResult.DROPPED_OFFLINE, rig.sender.post(request(2)))
        rig.link.notif = true
        assertEquals(PostResult.SENT, rig.sender.post(request(3)))
        assertEquals(2, rig.link.frames.size)
    }

    @Test
    fun aRemoveCancelsAPendingPostAndAlwaysGoesOut() = runTest {
        val rig = Rig(this)
        rig.sender.post(request(1, title = "a"))
        advanceTimeBy(100)
        assertEquals(PostResult.COALESCED, rig.sender.post(request(1, title = "b")))
        assertTrue(rig.sender.remove(nid(1)))
        advanceTimeBy(5_000)
        assertEquals(listOf("notif.post", "notif.remove"), rig.link.types())
        assertFalse(rig.sender.isAnnounced(nid(1)))
        // The same content after a remove is new again.
        assertEquals(PostResult.SENT, rig.sender.post(request(1, title = "a")))
    }

    @Test
    fun theLastStateOfABurstGoesOutWhenTheSecondEnds() = runTest {
        val rig = Rig(this)
        rig.sender.post(request(1, title = "v0"))
        for (i in 1..20) { advanceTimeBy(30); rig.sender.post(request(1, title = "v$i")) }
        assertEquals(1, rig.link.frames.size)
        advanceTimeBy(1_000)
        runCurrent()
        assertEquals(listOf("v0", "v20"), rig.link.frames.map { it.getValue("title").jsonPrimitive.content })
    }

    @Test
    fun theNidToKeyMapIsInMemoryAndBounded() = runTest {
        val rig = Rig(this)
        rig.sender.post(request(1))
        assertEquals("key1", rig.sender.keyOf(nid(1)))
        for (i in 2..300) { advanceTimeBy(600); rig.sender.post(request(i)) }
        assertTrue(rig.sender.announcedCount() <= MirrorSender.ANNOUNCED_LIMIT)
        assertEquals(null, rig.sender.keyOf(nid(1)))
    }

    @Test
    fun removeAnnouncedTakesDownTheNotificationsOfTheGivenAppsOnly() = runTest {
        val rig = Rig(this)
        rig.sender.post(request(1, pkg = "com.a"))
        rig.sender.post(request(2, pkg = "com.b"))
        assertEquals(1, rig.sender.removeAnnounced { it == "com.a" })
        assertEquals(listOf(nid(1)), rig.link.frames.filter { it.getValue("type").jsonPrimitive.content == "notif.remove" }.map { it.getValue("nid").jsonPrimitive.content })
        assertEquals(1, rig.sender.removeAnnounced())
        assertEquals(0, rig.sender.announcedCount())
    }

    @Test
    fun theStatusAndTheSyncGoOutOnConnectAndTheSyncHasNoContent() = runTest {
        val rig = Rig(this)
        rig.sender.onConnected(MirrorStatus(true, true, 6, true), listOf(nid(1), nid(2)))
        assertEquals(listOf("notif.status", "notif.sync"), rig.link.types())
        val status = rig.link.frames[0]
        assertEquals("granted", status.getValue("access").jsonPrimitive.content)
        assertEquals("6", status.getValue("apps").jsonPrimitive.content)
        assertEquals(2, rig.link.frames[1].getValue("nids").toString().split(",").size)
        // With no `notif` on the PC nothing is sent.
        val old = Link(notif = false)
        rig.current = old
        rig.sender.onConnected(MirrorStatus(true, true, 6, false), emptyList())
        assertTrue(old.frames.isEmpty())
    }

    @Test
    fun theStatusChangesGoOutAtMostOncePerFiveSeconds() = runTest {
        val rig = Rig(this)
        rig.sender.onConnected(MirrorStatus(true, true, 1, false), emptyList())
        rig.link.frames.clear()
        rig.sender.statusChanged(MirrorStatus(true, true, 1, false)) // unchanged: nothing
        assertTrue(rig.link.frames.isEmpty())
        advanceTimeBy(1_000)
        rig.sender.statusChanged(MirrorStatus(true, true, 2, false)) // inside the window: held
        rig.sender.statusChanged(MirrorStatus(true, true, 3, false)) // replaces the held one
        assertTrue(rig.link.frames.isEmpty())
        advanceTimeBy(4_000)
        runCurrent()
        assertEquals(listOf("3"), rig.link.frames.map { it.getValue("apps").jsonPrimitive.content })
        advanceTimeBy(6_000)
        rig.sender.statusChanged(MirrorStatus(false, false, 3, false)) // outside the window: at once
        assertEquals(2, rig.link.frames.size)
    }

    @Test
    fun aNewConnectionStartsCleanButKeepsTheKeysOfWhatWasAnnounced() = runTest {
        val rig = Rig(this)
        rig.sender.post(request(1, title = "same"))
        rig.current = Link()
        // The same content on a new connection is sent again (the PC may have lost its toast): the identical-post memory is per connection.
        assertEquals(PostResult.SENT, rig.sender.post(request(1, title = "same")))
        assertNotEquals(null, rig.sender.keyOf(nid(1)))
    }

    @Test
    fun theLogLineHasCountsOnly() = runTest {
        val rig = Rig(this)
        rig.sender.post(request(1, title = "SECRET-TITLE"))
        rig.sender.stats.drop("not_allowed")
        val line = rig.sender.stats.line()
        assertEquals("mirror: sent 1, removed 0, dropped {not_allowed: 1}", line)
        assertFalse(line.contains("SECRET"))
    }
}
