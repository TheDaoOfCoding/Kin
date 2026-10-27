package app.kinrelay.android.testing

import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.identity.Identity
import java.net.InetAddress
import java.net.Socket
import java.security.Principal
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLSocket
import javax.net.ssl.SSLSocketFactory
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509ExtendedTrustManager
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import mockwebserver3.Dispatcher
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import mockwebserver3.RecordedRequest
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString

/**
 * A stand-in for the Windows app, in this JVM: TLS 1.3 with a mandatory client certificate,
 * `POST /v1/pair` and the `/v1/ws` WebSocket, on a loopback port. It checks nothing the phone
 * does not need checked; its job is to let the phone code talk real TLS and real WebSocket.
 */
class FakePc(
    val identity: Identity = Identity.generate(),
    val name: String = "Desk PC",
    private val tlsProtocol: String = "TLSv1.3",
    private val rejectClientCertificates: Boolean = false,
    private val fixedPort: Int = 0,
) : AutoCloseable {
    class Recorded(val method: String, val path: String, val contentType: String?, val body: String, val clientDeviceId: String?)

    private val server = MockWebServer()
    val requests = CopyOnWriteArrayList<Recorded>()
    val pairedPhones: MutableSet<String> = mutableSetOf()

    /** The file endpoints (uploads from the phone, offers to it). */
    val files = FakePcFiles()

    /** Answers a `/v1/transfers...` request itself (returns the response) instead of [files]: for replaying the shared vectors. */
    @Volatile var canned: ((RecordedRequest) -> MockResponse?)? = null

    /** Frames the phone sent on its WebSocket, in order. */
    val received = LinkedBlockingQueue<String>()
    val sockets = CopyOnWriteArrayList<WebSocket>()

    @Volatile var sendHelloOnOpen = true
    @Volatile var helloProtocol = 1

    /** The `capabilities` the PC lists in its `hello` (null: none, like a 0.1.x PC). */
    @Volatile var helloCapabilities: List<String>? = null
    @Volatile var ackPings = true

    /** What the PC does with a `probe`: ack it (false: say nothing, a PC that closed this connection long ago), or answer `unsupported_type` like a PC that doesn't know it. */
    @Volatile var ackProbes = true
    @Volatile var probesUnknown = false

    /** What the PC does with a `text` from the phone: ack it, say nothing, or answer with this `error` code. */
    @Volatile var ackTexts = true
    @Volatile var errorForText: String? = null
    @Volatile var pairResponse: (() -> MockResponse)? = null
    @Volatile var onFrame: ((WebSocket, JsonObject) -> Unit)? = null
    @Volatile var wsUpgradeStatus: Int? = null

    /** Adds the phone to [pairedPhones] when it pairs, like the real PC does. */
    @Volatile var autoPair = false

    val port: Int get() = server.port

    fun start(): FakePc {
        val context = SSLContext.getInstance(tlsProtocol)
        context.init(arrayOf(ServerKeyManager(identity.certificate, identity.privateKey)), arrayOf(ClientCertTrust(rejectClientCertificates)), SecureRandom())
        server.useHttps(if (tlsProtocol == "TLSv1.3") context.socketFactory else OnlyProtocol(context.socketFactory, tlsProtocol))
        server.requireClientAuth()
        server.protocols = listOf(Protocol.HTTP_1_1)
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = route(request)
        }
        server.start(InetAddress.getLoopbackAddress(), fixedPort)
        return this
    }

    override fun close() {
        runCatching { server.close() }
    }

    fun send(text: String) = sockets.last().send(text)

    /** A `clipboard` message from the PC, written the way the real PC writes it (compact JSON). */
    fun sendClipboard(id: String, text: String) =
        send(buildJsonObject { put("type", "clipboard"); put("id", id); put("text", text) }.toString())

    fun sendLink(id: String, url: String, title: String? = null, sourceApp: String? = null) =
        send(buildJsonObject {
            put("type", "link"); put("id", id); put("url", url)
            title?.let { put("title", it) }
            sourceApp?.let { put("sourceApp", it) }
        }.toString())

    fun sendText(id: String, text: String, isUrl: Boolean) =
        send(buildJsonObject { put("type", "text"); put("id", id); put("text", text); put("isUrl", isUrl) }.toString())

    /** Offers files to the phone: registers them for `GET` and sends the `offer` frame. */
    fun sendOffer(id: String, vararg offered: FakePcFiles.OfferedFile): FakePcFiles.Offered =
        files.offered(id, *offered).also { send(files.offerFrame(it)) }

    fun sendCancel(transferId: String, reason: String = "user") =
        send("""{"type":"cancel","id":"pc-cancel-${System.nanoTime()}","transferId":"$transferId","reason":"$reason"}""")

    fun sendBinary(bytes: ByteArray) = sockets.last().send(ByteString.of(*bytes))

    fun closeSocket(code: Int, reason: String = "test") = sockets.last().close(code, reason)

    fun nextFrame(timeoutMillis: Long = 5_000): JsonObject? =
        received.poll(timeoutMillis, TimeUnit.MILLISECONDS)?.let { Json.parseToJsonElement(it).jsonObject }

    fun helloFrame(protocol: Int = helloProtocol): String {
        val capabilities = helloCapabilities?.joinToString(",", prefix = ",\"capabilities\":[", postfix = "]") { "\"$it\"" }.orEmpty()
        return """{"type":"hello","id":"pc-hello","protocol":$protocol,"name":"$name","platform":"windows","appVersion":"0.1.0"$capabilities}"""
    }

    private fun route(request: RecordedRequest): MockResponse {
        val phone = request.handshake?.peerCertificates?.firstOrNull()?.let { DeviceIds.deviceIdOfCertificate(it.encoded) }
        val small = request.bodySize < 4096
        requests += Recorded(request.method, request.url.encodedPath, request.headers["Content-Type"], if (small) request.body?.utf8().orEmpty() else "<${request.bodySize} bytes>", phone)
        return when {
            request.url.encodedPath == "/v1/pair" && request.method == "POST" -> pairResponse?.invoke() ?: autoPairing(phone).let { json(200, """{"name":"$name","deviceId":"${identity.deviceId}","protocol":1}""") }
            request.url.encodedPath == "/v1/ws" -> {
                val status = wsUpgradeStatus ?: if (phone != null && phone in pairedPhones) null else 401
                if (status != null) {
                    json(status, """{"error":"unauthorized","message":"This device is not paired."}""")
                } else {
                    MockResponse.Builder().webSocketUpgrade(PcSocket()).build()
                }
            }
            request.url.encodedPath.startsWith("/v1/transfers") ->
                canned?.invoke(request) ?: if (phone != null && phone in pairedPhones) files.handle(request) else json(401, """{"error":"unauthorized","message":"This device is not paired."}""")
            else -> json(404, """{"error":"not_found","message":"Unknown route."}""")
        }
    }

    private fun autoPairing(phone: String?) {
        if (autoPair && phone != null) pairedPhones += phone
    }

    private fun json(status: Int, body: String) = MockResponse.Builder()
        .code(status)
        .addHeader("Content-Type", "application/json; charset=utf-8")
        .body(body)
        .build()

    private inner class PcSocket : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            sockets += webSocket
            if (sendHelloOnOpen) webSocket.send(helloFrame())
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            received.add(text)
            val frame = runCatching { Json.parseToJsonElement(text).jsonObject }.getOrNull() ?: return
            onFrame?.invoke(webSocket, frame)
            val type = frame["type"]?.jsonPrimitive?.content
            val id = frame["id"]?.jsonPrimitive?.content
            files.onFrame(frame)?.let { webSocket.send(it) }
            when (type) {
                "ping" -> if (ackPings && id != null) webSocket.send("""{"type":"ack","ref":"$id"}""")
                "probe" -> if (id != null) {
                    if (probesUnknown) {
                        webSocket.send("""{"type":"error","ref":"$id","code":"unsupported_type","message":"Unknown message type."}""")
                    } else if (ackProbes) {
                        webSocket.send("""{"type":"ack","ref":"$id"}""")
                    }
                }
                "text" -> if (id != null) {
                    val error = errorForText
                    if (error != null) {
                        webSocket.send("""{"type":"error","ref":"$id","code":"$error","message":"test"}""")
                    } else if (ackTexts) {
                        webSocket.send("""{"type":"ack","ref":"$id"}""")
                    }
                }
                "link" -> if (id != null) webSocket.send("""{"type":"ack","ref":"$id"}""")
                "unpair" -> if (id != null) {
                    webSocket.send("""{"type":"ack","ref":"$id"}""")
                    webSocket.close(1000, "unpaired")
                }
            }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(code, null)
        }
    }

    /** Restricts the sockets of [delegate] to one protocol version. */
    private class OnlyProtocol(private val delegate: SSLSocketFactory, private val protocol: String) : SSLSocketFactory() {
        private fun restrict(socket: Socket): Socket = socket.also { (it as SSLSocket).enabledProtocols = arrayOf(protocol) }
        override fun getDefaultCipherSuites(): Array<String> = delegate.defaultCipherSuites
        override fun getSupportedCipherSuites(): Array<String> = delegate.supportedCipherSuites
        override fun createSocket(s: Socket, host: String?, port: Int, autoClose: Boolean) = restrict(delegate.createSocket(s, host, port, autoClose))
        override fun createSocket(host: String?, port: Int) = restrict(delegate.createSocket(host, port))
        override fun createSocket(host: String?, port: Int, local: InetAddress?, localPort: Int) = restrict(delegate.createSocket(host, port, local, localPort))
        override fun createSocket(host: InetAddress?, port: Int) = restrict(delegate.createSocket(host, port))
        override fun createSocket(address: InetAddress?, port: Int, local: InetAddress?, localPort: Int) = restrict(delegate.createSocket(address, port, local, localPort))
    }

    private class ServerKeyManager(private val cert: X509Certificate, private val key: PrivateKey) : X509ExtendedKeyManager() {
        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = null
        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?): String? = null
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?): Array<String> = arrayOf("pc")
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?): String = "pc"
        override fun chooseEngineServerAlias(keyType: String?, issuers: Array<out Principal>?, engine: SSLEngine?): String = "pc"
        override fun getCertificateChain(alias: String?): Array<X509Certificate> = arrayOf(cert)
        override fun getPrivateKey(alias: String?): PrivateKey = key
    }

    /** Accepts any client certificate chain, as the real PC does (trust is decided per request). */
    private class ClientCertTrust(private val reject: Boolean) : X509ExtendedTrustManager() {
        private fun check() {
            if (reject) throw CertificateException("test server refuses client certificates")
        }

        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) = check()
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) = check()
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) = check()
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) = Unit
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) = Unit
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }
}
