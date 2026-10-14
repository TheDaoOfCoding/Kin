package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.CloseRules
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.tls.AddressRacer
import app.kinrelay.android.tls.KinTls
import app.kinrelay.android.tls.RaceResult
import kotlinx.coroutines.CoroutineScope
import java.util.concurrent.TimeUnit
import javax.net.SocketFactory

/** The real [LinkEnvironment]: sockets from [socketFactory] (bound to one Wi-Fi network on Android). */
class DefaultLinkEnvironment(
    tls: KinTls,
    private val socketFactory: SocketFactory,
    private val discovery: suspend (shortId: String, timeoutMillis: Long) -> List<PairingAddress>,
    private val local: LocalHello,
    private val dedup: DedupMemory,
    private val scope: CoroutineScope,
    private val clock: Clock,
    private val log: KinLog,
    wsPingIntervalSeconds: Int,
    private val helloTimeoutMillis: Long = CloseRules.HELLO_TIMEOUT_MILLIS,
) : LinkEnvironment {
    private val racer = AddressRacer(tls, socketFactory, log)
    private val client = tls.httpClient(socketFactory).newBuilder()
        .callTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(wsPingIntervalSeconds.toLong(), TimeUnit.SECONDS)
        .build()

    override suspend fun race(addresses: List<PairingAddress>, timeoutMillis: Long): RaceResult =
        racer.race(addresses, timeoutMillis)

    override suspend fun discover(shortId: String, timeoutMillis: Long): List<PairingAddress> =
        discovery(shortId, timeoutMillis)

    override fun openSession(address: PairingAddress, listener: SessionListener): LinkConnection =
        LinkSession(client, address, local, dedup, scope, clock, log, listener, helloTimeoutMillis)
}
