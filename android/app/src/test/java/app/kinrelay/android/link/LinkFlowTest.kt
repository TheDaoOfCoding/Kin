package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.clipboard.ClipboardPort
import app.kinrelay.android.data.TextKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.PROTOCOL_VERSION
import app.kinrelay.android.share.ShareIntents
import app.kinrelay.android.testing.InMemoryTextStore
import app.kinrelay.android.testing.FakeUploadQueue
import app.kinrelay.android.testing.RecordingAlerts
import app.kinrelay.android.testing.str
import app.kinrelay.android.transfer.PeerInfo
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 2B.1 on the phone, without a network: a URL shared to Kin goes out as a `link` (queued first, in order, with its title), falls back to a `text`
 * for a PC that is an older version (decided when it goes on the wire), and a link from the PC is shown and recorded, never copied or opened by
 * itself. The text setting "Copy received text automatically" and the Copy button's text are covered here too.
 */
class LinkFlowTest {
    private val pc = PeerInfo("pc-device-id", "Desk PC")

    /** A connection that records what is sent and acks it. [understandsLink] is what the PC's latest `hello` said. */
    private class FakeConnection(var understandsLink: Boolean, private val deviceId: String = "pc-device-id") : LinkConnection {
        val frames = mutableListOf<String>()
        override val isReady = true
        override val peerDeviceId = deviceId
        override val peerUnderstandsLink get() = understandsLink
        override fun open() = Unit
        override suspend fun awaitEnd(): SessionEnd = error("not used")
        override fun closeNormally(reason: String) = Unit
        override suspend fun ping(timeoutMillis: Long): PingResult = error("not used")
        override suspend fun sendAcked(id: String, frame: String, timeoutMillis: Long): AckResult {
            frames += frame
            return AckResult.Acked
        }
        override suspend fun sendUnpairAndClose() = Unit
    }

    private class Rig(val pc: PeerInfo, var connection: FakeConnection?) {
        val store = InMemoryTextStore()
        val alerts = RecordingAlerts()
        val outbox = TextOutbox(store, KinLog.None)
        val flusher = OutboxFlusher(outbox, FakeUploadQueue(), { pc.deviceId }, alerts, KinLog.None)
        val sender = OutgoingSender(
            link = object : OutgoingLink {
                override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = connection
                override fun unreachableReason() = "not connected"
                override suspend fun peer() = pc
            },
            alerts = alerts, guard = ClipboardGuard({ 0L }), arrivals = { }, clock = { 0L }, log = KinLog.None, outbox = outbox, flusher = flusher,
        )
    }

    private fun rig(understandsLink: Boolean? = true) = Rig(pc, understandsLink?.let { FakeConnection(it) })

    private fun Rig.share(url: String, title: String? = null) = runBlocking { sender.sendLink(url, title, 0L, SendSource.SHARE_LINK) }

    private fun frame(text: String) = Json.parseToJsonElement(text).jsonObject

    // --- phone to PC ---

    @Test
    fun aSharedUrlGoesAsALinkWithItsTitleToAPcThatListsTheCapability() {
        val rig = rig(understandsLink = true)
        val outcome = rig.share("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s", "A video \u2013 YouTube")
        assertTrue(outcome is SendOutcome.Delivered)
        val sent = frame(rig.connection!!.frames.single())
        assertEquals("link", sent.getValue("type").str())
        assertEquals("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s", sent.getValue("url").str())
        assertEquals("A video \u2013 YouTube", sent.getValue("title").str())
        val row = rig.store.items.values.single()
        assertEquals(TransferStatus.DONE, row.status)
        assertEquals(TextKind.LINK, row.kind)
        assertEquals(sent.getValue("id").str(), row.id)
    }

