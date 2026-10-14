package app.kinrelay.android.link

import app.kinrelay.android.identity.Identity
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.CloseAction
import app.kinrelay.android.protocol.CloseRules
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.str
import app.kinrelay.android.tls.KinTls
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeUnit.MILLISECONDS
import java.util.concurrent.TimeUnit.SECONDS
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import okhttp3.OkHttpClient

class LinkSessionTest {
    private val phone = Identity.generate()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pcs = mutableListOf<FakePc>()

    private class Events : SessionListener {
        val hellos = LinkedBlockingQueue<Hello>()
        val pings = AtomicInteger()
        val unpaired = CountDownLatch(1)
        override fun onHelloAccepted(peer: Hello) { hellos.add(peer) }
        override fun onPingReceived(fromName: String) { pings.incrementAndGet() }
        override fun onPeerUnpaired() = unpaired.countDown()
    }

    @After
    fun tearDown() {
        scope.cancel()
        pcs.forEach { it.close() }
    }

    private fun pc(block: FakePc.() -> Unit = {}): FakePc = FakePc().apply { pairedPhones += phone.deviceId }.apply(block).start().also { pcs += it }

    private fun session(
        pc: FakePc,
        events: Events,
        pinned: String = pc.identity.deviceId,
        helloTimeoutMillis: Long = CloseRules.HELLO_TIMEOUT_MILLIS,
    ): LinkSession {
        val client: OkHttpClient = KinTls(phone, pinned).httpClient().newBuilder().callTimeout(0, MILLISECONDS).build()
        return LinkSession(
            client, PairingAddress("127.0.0.1", pc.port), LocalHello("Pixel 8", "android", "0.1.0"),
            DedupMemory(System::currentTimeMillis), scope, { System.nanoTime() / 1_000_000 }, KinLog.None, events, helloTimeoutMillis,
        )
    }

    private fun <T> blocking(block: suspend () -> T): T = runBlocking { withTimeout(15_000) { block() } }

    private fun connected(pc: FakePc, events: Events = Events()): Pair<LinkSession, Events> {
        val session = session(pc, events)
        session.open()
        assertNotNull("hello accepted", events.hellos.poll(5, SECONDS))
        return session to events
    }

    @Test
    fun theFirstFrameIsOurHelloWithAllFields() {
        val pc = pc()
        connected(pc)
        val hello = pc.nextFrame()!!
        assertEquals("hello", hello.getValue("type").str())
        assertEquals(1, hello.getValue("protocol").jsonPrimitive.long.toInt())
        assertEquals("Pixel 8", hello.getValue("name").str())
        assertEquals("android", hello.getValue("platform").str())
        assertEquals("0.1.0", hello.getValue("appVersion").str())
        assertTrue(Regex("[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}").matches(hello.getValue("id").str()))
    }

    @Test
    fun helloIsAcceptedAndThePeerNameIsKept() {
        val pc = pc()
        val events = Events()
        val session = session(pc, events)
        session.open()
        val peer = events.hellos.poll(5, SECONDS)!!
        assertEquals("Desk PC", peer.name)
        assertEquals("windows", peer.platform)
        assertTrue(session.isReady)
    }

    @Test
    fun pingMeasuresTheRoundTripToTheAck() {
        val pc = pc()
        val (session, _) = connected(pc)
        val result = blocking { session.ping() }
        assertTrue("got $result", result is PingResult.Rtt && result.millis >= 0 && result.millis < 5_000)
        pc.nextFrame() // hello
        val ping = pc.nextFrame()!!
        assertEquals("ping", ping.getValue("type").str())
        assertTrue(ping.getValue("sentAt").jsonPrimitive.long > 1_700_000_000_000L)
    }

