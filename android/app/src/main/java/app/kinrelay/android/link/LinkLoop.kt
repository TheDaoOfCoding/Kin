package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.CloseAction
import app.kinrelay.android.protocol.CloseRules
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.protocol.Offer
import app.kinrelay.android.protocol.OfferHistory
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

    /** A `text` from the PC (acked already). Called one at a time, in arrival order. */
    suspend fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean)

    /** A `clipboard` from the PC (acked already). Called one at a time, in arrival order. */
    suspend fun onClipboardReceived(id: String, text: String)

    /** Before an `offer` is acked: null accepts it, an error code refuses it. Cheap and synchronous. */
    fun acceptOffer(offer: Offer): String? = null

    /** What the phone knows about the ID of an `offer` (a repeat by this PC, a reuse, or new). Cheap and synchronous. */
    fun offerHistory(offer: Offer): OfferHistory = OfferHistory.NEW

    /** An `offer` this PC made before, acked again. Called one at a time, in arrival order. */
    suspend fun onOfferRepeated(fromName: String, offer: Offer) = Unit

    /** An `offer` (acked already). Called one at a time, in arrival order. It must return soon: the download runs elsewhere. */
    suspend fun onOfferReceived(fromName: String, offer: Offer) = Unit

    /** A `cancel` (acked already). Called one at a time, in arrival order. */
    suspend fun onCancelReceived(transferId: String, reason: String) = Unit
}

/**
 * Keeps one connection to the paired PC alive on one network. The connection order is
 * architecture.md, "Finding each other": the last working addresses (1.5 s), then mDNS (5 s),
 * then backoff. The backoff resets only on a user action or after a connection that kept an
 * accepted `hello` for 60 s ([CloseRules.resetsBackoff]); a network change creates a new loop.
 *
 * One exception, for the sake of a transfer that is in the middle of its files: while [workWaiting] says something waits for the link
 * (a transfer that was moving and now waits for the PC to come back), a step of the backoff is at most [WAITING_RETRY_MILLIS]. The steps
 * are what keeps a PC that is off from costing the phone's battery, but 2 or 10 minutes of sleep for a PC that is back after a short hole
 * would leave the user's files lying there for no reason. Nobody waiting: the steps are exactly the ones above.
 */
class LinkLoop(
    private val scope: CoroutineScope,
    private val shortId: String,
    private val env: LinkEnvironment,
    private val host: LinkHost,
    private val clock: Clock,
    private val log: KinLog,
    private val backoff: Backoff = Backoff(),
    private val workWaiting: () -> Boolean = { false },
) {
    private val mutableState = MutableStateFlow<LinkState>(LinkState.Connecting())
    val state: StateFlow<LinkState> = mutableState

    private val wake = Channel<Unit>(Channel.CONFLATED)
    private val deliveries = DeliveryQueue(log)
    private var job: Job? = null
    @Volatile private var session: LinkConnection? = null
    @Volatile private var stopping = false

    val currentSession: LinkConnection? get() = session

    fun start() {
        deliveries.start(scope)
        job = scope.launch { run() }
    }

    /** The user did something: try again now, from the first backoff step. */
    fun userAction() {
        if (mutableState.value is LinkState.Connected) return
        backoff.reset()
        wake.trySend(Unit)
    }

    /**
     * The network came back or changed under a loop that is not connected (new addresses, the network became usable): the wait that
     * repeated failures built up was earned on the old network and says nothing about this one, so the backoff starts again and the next
     * attempt is made **now**. Repeated failures on an unchanged network still climb the steps. Does nothing while connected, and does not
     * wake a loop that stopped on purpose (replaced, update needed, pair again).
     */
    fun networkChanged() {
        if (mutableState.value !is LinkState.Connecting) return
        backoff.reset()
        wake.trySend(Unit)
    }

    /**
     * Something (a transfer) has started waiting for the link: a loop that sleeps a long backoff step tries now instead, and carries on at
     * the short steps while [workWaiting] holds. Does nothing while connected, and does not wake a loop that stopped on purpose.
     */
    fun workIsWaiting() {
        if (mutableState.value is LinkState.Connecting) wake.trySend(Unit)
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
        try {
            runConnections()
        } finally {
            deliveries.close()
        }
    }

    private suspend fun runConnections() {
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

                override fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean) =
                    deliveries.submit { host.onTextReceived(fromName, id, text, isUrl, sensitive) }

                override fun onClipboardReceived(id: String, text: String) =
                    deliveries.submit { host.onClipboardReceived(id, text) }

                override fun acceptOffer(offer: Offer): String? = host.acceptOffer(offer)

                override fun offerHistory(offer: Offer): OfferHistory = host.offerHistory(offer)

                override fun onOfferRepeated(fromName: String, offer: Offer) =
                    deliveries.submit { host.onOfferRepeated(fromName, offer) }

                override fun onOfferReceived(fromName: String, offer: Offer) =
                    deliveries.submit { host.onOfferReceived(fromName, offer) }

                override fun onCancelReceived(transferId: String, reason: String) =
                    deliveries.submit { host.onCancelReceived(transferId, reason) }
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
        val stepMillis = backoff.next()
        val delayMillis = if (workWaiting()) minOf(stepMillis, WAITING_RETRY_MILLIS) else stepMillis
        log.i("link", "PC not reachable; next attempt in ${delayMillis / 1000} s" + if (delayMillis < stepMillis) " (a transfer is waiting)" else "")
        mutableState.value = LinkState.Connecting(retryInMillis = delayMillis)
        wake.tryReceive().getOrNull()?.let { return }
        withTimeoutOrNull(delayMillis) { wake.receive() }
    }

    companion object {
        private const val RACE_TIMEOUT_MILLIS = 1_500L
        private const val DISCOVERY_TIMEOUT_MILLIS = 5_000L

        /** The longest wait between attempts while a transfer waits for the link. */
        const val WAITING_RETRY_MILLIS = 10_000L
    }
}