    @Test
    fun toAnOlderPcTheSameItemGoesAsATextWithTheSameIdAndTheBareUrl() {
        val rig = rig(understandsLink = false)
        assertTrue(rig.share("https://example.com/a?b=c#d", "A title") is SendOutcome.Delivered)
        val sent = frame(rig.connection!!.frames.single())
        assertEquals("text", sent.getValue("type").str())
        assertEquals("https://example.com/a?b=c#d", sent.getValue("text").str())
        assertEquals("true", sent.getValue("isUrl").str())
        assertFalse(sent.containsKey("title"))
        assertFalse(sent.containsKey("sourceApp"))
        val row = rig.store.items.values.single()
        assertEquals("the same id", row.id, sent.getValue("id").str())
        assertEquals("it stays a link in this phone's own history, with its title", TextKind.LINK, row.kind)
        assertEquals("A title", row.title)
    }

    @Test
    fun aLinkIsQueuedWhileThePcIsAwayAndGoesOutInOrderWhenItIsBack() {
        val rig = rig(understandsLink = null)
        val outcome = rig.share("https://example.com/first", "First")
        assertTrue("saved, not lost", outcome is SendOutcome.Queued)
        Thread.sleep(5) // the fake store orders by creation millisecond, then by random id: two shares in one millisecond would be a coin flip
        rig.share("https://example.com/second", null)
        assertEquals(2, rig.store.items.values.count { it.status == TransferStatus.QUEUED })
        assertTrue(rig.alerts.queued.isNotEmpty())

        val back = FakeConnection(understandsLink = true)
        runBlocking { rig.flusher.flush(back) }
        assertEquals(listOf("https://example.com/first", "https://example.com/second"), back.frames.map { frame(it).getValue("url").str() })
        assertTrue(back.frames.all { frame(it).getValue("type").str() == "link" })
        assertTrue(rig.store.items.values.all { it.status == TransferStatus.DONE })
    }

    @Test
    fun theChoiceBetweenLinkAndTextIsMadeWhenTheMessageGoesOnTheWire() {
        val rig = rig(understandsLink = null)
        rig.share("https://example.com/queued-for-an-old-pc", "Title")
        // The PC updated while the link waited: it goes out as a link now.
        val updated = FakeConnection(understandsLink = true)
        runBlocking { rig.flusher.flush(updated) }
        assertEquals("link", frame(updated.frames.single()).getValue("type").str())

        val rig2 = rig(understandsLink = null)
        rig2.share("https://example.com/for-a-pc-that-lost-it", "Title")
        val old = FakeConnection(understandsLink = false)
        runBlocking { rig2.flusher.flush(old) }
        assertEquals("text", frame(old.frames.single()).getValue("type").str())
    }

    @Test
    fun aTextThatIsNotAWebLinkIsNeitherSentNorQueued() {
        val rig = rig()
        for (bad in listOf("javascript:alert(1)", "file:///sdcard/x", "intent://x#Intent;end", "not a link", "")) {
            assertEquals(SendOutcome.NotAWebLink, rig.share(bad))
        }
        assertTrue(rig.store.items.isEmpty())
        assertTrue(rig.connection!!.frames.isEmpty())
        assertEquals("That isn't a web link.", rig.alerts.messages.first())
    }

    @Test
    fun theSharedUrlIsTrimmedBeforeItIsCheckedAndATitleThatIsJustTheUrlIsDropped() {
        val rig = rig()
        assertTrue(rig.share("  https://example.com/page \n", "https://example.com/page") is SendOutcome.Delivered)
        val sent = frame(rig.connection!!.frames.single())
        assertEquals("https://example.com/page", sent.getValue("url").str())
        assertFalse(sent.containsKey("title"))
    }

    // --- the share intent ---

    @Test
    fun aShareOfExactlyOneWebLinkBecomesALink() {
        assertEquals("https://example.com/a", ShareIntents.linkOf("https://example.com/a", sensitive = false))
        assertEquals("trimmed", "https://example.com/a", ShareIntents.linkOf("\n https://example.com/a \n", sensitive = false))
        assertEquals("https://youtu.be/abc?t=83", ShareIntents.linkOf("https://youtu.be/abc?t=83", sensitive = false))
    }

