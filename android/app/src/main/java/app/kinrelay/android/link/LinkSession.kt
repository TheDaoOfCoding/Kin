package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.media.MediaLink
import app.kinrelay.android.media.MediaMsg
import app.kinrelay.android.notifmirror.MirrorLink
import app.kinrelay.android.notifmirror.NotifWire
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.presence.PresenceLink
import app.kinrelay.android.presence.PresenceSnapshot
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.CloseRules
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.protocol.LinkMessage
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.MessageReceiver
import app.kinrelay.android.protocol.Offer
import app.kinrelay.android.protocol.OfferHistory
import app.kinrelay.android.protocol.ReceiverRole
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.ConnectionPool
import okhttp3.Dispatcher
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/** How one WebSocket ended. */
class SessionEnd(
    val closeCode: Int?,
    val upgradeStatus: Int?,
    val reason: String,
    /** How long the peer's hello had been accepted, or null if it never was. */
    val helloAcceptedForMillis: Long?,
    val peerUnpaired: Boolean,
)

/** How a message that needs an ack ended. */
sealed interface AckResult {
    data object Acked : AckResult

    /** The peer answered with an `error` for it. No retry (protocol.md, Message IDs and acks). */
    data class Refused(val code: String, val message: String) : AckResult

    /** No ack within the timeout, or the connection ended first. The message may or may not have arrived. */
    data object NoReply : AckResult

    /** There was no connection to send on. */
    data object NoConnection : AckResult
}

/** Where the HTTPS requests of a transfer go: the PC the WebSocket is connected to, over the same TLS setup and network. */
class HttpEndpoint(val client: OkHttpClient, val address: PairingAddress)

/** One connection to the PC, as the reconnect loop and the UI see it. */
interface LinkConnection : MirrorLink, MediaLink, PresenceLink {
    override val isReady: Boolean

    /** The PC's latest accepted `hello` on this connection lists `presence` (2B.4): `presence` may go to it. Checked per message by [app.kinrelay.android.presence.PresenceSender]. */
    override val peerListsPresence: Boolean get() = false

    /**
     * Tells the PC the phone's capabilities changed (2B.4: `ring` is listed only while "Let my PC ring this phone" is on): a new `hello` goes on the open connection and the receiver's own
     * list changes with it (protocol.md allows a later `hello`). A connection that is not ready ignores it (the next `hello` carries the new list).
     */
    fun updateCapabilities(capabilities: List<String>) = Unit

    /** The PC's latest accepted `hello` on this connection lists `media` (2B.3): `media.*` messages may go to it. Checked per message by [app.kinrelay.android.media.MediaSender]. */
    override val peerUnderstandsMedia: Boolean get() = false

    /** The PC's name from its latest accepted `hello` (the viewer says "<app> on <PC>"), or null before one. */
    val peerName: String? get() = null

    /**
     * The PC's latest accepted `hello` on this connection lists the `notif` capability (2B.2): it will show `notif.post`, and `notif.*` messages may
     * go to it. Checked when each message goes on the wire ([MirrorSender] is the only sender); with no `notif` nothing is sent.
     */
    override val peerUnderstandsNotif: Boolean get() = false

    /** Puts a `notif.*` frame on the socket with no ack, no queue and no re-send (ephemeral messages). False when the connection can't take it. */
    override fun sendEphemeral(frame: String): Boolean = false

    /**
     * The PC's latest accepted `hello` on this connection lists the `link` capability (protocol.md, "Versioning"): a `link` may go to it as a
     * `link`; otherwise the same item goes as a `text`. Decided when the message goes on the wire.
     */
    val peerUnderstandsLink: Boolean get() = false

    /** The device ID of the PC this connection is pinned to (the TLS handshake accepted only that key). A transfer or a queued message is for one PC. */
    val peerDeviceId: String

    /** The HTTPS side of this connection (file bytes go over separate requests, never over the WebSocket). Null in tests that have none. */
    val httpEndpoint: HttpEndpoint? get() = null

    fun open()
    suspend fun awaitEnd(): SessionEnd
    fun closeNormally(reason: String)
    suspend fun ping(timeoutMillis: Long = 5_000L): PingResult

