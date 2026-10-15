package app.kinrelay.android.link

import kotlinx.coroutines.launch
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.awaitCancellation
import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.clipboard.ClipboardPort
import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.data.TextSource
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.FakeUploadQueue
import app.kinrelay.android.testing.InMemoryTextStore
import app.kinrelay.android.testing.RecordingAlerts
import app.kinrelay.android.testing.str
import app.kinrelay.android.tls.KinTls
import app.kinrelay.android.transfer.PeerInfo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/** The queue of milestone 1.4: persisted first, flushed in creation order after the hello, re-queued on a missing ack, expiring, cancelable. */
class QueueTest {
    private val phone = Identity.generate()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pcs = mutableListOf<FakePc>()
    private val now = AtomicLong(10_000_000)
    private val ids = AtomicInteger()

    @After
    fun tearDown() {
        scope.cancel()
        pcs.forEach { it.close() }
    }

    private fun pc(block: FakePc.() -> Unit = {}): FakePc = FakePc().apply { pairedPhones += phone.deviceId }.apply(block).start().also { pcs += it }

    private fun <T> blocking(block: suspend () -> T): T = runBlocking { withTimeout(20_000) { block() } }

    private fun connect(pc: FakePc): LinkSession {
        val client = KinTls(phone, pc.identity.deviceId).httpClient().newBuilder().callTimeout(0, TimeUnit.MILLISECONDS).build()
        val hellos = LinkedBlockingQueue<Hello>()
        val listener = object : SessionListener {
            override fun onHelloAccepted(peer: Hello) { hellos.add(peer) }
            override fun onPingReceived(fromName: String) = Unit
            override fun onPeerUnpaired() = Unit
            override fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean) = Unit
            override fun onClipboardReceived(id: String, text: String) = Unit
        }
        val session = LinkSession(
            client, PairingAddress("127.0.0.1", pc.port), LocalHello("Pixel 8", "android", "0.1.0"),
            DedupMemory(System::currentTimeMillis), scope, { System.nanoTime() / 1_000_000 }, KinLog.None, listener, pc.identity.deviceId,
        )
        session.open()
        assertTrue("hello accepted", hellos.poll(5, TimeUnit.SECONDS) != null)
        pc.nextFrame() // the phone's hello
        return session
    }

    private class Parts(
        val store: InMemoryTextStore,
        val outbox: TextOutbox,
        val uploads: FakeUploadQueue,
        val alerts: RecordingAlerts,
        val flusher: OutboxFlusher,
    )

    private fun parts(pcId: String, store: InMemoryTextStore = InMemoryTextStore(), ackMillis: Long = 400, paired: String = pcId): Parts {
        val outbox = TextOutbox(store, KinLog.None, nowMillis = now::get, newId = { "m-${ids.incrementAndGet()}" }, ackTimeoutMillis = ackMillis)
        val uploads = FakeUploadQueue()
        val alerts = RecordingAlerts()
        return Parts(store, outbox, uploads, alerts, OutboxFlusher(outbox, uploads, { paired }, alerts, KinLog.None))
    }

    private fun Parts.enqueue(text: String, peer: PeerInfo, sensitive: Boolean = false, source: String = TextSource.SHARE) =
        blocking { outbox.enqueue(text, isUrl = false, sensitive = sensitive, source = source, peer = peer) }

    private fun FakePc.textFrames(count: Int): List<String> = (1..count).map { frameOfType("text").getValue("id").str() }

    private fun FakePc.frameOfType(type: String, timeoutMillis: Long = 5_000): kotlinx.serialization.json.JsonObject {
        val deadline = System.currentTimeMillis() + timeoutMillis
        while (System.currentTimeMillis() < deadline) {
            val f = nextFrame(200) ?: continue
            if (f.getValue("type").str() == type) return f
        }
        error("no $type frame")
    }

    private fun FakePc.noTextWithin(millis: Long) = run {
        val deadline = System.currentTimeMillis() + millis
        var seen = false
        while (System.currentTimeMillis() < deadline) {
            val f = nextFrame(50) ?: continue
            if (f.getValue("type").str() == "text") seen = true
        }
        !seen
    }

    // ---------------------------------------------------------------- persisted first

    @Test
    fun aTextIsInTheStoreAsQueuedBeforeTheFirstAttemptReachesThePc() {
        val pc = pc()
        val p = parts(pc.identity.deviceId)
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val sawInStore = CopyOnWriteArrayList<String?>()
        pc.onFrame = { _, frame -> if (frame["type"]?.str() == "text") sawInStore += p.store.items[frame.getValue("id").str()]?.status }
        val session = connect(pc)
        val item = p.enqueue("hello", peer)
        assertEquals("queued", p.store.items.getValue(item.id).status)

        blocking { p.flusher.flush(session) }

        assertEquals("the PC saw the frame while the row existed (active: being sent)", listOf("active"), sawInStore.toList())
        assertEquals("done", p.store.items.getValue(item.id).status)
        assertEquals("hello", p.store.items.getValue(item.id).body)
    }

    @Test
    fun theQueueSurvivesAProcessKill() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val store = InMemoryTextStore() // what Room keeps on disk
        val before = parts(pc.identity.deviceId, store)
        val a = before.enqueue("first", peer)
        now.addAndGet(10)
        val b = before.enqueue("second", peer)
        // The process dies while the first one is being sent: its row says `active`.
        blocking { store.claim(a.id) }
        assertEquals("active", store.items.getValue(a.id).status)

        // A new process: new objects over the same database.
        val after = parts(pc.identity.deviceId, store)
        blocking { after.outbox.recoverAtStart() }
        assertEquals("queued", store.items.getValue(a.id).status)
        val session = connect(pc)
        val report = blocking { after.flusher.flush(session) }

        assertEquals(2, report.texts)
        assertEquals("sent in creation order", listOf(a.id, b.id), pc.textFrames(2))
        assertEquals(listOf("done", "done"), listOf(a.id, b.id).map { store.items.getValue(it).status })
    }

    // ---------------------------------------------------------------- order

    @Test
    fun theQueueGoesOutInCreationOrderTextsAndUploadsTogetherAfterTheHello() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val events = CopyOnWriteArrayList<String>()
        pc.onFrame = { _, frame -> if (frame["type"]?.str() == "text") events += "text:${frame.getValue("id").str()}" }
        p.uploads.onStart = { events += "upload:$it" }
        val t1 = p.enqueue("one", peer)
        now.addAndGet(10)
        p.uploads.waiting += QueuedUpload("up-1", now.get(), pc.identity.deviceId)
        now.addAndGet(10)
        val t2 = p.enqueue("two", peer)
        now.addAndGet(10)
        p.uploads.waiting += QueuedUpload("up-2", now.get(), pc.identity.deviceId)
        now.addAndGet(10)
        val t3 = p.enqueue("three", peer)

        val session = connect(pc) // the hello is accepted before anything goes out
        val report = blocking { p.flusher.flush(session) }

        assertEquals(3, report.texts)
        assertEquals(2, report.uploads)
        assertEquals(listOf("text:${t1.id}", "upload:up-1", "text:${t2.id}", "upload:up-2", "text:${t3.id}"), events.toList())
    }

    @Test
    fun anUploadIsStartedNotWaitedForSoABigFileDoesNotHoldBackTheTextsAfterIt() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        p.uploads.waiting += QueuedUpload("big", now.get() - 100, pc.identity.deviceId)
        p.enqueue("after the file", peer)
        val session = connect(pc)
        val started = System.nanoTime()
        blocking { p.flusher.flush(session) }
        assertTrue("the text did not wait for a file", (System.nanoTime() - started) / 1_000_000 < 3_000)
        assertEquals(listOf("big"), p.uploads.started.toList())
        pc.frameOfType("text")
    }

    @Test
    fun aNewSendWhileConnectedGoesOutBehindWhatWasQueuedEarlier() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val old = p.enqueue("queued earlier", peer)
        now.addAndGet(50)
        val fresh = p.enqueue("sent just now", peer)
        val session = connect(pc)
        blocking { p.flusher.flush(session) } // what a send does after queueing: it flushes the whole queue
        assertEquals(listOf(old.id, fresh.id), pc.textFrames(2))
    }

    // ---------------------------------------------------------------- ack timeout re-queues

    @Test
    fun noAckWithinTheTimeoutRequeuesTheItemAndTheRestWaitThenItGoesOutAgainWithTheSameId() {
        val first = pc { ackTexts = false }
        val peer = PeerInfo(first.identity.deviceId, "Desk PC")
        val p = parts(first.identity.deviceId, ackMillis = 300)
        val a = p.enqueue("never acked", peer)
        now.addAndGet(10)
        val b = p.enqueue("behind it", peer)
        p.outbox.markReportedQueued(a.id)
        val dead = connect(first)

        val report = blocking { p.flusher.flush(dead) }

        assertTrue("the flush stopped: the connection is treated as dead", report.stoppedEarly)
        assertEquals("not failed", "queued", p.store.items.getValue(a.id).status)
        assertEquals(1, p.store.items.getValue(a.id).attempts)
        assertEquals("the one behind it was not even tried", 0, p.store.items.getValue(b.id).attempts)
        assertEquals(a.id, first.frameOfType("text").getValue("id").str())

        // The next hello (the PC acks now): the same item goes out again with the same ID, then the next one.
        first.ackTexts = true
        val again = connect(first)
        val second = blocking { p.flusher.flush(again) }
        assertEquals(2, second.texts)
        assertEquals(listOf(a.id, b.id), first.textFrames(2))
        assertEquals("done", p.store.items.getValue(a.id).status)
        assertEquals("the user is told the queued one went out", listOf("Desk PC"), p.alerts.sentLater.toList())
    }

    @Test
    fun anErrorFromThePcFailsTheItemAndItIsNotSentAgain() {
        val pc = pc { errorForText = "internal" }
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val a = p.enqueue("bad", peer)
        val session = connect(pc)
        blocking { p.flusher.flush(session) }
        blocking { p.flusher.flush(session) }
        assertEquals("failed", p.store.items.getValue(a.id).status)
        assertEquals("internal", p.store.items.getValue(a.id).errorCode)
        pc.frameOfType("text")
        assertTrue("sent once", pc.noTextWithin(300))
    }

    // ---------------------------------------------------------------- expiry (fake clock)

    @Test
    fun aQueuedTextExpiresAfterSevenDaysAndNeverGoesOut() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val created = now.get()
        val old = p.enqueue("old", peer)
        val session = connect(pc)

        now.set(created + QueuePolicy.EXPIRY_MILLIS - 1) // one millisecond short of 7 days: still good
        now.addAndGet(0)
        val fresh = p.enqueue("fresh", peer) // created now, so far from expiring
        val early = blocking { p.flusher.flush(session) }
        assertEquals("not yet expired: both went out", 2, early.texts)
        assertEquals(0, early.expired)

        val stale = p.enqueue("stale", peer)
        now.set(stale.createdAtMillis + QueuePolicy.EXPIRY_MILLIS) // exactly 7 days: expired
        val late = blocking { p.flusher.flush(session) }
        assertEquals(1, late.expired)
        assertEquals(0, late.texts)
        val expired = p.store.items.getValue(stale.id)
        assertEquals("failed", expired.status)
        assertEquals("expired", expired.errorCode)
        assertEquals(listOf(QueuePolicy.EXPIRY_MILLIS), p.uploads.expiredCalls.takeLast(1))
        assertEquals(listOf(old.id, fresh.id), pc.textFrames(2))
        assertTrue("the expired one never reached the PC", pc.noTextWithin(300))
    }

    @Test
    fun theExpiryEdgeIsExactlySevenDays() {
        assertFalse(QueuePolicy.isExpired(1_000, 1_000 + QueuePolicy.EXPIRY_MILLIS - 1))
        assertTrue(QueuePolicy.isExpired(1_000, 1_000 + QueuePolicy.EXPIRY_MILLIS))
        assertEquals(7L * 24 * 3_600_000, QueuePolicy.EXPIRY_MILLIS)
    }

    @Test
    fun expiryDoesNotNeedAConnection() {
        val pc = pc()
        val p = parts(pc.identity.deviceId)
        val a = p.enqueue("waiting", PeerInfo(pc.identity.deviceId, "Desk PC"))
        now.addAndGet(QueuePolicy.EXPIRY_MILLIS)
        assertEquals(1, blocking { p.flusher.expire() })
        assertEquals("expired", p.store.items.getValue(a.id).errorCode)
    }

    // ---------------------------------------------------------------- cancel

    @Test
    fun aCanceledQueuedItemNeverGoesOut() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val a = p.enqueue("changed my mind", peer)
        now.addAndGet(5)
        val b = p.enqueue("keep this", peer)

        assertTrue(blocking { p.outbox.cancel(a.id) })
        assertEquals("canceled", p.store.items.getValue(a.id).status)
        val session = connect(pc)
        blocking { p.flusher.flush(session) }

        assertEquals("only the other one was sent", listOf(b.id), pc.textFrames(1))
        assertTrue(pc.noTextWithin(300))
        assertEquals(0, p.store.items.getValue(a.id).attempts)
    }

    @Test
    fun aCancelWinsOverTheFlushThatHadAlreadyListedTheItem() {
        val pc = pc()
        val p = parts(pc.identity.deviceId)
        val a = p.enqueue("race", PeerInfo(pc.identity.deviceId, "Desk PC"))
        val session = connect(pc)
        // The flush lists the queue, then the user cancels, then the flush gets to the item: the claim fails and nothing is sent.
        val listed = blocking { p.outbox.queued() }
        assertTrue(blocking { p.outbox.cancel(a.id) })
        assertEquals(TextSendResult.Skipped, blocking { p.outbox.sendOne(session, listed.single()) })
        assertTrue(pc.noTextWithin(300))
    }

    @Test
    fun anItemThatIsBeingSentCannotBeCanceledAndADoneOneNeither() {
        val pc = pc()
        val p = parts(pc.identity.deviceId)
        val a = p.enqueue("on its way", PeerInfo(pc.identity.deviceId, "Desk PC"))
        blocking { p.store.claim(a.id) }
        assertFalse(blocking { p.outbox.cancel(a.id) })
        assertEquals("active", p.store.items.getValue(a.id).status)
        blocking { p.store.finish(a.id, TransferStatus.DONE, null, null, 1) }
        assertFalse(blocking { p.outbox.cancel(a.id) })
        assertEquals("done", p.store.items.getValue(a.id).status)
    }

    // ---------------------------------------------------------------- other PCs, unpair

    @Test
    fun whatWasMeantForAnUnpairedPcFailsAndGoesToNoOne() {
        val pc = pc()
        val p = parts(pc.identity.deviceId) // the paired PC is this one
        val meantForOld = p.enqueue("for the old PC", PeerInfo("old-pc", "Old PC"))
        val mine = p.enqueue("for this PC", PeerInfo(pc.identity.deviceId, "Desk PC"))
        val session = connect(pc)

        blocking { p.flusher.flush(session) }

        assertEquals("failed", p.store.items.getValue(meantForOld.id).status)
        assertEquals("peer_changed", p.store.items.getValue(meantForOld.id).errorCode)
        assertEquals(listOf(mine.id), pc.textFrames(1))
        assertTrue(pc.noTextWithin(300))
    }

    @Test
    fun whatWaitsForThePairedPcStaysQueuedWhileOnlyAnotherPcsConnectionIsUp() {
        val old = pc()
        val p = parts(old.identity.deviceId, paired = "the-new-pc") // re-paired: the old PC's connection is still current
        val forNew = p.enqueue("for the new PC", PeerInfo("the-new-pc", "New PC"))
        val session = connect(old)
        blocking { p.flusher.flush(session) }
        assertEquals("queued", p.store.items.getValue(forNew.id).status)
        assertEquals(0, p.store.items.getValue(forNew.id).attempts)
        assertTrue(old.noTextWithin(300))
    }

    @Test
    fun unpairingCancelsWhatWasQueuedForThatPc() {
        val p = parts("pc-1")
        val a = p.enqueue("a", PeerInfo("pc-1", "One"))
        val b = p.enqueue("b", PeerInfo("pc-2", "Two"))
        blocking { p.outbox.onUnpaired("pc-1") }
        assertEquals("canceled", p.store.items.getValue(a.id).status)
        assertEquals("unpaired", p.store.items.getValue(a.id).errorCode)
        assertEquals("queued", p.store.items.getValue(b.id).status)
    }

    // ---------------------------------------------------------------- retry, privacy

    @Test
    fun retryOfAFailedTextSendsItAgainAsANewMessage() {
        val pc = pc { errorForText = "internal" }
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val a = p.enqueue("try again", peer)
        val session = connect(pc)
        blocking { p.flusher.flush(session) }
        assertEquals("failed", p.store.items.getValue(a.id).status)
        pc.frameOfType("text")

        pc.errorForText = null
        assertTrue(blocking { p.outbox.retry(a.id) })
        // The failed row stays as it ended and points at the new one (a new ID: the PC would repeat its first answer to the old one).
        val retried = p.store.items.values.single { it.id != a.id }
        assertEquals("failed", p.store.items.getValue(a.id).status)
        assertEquals(retried.id, p.store.items.getValue(a.id).retriedAs)
        assertEquals("queued", retried.status)
        assertEquals("try again", retried.body)
        blocking { p.flusher.flush(session) }
        assertEquals(retried.id, pc.frameOfType("text").getValue("id").str())
        assertEquals("done", p.store.items.getValue(retried.id).status)
        assertFalse(retried.id == a.id)
        assertEquals("the failed first attempt stays counted: a retried text is not a first-try success", 2, p.store.items.getValue(retried.id).attempts)
    }

    @Test
    fun aSensitiveTextKeepsItsBodyOnlyWhileItWaits() {
        val pc = pc()
        val p = parts(pc.identity.deviceId)
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val a = p.enqueue("hunter2", peer, sensitive = true, source = TextSource.CLIPBOARD)
        assertEquals("hunter2", p.store.items.getValue(a.id).body)
        val session = connect(pc)
        blocking { p.flusher.flush(session) }
        assertEquals("done", p.store.items.getValue(a.id).status)
        assertNull("nothing of it is kept once it is over", p.store.items.getValue(a.id).body)
        assertFalse("and it can't be retried", blocking { p.outbox.retry(a.id) })
    }

    @Test
    fun aReceivedSensitiveTextIsNeverStoredAndAPlainOneIsHistory() {
        val p = parts("pc")
        blocking { p.outbox.recordIncoming("pc", "Desk PC", "in-1", "secret", isUrl = false, sensitive = true) }
        assertTrue(p.store.items.isEmpty())
        blocking { p.outbox.recordIncoming("pc", "Desk PC", "in-2", "hello phone", isUrl = false, sensitive = false) }
        val row = p.store.items.getValue("in-2")
        assertEquals("down", row.direction)
        assertEquals("done", row.status)
        assertEquals(TextSource.PC, row.source)
    }

    @Test
    fun theAutomaticClipboardIsNeverQueuedOrStored() {
        val p = parts("pc")
        val written = CopyOnWriteArrayList<String>()
        val handler = IncomingHandler(object : ClipboardPort { override suspend fun write(text: String, sensitive: Boolean) { written += text } }, p.alerts, ClipboardGuard({ 0L }), KinLog.None)
        blocking { handler.onClipboard("copied on the PC") }
        assertEquals(listOf("copied on the PC"), written.toList())
        assertTrue("nothing was queued or stored for it", p.store.items.isEmpty())
        assertTrue(p.alerts.texts.isEmpty())
    }

    // ---------------------------------------------------------------- the sender on top of the queue

    @Test
    fun theSenderSavesFirstThenSendsAndAWifiOffSendArrivesWithinSecondsOfTheConnectionComingBack() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        var session: LinkSession? = null
        val link = object : OutgoingLink {
            override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = session.readyFor(peerDeviceId)
            override fun unreachableReason() = "This phone isn't on Wi-Fi."
            override suspend fun peer() = peer
        }
        val sender = OutgoingSender(link, p.alerts, ClipboardGuard({ 0L }), { }, { System.nanoTime() / 1_000_000 }, KinLog.None, p.outbox, p.flusher, connectWaitMillis = 100)

        // Wi-Fi is off: the text is queued and the user is told so.
        val outcome = blocking { sender.send("typed while offline", 0, SendSource.SHARE) }
        assertEquals(SendOutcome.Queued("This phone isn't on Wi-Fi."), outcome)
        assertEquals("queued", p.store.items.values.single().status)

        // Wi-Fi comes back: the connection is up and the hello is accepted; the queue goes out at once.
        val back = System.nanoTime()
        session = connect(pc)
        blocking { p.flusher.flush(session!!) }
        val frame = pc.frameOfType("text")
        val seconds = (System.nanoTime() - back) / 1e9
        assertEquals("typed while offline", frame.getValue("text").str())
        assertTrue("arrived $seconds s after the connection came back", seconds < 3)
        assertEquals("done", p.store.items.values.single().status)
        assertEquals(listOf("Desk PC"), p.alerts.sentLater.toList())
    }

    @Test
    fun withNoPairedPcNothingIsQueued() {
        val p = parts("x")
        val link = object : OutgoingLink {
            override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = null
            override fun unreachableReason() = "x"
            override suspend fun peer(): PeerInfo? = null
        }
        val sender = OutgoingSender(link, p.alerts, ClipboardGuard({ 0L }), { }, { 0L }, KinLog.None, p.outbox, p.flusher)
        assertEquals(SendOutcome.NotPaired, blocking { sender.send("hi", 0, SendSource.SHARE) })
        assertTrue(p.store.items.isEmpty())
    }

    /** A connection whose sends never get an answer: the ack is awaited until the coroutine is cancelled. */
    private class HangingConnection(override val peerDeviceId: String) : LinkConnection {
        override val isReady = true
        override fun open() = Unit
        override suspend fun awaitEnd(): SessionEnd = awaitCancellation()
        override fun closeNormally(reason: String) = Unit
        override suspend fun ping(timeoutMillis: Long) = PingResult.NoConnection
        override suspend fun sendAcked(id: String, frame: String, timeoutMillis: Long): AckResult = awaitCancellation()
        override suspend fun sendUnpairAndClose() = Unit
    }

    @Test
    fun aTextWhoseSendIsCancelledWhileWaitingForTheAckGoesBackToTheQueueInsteadOfStickingInSending() {
        val pc = pc()
        val peer = PeerInfo(pc.identity.deviceId, "Desk PC")
        val p = parts(pc.identity.deviceId)
        val item = p.enqueue("do not get stuck", peer)

        // The service's scope ends (a stop, a network change, the process being torn down) in the middle of the ack wait.
        val job = scope.launch { p.outbox.sendOne(HangingConnection(pc.identity.deviceId), item) }
        val deadline = System.currentTimeMillis() + 5_000
        while (p.store.items.getValue(item.id).status != "active") {
            check(System.currentTimeMillis() < deadline) { "never became active" }
            Thread.sleep(5)
        }
        blocking { job.cancelAndJoin() }

        assertEquals("not stuck in sending: queued again", "queued", p.store.items.getValue(item.id).status)
        assertEquals("the attempt counts", 1, p.store.items.getValue(item.id).attempts)
        // And it goes out later, under the same ID.
        val session = connect(pc)
        blocking { p.flusher.flush(session) }
        assertEquals(item.id, pc.frameOfType("text").getValue("id").str())
        assertEquals("done", p.store.items.getValue(item.id).status)
    }
}
