package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.CloseAction
import app.kinrelay.android.protocol.CloseRules
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.tls.RaceResult
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/** What the loop needs from the network it runs on. One environment per Wi-Fi network. */
interface LinkEnvironment {
    suspend fun race(addresses: List<PairingAddress>, timeoutMillis: Long): RaceResult

    /** mDNS lookup for the PC with this short ID, for at most [timeoutMillis]. */
    suspend fun discover(shortId: String, timeoutMillis: Long): List<PairingAddress>

    fun openSession(address: PairingAddress, listener: SessionListener): LinkConnection
}

/** What the loop reports to and asks from the rest of the app. */
interface LinkHost {
    suspend fun lastAddresses(): List<PairingAddress>
    suspend fun onConnected(address: PairingAddress, peer: Hello)
    suspend fun onDisconnected(end: SessionEnd, action: CloseAction)
    fun onPingReceived(fromName: String)
    suspend fun onPeerUnpaired()
}

/**
 * Keeps one connection to the paired PC alive on one network. The connection order is
 * architecture.md, "Finding each other": the last working addresses (1.5 s), then mDNS (5 s),
 * then backoff. The backoff resets only on a user action or after a connection that kept an
 * accepted `hello` for 60 s ([CloseRules.resetsBackoff]); a network change creates a new loop.
 */
class LinkLoop(
    private val scope: CoroutineScope,
    private val shortId: String,
    private val env: LinkEnvironment,
    private val host: LinkHost,
    private val clock: Clock,
    private val log: KinLog,
    private val backoff: Backoff = Backoff(),
) {
    private val mutableState = MutableStateFlow<LinkState>(LinkState.Connecting())
    val state: StateFlow<LinkState> = mutableState

    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var job: Job? = null
    @Volatile private var session: LinkConnection? = null
    @Volatile private var stopping = false

    val currentSession: LinkConnection? get() = session

    fun start() {
        job = scope.launch { run() }
    }

    /** The user did something: try again now, from the first backoff step. */
    fun userAction() {
        if (mutableState.value is LinkState.Connected) return
        backoff.reset()
        wake.trySend(Unit)
    }

    /**
     * Stops the loop. An open connection is closed cleanly and the loop ends through its normal
     * path, so the disconnect is logged and counted like any other.
     */
    fun stop(reason: String) {
        stopping = true
        val open = session
        if (open != null) open.closeNormally(reason) else job?.cancel()
    }

    private suspend fun run() {
        while (!stopping) {
            mutableState.value = LinkState.Connecting()
            val address = findAddress()
            if (address == null) {
                waitBeforeRetry()
                continue
            }
            val listener = object : SessionListener {
                override fun onHelloAccepted(peer: Hello) {
                    mutableState.value = LinkState.Connected(peer.name, address)
                    scope.launch { host.onConnected(address, peer) }
                }

                override fun onPingReceived(fromName: String) = host.onPingReceived(fromName)

                override fun onPeerUnpaired() {
                    scope.launch { host.onPeerUnpaired() }
                }
            }
            val opened = env.openSession(address, listener)
            session = opened
            opened.open()
            if (stopping) opened.closeNormally("stopping")
            val end = opened.awaitEnd()
            session = null

            val action = CloseRules.action(end.closeCode, end.upgradeStatus)
            log.i("link", "connection ended: ${end.reason} (code ${end.closeCode}, upgrade ${end.upgradeStatus}, " +
                "hello accepted for ${end.helloAcceptedForMillis} ms) -> $action")
            host.onDisconnected(end, action)
            if (end.peerUnpaired || stopping) return

            val stable = CloseRules.resetsBackoff(end.helloAcceptedForMillis)
            if (stable) backoff.reset()
            when (action) {
                CloseAction.RECONNECT_WITH_BACKOFF -> if (!stable) waitBeforeRetry()
                CloseAction.RECONNECT_ON_USER_ACTION -> { mutableState.value = LinkState.Replaced; wake.receive() }
                CloseAction.STOP_UPDATE_APP -> { mutableState.value = LinkState.UpdateApp; wake.receive() }
                CloseAction.STOP_PAIR_AGAIN -> { mutableState.value = LinkState.PairAgain; awaitCancellation() }
            }
        }
    }

    private suspend fun findAddress(): PairingAddress? {
        val known = host.lastAddresses()
        (env.race(known, RACE_TIMEOUT_MILLIS) as? RaceResult.Winner)?.let { return it.address }
        log.i("link", "last addresses failed; looking up the PC with mDNS")
        val found = env.discover(shortId, DISCOVERY_TIMEOUT_MILLIS).filter { it !in known }
        if (found.isEmpty()) return null
        return (env.race(found, RACE_TIMEOUT_MILLIS) as? RaceResult.Winner)?.address
    }

    private suspend fun waitBeforeRetry() {
        val delayMillis = backoff.next()
        log.i("link", "PC not reachable; next attempt in ${delayMillis / 1000} s")
        mutableState.value = LinkState.Connecting(retryInMillis = delayMillis)
        wake.tryReceive().getOrNull()?.let { return }
        withTimeoutOrNull(delayMillis) { wake.receive() }
    }

    private companion object {
        const val RACE_TIMEOUT_MILLIS = 1_500L
        const val DISCOVERY_TIMEOUT_MILLIS = 5_000L
    }
}
