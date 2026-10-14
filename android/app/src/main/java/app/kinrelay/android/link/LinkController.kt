package app.kinrelay.android.link

import android.content.Context
import android.content.Intent
import app.kinrelay.android.AppGraph
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** How the UI and the debug hooks talk to the running service, which may or may not exist. */
class LinkController(private val graph: AppGraph) {
    @Volatile var runtime: LinkRuntime? = null

    val state = MutableStateFlow<LinkState>(LinkState.Stopped)

    private val mutableLastPing = MutableStateFlow<PingResult?>(null)
    val lastPing: StateFlow<PingResult?> = mutableLastPing

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

    /** Removes the PC here first, then tells it if connected (protocol.md, "Unpair"), then stops the service. */
    suspend fun unpair(context: Context) {
        val device = graph.pairedDevice() ?: return
        graph.log.i("link", "unpairing from '${device.name}'")
        graph.database.devices().delete(device.deviceId)
        runtime?.currentSession?.sendUnpairAndClose()
        context.stopService(Intent(context, LinkService::class.java))
        mutableLastPing.value = null
    }
}