    @Test
    fun anythingElseStaysAText() {
        assertNull("a sentence with a link in it", ShareIntents.linkOf("look at https://example.com/a now", sensitive = false))
        assertNull("two links", ShareIntents.linkOf("https://a.example/ https://b.example/", sensitive = false))
        assertNull(ShareIntents.linkOf("javascript:alert(1)", sensitive = false))
        assertNull(ShareIntents.linkOf("ftp://example.com/", sensitive = false))
        assertNull(ShareIntents.linkOf(null, sensitive = false))
        assertNull("a sensitive text is never a link", ShareIntents.linkOf("https://example.com/reset?t=secret", sensitive = true))
    }

    @Test
    fun aSharedUrlThatFailsTheTightenedRuleIsNotALinkAndGoesAsAPlainText() {
        // B1 (changelog 55): user information, a backslash or an escape in the authority. The share is not a `link` and its text has `isUrl` false, so
        // nobody is offered an Open action for it; the text itself reaches the PC unchanged.
        for (bad in listOf("https://evil.example\\@google.com", "https://google.com@evil.example/", "https://user:pw@example.com:8443/x", "https://goog%6Ce.com/")) {
            assertNull("not a link: $bad", ShareIntents.linkOf(bad, sensitive = false))
            val rig = rig()
            assertTrue(rig.sender.run { runBlocking { send(bad, 0L, SendSource.SHARE) } } is SendOutcome.Delivered)
            val sent = frame(rig.connection!!.frames.single())
            assertEquals(bad, "text", sent.getValue("type").str())
            assertEquals(bad, sent.getValue("text").str())
            assertEquals("isUrl is false: $bad", "false", sent.getValue("isUrl").toString())
        }
    }

    @Test
    fun theTitleIsTheSubjectOfTheShareThenItsTitle() {
        assertEquals("Subject", ShareIntents.titleOf("Subject", "Title"))
        assertEquals("Title", ShareIntents.titleOf(null, "Title"))
        assertEquals("Title", ShareIntents.titleOf("  ", "Title"))
        assertNull(ShareIntents.titleOf(null, null))
        assertNull(ShareIntents.titleOf("", ""))
    }

    // --- PC to phone ---

    private class Clipboard : ClipboardPort {
        val writes = mutableListOf<Pair<String, Boolean>>()
        override suspend fun write(text: String, sensitive: Boolean) { writes += text to sensitive }
    }

    private fun handler(clipboard: Clipboard, alerts: RecordingAlerts, log: KinLog = KinLog.None, copyAutomatically: Boolean = true, lockScreen: Boolean = false) =
        IncomingHandler(clipboard, alerts, ClipboardGuard({ 0L }), log, copyAutomatically = { copyAutomatically }, lockScreenLinks = { lockScreen })

    @Test
    fun aLinkFromThePcIsShownWithItsRealAddressAndIsNeverCopiedOrOpened() {
        val clipboard = Clipboard()
        val alerts = RecordingAlerts()
        val logged = mutableListOf<String>()
        val log = object : KinLog { override fun log(level: Char, component: String, message: String) { logged += message } }
        val shown = runBlocking { handler(clipboard, alerts, log).onLink("Desk PC", "l1", "https://evil.example/login?token=SECRET", "https://accounts.google.com Sign in", "Chrome") }
        assertNotNull(shown)
        val alert = alerts.links.poll()!!
        assertEquals("Desk PC", alert.pc)
        assertEquals("evil.example", alert.link.domain)
        assertEquals("https://evil.example/login?token=SECRET", alert.link.url)
        assertFalse("the lock screen preview is off by default", alert.lockScreenPreviews)
        assertTrue("a link never goes on the clipboard by itself", clipboard.writes.isEmpty())
        assertTrue("and no text notification is made", alerts.texts.isEmpty())
        assertTrue("the URL (which may carry a token) is not logged", logged.none { it.contains("SECRET") || it.contains("login") })
    }

