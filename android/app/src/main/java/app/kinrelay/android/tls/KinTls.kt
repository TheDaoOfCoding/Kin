package app.kinrelay.android.tls

import app.kinrelay.android.identity.Identity
import okhttp3.ConnectionSpec
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.TlsVersion
import java.util.concurrent.TimeUnit
import javax.net.SocketFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocketFactory

/** TLS 1.3 with our client certificate and a pin on the PC's key. One instance per paired (or pairing) PC. */
class KinTls(identity: Identity, pinnedDeviceId: String) {
    private val trustManager = PinnedTrustManager(pinnedDeviceId)

    val sslSocketFactory: SSLSocketFactory = SSLContext.getInstance("TLSv1.3").run {
        init(arrayOf(ClientKeyManager(identity)), arrayOf(trustManager), null)
        socketFactory
    }

    /**
     * The only HTTP client of the app. [socketFactory] binds sockets to a specific network.
     * The hostname verifier accepts any name because the pin has already been enforced by the
     * trust manager (the PC is reached by IP address, with a self-signed certificate); it exists
     * only on this client and nothing else in the app uses it.
     */
    fun httpClient(socketFactory: SocketFactory = SocketFactory.getDefault()): OkHttpClient =
        OkHttpClient.Builder()
            .sslSocketFactory(sslSocketFactory, trustManager)
            .hostnameVerifier { _, _ -> true }
            .connectionSpecs(listOf(TLS_1_3_ONLY))
            .protocols(listOf(Protocol.HTTP_1_1))
            .socketFactory(socketFactory)
            .retryOnConnectionFailure(false)
            .connectTimeout(CONNECT_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .callTimeout(CALL_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .build()

    private companion object {
        val TLS_1_3_ONLY: ConnectionSpec = ConnectionSpec.Builder(ConnectionSpec.RESTRICTED_TLS)
            .tlsVersions(TlsVersion.TLS_1_3)
            .build()
        const val CONNECT_TIMEOUT_SECONDS = 5L
        const val CALL_TIMEOUT_SECONDS = 15L
    }
}