    @Test
    fun anUnackedPingIsNoReplyAndTheConnectionIsTreatedAsDead() {
        val pc = pc { ackPings = false }
        val (session, _) = connected(pc)
        assertEquals(PingResult.NoReply, blocking { session.ping(timeoutMillis = 300) })
        val end = blocking { session.awaitEnd() }
        assertEquals(CloseAction.RECONNECT_WITH_BACKOFF, CloseRules.action(end.closeCode, end.upgradeStatus))
    }

    @Test
    fun pingWithoutAConnectionSendsNothing() {
        val pc = pc { sendHelloOnOpen = false }
        val session = session(pc, Events())
        assertEquals(PingResult.NoConnection, blocking { session.ping() })
        session.open()
        assertEquals(PingResult.NoConnection, blocking { session.ping() }) // opened, but the PC's hello has not been accepted
        session.closeNormally("test")
        assertTrue(pc.received.none { it.contains("\"ping\"") })
    }

    @Test
    fun anIncomingPingIsAckedOnceAndShownOnce() {
        val pc = pc()
        val (_, events) = connected(pc)
        pc.nextFrame() // our hello
        pc.send("""{"type":"ping","id":"pc-ping-1","sentAt":1760443200000}""")
        val ack = pc.nextFrame()!!
        assertEquals("""{"type":"ack","ref":"pc-ping-1"}""", ack.toString())
        pc.send("""{"type":"ping","id":"pc-ping-1","sentAt":1760443200000}""")
        assertEquals("pc-ping-1", pc.nextFrame()!!.getValue("ref").str())
        assertEquals("a duplicate id shows one notification", 1, events.pings.get())
    }

    @Test
    fun replacedWithCode4001EndsAndWaitsForTheUser() {
        val pc = pc()
        val (session, _) = connected(pc)
        pc.closeSocket(4001, "replaced")
        val end = blocking { session.awaitEnd() }
        assertEquals(4001, end.closeCode)
        assertNotNull(end.helloAcceptedForMillis)
        assertEquals(CloseAction.RECONNECT_ON_USER_ACTION, CloseRules.action(end.closeCode, end.upgradeStatus))
    }

    @Test
    fun aFutureProtocolHelloGetsUnsupportedVersionAnd4002() {
        val pc = pc { sendHelloOnOpen = false }
        val events = Events()
        val session = session(pc, events)
        session.open()
        pc.nextFrame() // our hello
        waitForSocket(pc)
        pc.send("""{"type":"hello","id":"h","protocol":2,"device":{"name":"Desk PC"}}""")
        val error = pc.nextFrame()!!
        assertEquals("error", error.getValue("type").str())
        assertEquals("unsupported_version", error.getValue("code").str())
        assertEquals("h", error.getValue("ref").str())
        val end = blocking { session.awaitEnd() }
        assertEquals(4002, end.closeCode)
        assertNull("never accepted", end.helloAcceptedForMillis)
        assertTrue(events.hellos.isEmpty())
        assertEquals(CloseAction.STOP_UPDATE_APP, CloseRules.action(end.closeCode))
    }

    @Test
    fun noHelloInTimeClosesWith1002() {
        val pc = pc { sendHelloOnOpen = false }
        val session = session(pc, Events(), helloTimeoutMillis = 400)
        session.open()
        val end = blocking { session.awaitEnd() }
        assertEquals(1002, end.closeCode)
        assertNull(end.helloAcceptedForMillis)
    }

    @Test
    fun aMessageBeforeHelloGetsBadRequestAndAClose1002() {
        val pc = pc { sendHelloOnOpen = false }
        val session = session(pc, Events())
        session.open()
        pc.nextFrame()
        waitForSocket(pc)
        pc.send("""{"type":"ping","id":"early","sentAt":1}""")
        val error = pc.nextFrame()!!
        assertEquals("bad_request", error.getValue("code").str())
        assertEquals("early", error.getValue("ref").str())
        assertEquals(1002, blocking { session.awaitEnd() }.closeCode)
    }