    @Test
    fun theLockScreenSettingReachesTheNotification() {
        val alerts = RecordingAlerts()
        runBlocking { handler(Clipboard(), alerts, lockScreen = true).onLink("Desk PC", "l1", "https://example.com/", null, null) }
        assertTrue(alerts.links.poll()!!.lockScreenPreviews)
    }

    @Test
    fun aLinkThatIsNotAWebLinkIsRefusedAndShowsNothing() {
        val alerts = RecordingAlerts()
        for (bad in listOf("javascript:alert(1)", "file:///C:/x", " https://example.com/", "intent://x#Intent;end")) {
            assertNull(runBlocking { handler(Clipboard(), alerts).onLink("Desk PC", "l1", bad, "t", null) })
        }
        assertTrue(alerts.links.isEmpty())
    }

    @Test
    fun aTextFromThePcIsCopiedAutomaticallyByDefaultAndTheNotificationStillHasItsCopyText() {
        val clipboard = Clipboard()
        val alerts = RecordingAlerts()
        runBlocking { handler(clipboard, alerts).onText("Desk PC", "t1", "hello", isUrl = false) }
        assertEquals(listOf("hello" to false), clipboard.writes)
        val alert = alerts.texts.poll()!!
        assertTrue("it says the text is on the clipboard already", alert.copiedAlready)
        assertEquals("the Copy button's text", "hello", alert.copyText)
    }

    @Test
    fun withTheSettingOffATextIsNotCopiedAndTheCopyButtonDoesIt() {
        val clipboard = Clipboard()
        val alerts = RecordingAlerts()
        val guard = ClipboardGuard({ 0L })
        val handler = IncomingHandler(clipboard, alerts, guard, KinLog.None, copyAutomatically = { false })
        runBlocking { handler.onText("Desk PC", "t1", "keep my clipboard", isUrl = false) }
        assertTrue("the clipboard is left alone", clipboard.writes.isEmpty())
        val alert = alerts.texts.poll()!!
        assertFalse(alert.copiedAlready)
        assertEquals("keep my clipboard", alert.copyText)
        assertFalse("nothing was put on the clipboard, so the loop guard has nothing to skip", guard.isEcho("keep my clipboard"))
        assertTrue("no 'couldn't copy' notice for a copy that was not asked for", alerts.failures.isEmpty())
    }

    @Test
    fun aSensitiveTextKeepsItsPrivacyEvenWithTheCopyButton() {
        val alerts = RecordingAlerts()
        val clipboard = Clipboard()
        runBlocking { handler(clipboard, alerts, copyAutomatically = false).onText("Desk PC", "t1", "s3cret", isUrl = false, sensitive = true) }
        val alert = alerts.texts.poll()!!
        assertNull("no preview", alert.text)
        assertTrue(alert.sensitive)
        assertEquals("s3cret", alert.copyText)
        assertTrue(clipboard.writes.isEmpty())
    }

    @Test
    fun theAppSaysItUnderstandsLinksInItsHello() {
        assertTrue(app.kinrelay.android.protocol.Frames.hello("h", "Pixel", "android", "0.1.1").contains("\"capabilities\":[\"link\",\"notif\",\"media\",\"presence\",\"alert\"]"))
        // `ring` is listed only while "Let my PC ring this phone" is on (2B.4); `alert` always (2B.5).
        assertTrue(app.kinrelay.android.protocol.Frames.hello("h", "Pixel", "android", "0.1.1", app.kinrelay.android.protocol.Frames.withRing(true)).contains("\"presence\",\"alert\",\"ring\"]"))
        assertEquals(1, PROTOCOL_VERSION)
    }
}
