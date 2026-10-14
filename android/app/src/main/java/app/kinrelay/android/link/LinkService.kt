package app.kinrelay.android.link

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.provider.Settings
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.kinrelay.android.data.KinSettings
import app.kinrelay.android.graph
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/**
 * The foreground service (type `connectedDevice`) that holds the connection to the paired PC.
 * Everything it does is in [LinkRuntime]; this class is the Android lifecycle around it.
 */
class LinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var runtime: LinkRuntime? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val graph = applicationContext.graph
        val notifications = Notifications(this)
        try {
            ServiceCompat.startForeground(
                this, Notifications.LINK_ID, notifications.link("Starting…"),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } catch (e: Exception) {
            graph.log.e("service", "could not enter the foreground: ${e.javaClass.simpleName}: ${e.message}")
            stopSelf()
            return
        }
        scope.launch {
            val bootCount = Settings.Global.getInt(contentResolver, Settings.Global.BOOT_COUNT, -1)
            when (graph.settings.markServiceStarted(bootCount)) {
                KinSettings.PreviousRun.CLEAN -> graph.log.i("service", "service started")
                KinSettings.PreviousRun.REBOOT -> graph.log.i("service", "service started after a reboot")
                KinSettings.PreviousRun.KILLED -> {
                    graph.log.i("service", "service started")
                    graph.log.w("service", "kill detected: the previous run ended without a clean stop")
                }
            }
            val device = graph.pairedDevice()
            if (device == null) {
                graph.log.i("service", "no paired PC; stopping")
                stopSelf()
                return@launch
            }
            val linkRuntime = LinkRuntime(this@LinkService, graph, scope, notifications) { stopSelf() }
            runtime = linkRuntime
            graph.link.runtime = linkRuntime
            linkRuntime.start()
            combine(linkRuntime.state, graph.database.devices().observeAll()) { state, devices ->
                describe(state, devices.firstOrNull()?.name ?: device.name)
            }.collect { notifications.updateLink(it) }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        val graph = applicationContext.graph
        runtime?.stop()
        runtime = null
        graph.link.runtime = null
        graph.link.state.value = LinkState.Stopped
        scope.cancel()
        runBlocking(Dispatchers.IO) { graph.settings.markServiceStopped() }
        graph.log.i("service", "service stopped")
        super.onDestroy()
    }

    private fun describe(state: LinkState, pcName: String): String = when (state) {
        is LinkState.Connected -> "Connected to $pcName"
        is LinkState.Connecting, LinkState.Stopped -> "Looking for $pcName"
        LinkState.NotOnWifi -> "Not on Wi-Fi"
        LinkState.Replaced -> "$pcName connected from elsewhere. Open Kin to reconnect"
        LinkState.UpdateApp -> "Update the app: versions don't match"
        LinkState.PairAgain -> "$pcName no longer trusts this phone. Pair again"
    }

    companion object {
        /** Starts the service. Allowed from a visible activity and from a boot broadcast. */
        fun start(context: Context) {
            try {
                ContextCompat.startForegroundService(context, Intent(context, LinkService::class.java))
            } catch (e: Exception) {
                context.graph.log.e("service", "start refused: ${e.javaClass.simpleName}: ${e.message}")
            }
        }
    }
}