    @Test
    fun garbageBeforeHelloKeepsTheConnectionOpen() {
        val pc = pc { sendHelloOnOpen = false }
        val events = Events()
        val session = session(pc, events)
        session.open()
        pc.nextFrame()
        waitForSocket(pc)
        pc.send("hello")
        assertEquals("bad_request", pc.nextFrame()!!.getValue("code").str())
        pc.send(pc.helloFrame())
        assertNotNull(events.hellos.poll(5, SECONDS))
        assertTrue(session.isReady)
    }

    @Test
    fun aFrameOverOneMiBClosesWith1009AndNoReply() {
        val pc = pc()
        val (session, _) = connected(pc)
        pc.nextFrame()
        pc.send("""{"type":"ping","id":"big","sentAt":1,"pad":"${"a".repeat(1_048_576)}"}""")
        val end = blocking { session.awaitEnd() }
        assertEquals(1009, end.closeCode)
        assertTrue("no reply to an oversize frame", pc.received.isEmpty())
    }

    @Test
    fun aBinaryFrameClosesWith1003() {
        val pc = pc()
        val (session, _) = connected(pc)
        pc.sendBinary(byteArrayOf(1, 2, 3))
        assertEquals(1003, blocking { session.awaitEnd() }.closeCode)
    }

    @Test
    fun unpairFromThePcIsAckedThenTheSessionEnds() {
        val pc = pc()
        val (session, events) = connected(pc)
        pc.nextFrame()
        pc.send("""{"type":"unpair","id":"u1"}""")
        assertEquals("u1", pc.nextFrame()!!.getValue("ref").str())
        assertTrue(events.unpaired.await(5, SECONDS))
        val end = blocking { session.awaitEnd() }
        assertTrue(end.peerUnpaired)
        assertEquals(1000, end.closeCode)
    }

    @Test
    fun unpairFromThePhoneWaitsForTheAckThenCloses() {
        val pc = pc()
        val (session, _) = connected(pc)
        blocking { session.sendUnpairAndClose() }
        val end = blocking { session.awaitEnd() }
        assertEquals(1000, end.closeCode)
        val frames = generateSequence { pc.nextFrame(200) }.toList()
        assertTrue(frames.any { it.getValue("type").str() == "unpair" })
    }

    @Test
    fun anUnpairedPhoneGetsA401OnTheUpgradeAndStops() {
        val pc = pc { pairedPhones.clear() }
        val session = session(pc, Events())
        session.open()
        val end = blocking { session.awaitEnd() }
        assertEquals(401, end.upgradeStatus)
        assertEquals(CloseAction.STOP_PAIR_AGAIN, CloseRules.action(end.closeCode, end.upgradeStatus))
    }

    @Test
    fun aRefusedClientCertificateIsAFailedConnectionNeverConnected() {
        val pc = FakePc(rejectClientCertificates = true).apply { pairedPhones += phone.deviceId }.start().also { pcs += it }
        val events = Events()
        val session = session(pc, events)
        session.open()
        val end = blocking { session.awaitEnd() }
        assertNull(end.helloAcceptedForMillis)
        assertTrue(events.hellos.isEmpty())
        assertNull("no close code: a network-level failure", end.closeCode)
        assertEquals(CloseAction.RECONNECT_WITH_BACKOFF, CloseRules.action(end.closeCode, end.upgradeStatus))
    }

    @Test
    fun aWrongPinNeverReachesTheUpgrade() {
        val pc = pc()
        val session = session(pc, Events(), pinned = Identity.generate().deviceId)
        session.open()
        val end = blocking { session.awaitEnd() }
        assertNull(end.helloAcceptedForMillis)
        assertTrue(pc.requests.isEmpty())
        assertTrue(end.reason, end.reason.contains("failure"))
    }

    private fun waitForSocket(pc: FakePc) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (pc.sockets.isEmpty() && System.nanoTime() < deadline) Thread.sleep(10)
        assertTrue("PC has a socket", pc.sockets.isNotEmpty())
    }
}
