package app.kinrelay.android.notifmirror

import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** [MirrorEngine] and [ActionHandler] over fake notifications: the service's decisions, with no Android classes. */
@OptIn(ExperimentalCoroutinesApi::class)
class MirrorEngineTest {
    private class Link : MirrorLink {
        var ready = true
        val frames = ArrayList<JsonObject>()
        override val isReady get() = ready
        override val peerUnderstandsNotif = true
        override fun sendEphemeral(frame: String): Boolean { frames += Json.parseToJsonElement(frame).jsonObject; return true }
        fun types() = frames.map { it.getValue("type").jsonPrimitive.content }
    }

    private class Fake(val snapshot: NotifSnapshot, var cheapReads: Int = 0)

    private class Port : SbnPort<Fake> {
        var quickReads = 0
        var snapshotReads = 0
        var iconReads = 0
        override fun quick(sbn: Fake): QuickInfo { quickReads++; return QuickInfo(sbn.snapshot.packageName, sbn.snapshot.channelId, sbn.snapshot.currentUser) }
        override fun key(sbn: Fake) = sbn.snapshot.key
        override fun snapshot(sbn: Fake): NotifSnapshot { snapshotReads++; return sbn.snapshot }
        override fun appLabel(packageName: String) = "App $packageName"
        override fun icon(packageName: String, sbn: Fake): IconData? { iconReads++; return null }
    }

    private class Rig(scope: TestScope, policy: MirrorPolicy) {
        val link = Link()
        val port = Port()
        val logs = ArrayList<String>()
        val log = object : KinLog { override fun log(level: Char, component: String, message: String) { logs += "$level $component $message" } }
        var policy = policy
        val clock = Clock { scope.testScheduler.currentTime }
        val filter = MirrorFilter("app.kinrelay.android", null) { it == "com.bank" }
        val sender = MirrorSender(scope.backgroundScope, clock, { link }, log)
        val engine = MirrorEngine(port, { this.policy }, filter, sender, clock, log)
    }

    private fun snap(pkg: String = "com.chat", id: Int = 1, title: String? = "Alice", text: String? = "Secret hello", ongoing: Boolean = false) =
        Fake(NotifSnapshot(key = "0|$pkg|$id|null|10100", packageName = pkg, title = title, text = text, ongoing = ongoing))

    private val allowed = MirrorPolicy(enabled = true, allowed = setOf("com.chat", "com.bank"))

