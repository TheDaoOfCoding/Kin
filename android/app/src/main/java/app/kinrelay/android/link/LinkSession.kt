package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.CloseRules
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.Hello
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
interface LinkConnection {
    val isReady: Boolean

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
}

interface SessionListener {
    fun onHelloAccepted(peer: Hello)
    fun onPingReceived(fromName: String)
    fun onPeerUnpaired()

    /** A valid `text` from the PC, already acked. Called in arrival order from the socket's thread: don't block. */
    fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean)

    /** A valid `clipboard` from the PC, already acked. */
    fun onClipboardReceived(id: String, text: String)

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
data class LocalHello(val name: String, val platform: String, val appVersion: String)

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
) : LinkConnection {
    private val receiver = MessageReceiver(dedup, ReceiverRole.PHONE, listener::acceptOffer, listener::offerHistory)
    private val ended = CompletableDeferred<SessionEnd>()
    private val pending = ConcurrentHashMap<String, CompletableDeferred<Action>>()
    private var socket: WebSocket? = null
    private var openedAt = 0L
    @Volatile private var helloAcceptedAt: Long? = null
    @Volatile private var ourCloseCode: Int? = null
    @Volatile private var peerCloseCode: Int? = null
    @Volatile private var peerUnpaired = false
    private var helloTimeout: Job? = null

    override val isReady: Boolean get() = helloAcceptedAt != null && !ended.isCompleted

    /** File requests wait longer than the connection's own defaults: a response may follow a long upload, and a busy PC may be slow to write. */
    override val httpEndpoint: HttpEndpoint = HttpEndpoint(
        client.newBuilder().readTimeout(HTTP_READ_TIMEOUT_SECONDS, TimeUnit.SECONDS).writeTimeout(HTTP_WRITE_TIMEOUT_SECONDS, TimeUnit.SECONDS).build(),
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
        // Anyone still waiting for an ack gets an answer that isn't one, instead of a cancellation.
        pending.values.forEach { it.complete(Action.CloseOnly) }
    }

    private fun handle(outcome: app.kinrelay.android.protocol.Outcome) {
        outcome.replies.forEach { send(it) }
        when (val action = outcome.action) {
            is Action.HandshakeOk -> {
                if (helloAcceptedAt == null) helloAcceptedAt = clock.elapsedMillis()
                helloTimeout?.cancel()
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

    private inner class SocketListener : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            openedAt = clock.elapsedMillis()
            log.i("link", "websocket open to $address")
            send(Frames.hello(MessageIds.newId(), Frames.sanitize(local.name), local.platform, local.appVersion))
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
    }
}
