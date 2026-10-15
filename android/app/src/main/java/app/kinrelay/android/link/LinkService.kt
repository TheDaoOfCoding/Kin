package app.kinrelay.android.link

import android.app.Service
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
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
import kotlinx.coroutines.withContext

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
        val notifications = graph.notifications
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
            // Same single thread as the "stopped" marker of the previous run, so the two can't swap places.
            when (withContext(graph.serviceBookkeeping) { graph.settings.markServiceStarted(bootCount) }) {
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
            val linkRuntime = LinkRuntime(this@LinkService, graph, scope, notifications, onStopRequested = { stopSelf() })
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
        // Not on this (main) thread: the app's own scope finishes the bookkeeping after the service is gone.
        graph.scope.launch(graph.serviceBookkeeping) {
            graph.settings.markServiceStopped()
            graph.log.i("service", "service stopped")
        }
        super.onDestroy()
    }

    private fun describe(state: LinkState, pcName: String): String = when (state) {
        is LinkState.Connected -> "Connected to $pcName"
        is LinkState.Connecting, LinkState.Stopped -> "Looking for $pcName"
        LinkState.NotOnWifi -> "Not on Wi-Fi"
        LinkState.Replaced -> "$pcName connected from elsewhere. Open Kin to reconnect"
        LinkState.UpdateApp -> "Update the app: versions don't match"
        LinkState.PairAgain -> "$pcName no longer trusts this phone. Pair again"
        LinkState.PermissionNeeded -> "Nearby devices permission needed"
    }

    companion object {
        /**
         * Starts the service (a visible activity may) and gives it read access to [uris] with `FLAG_GRANT_READ_URI_PERMISSION`.
         * A share only grants access to the activity that receives it; the share activity finishes at once, and a grant held by the
         * service outlives it. The service itself ignores the payload: the grant is the point.
         */
        fun startWithGrant(context: Context, uris: List<Uri>) {
            if (uris.isEmpty()) return start(context)
            val intent = Intent(context, LinkService::class.java).apply {
                clipData = ClipData.newRawUri("shared files", uris[0]).also { clip -> uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) } }
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (e: Exception) {
                context.graph.log.e("service", "start with files refused: ${e.javaClass.simpleName}: ${e.message}")
            }
        }

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
