package app.kinrelay.android.link

import android.content.Context
import android.content.Intent
import android.os.SystemClock
import app.kinrelay.android.AppGraph
import app.kinrelay.android.transfer.PeerInfo
import app.kinrelay.android.transfer.TransferLink
import app.kinrelay.android.ui.UiText
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.util.concurrent.atomic.AtomicInteger

/** How the UI and the debug hooks talk to the running service, which may or may not exist. */
class LinkController(private val graph: AppGraph) : OutgoingLink, TransferLink {
    @Volatile var runtime: LinkRuntime? = null

    val state = MutableStateFlow<LinkState>(LinkState.Stopped)

    private val waiting = AtomicInteger()

    /** Something (a transfer, a send) is waiting for a connection right now: the reconnect loop keeps its steps short ([LinkLoop]). */
    val transferWaiting: Boolean get() = waiting.get() > 0

    private val mutableLastPing = MutableStateFlow<PingResult?>(null)
    val lastPing: StateFlow<PingResult?> = mutableLastPing

    /** A PC was just paired. A running service reconnects to it instead of the PC that was paired before. */
    fun devicePaired() {
        runtime?.reloadPairedDevice()
    }

    /** The user did something: retry the connection now. */
    fun userAction() {
        runtime?.userAction()
    }

    /** Starts the service if a PC is paired. Only call this while the app is visible or from a boot broadcast. */
    suspend fun ensureRunning(context: Context) {
        if (graph.pairedDevice() != null) LinkService.start(context)
    }

    suspend fun ping(): PingResult {
        userAction()
        val session = runtime?.currentSession
        val result = session?.ping() ?: PingResult.NoConnection
        graph.log.i("link", "ping result: $result")
        mutableLastPing.value = result
        if (result is PingResult.Rtt) graph.settings.setLastRtt(result.millis)
        return result
    }

    /** The current connection, if it is ready and goes to the PC [peerDeviceId] (right after a re-pair the old one is briefly still current). */
    private fun readySession(peerDeviceId: String): LinkConnection? = runtime?.currentSession.readyFor(peerDeviceId)

    /**
     * A connection for a send. If there is none, it starts the service if needed, asks the loop to try now,
     * and waits up to [waitMillis]. It gives up early when waiting can't help (no permission, the PC doesn't
     * trust this phone, versions differ).
     */
    override suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection? = connection(waitMillis, kick = true, peerDeviceId)

    /** [await], but with [kick] false it only waits (a transfer waiting for the PC to come back must not reset the backoff). */
    override suspend fun connection(waitMillis: Long, kick: Boolean, peerDeviceId: String): LinkConnection? {
        readySession(peerDeviceId)?.let { return it }
        if (graph.pairedDevice() == null) return null
        if (kick) {
            ensureRunning(graph.appContext)
            userAction()
        }
        val deadline = SystemClock.elapsedRealtime() + waitMillis
        waiting.incrementAndGet()
        try {
            // A loop that sleeps a long backoff step (earned while nothing waited) tries now; it then retries every few seconds, not minutes.
            runtime?.workIsWaiting()
            while (true) {
                readySession(peerDeviceId)?.let { return it }
                val state = state.value
                val hopeless = state == LinkState.PermissionNeeded || state == LinkState.PairAgain || state == LinkState.UpdateApp
                if (hopeless || SystemClock.elapsedRealtime() >= deadline) return null
                delay(POLL_MILLIS)
            }
        } finally {
            waiting.decrementAndGet()
        }
    }

    override fun unreachableReason(): String = UiText.unreachable(state.value)


    override suspend fun peer(): PeerInfo? = graph.pairedDevice()?.let { PeerInfo(it.deviceId, it.name) }

    /** Removes the PC here first, then tells it if connected (protocol.md, "Unpair"), then stops the service. */
    suspend fun unpair(context: Context) {
        val device = graph.pairedDevice() ?: return
        graph.log.i("link", "unpairing from '${device.name}'")
        graph.database.devices().delete(device.deviceId)
        // Transfers with this PC end here: nothing of them may go to a PC that is paired later.
        graph.peerGone(device.deviceId)
        runtime?.currentSession?.takeIf { it.peerDeviceId == device.deviceId }?.sendUnpairAndClose()
        context.stopService(Intent(context, LinkService::class.java))
        mutableLastPing.value = null
    }

    private companion object {
        const val POLL_MILLIS = 100L
    }
}

/** This connection if it is ready and goes to the PC [peerDeviceId]; null otherwise. A connection to another PC is never used for it. */
fun LinkConnection?.readyFor(peerDeviceId: String): LinkConnection? = this?.takeIf { it.isReady && it.peerDeviceId == peerDeviceId }