    /**
     * Sends [frame] (whose `id` is [id]) and waits up to [timeoutMillis] for its `ack`. With no ack the
     * connection is treated as dead, as for any unacked message.
     */
    suspend fun sendAcked(id: String, frame: String, timeoutMillis: Long = 5_000L): AckResult
    suspend fun sendUnpairAndClose()

    /**
     * Whether the PC still answers, for a transfer that has stopped moving. The WebSocket's own ping is far too slow for that (240 s, and
     * 240 s more for the pong), and a JSON `ping` would pop up "Ping from <phone>" on the PC every time, so this asks the network: a new
     * TLS connection to the PC, which a silent drop (packets vanish, no reset) can't complete. If it doesn't answer in [timeoutMillis] the
     * connection is abandoned (it ends, so the loop reconnects and the transfers wait for the new one) and the answer is false.
     * True also when the connection can't tell (tests).
     */
    suspend fun checkAlive(timeoutMillis: Long = 10_000L): Boolean = true

    /** [checkAlive] found the PC silent and abandoned this connection (so the end of it is the network's doing, not the PC's). */
    val foundDead: Boolean get() = false
}

interface SessionListener {
    fun onHelloAccepted(peer: Hello)
    fun onPingReceived(fromName: String)
    fun onPeerUnpaired()

    /** A valid `text` from the PC, already acked. Called in arrival order from the socket's thread: don't block. */
    fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean)

    /** A valid `clipboard` from the PC, already acked. */
    fun onClipboardReceived(id: String, text: String)

    /** A valid `link` from the PC, already acked (the title and source app are cleaned). Called in arrival order from the socket's thread: don't block. */
    fun onLinkReceived(fromName: String, id: String, link: LinkMessage) = Unit

    /** A valid, new `alert` from the PC (2B.5), already acked (the text and title are cleaned). Called in arrival order from the socket's thread: don't block. */
    fun onAlertReceived(fromName: String, id: String, alert: app.kinrelay.android.alerts.AlertMessage) = Unit

    /** A valid `notif.dismiss` (2B.2). Ephemeral: no ack. Don't block. */
    fun onNotifDismiss(nid: String) = Unit

    /**
     * A valid, new `notif.action` (2B.2): fire it and return the frame to answer with, an `ack` or an `error` for `request.id`. The caller sends the
     * answer and remembers it for the de-duplication. Runs on a coroutine of the session's scope.
     */
    suspend fun onNotifAction(request: NotifWire.ActionRequest): String = Frames.error("disabled", "Notification mirroring isn't available.", request.id)

    /**
     * A valid `media.*` message (2B.3). Ephemeral: no ack. Called in arrival order from the socket's thread: don't block (the app queues it and handles it on its own coroutine,
     * so a state and the clear after it can't swap places).
     */
    fun onMedia(message: MediaMsg) = Unit

    /** A `media.*` message was dropped silently (a side's `hello` didn't list `media`, or an image that failed its checks): counted by the app. */
    fun onMediaDropped(notListed: Boolean) = Unit

    /** A valid `presence` from the PC (2B.4). Ephemeral: no ack. It replaces the PC's previous snapshot on this connection. Called in arrival order from the socket's thread: don't block. */
    fun onPresence(snapshot: PresenceSnapshot) = Unit

    /** A `presence` was dropped silently (a side's `hello` didn't list `presence`): counted by the app. */
    fun onPresenceDropped() = Unit

    /**
     * A valid, new `ring.start` (2B.4): start ringing (or restart the 30 s) and call [reply] **once** with the frame to answer with (`ack`, or an `error` with the request's `ref`); the
     * session sends it and remembers it for the de-duplication. A `ring.status` follows after the `ack`. Runs on a coroutine of the session's scope.
     */
    suspend fun onRingStart(id: String, reply: (String) -> Unit) = reply(Frames.error("disabled", "Ringing isn't available.", id))

    /** A valid, new `ring.stop`: stop ringing; [reply] is always an `ack`. */
    suspend fun onRingStop(id: String, reply: (String) -> Unit) = reply(Frames.ack(id))

    /**
     * A valid, new `offer`, before it is acked: null accepts it, an error code (`disk_full`) answers with an `error` instead of the
     * ack. Runs on the socket's thread: keep it cheap.
     */
    fun acceptOffer(offer: Offer): String? = null

    /**
     * What the phone knows about the ID of an `offer`, asked first: a repeat by this PC is acked again (and [onOfferRepeated] follows), any
     * other reuse is `bad_request`. Runs on the socket's thread: keep it cheap.
     */
    fun offerHistory(offer: Offer): OfferHistory = OfferHistory.NEW

    /** An `offer` this PC made before, acked again (nothing is downloaded a second time). Don't block. */
    fun onOfferRepeated(fromName: String, offer: Offer) = Unit

    /** A valid `offer` that was acked. Don't block: start the download on another coroutine. */
    fun onOfferReceived(fromName: String, offer: Offer) = Unit

    /** A `cancel` for a transfer, already acked. The transfer may be unknown or finished: then there is nothing to do. */
    fun onCancelReceived(transferId: String, reason: String) = Unit
}