    @Test
    fun aNotificationOfAnAppThatIsNotTickedIsNeverRead() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onPosted(snap("com.other"))
        assertEquals(0, rig.port.snapshotReads)
        assertEquals(0, rig.port.iconReads)
        assertEquals(1, rig.sender.stats.dropped("not_allowed"))
        assertTrue(rig.link.frames.isEmpty())
    }

    @Test
    fun withTheMasterSwitchOffNothingIsReadAtAll() = runTest {
        val rig = Rig(this, allowed.copy(enabled = false))
        rig.engine.onPosted(snap())
        assertEquals(0, rig.port.quickReads)
        assertEquals(0, rig.port.snapshotReads)
        assertEquals(0L, rig.sender.stats.total())
    }

    @Test
    fun withNowhereToSendNothingIsReadAndItIsCountedAsOffline() = runTest {
        val rig = Rig(this, allowed)
        rig.link.ready = false
        rig.engine.onPosted(snap())
        assertEquals(0, rig.port.snapshotReads)
        assertEquals(1, rig.sender.stats.dropped("offline"))
    }

    @Test
    fun anAllowedNotificationIsEncodedAndSent() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onPosted(snap())
        assertEquals(listOf("notif.post"), rig.link.types())
        assertEquals("App com.chat", rig.link.frames[0].getValue("app").jsonPrimitive.content)
        assertEquals(1, rig.sender.stats.sent.get())
    }

    @Test
    fun aSkippedNotificationIsCountedByItsReason() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onPosted(snap(ongoing = true))
        rig.engine.onPosted(snap(id = 2, title = null, text = null))
        assertEquals(1, rig.sender.stats.dropped("ongoing"))
        assertEquals(1, rig.sender.stats.dropped("empty"))
        assertTrue(rig.link.frames.isEmpty())
    }

    @Test
    fun aFlaggedAppSendsOnlyItsName() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onPosted(snap("com.bank", text = "Balance 1,234 and code 482913"))
        val frame = rig.link.frames.single()
        assertTrue(frame.getValue("sensitive").jsonPrimitive.content == "true")
        assertFalse(frame.toString().contains("Balance")); assertFalse(frame.toString().contains("482913"))
        assertEquals("a flagged app's post carries no icon", 0, rig.port.iconReads)
    }

    @Test
    fun aNotificationThatStopsQualifyingTakesItsToastDown() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onPosted(snap())
        advanceTimeBy(2_000)
        rig.engine.onPosted(snap(ongoing = true)) // the same key, now ongoing
        assertEquals(listOf("notif.post", "notif.remove"), rig.link.types())
    }

    @Test
    fun aRemovalSendsARemoveExceptForTheSystemRegroupingAndAListenerCancelIsOneToo() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onPosted(snap())
        for (reason in listOf(13, 16)) rig.engine.onRemoved(snap(), reason)
        assertEquals(listOf("notif.post"), rig.link.types())
        rig.engine.onRemoved(snap(), 1) // REASON_CLICK: read on the phone
        assertEquals(listOf("notif.post", "notif.remove"), rig.link.types())
        // Another listener (a watch) dismissed it (REASON_LISTENER_CANCEL): the PC's toast goes too.
        rig.engine.onPosted(snap(id = 2))
        rig.engine.onRemoved(snap(id = 2), 10)
        assertEquals("notif.remove", rig.link.types().last())
    }

    @Test
    fun aRemovalOfSomethingNeverAnnouncedSendsNothingUnlessItWouldHaveBeenMirrored() = runTest {
        val rig = Rig(this, allowed)
        rig.engine.onRemoved(snap(ongoing = true), 2) // never mirrored: an ongoing one
        assertTrue(rig.link.frames.isEmpty())
        rig.engine.onRemoved(snap(id = 5), 2) // after a restart: it would have been mirrored, so the toast goes
        assertEquals(listOf("notif.remove"), rig.link.types())
        rig.engine.onRemoved(snap("com.other"), 2)
        assertEquals(1, rig.link.frames.size)
    }

    @Test
    fun theRemovalThatFollowsAPcDismissIsNotEchoed() = runTest {
        val rig = Rig(this, allowed)
        val item = snap()
        rig.engine.onPosted(item)
        rig.engine.noteSelfCancel(Nid.of(item.snapshot.key))
        rig.engine.onRemoved(item, 2) // whatever the reason code, within 5 s
        assertEquals(listOf("notif.post"), rig.link.types())
        assertFalse(rig.sender.isAnnounced(Nid.of(item.snapshot.key)))
        // After 5 s the memory is gone: a later removal is a normal one.
        rig.engine.onPosted(item)
        rig.engine.noteSelfCancel(Nid.of(item.snapshot.key))
        advanceTimeBy(6_000)
        rig.engine.onRemoved(item, 2)
        assertEquals("notif.remove", rig.link.types().last())
    }

    @Test
    fun theSyncListsOnlyWhatIsActiveAllowedAndNotSkipped() = runTest {
        val rig = Rig(this, allowed)
        val active = listOf(snap("com.chat", 1), snap("com.chat", 2, ongoing = true), snap("com.other", 3), snap("com.bank", 4), snap("com.chat", 5, title = null, text = null))
        val nids = rig.engine.activeNids(active)
        assertEquals(listOf(Nid.of("0|com.chat|1|null|10100"), Nid.of("0|com.bank|4|null|10100")), nids)
        assertEquals(emptyList<String>(), Rig(this, allowed.copy(enabled = false)).engine.activeNids(active))
    }

    @Test
    fun noContentOrPackageOfANonAllowedAppIsEverLogged() = runTest {
        val rig = Rig(this, allowed)
        val secret = "TOP-SECRET-TEXT-482913"
        rig.engine.onPosted(snap("com.other", title = "SECRET-TITLE", text = secret))
        rig.engine.onPosted(snap("com.chat", title = "SECRET-TITLE", text = secret))
        rig.engine.onPosted(snap("com.chat", id = 2, title = "SECRET-TITLE", text = secret, ongoing = true))
        rig.engine.onRemoved(snap("com.chat", title = "SECRET-TITLE", text = secret), 2)
        advanceTimeBy(70_000)
        rig.engine.onPosted(snap("com.chat", id = 9))
        rig.sender.stats.maybeLog(Long.MAX_VALUE / 2, rig.log)
        val all = rig.logs.joinToString("\n") + "\n" + rig.sender.stats.line()
        for (needle in listOf("SECRET", "482913", "com.other", "0|com.chat", Nid.of("0|com.chat|1|null|10100"))) assertFalse("log has $needle:\n$all", needle in all)
        assertTrue(all.contains("mirror: sent"))
    }
}
