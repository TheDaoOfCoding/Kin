package app.kinrelay.android.link

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import java.net.Inet4Address
import kotlin.coroutines.resume

/**
 * Looks the PC up by its mDNS service `_kinrelay._tcp` (protocol.md, "Discovery") for a short
 * time. Only used while reconnecting, never while connected.
 */
class MdnsLookup(context: Context, private val log: KinLog) {
    private val nsd = context.applicationContext.getSystemService(NsdManager::class.java)

    suspend fun discover(shortId: String, timeoutMillis: Long): List<PairingAddress> {
        val candidates = Channel<NsdServiceInfo>(Channel.UNLIMITED)
        val listener = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(info: NsdServiceInfo) {
                candidates.trySend(info)
            }

            override fun onServiceLost(info: NsdServiceInfo) = Unit
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                log.w("mdns", "discovery failed to start: $errorCode")
                candidates.close()
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }
        try {
            nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (e: RuntimeException) {
            log.w("mdns", "discovery unavailable: ${e.message}")
            return emptyList()
        }
        try {
            return withTimeoutOrNull(timeoutMillis) {
                val found = mutableListOf<PairingAddress>()
                for (candidate in candidates) {
                    val resolved = resolve(candidate) ?: continue
                    toAddress(resolved, shortId)?.let { found += it; break }
                }
                found
            } ?: emptyList()
        } finally {
            runCatching { nsd.stopServiceDiscovery(listener) }
        }
    }

    @Suppress("DEPRECATION") // still the one call that works from API 29; its replacement needs API 34
    private suspend fun resolve(info: NsdServiceInfo): NsdServiceInfo? = suspendCancellableCoroutine { continuation ->
        nsd.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                if (continuation.isActive) continuation.resume(null)
            }

            override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                if (continuation.isActive) continuation.resume(serviceInfo)
            }
        })
    }

    @Suppress("DEPRECATION")
    private fun toAddress(info: NsdServiceInfo, shortId: String): PairingAddress? {
        val id = info.attributes["id"]?.toString(Charsets.UTF_8)
        val version = info.attributes["v"]?.toString(Charsets.UTF_8)
        if (id != shortId || version != "1") return null
        val host = (info.host as? Inet4Address)?.hostAddress ?: return null
        return PairingAddress(host, info.port)
    }

    private companion object {
        const val SERVICE_TYPE = "_kinrelay._tcp"
    }
}