/** The phone's own name, sent in its `hello`. */
data class LocalHello(val name: String, val platform: String, val appVersion: String, val capabilities: List<String> = Frames.LOCAL_CAPABILITIES)

/**
 * One WebSocket to the PC at `/v1/ws`: `hello` exchange, the validation rules of
 * [MessageReceiver], replies, pings with round-trip time, and unpair. It ends exactly once
 * ([awaitEnd]); what to do next is the caller's business.
 */
class LinkSession(
    private val client: OkHttpClient,
    private val address: PairingAddress,
    private val local: LocalHello,
    dedup: DedupMemory,
    private val scope: CoroutineScope,
    private val clock: Clock,
    private val log: KinLog,
    private val listener: SessionListener,
    override val peerDeviceId: String,
    private val helloTimeoutMillis: Long = CloseRules.HELLO_TIMEOUT_MILLIS,
    /** Opens (and closes) a new connection to the PC at the address within the time: the probe of [checkAlive]. */
    private val reachable: suspend (PairingAddress, Long) -> Boolean = { _, _ -> true },
    /** One token of the phone's alert bucket (2B.5): false answers an `alert` with `rate_limited`. */
    alertAdmit: () -> Boolean = { true },
) : LinkConnection {
    // A `notif.action` is answered after the action fired, not by the receiver (`notifAction` returns null: "later", see the NotifAction branch of [handle]).
    // A `ring.start` / `ring.stop` is answered the same way, after the ring started or stopped (`ringAnswer` returns null: see [answerRing]).
    private val receiver = MessageReceiver(dedup, ReceiverRole.PHONE, listener::acceptOffer, listener::offerHistory, notifAction = { null }, ringAnswer = { _, _ -> null }, alertAdmit = alertAdmit)
        .also { it.ownCapabilities = local.capabilities.toSet() }

    // The capabilities this phone wants its `hello` to say **now** (a switch changed before the connection was ready changes them too), and what the last `hello` sent actually said.
    @Volatile private var wantedCapabilities: List<String> = local.capabilities
    @Volatile private var sentCapabilities: List<String>? = null
    private val ended = CompletableDeferred<SessionEnd>()
    private val pending = ConcurrentHashMap<String, CompletableDeferred<Action>>()
    private val actionsInProgress: MutableSet<String> = ConcurrentHashMap.newKeySet()
    private var socket: WebSocket? = null
    private var openedAt = 0L
    @Volatile private var helloAcceptedAt: Long? = null
    @Volatile private var ourCloseCode: Int? = null
    @Volatile private var peerCloseCode: Int? = null
    @Volatile private var peerUnpaired = false
    @Volatile override var foundDead = false
        private set
    private var helloTimeout: Job? = null

    override val isReady: Boolean get() = helloAcceptedAt != null && !ended.isCompleted

    override val peerUnderstandsLink: Boolean get() = receiver.peer?.understandsLink == true

    override val peerUnderstandsNotif: Boolean get() = receiver.peer?.understandsNotif == true

    override val peerUnderstandsMedia: Boolean get() = receiver.peer?.understandsMedia == true

    override val peerListsPresence: Boolean get() = receiver.peer?.understandsPresence == true

    override fun updateCapabilities(capabilities: List<String>) {
        // Nothing changed (the switch's first value after a connection that already said it): no new `hello`.
        if (wantedCapabilities.toSet() == capabilities.toSet() && receiver.ownCapabilities == capabilities.toSet()) return
        wantedCapabilities = capabilities
        receiver.ownCapabilities = capabilities.toSet()
        // Before the connection is ready the change is applied once it is (see [sayHelloAgainIfChanged]); the first `hello` goes out with whatever is wanted at that moment.
        if (isReady) sendHello(capabilities)
    }

    private fun sendHello(capabilities: List<String>): Boolean {
        sentCapabilities = capabilities
        return send(Frames.hello(MessageIds.newId(), Frames.sanitize(local.name), local.platform, local.appVersion, capabilities))
    }

    /** The peer's `hello` was accepted: a capability change made after our own `hello` went out and before this point (the 2B.4 review: it used to be lost) goes out now. */
    private fun sayHelloAgainIfChanged() {
        val sent = sentCapabilities ?: return
        if (sent.toSet() != wantedCapabilities.toSet()) sendHello(wantedCapabilities)
    }

    override val peerName: String? get() = receiver.peer?.name

    override fun sendEphemeral(frame: String): Boolean = isReady && send(frame)

    /** File requests wait longer than the connection's own defaults: a response may follow a long upload, and a busy PC may be slow to write. */
    override val httpEndpoint: HttpEndpoint = HttpEndpoint(
        client.newBuilder()
            .readTimeout(HTTP_READ_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .writeTimeout(HTTP_WRITE_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            // Several files of one transfer are on the wire at once (protocol.md: up to 4), each on its own connection. A pool of its own
            // that keeps the connections of a batch (a TLS 1.3 handshake with a client certificate per file would be the cost of a small file),
            // and lets them go after 20 s: the PC closes a connection that moved nothing for 30 s, and a reused one that it closed would fail.
            .connectionPool(ConnectionPool(HTTP_POOL_IDLE_CONNECTIONS, HTTP_POOL_KEEP_ALIVE_SECONDS, TimeUnit.SECONDS))
            .dispatcher(Dispatcher().apply { maxRequests = HTTP_POOL_IDLE_CONNECTIONS * 2; maxRequestsPerHost = HTTP_POOL_IDLE_CONNECTIONS * 2 })
            .build(),
        address,
    )

    override fun open() {
        val request = Request.Builder().url("https://${address.host}:${address.port}/v1/ws").build()
        socket = client.newWebSocket(request, SocketListener())
    }

    override suspend fun awaitEnd(): SessionEnd = ended.await()

    /** Closes with 1000 and waits for the end. Used when the network is lost or the app stops. */
    override fun closeNormally(reason: String) = closeOurs(1000, reason)

    /** Sends a `ping` and measures the round trip to its `ack` with a monotonic clock. */
    override suspend fun ping(timeoutMillis: Long): PingResult {
        val id = MessageIds.newId()
        val started = System.nanoTime()
        return when (val result = sendAcked(id, Frames.ping(id, System.currentTimeMillis()), timeoutMillis)) {
            AckResult.Acked -> PingResult.Rtt((System.nanoTime() - started) / 1_000_000.0)
            is AckResult.Refused -> PingResult.Failed(result.code, result.message)
            AckResult.NoReply -> PingResult.NoReply
            AckResult.NoConnection -> PingResult.NoConnection
        }
    }

    override suspend fun sendAcked(id: String, frame: String, timeoutMillis: Long): AckResult {
        if (!isReady) return AckResult.NoConnection
        val reply = CompletableDeferred<Action>().also { pending[id] = it }
        if (!send(frame)) {
            pending.remove(id)
            return AckResult.NoConnection
        }
        val action = withTimeoutOrNull(timeoutMillis) { reply.await() }
        pending.remove(id)
        return when (action) {
            is Action.MarkDelivered -> AckResult.Acked
            is Action.MarkFailed -> AckResult.Refused(action.code, action.message)
            else -> {
                log.w("link", "message $id not acked in $timeoutMillis ms; treating the connection as dead")
                socket?.cancel()
                AckResult.NoReply
            }
        }
    }

    override suspend fun checkAlive(timeoutMillis: Long): Boolean {
        if (!isReady) return false
        if (reachable(address, timeoutMillis)) return true
        if (ended.isCompleted) return false
        foundDead = true
        log.w("link", "the PC doesn't answer a new connection within $timeoutMillis ms; treating the connection as dead")
        socket?.cancel()
        // `cancel` ends the session at once; if it somehow doesn't, end it here so nobody keeps waiting on a dead link.
        if (withTimeoutOrNull(CLOSE_GRACE_MILLIS) { ended.await() } == null) finish("abandoned: the PC doesn't answer")
        return false
    }

    /** Sends `unpair` (the caller has already removed the PC locally), waits up to 5 s for the ack, then closes. */
    override suspend fun sendUnpairAndClose() {
        if (!isReady) return
        val id = MessageIds.newId()
        val reply = CompletableDeferred<Action>().also { pending[id] = it }
        if (send(Frames.unpair(id))) withTimeoutOrNull(ACK_TIMEOUT_MILLIS) { reply.await() }
        pending.remove(id)
        closeOurs(1000, "unpair")
    }

    private fun send(frame: String): Boolean = socket?.send(frame) == true

    private fun closeOurs(code: Int, reason: String) {
        if (ended.isCompleted || ourCloseCode != null) return
        ourCloseCode = code
        log.i("link", "closing with $code ($reason)")
        socket?.close(code, reason.take(100))
        // If the peer never answers the close, drop the connection after a while.
        scope.launch {
            delay(CLOSE_GRACE_MILLIS)
            socket?.cancel()
        }
    }

    private fun finish(reason: String, upgradeStatus: Int? = null) {
        val acceptedAt = helloAcceptedAt
        val end = SessionEnd(
            closeCode = ourCloseCode ?: peerCloseCode,
            upgradeStatus = upgradeStatus,
            reason = reason,
            helloAcceptedForMillis = acceptedAt?.let { clock.elapsedMillis() - it },
            peerUnpaired = peerUnpaired,
        )
        if (!ended.complete(end)) return
        helloTimeout?.cancel()
        // The file connections of this session are idle sockets to a PC that is gone (or about to be reached by a new session with a pool of
        // its own): close them now instead of leaving them to the 20 s keep-alive.
        httpEndpoint.client.connectionPool.evictAll()
        // Anyone still waiting for an ack gets an answer that isn't one, instead of a cancellation.
        pending.values.forEach { it.complete(Action.CloseOnly) }
    }

    private fun handle(outcome: app.kinrelay.android.protocol.Outcome) {
        outcome.replies.forEach { send(it) }
        when (val action = outcome.action) {
            is Action.HandshakeOk -> {
                if (helloAcceptedAt == null) helloAcceptedAt = clock.elapsedMillis()
                helloTimeout?.cancel()
                sayHelloAgainIfChanged()
                listener.onHelloAccepted(action.hello)
            }
            is Action.MarkDelivered -> pending[action.ref]?.complete(action)
            is Action.MarkFailed -> {
                log.w("link", "peer reported ${action.code} for ${action.ref}: ${action.message}")
                pending[action.ref]?.complete(action)
            }
            is Action.ShowErrorOnly -> log.w("link", "peer error ${action.code}: ${action.message}")
            is Action.ShowPing -> listener.onPingReceived(receiver.peer?.name ?: "PC")
            is Action.DeliverText -> listener.onTextReceived(receiver.peer?.name ?: "PC", action.id, action.text, action.isUrl, action.sensitive)
            is Action.SetClipboard -> listener.onClipboardReceived(action.id, action.text)
            is Action.DeliverLink -> listener.onLinkReceived(receiver.peer?.name ?: "PC", action.id, action.link)
            is Action.DeliverAlert -> listener.onAlertReceived(receiver.peer?.name ?: "PC", action.id, action.alert)
            is Action.NotifDismiss -> listener.onNotifDismiss(action.nid)
            is Action.NotifAction -> {
                // A repeat of an ID whose action is still firing has no recorded answer yet: it is ignored here, so the action never fires twice.
                if (actionsInProgress.add(action.request.id)) {
                    scope.launch {
                        try {
                            val answer = listener.onNotifAction(action.request)
                            // The answer is the one response of this ID: a repeat gets it again and the action never fires twice.
                            receiver.recordResponse(action.request.id, answer)
                            send(answer)
                        } finally {
                            actionsInProgress.remove(action.request.id)
                        }
                    }
                }
            }
            Action.NotifIgnored, is Action.NotifReceived -> Unit
            is Action.Media -> listener.onMedia(action.message)
            is Action.MediaDropped -> listener.onMediaDropped(action.notListed)
            is Action.Presence -> listener.onPresence(action.snapshot)
            Action.PresenceDropped -> listener.onPresenceDropped()
            is Action.RingStart -> answerRing(action.id) { reply -> listener.onRingStart(action.id, reply) }
            is Action.RingStop -> answerRing(action.id) { reply -> listener.onRingStop(action.id, reply) }
            is Action.RingStatusReceived -> Unit
            is Action.StartDownload -> listener.onOfferReceived(receiver.peer?.name ?: "PC", action.offer)
            is Action.RepeatOffer -> listener.onOfferRepeated(receiver.peer?.name ?: "PC", action.offer)
            is Action.CancelTransfer -> listener.onCancelReceived(action.transferId, action.reason)
            is Action.OfferRefused -> log.w("link", "an offer was refused: ${action.code}")
            is Action.MarkTransferDone -> Unit
            is Action.RemovePeerAndClose -> {
                peerUnpaired = true
                listener.onPeerUnpaired()
            }
            is Action.IgnoreDuplicate, Action.CloseOnly, Action.Rejected -> Unit
        }
        outcome.close?.let { closeOurs(it, "protocol") }
    }

    /**
     * `ring.start` and `ring.stop`: the answer is the one response of this ID (a repeat gets it again and nothing runs twice). A repeat whose first run is still going has no recorded answer yet: it is
     * ignored here. The listener calls the `reply` it is given once; it is sent and remembered at that moment, so the `ack` goes out before the `ring.status` that follows it.
     */
    private fun answerRing(id: String, run: suspend ((String) -> Unit) -> Unit) {
        if (!actionsInProgress.add(id)) return
        scope.launch {
            try {
                var answered = false
                run { frame ->
                    if (!answered) {
                        answered = true
                        receiver.recordAckOnly(id, frame)
                        send(frame)
                    }
                }
            } finally {
                actionsInProgress.remove(id)
            }
        }
    }

    private inner class SocketListener : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            openedAt = clock.elapsedMillis()
            log.i("link", "websocket open to $address")
            sendHello(wantedCapabilities)
            helloTimeout = scope.launch {
                delay(helloTimeoutMillis + 1)
                CloseRules.helloTimeoutClose(receiver.helloAccepted, clock.elapsedMillis() - openedAt, helloTimeoutMillis)?.let {
                    log.w("link", "no hello within 10 s")
                    closeOurs(it, "no hello within 10 seconds")
                }
            }
        }

        override fun onMessage(webSocket: WebSocket, text: String) = handle(receiver.onText(text))

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) = handle(receiver.onBinary())

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            peerCloseCode = code
            log.i("link", "peer is closing: $code '$reason'")
            webSocket.close(1000, null)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            finish("closed ${ourCloseCode?.let { "by us" } ?: "by the peer"}: $code '$reason'")
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            val status = response?.code?.takeIf { it != 101 }
            finish("failure: ${t.javaClass.simpleName}: ${t.message}", upgradeStatus = status)
        }
    }

    private companion object {
        const val ACK_TIMEOUT_MILLIS = 5_000L
        const val CLOSE_GRACE_MILLIS = 5_000L
        const val HTTP_READ_TIMEOUT_SECONDS = 60L
        const val HTTP_WRITE_TIMEOUT_SECONDS = 30L
        const val HTTP_POOL_IDLE_CONNECTIONS = 8
        const val HTTP_POOL_KEEP_ALIVE_SECONDS = 20L
    }
}
