package app.kinrelay.android.link

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.SystemClock
import app.kinrelay.android.AppGraph
import app.kinrelay.android.Permissions
import app.kinrelay.android.clipboard.AndroidClipboard
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.CloseAction
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.tls.KinTls
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * What the foreground service runs: it watches for Wi-Fi, and while there is one it keeps a
 * [LinkLoop] on that network. When the last Wi-Fi network is lost the loop is closed cleanly and nothing
 * is tried (there is no bridge in the MVP, so mobile data is never used). With two Wi-Fi networks, losing
 * one moves the loop to the other ([WifiTracker]). Without the Nearby devices permission (Android 17) no
 * connection is attempted and the state says so.
 */
class LinkRuntime(
    context: Context,
    private val graph: AppGraph,
    private val scope: CoroutineScope,
    private val notifications: Notifications,
    private val onStopRequested: () -> Unit,
    private val permissionGranted: () -> Boolean = { Permissions.localNetworkGranted(context) },
) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val mdns = MdnsLookup(context, graph.log)
    private val clock = Clock { SystemClock.elapsedRealtime() }
    private val incoming = IncomingHandler(AndroidClipboard(context), notifications, graph.clipboardGuard, graph.log)
    private val lock = Mutex()
    private val wifi = WifiTracker<Network>()
    private var network: Network? = null
    private var loop: LinkLoop? = null
    private var stateJob: Job? = null
    private val mutableState = MutableStateFlow<LinkState>(LinkState.NotOnWifi)
    val state: StateFlow<LinkState> = mutableState

    val currentSession: LinkConnection? get() = loop?.currentSession

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            graph.log.i("link", "Wi-Fi available: $network")
            scope.launch {
                lock.withLock {
                    wifi.available(network)
                    switchTo(network)
                }
            }
        }

        override fun onLost(network: Network) {
            scope.launch {
                lock.withLock {
                    val lost = wifi.lost(network)
                    if (!lost.wasCurrent) return@withLock
                    graph.log.i("link", "Wi-Fi lost: $network")
                    stopLoop("Wi-Fi lost")
                    this@LinkRuntime.network = null
                    val next = lost.next
                    if (next != null) {
                        graph.log.i("link", "another Wi-Fi network is still up: $next")
                        switchTo(next)
                    } else {
                        setState(LinkState.NotOnWifi)
                    }
                }
            }
        }
    }

    fun start() {
        val request = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build()
        connectivity.registerNetworkCallback(request, callback)
    }

    fun stop() {
        runCatching { connectivity.unregisterNetworkCallback(callback) }
        stopLoop("service stopping")
        setState(LinkState.Stopped)
    }

    /**
     * A PC was paired while the service runs. The loop was built for the PC that was paired before (its ID is
     * pinned in its TLS setup), so it is rebuilt for the new one.
     */
    fun reloadPairedDevice() {
        scope.launch { lock.withLock { network?.let { switchTo(it) } } }
    }

    fun userAction() {
        val running = loop
        if (running != null) {
            running.userAction()
        } else if (mutableState.value == LinkState.PermissionNeeded) {
            // The user may have allowed it in the settings: look again.
            scope.launch { lock.withLock { network?.let { if (permissionGranted()) switchTo(it) } } }
        }
    }

    private suspend fun switchTo(newNetwork: Network) {
        stopLoop("network changed")
        network = newNetwork
        if (!permissionGranted()) {
            graph.log.w("link", "the Nearby devices permission is missing; not connecting")
            setState(LinkState.PermissionNeeded)
            return
        }
        val device = graph.pairedDevice() ?: run { onStopRequested(); return }
        val identity = graph.identity()
        val shortId = DeviceIds.shortIdOfDeviceId(device.deviceId) ?: return
        val environment = DefaultLinkEnvironment(
            tls = KinTls(identity, device.deviceId),
            socketFactory = newNetwork.socketFactory,
            discovery = mdns::discover,
            local = LocalHello(graph.deviceInfo.name, "android", graph.deviceInfo.appVersion),
            dedup = graph.dedup(),
            scope = scope,
            clock = clock,
            log = graph.log,
            wsPingIntervalSeconds = graph.settings.pingIntervalSeconds(),
        )
        val newLoop = LinkLoop(scope, shortId, environment, Host(device), clock, graph.log)
        loop = newLoop
        stateJob = scope.launch { newLoop.state.collect(::setState) }
        newLoop.start()
    }

    private fun stopLoop(reason: String) {
        stateJob?.cancel()
        loop?.stop(reason)
        loop = null
    }

    private fun setState(state: LinkState) {
        // A permission revoked while the service runs only shows up as connection attempts that time out.
        val shown = if (state is LinkState.Connecting && !permissionGranted()) LinkState.PermissionNeeded else state
        mutableState.value = shown
        graph.link.state.value = shown
    }

    private inner class Host(private val device: DeviceEntity) : LinkHost {
        override suspend fun lastAddresses(): List<PairingAddress> {
            val stored = graph.pairedDevice()?.lastAddresses ?: device.lastAddresses
            return stored.split(',').mapNotNull { entry ->
                val host = entry.substringBeforeLast(':', "")
                val port = entry.substringAfterLast(':', "").toIntOrNull()
                if (host.isEmpty() || port == null) null else PairingAddress(host, port)
            }
        }

        override suspend fun onConnected(address: PairingAddress, peer: Hello) {
            val devices = graph.database.devices()
            val others = lastAddresses().filter { it != address }
            devices.updateSeen(device.deviceId, System.currentTimeMillis(), (listOf(address) + others).joinToString(","))
            devices.updateName(device.deviceId, peer.name)
            graph.settings.countConnect()
            graph.log.i("link", "connected to '${peer.name}' via $address")
        }

        override suspend fun onDisconnected(end: SessionEnd, action: CloseAction) {
            if (end.helloAcceptedForMillis != null) {
                graph.settings.countDisconnect()
                graph.log.i("link", "disconnected: ${end.reason} (code ${end.closeCode}) after ${end.helloAcceptedForMillis} ms")
            }
        }

        override fun onPingReceived(fromName: String) {
            graph.log.i("link", "ping received from '$fromName'")
            notifications.pingFrom(fromName)
        }

        override suspend fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean) =
            incoming.onText(fromName, id, text, isUrl)

        override suspend fun onClipboardReceived(id: String, text: String) = incoming.onClipboard(text)

        override suspend fun onPeerUnpaired() {
            graph.log.i("link", "the PC unpaired this phone")
            graph.database.devices().delete(device.deviceId)
            onStopRequested()
        }
    }
}
