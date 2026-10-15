package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.clipboard.ClipboardPort
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.protocol.MAX_TEXT_BYTES
import app.kinrelay.android.protocol.Utf8
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.FakeUploadQueue
import app.kinrelay.android.transfer.PeerInfo
import app.kinrelay.android.testing.InMemoryTextStore
import app.kinrelay.android.testing.RecordingAlerts
import app.kinrelay.android.testing.str
import app.kinrelay.android.tls.KinTls
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit.MILLISECONDS
import java.util.concurrent.TimeUnit.SECONDS
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * milestone 1.2 end to end against the fake PC (real TLS 1.3, real WebSocket, MockWebServer): what the phone
 * does when the PC sends `clipboard` and `text`, and how it sends `text` and waits for the ack.
 */
class TextFlowTest {
    private val phone = Identity.generate()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pcs = mutableListOf<FakePc>()
    private val uuidV7 = Regex("[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")

    @After
    fun tearDown() {
        scope.cancel()
        pcs.forEach { it.close() }
    }

    // --- fakes for the Android parts ---

    private class FakeClipboard : ClipboardPort {
        class Write(val text: String, val sensitive: Boolean)
        val writes = LinkedBlockingQueue<Write>()
        var fail = false
        override suspend fun write(text: String, sensitive: Boolean) {
            if (fail) throw SecurityException("no clipboard")
            writes.add(Write(text, sensitive))
        }
    }

    private class Harness(val pc: FakePc, val session: LinkSession) {
        val clipboard = FakeClipboard()
        val alerts = RecordingAlerts()
        val guard = ClipboardGuard({ now })
        val arrivals = CopyOnWriteArrayList<Long>()
        @Volatile var now = 1_000_000L
        lateinit var queue: DeliveryQueue
        val store = InMemoryTextStore()
    }

    private fun pc(block: FakePc.() -> Unit = {}): FakePc = FakePc().apply { pairedPhones += phone.deviceId }.apply(block).start().also { pcs += it }

    private fun <T> blocking(block: suspend () -> T): T = runBlocking { withTimeout(15_000) { block() } }

