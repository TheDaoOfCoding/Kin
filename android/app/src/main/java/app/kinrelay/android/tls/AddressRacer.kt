package app.kinrelay.android.tls

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.net.InetSocketAddress
import java.net.SocketTimeoutException
import java.util.concurrent.ConcurrentLinkedQueue
import javax.net.SocketFactory
import javax.net.ssl.SSLSocket

sealed interface RaceResult {
    data class Winner(val address: PairingAddress) : RaceResult
    data class Failed(val failures: Map<PairingAddress, Throwable>) : RaceResult {
        val pinMismatch: Boolean get() = failures.values.any { it.hasCause<PinMismatchException>() }
    }
}

internal inline fun <reified T : Throwable> Throwable.hasCause(): Boolean =
    generateSequence(this) { it.cause }.any { it is T }

/**
 * Tries every address at once with a real TLS 1.3 handshake (client certificate included) and
 * returns the first one whose server key matches the pin. Nothing is sent after the handshake,
 * so this never opens a second WebSocket on the PC.
 */
class AddressRacer(
    private val tls: KinTls,
    private val socketFactory: SocketFactory = SocketFactory.getDefault(),
    private val log: KinLog = KinLog.None,
) {
    suspend fun race(addresses: List<PairingAddress>, timeoutMillis: Long): RaceResult {
        if (addresses.isEmpty()) return RaceResult.Failed(emptyMap())
        val open = ConcurrentLinkedQueue<java.net.Socket>()
        val results = Channel<Pair<PairingAddress, Throwable?>>(addresses.size)
        return coroutineScope {
            addresses.forEach { address ->
                launch(Dispatchers.IO) {
                    results.send(address to runCatching { probe(address, timeoutMillis, open) }.exceptionOrNull())
                }
            }
            val failures = LinkedHashMap<PairingAddress, Throwable>()
            val winner = withTimeoutOrNull(timeoutMillis) {
                repeat(addresses.size) {
                    val (address, error) = results.receive()
                    if (error == null) return@withTimeoutOrNull address
                    failures[address] = error
                }
                null
            }
            // Closing the sockets unblocks probes that are still connecting, so the scope ends at once.
            open.forEach { runCatching { it.close() } }
            if (winner != null) {
                RaceResult.Winner(winner)
            } else {
                addresses.filter { it !in failures }.forEach { failures[it] = SocketTimeoutException("no answer in $timeoutMillis ms") }
                RaceResult.Failed(failures)
            }
        }
    }

    private fun probe(address: PairingAddress, timeoutMillis: Long, open: ConcurrentLinkedQueue<java.net.Socket>) {
        val started = System.nanoTime()
        fun elapsed() = (System.nanoTime() - started) / 1_000_000
        val plain = socketFactory.createSocket()
        open.add(plain)
        plain.connect(InetSocketAddress(address.host, address.port), timeoutMillis.toInt())
        log.i("tls", "$address: TCP connected after ${elapsed()} ms")
        val ssl = tls.sslSocketFactory.createSocket(plain, address.host, address.port, true) as SSLSocket
        open.add(ssl)
        ssl.enabledProtocols = arrayOf("TLSv1.3")
        ssl.soTimeout = timeoutMillis.toInt()
        ssl.startHandshake()
        log.i("tls", "$address: TLS ${ssl.session.protocol} handshake done after ${elapsed()} ms")
        ssl.close()
    }
}
