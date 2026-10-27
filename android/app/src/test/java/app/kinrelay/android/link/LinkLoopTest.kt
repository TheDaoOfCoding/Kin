package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.CloseAction
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.tls.RaceResult
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class LinkLoopTest {
    private val known = PairingAddress("10.0.2.2", 47821)
    private val found = PairingAddress("192.168.1.20", 47821)
    private val peer = Hello("pc", "Desk PC", "windows", "0.1.0")

    private class FakeConnection(val address: PairingAddress, val listener: SessionListener) : LinkConnection {
        val ended = CompletableDeferred<SessionEnd>()
        var closedNormally: String? = null
        override val isReady = true
        override val peerDeviceId = "pc"
        override fun open() = Unit
        override suspend fun awaitEnd() = ended.await()
        override fun closeNormally(reason: String) {
            closedNormally = reason
            ended.complete(end(1000))
        }
        override suspend fun ping(timeoutMillis: Long) = PingResult.NoConnection
        override suspend fun sendAcked(id: String, frame: String, timeoutMillis: Long) = AckResult.NoConnection
        override suspend fun sendUnpairAndClose() = Unit

        var probes = 0
        var probeAnswers = true
        /** Holds the probe's answer back until completed (a probe in flight). */
        var probeGate: CompletableDeferred<Unit>? = null
        override suspend fun probe(timeoutMillis: Long): Boolean {
            probes++
            probeGate?.await()
            // No reply: the real session abandons the socket, so the connection ends (it had been up for 10 s: not "stable").
            if (!probeAnswers) ended.complete(end(null, helloFor = 10_000))
            return probeAnswers
        }
    }

    private class Env(private val scope: TestScope) : LinkEnvironment {
        val raceTimes = mutableListOf<Long>()
        val raceTimeouts = mutableListOf<Long>()
        val discoverTimes = mutableListOf<Long>()
        val discoverTimeouts = mutableListOf<Long>()
        val sessions = mutableListOf<FakeConnection>()
        val sessionTimes = mutableListOf<Long>()
        var winners: Set<PairingAddress> = emptySet()
        var discovered: List<PairingAddress> = emptyList()

        override suspend fun race(addresses: List<PairingAddress>, timeoutMillis: Long): RaceResult {
            raceTimes += scope.currentTime
            raceTimeouts += timeoutMillis
            return addresses.firstOrNull { it in winners }?.let { RaceResult.Winner(it) }
                ?: RaceResult.Failed(addresses.associateWith { java.net.SocketTimeoutException("no answer") })
        }

        override suspend fun discover(shortId: String, timeoutMillis: Long): List<PairingAddress> {
            discoverTimes += scope.currentTime
            discoverTimeouts += timeoutMillis
            return discovered
        }

        override fun openSession(address: PairingAddress, listener: SessionListener): LinkConnection {
            sessionTimes += scope.currentTime
            return FakeConnection(address, listener).also { sessions += it }
        }
    }

    private class Host(var addresses: List<PairingAddress>) : LinkHost {
        val connected = mutableListOf<PairingAddress>()
        val disconnected = mutableListOf<Pair<SessionEnd, CloseAction>>()
        var unpaired = 0
        val delivered = mutableListOf<String>()
        override suspend fun lastAddresses() = addresses
        override suspend fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean) {
            // The first message is slow on purpose: the second must still wait for it.
            if (text == "slow") kotlinx.coroutines.delay(1_000)
            delivered += "text:$text"
        }
        override suspend fun onClipboardReceived(id: String, text: String) { delivered += "clipboard:$text" }
        val helloSeen = mutableListOf<LinkConnection>()
        override fun onHelloAccepted(connection: LinkConnection) { helloSeen += connection }
        override suspend fun onConnected(address: PairingAddress, peer: Hello) { connected += address }
        override suspend fun onDisconnected(end: SessionEnd, action: CloseAction) { disconnected += end to action }
        override fun onPingReceived(fromName: String) = Unit
        override suspend fun onPeerUnpaired() { unpaired++ }
    }

    private companion object {
        fun end(code: Int?, helloFor: Long? = null, upgrade: Int? = null, unpaired: Boolean = false) =
            SessionEnd(code, upgrade, "test", helloFor, unpaired)
    }

    private fun TestScope.loop(env: Env, host: Host, workWaiting: () -> Boolean = { false }) =
        LinkLoop(this, "ABCDEFGH", env, host, { currentTime }, KinLog.None, workWaiting = workWaiting)

    private fun TestScope.advance(millis: Long) {
        advanceTimeBy(millis)
        runCurrent()
    }

    @Test
    fun aLaterHelloOfTheSameConnectionTellsTheMirrorButIsNotAnotherConnect() = runTest {
        // The PC turns "Show phone notifications" on or off: it sends a new hello on the open connection (protocol.md, Capability).
        val env = Env(this).apply { winners = setOf(known) }
        val host = Host(listOf(known))
        val loop = loop(env, host)
        loop.start()
        advance(10)
        val session = env.sessions.single()
        session.listener.onHelloAccepted(peer)
        session.listener.onHelloAccepted(peer.copy(capabilities = setOf("link", "notif")))
        advance(10)
        assertEquals("connected once", listOf(known), host.connected)
        assertEquals("the mirror hears every hello", 2, host.helloSeen.size)
        loop.stop("test")
    }

    @Test
    fun backoffIs5s30s2minThenEvery10Min() = runTest {
        val env = Env(this)
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        advance(1_355_000 + 1)
        assertEquals(listOf(0L, 5_000, 35_000, 155_000, 755_000, 1_355_000), env.discoverTimes)
        loop.stop("test")
    }

    @Test
    fun whileATransferWaitsEveryBackoffStepIsAtMost10Seconds() = runTest {
        val env = Env(this)
        val loop = loop(env, Host(listOf(known)), workWaiting = { true })
        loop.start()
        advance(60_001)
        assertEquals(listOf(0L, 5_000, 15_000, 25_000, 35_000, 45_000, 55_000), env.discoverTimes)
        loop.stop("test")
    }

    @Test
    fun theBackoffIsTheUsualOneAgainAsSoonAsNothingWaits() = runTest {
        var waiting = true
        val env = Env(this)
        val loop = loop(env, Host(listOf(known)), workWaiting = { waiting })
        loop.start()
        advance(15_001) // attempts at 0, 5 s and 15 s; the third step (120 s) is capped to 10 s while someone waits
        assertEquals(listOf(0L, 5_000, 15_000), env.discoverTimes)
        waiting = false
        advance(25_000) // 25 s: the step now running was chosen while waiting (10 s); the next ones are the idle steps
        assertEquals(listOf(0L, 5_000, 15_000, 25_000), env.discoverTimes)
        advance(600_000)
        assertEquals("idle again: every 10 min", listOf(0L, 5_000, 15_000, 25_000, 625_000), env.discoverTimes)
        loop.stop("test")
    }

    @Test
    fun aTransferThatStartsWaitingWakesALoopThatSleepsALongStep() = runTest {
        var waiting = false
        val env = Env(this)
        val loop = loop(env, Host(listOf(known)), workWaiting = { waiting })
        loop.start()
        advance(40_000) // attempts at 0, 5 s, 35 s; now asleep until 155 s
        assertEquals(listOf(0L, 5_000, 35_000), env.discoverTimes)

        waiting = true
        loop.workIsWaiting()
        runCurrent()
        assertEquals("tried at once", listOf(0L, 5_000, 35_000, 40_000), env.discoverTimes)
        advance(10_000)
        assertEquals("then at the short step", listOf(0L, 5_000, 35_000, 40_000, 50_000), env.discoverTimes)
        loop.stop("test")
    }

    @Test
    fun aTransferWaitingDoesNotWakeALoopThatStoppedOnPurposeOrIsConnected() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)), workWaiting = { true })
        loop.start()
        runCurrent()
        env.sessions.single().listener.onHelloAccepted(peer)
        runCurrent()
        loop.workIsWaiting() // connected: nothing to do
        runCurrent()
        assertEquals(1, env.sessions.size)

        env.sessions[0].ended.complete(end(4001, helloFor = 1_000))
        runCurrent()
        assertEquals(LinkState.Replaced, loop.state.value)
        loop.workIsWaiting()
        runCurrent()
        advance(60_000)
        assertEquals("replaced: only the user wakes it", 1, env.sessions.size)
        loop.stop("test")
    }

    @Test
    fun connectionOrderIsLastAddressesThenMdnsWithTheirTimeouts() = runTest {
        val env = Env(this).apply { discovered = listOf(found); winners = setOf(found) }
        val host = Host(listOf(known))
        val loop = loop(env, host)
        loop.start()
        runCurrent()
        assertEquals(listOf(1_500L, 1_500L), env.raceTimeouts)
        assertEquals(listOf(5_000L), env.discoverTimeouts)
        assertEquals(found, env.sessions.single().address)
        loop.stop("test")
    }

    @Test
    fun theInstantPathSkipsMdnsWhenALastAddressAnswers() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        assertTrue(env.discoverTimes.isEmpty())
        assertEquals(known, env.sessions.single().address)
        loop.stop("test")
    }

    @Test
    fun helloAcceptedMakesItConnectedAndSavesTheAddress() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val host = Host(listOf(known))
        val loop = loop(env, host)
        loop.start()
        runCurrent()
        env.sessions.single().listener.onHelloAccepted(peer)
        runCurrent()
        assertEquals(LinkState.Connected("Desk PC", known), loop.state.value)
        assertEquals(listOf(known), host.connected)
        loop.stop("test")
    }

    @Test
    fun aConnectionThatClosesAtOnceDoesNotResetTheBackoff() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        // accepted hello, then closed 5 s later (not stable): wait the first step (5 s) before the next try
        env.sessions[0].ended.complete(end(1000, helloFor = 5))
        runCurrent()
        assertEquals(1, env.sessions.size)
        advance(4_999)
        assertEquals(1, env.sessions.size)
        advance(1)
        assertEquals(2, env.sessions.size)
        // the second quick close waits the second step (30 s)
        env.sessions[1].ended.complete(end(1000, helloFor = 5))
        runCurrent()
        advance(29_999)
        assertEquals(2, env.sessions.size)
        advance(1)
        assertEquals(3, env.sessions.size)
        loop.stop("test")
    }

    @Test
    fun aConnectionStable60sResetsTheBackoffAndReconnectsAtOnce() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        env.sessions[0].ended.complete(end(1000, helloFor = 5)) // pushes the backoff to the second step
        runCurrent()
        advance(5_000)
        env.sessions[1].ended.complete(end(null, helloFor = 60_000)) // stable
        runCurrent()
        assertEquals("immediate reconnect after a stable connection", 3, env.sessions.size)
        // and the backoff is back at the first step
        env.winners = emptySet()
        env.sessions[2].ended.complete(end(1000, helloFor = 1))
        runCurrent()
        val before = env.discoverTimes.size
        advance(5_000)
        assertEquals(before + 1, env.discoverTimes.size)
        loop.stop("test")
    }

    @Test
    fun textAndClipboardFromThePcAreDeliveredInArrivalOrderEvenWhenTheFirstIsSlow() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val host = Host(listOf(known))
        val loop = loop(env, host)
        loop.start()
        runCurrent()
        val listener = env.sessions.single().listener
        listener.onHelloAccepted(peer)
        listener.onTextReceived("Desk PC", "a", "slow", false, false)
        listener.onClipboardReceived("b", "second")
        listener.onTextReceived("Desk PC", "c", "third", true, false)
        runCurrent()
        assertTrue("the slow one holds the others back", host.delivered.isEmpty())
        advance(1_000)
        assertEquals(listOf("text:slow", "clipboard:second", "text:third"), host.delivered)
        loop.stop("test")
    }

    @Test
    fun replacedWaitsForAUserActionAndThenReconnects() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        env.sessions[0].ended.complete(end(4001, helloFor = 3_600_000))
        runCurrent()
        assertEquals(LinkState.Replaced, loop.state.value)
        advance(3_600_000)
        assertEquals("no automatic reconnect", 1, env.sessions.size)
        loop.userAction()
        runCurrent()
        assertEquals(2, env.sessions.size)
        loop.stop("test")
    }

    @Test
    fun updateTheAppStopsRetryingUntilTheUserActs() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        env.sessions[0].ended.complete(end(4002))
        runCurrent()
        assertEquals(LinkState.UpdateApp, loop.state.value)
        advance(3_600_000)
        assertEquals(1, env.sessions.size)
        loop.stop("test")
    }

    @Test
    fun unauthorizedStopsForGoodEvenAfterAUserAction() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        env.sessions[0].ended.complete(end(null, upgrade = 401))
        runCurrent()
        assertEquals(LinkState.PairAgain, loop.state.value)
        loop.userAction()
        advance(3_600_000)
        assertEquals(1, env.sessions.size)
        loop.stop("test")
    }

    @Test
    fun code4003AlsoMeansPairAgain() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        env.sessions[0].ended.complete(end(4003, helloFor = 100))
        runCurrent()
        assertEquals(LinkState.PairAgain, loop.state.value)
        loop.stop("test")
    }

    @Test
    fun aUserActionDuringTheBackoffTriesAtOnceFromTheFirstStep() = runTest {
        val env = Env(this)
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        advance(5_000)
        advance(30_000 - 1_000) // t = 34 s, waiting for the 35 s attempt
        loop.userAction()
        runCurrent()
        assertEquals(34_000L, env.discoverTimes.last())
        val attempts = env.discoverTimes.size
        advance(4_999)
        assertEquals(attempts, env.discoverTimes.size)
        advance(1)
        assertEquals("next wait is the first step again (5 s)", attempts + 1, env.discoverTimes.size)
        loop.stop("test")
    }

    @Test
    fun theLoopEndsWhenThePcUnpairedThePhone() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val host = Host(listOf(known))
        val loop = loop(env, host)
        loop.start()
        runCurrent()
        env.sessions[0].listener.onPeerUnpaired()
        env.sessions[0].ended.complete(end(1000, helloFor = 10, unpaired = true))
        runCurrent()
        advance(3_600_000)
        assertEquals(1, host.unpaired)
        assertEquals(1, env.sessions.size)
    }

    @Test
    fun stopClosesTheConnectionCleanly() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val host = Host(listOf(known))
        val loop = loop(env, host)
        loop.start()
        runCurrent()
        env.sessions.single().listener.onHelloAccepted(peer)
        runCurrent()
        loop.stop("Wi-Fi lost")
        runCurrent()
        assertEquals("Wi-Fi lost", env.sessions.single().closedNormally)
        assertEquals("the disconnect is reported like any other", 1, host.disconnected.size)
        advance(3_600_000)
        assertEquals(1, env.sessions.size)
    }

    // ---------------------------------------------------------------- a user action while connected (a half-open link)

    /** A loop connected to [known] with the hello accepted. */
    private fun TestScope.connectedLoop(env: Env, host: Host = Host(listOf(known))): LinkLoop {
        env.winners = setOf(known)
        val loop = loop(env, host)
        loop.start()
        runCurrent()
        env.sessions.single().listener.onHelloAccepted(peer)
        runCurrent()
        assertTrue(loop.state.value is LinkState.Connected)
        return loop
    }

    @Test
    fun aUserActionWhileConnectedProbesTheConnection() = runTest {
        val env = Env(this)
        val loop = connectedLoop(env)
        loop.userAction()
        runCurrent()
        assertEquals(1, env.sessions.single().probes)
        assertEquals("an answered probe leaves the connection alone", null, env.sessions.single().closedNormally)
        assertEquals(LinkState.Connected("Desk PC", known), loop.state.value)
        loop.stop("test")
    }

    @Test
    fun aProbeThatFailsEndsTheConnectionAndTheLoopReconnectsAtOnce() = runTest {
        val env = Env(this)
        val loop = connectedLoop(env)
        advance(10_000)
        env.sessions[0].probeAnswers = false
        val at = currentTime
        loop.userAction()
        runCurrent()
        assertEquals(1, env.sessions[0].probes)
        assertEquals("a new connection at once, not after a backoff step", 2, env.sessions.size)
        assertEquals(at, env.sessionTimes[1])
        loop.stop("test")
    }

    @Test
    fun twoUserActionsWithin30SecondsProbeOnce() = runTest {
        val env = Env(this)
        val loop = connectedLoop(env)
        val session = env.sessions.single()
        loop.userAction()
        runCurrent()
        advance(LinkLoop.PROBE_INTERVAL_MILLIS - 1)
        loop.userAction()
        runCurrent()
        assertEquals("the last probe finished less than 30 s ago", 1, session.probes)
        advance(1)
        loop.userAction()
        runCurrent()
        assertEquals(2, session.probes)
        loop.stop("test")
    }

    @Test
    fun onlyOneProbeIsInFlight() = runTest {
        val env = Env(this)
        val loop = connectedLoop(env)
        val session = env.sessions.single()
        val gate = CompletableDeferred<Unit>()
        session.probeGate = gate
        loop.userAction()
        runCurrent()
        advance(40_000)
        loop.userAction()
        runCurrent()
        assertEquals(1, session.probes)
        gate.complete(Unit)
        runCurrent()
        loop.userAction()
        runCurrent()
        assertEquals("the in-flight one just finished: still within 30 s", 1, session.probes)
        loop.stop("test")
    }

    @Test
    fun aUserActionWhileConnectingKeepsTodaysBehaviourAndProbesNothing() = runTest {
        // A connection is open but the PC's hello hasn't been accepted: Connecting, so a user action only resets the backoff and wakes the loop.
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        assertTrue(loop.state.value is LinkState.Connecting)
        loop.userAction()
        runCurrent()
        assertEquals(0, env.sessions.single().probes)
        assertEquals("the connection was not touched", null, env.sessions.single().closedNormally)
        assertEquals(1, env.sessions.size)
        loop.stop("test")
    }

    // ---------------------------------------------------------------- the network came back (1.4 review)

    @Test
    fun aNetworkThatComesBackResetsTheBackoffAndTriesAtOnce() = runTest {
        val env = Env(this)
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        advance(36_000) // attempts at 0, 5 s and 35 s: the next wait would be 2 minutes
        assertEquals(listOf(0L, 5_000, 35_000), env.discoverTimes)

        loop.networkChanged()
        runCurrent()

        assertEquals("tried at once, not after the 2 minutes", listOf(0L, 5_000, 35_000, 36_000), env.discoverTimes)
        // The steps start again from the first: 5 s, then 30 s. Repeated failures on a network that stays the same still climb.
        advance(4_999)
        assertEquals(4, env.discoverTimes.size)
        advance(1)
        assertEquals(listOf(0L, 5_000, 35_000, 36_000, 41_000), env.discoverTimes)
        advance(29_999)
        assertEquals(5, env.discoverTimes.size)
        advance(1)
        assertEquals(71_000L, env.discoverTimes.last())
        loop.stop("test")
    }

    @Test
    fun aNetworkChangeWhileConnectedDoesNothing() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        env.sessions.single().listener.onHelloAccepted(peer)
        runCurrent()

        loop.networkChanged()
        runCurrent()

        assertEquals(LinkState.Connected("Desk PC", known), loop.state.value)
        assertEquals("the connection was not touched", 1, env.sessions.size)
        assertEquals(null, env.sessions.single().closedNormally)
        loop.stop("test")
    }

    @Test
    fun aNetworkChangeDoesNotWakeALoopThatStoppedOnPurpose() = runTest {
        val env = Env(this).apply { winners = setOf(known) }
        val loop = loop(env, Host(listOf(known)))
        loop.start()
        runCurrent()
        // Another phone took this PC's place (4001): the loop waits for the user, not for the network.
        env.sessions[0].ended.complete(end(4001, helloFor = 1_000))
        runCurrent()
        assertEquals(LinkState.Replaced, loop.state.value)

        loop.networkChanged()
        runCurrent()
        advance(60_000)

        assertEquals(1, env.sessions.size)
        loop.stop("test")
    }
}