    /** A connected session whose incoming messages go through the same classes the service uses. */
    private fun connect(pc: FakePc = pc()): Harness {
        val client: OkHttpClient = KinTls(phone, pc.identity.deviceId).httpClient().newBuilder().callTimeout(0, MILLISECONDS).build()
        lateinit var harness: Harness
        val hellos = LinkedBlockingQueue<Hello>()
        val listener = object : SessionListener {
            override fun onHelloAccepted(peer: Hello) { hellos.add(peer) }
            override fun onPingReceived(fromName: String) = Unit
            override fun onPeerUnpaired() = Unit
            override fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean) =
                harness.queue.submit { IncomingHandler(harness.clipboard, harness.alerts, harness.guard, KinLog.None).onText(fromName, id, text, isUrl, sensitive) }
            override fun onClipboardReceived(id: String, text: String) =
                harness.queue.submit { IncomingHandler(harness.clipboard, harness.alerts, harness.guard, KinLog.None).onClipboard(text) }
        }
        val session = LinkSession(
            client, PairingAddress("127.0.0.1", pc.port), LocalHello("Pixel 8", "android", "0.1.0"),
            DedupMemory(System::currentTimeMillis), scope, { System.nanoTime() / 1_000_000 }, KinLog.None, listener, pc.identity.deviceId,
        )
        harness = Harness(pc, session)
        harness.queue = DeliveryQueue(KinLog.None).also { it.start(scope) }
        session.open()
        assertNotNull("hello accepted", hellos.poll(5, SECONDS))
        pc.nextFrame() // the phone's hello
        return harness
    }

    private fun Harness.defaultLink(): OutgoingLink = object : OutgoingLink {
        override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = session.readyFor(peerDeviceId)
        override fun unreachableReason() = "not connected"
        override suspend fun peer() = PeerInfo(pc.identity.deviceId, "Desk PC")
    }

    private fun Harness.sender(
        ackTimeoutMillis: Long = OutgoingSender.ACK_TIMEOUT_MILLIS,
        link: OutgoingLink? = null,
        textFiles: TextFileSender = TextFileSender.None,
        log: KinLog = KinLog.None,
    ): OutgoingSender {
        val outbox = TextOutbox(store, log, ackTimeoutMillis = ackTimeoutMillis)
        val flusher = OutboxFlusher(outbox, FakeUploadQueue(), { pc.identity.deviceId }, alerts, log)
        return OutgoingSender(
            link = link ?: defaultLink(),
            alerts = alerts, guard = guard, arrivals = { arrivals += it }, clock = { System.nanoTime() / 1_000_000 }, log = log,
            outbox = outbox, flusher = flusher, textFiles = textFiles,
        )
    }
    private fun Harness.sendText(
        text: String,
        source: SendSource = SendSource.SHARE,
        sender: OutgoingSender = sender(),
        sensitive: Boolean = false,
    ): SendOutcome = blocking { sender.send(text, tapAtMillis = System.nanoTime() / 1_000_000, source = source, sensitive = sensitive) }

    private fun FakePc.nextOfType(type: String, timeoutMillis: Long = 5_000): JsonObject? {
        val deadline = System.currentTimeMillis() + timeoutMillis
        while (System.currentTimeMillis() < deadline) {
            val frame = nextFrame((deadline - System.currentTimeMillis()).coerceAtLeast(1)) ?: return null
            if (frame.getValue("type").str() == type) return frame
        }
        return null
    }

    // --- PC -> phone ---

    @Test
    fun clipboardFromThePcSetsTheClipboardAndIsAcked() {
        val h = connect()
        h.pc.sendClipboard("c-1", "copied on the PC")
        val write = h.clipboard.writes.poll(5, SECONDS)!!
        assertEquals("copied on the PC", write.text)
        assertTrue("the preview is hidden from the system's confirmation", write.sensitive)
        val ack = h.pc.nextOfType("ack")!!
        assertEquals("c-1", ack.getValue("ref").str())
        assertTrue("a clipboard message is not shown or stored: no notification", h.alerts.texts.isEmpty())
    }

    @Test
    fun clipboardFromThePcIsRememberedForTheLoopGuard() {
        val h = connect()
        h.pc.sendClipboard("c-2", "from the PC")
        h.clipboard.writes.poll(5, SECONDS)!!
        assertTrue(h.guard.isEcho("from the PC"))
        assertFalse(h.guard.isEcho("something else"))
    }

    @Test
    fun textFromThePcSetsTheClipboardPostsTheNotificationAndIsAcked() {
        val h = connect()
        h.pc.sendText("t-1", "hello from the PC", isUrl = false)
        val alert = h.alerts.texts.poll(5, SECONDS)!!
        assertEquals("Desk PC", alert.pc)
        assertEquals("hello from the PC", alert.text)
        assertEquals("t-1", alert.id)
        assertNull("not a link: no Open action", alert.openUrl)
        val write = h.clipboard.writes.poll(5, SECONDS)!!
        assertEquals("hello from the PC", write.text)
        assertFalse(write.sensitive)
        assertEquals("t-1", h.pc.nextOfType("ack")!!.getValue("ref").str())
    }

    @Test
    fun aLinkFromThePcGetsAnOpenAction() {
        val h = connect()
        h.pc.sendText("t-2", "https://example.com/a?b=c#d", isUrl = true)
        assertEquals("https://example.com/a?b=c#d", h.alerts.texts.poll(5, SECONDS)!!.openUrl)
    }

    @Test
    fun anUnsafeSchemeNeverGetsAnOpenActionEvenWhenIsUrlSaysTrue() {
        val h = connect()
        for ((i, text) in listOf("javascript:alert(1)", "intent://x#Intent;scheme=http;end", "file:///sdcard/x", "ftp://host/x", "kinrelay://pair?v=1").withIndex()) {
            h.pc.sendText("bad-$i", text, isUrl = true)
            val alert = h.alerts.texts.poll(5, SECONDS)!!
            assertEquals(text, alert.text)
            assertNull("no Open for $text", alert.openUrl)
        }
    }

    @Test
    fun aLinkWithIsUrlFalseGetsNoOpenActionToo() {
        val h = connect()
        h.pc.sendText("t-3", "https://example.com", isUrl = false)
        assertNull(h.alerts.texts.poll(5, SECONDS)!!.openUrl)
    }

    @Test
    fun aRepeatedIdIsAckedAgainButDeliveredOnce() {
        val h = connect()
        h.pc.sendText("same", "once", isUrl = false)
        h.pc.sendText("same", "once", isUrl = false)
        h.pc.sendText("next", "twice", isUrl = false)
        assertEquals("once", h.alerts.texts.poll(5, SECONDS)!!.text)
        assertEquals("twice", h.alerts.texts.poll(5, SECONDS)!!.text)
        val acks = generateSequence { h.pc.nextOfType("ack", 1_500) }.take(3).map { it.getValue("ref").str() }.toList()
        assertEquals(listOf("same", "same", "next"), acks)
        assertNull("no third notification", h.alerts.texts.poll(300, MILLISECONDS))
    }

    @Test
    fun clipboardMessagesArriveInOrder() {
        val h = connect()
        for (i in 1..20) h.pc.sendClipboard("o-$i", "text $i")
        val written = (1..20).map { h.clipboard.writes.poll(5, SECONDS)!!.text }
        assertEquals((1..20).map { "text $it" }, written)
    }

    @Test
    fun aFailingClipboardWriteStillNotifiesAndAcks() {
        val h = connect()
        h.clipboard.fail = true
        h.pc.sendText("t-4", "still shown", isUrl = false)
        assertEquals("still shown", h.alerts.texts.poll(5, SECONDS)!!.text)
        assertEquals("t-4", h.pc.nextOfType("ack")!!.getValue("ref").str())
        assertEquals("Couldn't copy the text from your PC", h.alerts.failureTitles.poll(5, SECONDS))
    }

    @Test
    fun aFailingClipboardWriteOfAClipboardMessageTellsTheUser() {
        val h = connect()
        h.clipboard.fail = true
        h.pc.sendClipboard("c-4", "too big for Android")
        assertEquals("c-4", h.pc.nextOfType("ack")!!.getValue("ref").str())
        assertEquals("Couldn't copy the text from your PC", h.alerts.failureTitles.poll(5, SECONDS))
    }

    @Test
    fun anInvalidTextFromThePcGetsAnErrorAndTheConnectionStaysOpen() {
        val h = connect()
        h.pc.send("""{"type":"text","id":"bad-1","text":"","isUrl":false}""")
        val error = h.pc.nextOfType("error")!!
        assertEquals("bad_request", error.getValue("code").str())
        assertEquals("bad-1", error.getValue("ref").str())
        h.pc.sendText("good", "fine", isUrl = false)
        assertEquals("fine", h.alerts.texts.poll(5, SECONDS)!!.text)
        assertTrue(h.session.isReady)
    }

    // --- phone -> PC ---

    @Test
    fun textSentByThePhoneIsAckedAndTheArrivalTimeIsRecorded() {
        val h = connect()
        val outcome = h.sendText("hello PC")
        assertTrue("got $outcome", outcome is SendOutcome.Delivered)
        val arrival = (outcome as SendOutcome.Delivered).arrivalMillis
        assertTrue("arrival $arrival ms", arrival in 0..5_000)
        assertEquals(listOf(arrival), h.arrivals.toList())
        assertEquals(listOf("Desk PC" to arrival), h.alerts.sent.toList())
        val frame = h.pc.nextOfType("text")!!
        assertEquals("hello PC", frame.getValue("text").str())
        assertEquals("false", frame.getValue("isUrl").jsonPrimitive.content)
        assertTrue(uuidV7.matches(frame.getValue("id").str()))
    }

    @Test
    fun isUrlIsTrueOnlyForOneWholeHttpUrl() {
        val h = connect()
        val cases = listOf(
            "https://example.com/a?b=c#d" to true,
            "  http://example.com  " to true,
            "see https://example.com" to false,
            "https://example.com and more" to false,
            "ftp://example.com" to false,
            "example.com" to false,
            "http://" to false,
        )
        for ((text, expected) in cases) {
            assertTrue(h.sendText(text) is SendOutcome.Delivered)
            val frame = h.pc.nextOfType("text")!!
            assertEquals(text, frame.getValue("text").str())
            assertEquals("isUrl of '$text'", expected, frame.getValue("isUrl").jsonPrimitive.content.toBoolean())
        }
    }

    @Test
    fun anUnpairedSurrogateIsReplacedBeforeSending() {
        val h = connect()
        assertTrue(h.sendText("a\uD800b") is SendOutcome.Delivered)
        assertEquals("a�b", h.pc.nextOfType("text")!!.getValue("text").str())
    }

    @Test
    fun aTextOfExactly128KiBIsAMessageAndOneByteMoreGoesAsAFile() {
        val h = connect()
        val files = CopyOnWriteArrayList<String>()
        val sender = h.sender(textFiles = TextFileSender { files += it; true })
        val exact = "a".repeat(MAX_TEXT_BYTES)
        assertTrue(h.sendText(exact, sender = sender) is SendOutcome.Delivered)
        assertEquals(exact.length, h.pc.nextOfType("text")!!.getValue("text").str().length)
        assertTrue(files.isEmpty())

        assertEquals(SendOutcome.SentAsFile, h.sendText(exact + "a", sender = sender))
        assertTrue("nothing was sent as a message for the long one", h.pc.nextOfType("text", 400) == null)
        assertEquals(listOf(exact + "a"), files.toList())
    }

    @Test
    fun theThresholdIsMeasuredInUtf8BytesOnTheWireForm() {
        val h = connect()
        val files = CopyOnWriteArrayList<String>()
        val sender = h.sender(textFiles = TextFileSender { files += it; true })
        val fits = "é".repeat(MAX_TEXT_BYTES / 2) // 2 bytes each: exactly 128 KiB
        assertTrue(h.sendText(fits, sender = sender) is SendOutcome.Delivered)
        assertEquals(SendOutcome.SentAsFile, h.sendText(fits + "é", sender = sender))
        // CRLF counts as one byte on the wire, so this text is 128 KiB on the wire although the clipboard holds more.
        val crlf = "ab\r\n".repeat(43_690) // 174,760 bytes in the clipboard, 131,070 on the wire
        assertTrue(h.sendText(crlf, sender = sender) is SendOutcome.Delivered)
        assertEquals(1, files.size)
    }

    @Test
    fun theTextGoesOutWithLfOnlyAndALoneCrStays() {
        val h = connect()
        assertTrue(h.sendText("a\r\nb\rc\r\r\nd") is SendOutcome.Delivered)
        assertEquals("a\nb\rc\r\nd", h.pc.nextOfType("text")!!.getValue("text").str())
    }

    @Test
    fun aSensitiveTextIsFlaggedOnTheWireAndAPlainOneHasNoField() {
        val h = connect()
        assertTrue(h.sendText("hunter2", sensitive = true) is SendOutcome.Delivered)
        val sensitive = h.pc.nextOfType("text")!!
        assertEquals("true", sensitive.getValue("sensitive").jsonPrimitive.content)
        assertTrue(h.sendText("plain") is SendOutcome.Delivered)
        assertNull(h.pc.nextOfType("text")!!["sensitive"])
    }

    @Test
    fun aSensitiveTextOver128KiBIsNotSentAtAll() {
        val h = connect()
        val files = CopyOnWriteArrayList<String>()
        val sender = h.sender(textFiles = TextFileSender { files += it; true })
        assertEquals(SendOutcome.TooLong, h.sendText("a".repeat(MAX_TEXT_BYTES + 1), sender = sender, sensitive = true))
        assertTrue("a file has no sensitive mark, so no file either", files.isEmpty())
        assertTrue(h.pc.nextOfType("text", 400) == null)
        assertEquals(listOf("That text is sensitive and too long to send."), h.alerts.messages.toList())
    }

    @Test
    fun aLongTextWithNoFileQueueIsTooLong() {
        val h = connect()
        assertEquals(SendOutcome.TooLong, h.sendText("a".repeat(MAX_TEXT_BYTES + 1)))
        assertEquals(listOf("That text is too long to send."), h.alerts.messages.toList())
    }

    @Test
    fun nothingAboutASensitiveTextIsLogged() {
        val h = connect()
        val lines = CopyOnWriteArrayList<String>()
        val log = object : KinLog {
            override fun log(level: Char, component: String, message: String) { lines += message }
        }
        val secret = "correct horse battery staple"
        assertTrue(h.sendText(secret, sender = h.sender(log = log), sensitive = true) is SendOutcome.Delivered)
        val hash = ClipboardGuard.hash(secret).take(8)
        assertTrue(lines.joinToString("\n"), lines.none { it.contains(secret) || it.contains(hash) || it.contains("len=${secret.length}") || it.contains("${secret.length} bytes") })
        assertTrue("the sender logged something, so the check above means something", lines.any { it.contains("sensitive text sent") })
        // A sensitive text never reaches the database (it can't be queued, so nothing of it is stored).
        assertTrue(h.store.items.isEmpty())
    }

    @Test
    fun aSensitiveTextWithNoConnectionIsNotQueuedAndTheUserIsTold() {
        val h = connect()
        val none = object : OutgoingLink {
            override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = null
            override fun unreachableReason() = "This phone isn't on Wi-Fi."
            override suspend fun peer() = PeerInfo(h.pc.identity.deviceId, "Desk PC")
        }
        val outcome = h.sendText("hunter2", sender = h.sender(link = none), sensitive = true)
        assertEquals(SendOutcome.NotSent("This phone isn't on Wi-Fi."), outcome)
        assertTrue("nothing is stored, not even for a moment", h.store.items.isEmpty())
        assertTrue("and it isn't reported as queued", h.alerts.queued.isEmpty())
        assertEquals("the user is told, in a notification", 1, h.alerts.failures.size)
        assertTrue(h.alerts.failures.single().first.startsWith("Not sent: Desk PC offline"))
        assertTrue(h.pc.nextOfType("text", 400) == null)
        assertTrue(h.arrivals.isEmpty())
    }

    @Test
    fun aSensitiveTextThatGetsNoAckIsNotQueuedEither() {
        val h = connect(pc { ackTexts = false })
        val outcome = h.sendText("hunter2", sender = h.sender(ackTimeoutMillis = 300), sensitive = true)
        assertTrue(outcome is SendOutcome.NotSent)
        assertTrue("not stored for a later try", h.store.items.isEmpty())
        assertTrue(h.alerts.queued.isEmpty())
        assertEquals(1, h.alerts.failures.size)
        assertTrue(h.arrivals.isEmpty())
    }

    @Test
    fun noAckWithinTheTimeoutRequeuesTheTextAndTheConnectionIsTreatedAsDead() {
        val h = connect(pc { ackTexts = false })
        val outcome = h.sendText("nobody answers", sender = h.sender(ackTimeoutMillis = 300))
        assertEquals(SendOutcome.Queued("No reply from Desk PC yet.", afterNoReply = true), outcome)
        assertTrue(h.arrivals.isEmpty())
        assertEquals("not a failure: no 'failed' notification", 0, h.alerts.failures.size)
        assertEquals(listOf(Triple<String?, String, Boolean>("Desk PC", "No reply from Desk PC yet.", true)), h.alerts.queued.toList())
        // Back in the queue, not failed, and it keeps its ID for the next try.
        val item = h.store.items.values.single()
        assertEquals("queued", item.status)
        assertEquals(1, item.attempts)
        blocking { h.session.awaitEnd() } // the socket was cancelled, so the session ends
        assertFalse(h.session.isReady)
    }

    @Test
    fun theDefaultAckTimeoutIsFiveSeconds() {
        assertEquals(5_000L, OutgoingSender.ACK_TIMEOUT_MILLIS)
        val h = connect(pc { ackTexts = false })
        val started = System.nanoTime()
        assertTrue(h.sendText("waiting for the full timeout") is SendOutcome.Queued)
        val waitedMillis = (System.nanoTime() - started) / 1_000_000
        assertTrue("waited $waitedMillis ms", waitedMillis in 4_900..7_000)
    }

    @Test
    fun anErrorFromThePcIsRefusedAndNotRetried() {
        val h = connect(pc { errorForText = "internal" })
        assertEquals(SendOutcome.Refused("internal"), h.sendText("rejected"))
        assertTrue(h.arrivals.isEmpty())
        assertEquals(1, h.alerts.failures.size)
        assertTrue(h.pc.nextOfType("text") != null)
        assertTrue("sent once", h.pc.nextOfType("text", 400) == null)
    }

    @Test
    fun withNoConnectionTheTextIsSavedInTheQueueAndNothingIsSent() {
        val h = connect()
        val none = object : OutgoingLink {
            override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = null
            override fun unreachableReason() = "This phone isn't on Wi-Fi."
            override suspend fun peer() = PeerInfo(h.pc.identity.deviceId, "Desk PC")
        }
        val outcome = h.sendText("lost", sender = h.sender(link = none))
        assertEquals(SendOutcome.Queued("This phone isn't on Wi-Fi.", afterNoReply = false), outcome)
        assertEquals(listOf(Triple<String?, String, Boolean>("Desk PC", "This phone isn't on Wi-Fi.", false)), h.alerts.queued.toList())
        assertEquals("a queued text is not a failure", 0, h.alerts.failures.size)
        val item = h.store.items.values.single()
        assertEquals("queued", item.status)
        assertEquals("lost", item.body)
        assertEquals(0, item.attempts)
        assertTrue(h.pc.nextOfType("text", 400) == null)
        assertTrue(h.arrivals.isEmpty())
    }

    @Test
    fun aSessionThatIsGoneIsNoConnection() {
        val h = connect()
        h.session.closeNormally("test")
        blocking { h.session.awaitEnd() }
        val result = blocking { h.session.sendAcked("x", """{"type":"text","id":"x","text":"t","isUrl":false}""") }
        assertEquals(AckResult.NoConnection, result)
    }

    @Test
    fun blankTextIsNotSent() {
        val h = connect()
        assertEquals(SendOutcome.Empty, h.sendText("  \n\t "))
        assertEquals(listOf("Nothing to send."), h.alerts.messages.toList())
        assertTrue(h.pc.nextOfType("text", 300) == null)
    }

    // --- loop prevention ---

    @Test
    fun theClipboardThePcJustSetIsNotSentBack() {
        val h = connect()
        h.pc.sendClipboard("g-1", "set by the PC")
        h.clipboard.writes.poll(5, SECONDS)!!
        assertEquals(SendOutcome.Echo, h.sendText("set by the PC", SendSource.CLIPBOARD))
        assertTrue(h.pc.nextOfType("text", 300) == null)
        assertEquals(listOf("The PC already has this text."), h.alerts.messages.toList())
    }

    @Test
    fun theGuardAppliesToTheClipboardPathsOnly() {
        val h = connect()
        h.pc.sendClipboard("g-2", "same text")
        h.clipboard.writes.poll(5, SECONDS)!!
        // The user selected the text and chose "Send to PC": that is an explicit act, not a possible loop.
        assertTrue(h.sendText("same text", SendSource.PROCESS_TEXT) is SendOutcome.Delivered)
        assertTrue(h.sendText("same text", SendSource.SHARE) is SendOutcome.Delivered)
    }

    @Test
    fun theGuardForgetsAfterAWhile() {
        val h = connect()
        h.pc.sendClipboard("g-3", "old text")
        h.clipboard.writes.poll(5, SECONDS)!!
        assertEquals(SendOutcome.Echo, h.sendText("old text", SendSource.CLIPBOARD))
        h.now += ClipboardGuard.WINDOW_MILLIS + 1
        assertTrue(h.sendText("old text", SendSource.CLIPBOARD) is SendOutcome.Delivered)
    }

    @Test
    fun differentClipboardTextIsSentNormally() {
        val h = connect()
        h.pc.sendClipboard("g-4", "from the PC")
        h.clipboard.writes.poll(5, SECONDS)!!
        assertTrue(h.sendText("typed on the phone", SendSource.CLIPBOARD) is SendOutcome.Delivered)
    }

    @Test
    fun theEchoPathLogsTheSkipWithNoFingerprintForASensitiveTextAndAKeyedFingerprintForAnOrdinaryOne() {
        app.kinrelay.android.log.LogFingerprint.install(ByteArray(32) { (it + 3).toByte() })
        try {
            val h = connect()
            val lines = CopyOnWriteArrayList<String>()
            val log = object : KinLog {
                override fun log(level: Char, component: String, message: String) { lines += message }
            }
            val secret = "1234 5678"
            h.guard.notePeerSet(secret)
            assertEquals(SendOutcome.Echo, h.sendText(secret, SendSource.CLIPBOARD, sender = h.sender(log = log), sensitive = true))
            val skip = lines.single()
            assertTrue("the skip is logged, so the e2e can see the guard work: $skip", skip.startsWith("send skipped (CLIPBOARD): the text is what the PC just set"))
            assertFalse("no fingerprint, no length, no text for a sensitive one: $skip", skip.contains("len=") || skip.contains("sha=") || skip.contains(secret) || skip.contains(ClipboardGuard.hash(secret).take(8)))
            lines.clear()

            val ordinary = "not a secret, just text"
            h.guard.notePeerSet(ordinary)
            assertEquals(SendOutcome.Echo, h.sendText(ordinary, SendSource.CLIPBOARD, sender = h.sender(log = log)))
            val line = lines.single()
            assertTrue(line, line.contains("len=${ordinary.length} sha="))
            assertFalse("the hash is keyed, not the SHA-256 of the text", line.contains(ClipboardGuard.hash(ordinary).take(8)))
            assertFalse(line.contains(ordinary))
        } finally {
            app.kinrelay.android.log.LogFingerprint.install(null)
        }
    }
}
