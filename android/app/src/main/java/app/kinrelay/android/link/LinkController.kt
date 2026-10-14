package app.kinrelay.android.link

import android.content.Context
import android.content.Intent
import android.os.SystemClock
import app.kinrelay.android.AppGraph
import app.kinrelay.android.ui.UiText
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** How the UI and the debug hooks talk to the running service, which may or may not exist. */
class LinkController(private val graph: AppGraph) : OutgoingLink {
    @Volatile var runtime: LinkRuntime? = null

    val state = MutableStateFlow<LinkState>(LinkState.Stopped)

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

    private fun readySession(): LinkConnection? = runtime?.currentSession?.takeIf { it.isReady }

    /**
     * A connection for a send. If there is none, it starts the service if needed, asks the loop to try now,
     * and waits up to [waitMillis]. It gives up early when waiting can't help (no permission, the PC doesn't
     * trust this phone, versions differ).
     */
    override suspend fun await(waitMillis: Long): LinkConnection? {
        readySession()?.let { return it }
        if (graph.pairedDevice() == null) return null
        ensureRunning(graph.appContext)
        userAction()
        val deadline = SystemClock.elapsedRealtime() + waitMillis
        while (true) {
            readySession()?.let { return it }
            val state = state.value
            val hopeless = state == LinkState.PermissionNeeded || state == LinkState.PairAgain || state == LinkState.UpdateApp
            if (hopeless || SystemClock.elapsedRealtime() >= deadline) return null
            delay(POLL_MILLIS)
        }
    }

    override fun unreachableReason(): String = UiText.unreachable(state.value)

    override suspend fun peerName(): String? = graph.pairedDevice()?.name

    /** Removes the PC here first, then tells it if connected (protocol.md, "Unpair"), then stops the service. */
    suspend fun unpair(context: Context) {
        val device = graph.pairedDevice() ?: return
        graph.log.i("link", "unpairing from '${device.name}'")
        graph.database.devices().delete(device.deviceId)
        runtime?.currentSession?.sendUnpairAndClose()
        context.stopService(Intent(context, LinkService::class.java))
        mutableLastPing.value = null
    }

    private companion object {
        const val POLL_MILLIS = 100L
    }
}
