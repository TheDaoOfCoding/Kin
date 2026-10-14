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
        override fun open() = Unit
        override suspend fun awaitEnd() = ended.await()
        override fun closeNormally(reason: String) {
            closedNormally = reason
            ended.complete(end(1000))
        }
        override suspend fun ping(timeoutMillis: Long) = PingResult.NoConnection
        override suspend fun sendUnpairAndClose() = Unit
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
        override suspend fun lastAddresses() = addresses
        override suspend fun onConnected(address: PairingAddress, peer: Hello) { connected += address }
        override suspend fun onDisconnected(end: SessionEnd, action: CloseAction) { disconnected += end to action }
        override fun onPingReceived(fromName: String) = Unit
        override suspend fun onPeerUnpaired() { unpaired++ }
    }

    private companion object {
        fun end(code: Int?, helloFor: Long? = null, upgrade: Int? = null, unpaired: Boolean = false) =
            SessionEnd(code, upgrade, "test", helloFor, unpaired)
    }

    private fun TestScope.loop(env: Env, host: Host) = LinkLoop(this, "ABCDEFGH", env, host, { currentTime }, KinLog.None)

    private fun TestScope.advance(millis: Long) {
        advanceTimeBy(millis)
        runCurrent()
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
}
