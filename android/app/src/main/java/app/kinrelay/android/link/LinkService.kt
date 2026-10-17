package app.kinrelay.android.link

import android.app.Service
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.kinrelay.android.data.KinSettings
import app.kinrelay.android.graph
import app.kinrelay.android.notify.TrailingThrottle
import app.kinrelay.android.pause.PauseSupport
import app.kinrelay.android.presence.PeerPauseText
import app.kinrelay.android.presence.PresenceText
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
    private val mainHandler = Handler(Looper.getMainLooper())
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
            // A crash in the middle of a ring must not leave the alarm volume at maximum: put it back before anything else (2B.4, design 5.4).
            graph.ring.restoreAtStart()
            val linkRuntime = LinkRuntime(this@LinkService, graph, scope, notifications, onStopRequested = { stopSelf() })
            runtime = linkRuntime
            graph.link.runtime = linkRuntime
            linkRuntime.start()
            graph.shots.reevaluate()
            // The persistent notification (2B.4): what the PC says about itself (its pause, a laptop's battery) and this phone's own pause. A **battery-only** change is posted at most once
            // per 30 s (the trailing throttle); a change of state or of a pause is posted at once.
            var latest = ""
            var lastFull: String? = null
            var lastStructural: String? = null
            val throttle = TrailingThrottle(
                intervalMillis = BATTERY_NOTIFICATION_MILLIS,
                now = { SystemClock.elapsedRealtime() },
                schedule = { delay, task -> mainHandler.postDelayed(task, delay) },
                action = { notifications.updateLink(latest) },
            )
            combine(linkRuntime.state, graph.database.devices().observeAll(), graph.presence.view, graph.pause.view) { state, devices, peer, _ ->
                // What the pause view says is "look again"; the notification shows the wall clock now (a pause that ended during deep sleep is not shown as running).
                val pause = graph.pause.viewNow()
                val base = describe(state, devices.firstOrNull()?.name ?: device.name)
                val connected = state is LinkState.Connected
                val peerPause = if (connected && peer.peerPaused) {
                    PeerPauseText(peer.peerPausedUntilResumed, peer.peerPausedUntilWall?.let { PauseSupport.time(this@LinkService, it) })
                } else {
                    null
                }
                val own = if (pause.paused) " · Paused " + PauseSupport.sentence(this@LinkService, pause.state).replaceFirstChar { it.lowercase() } else ""
                val battery = if (connected) peer.battery else null
                Pair(
                    PresenceText.connected(base, null, peer.showOn, peerPause) + own,
                    PresenceText.connected(base, battery, peer.showOn, peerPause) + own,
                )
            }.collect { (structural, full) ->
                if (full == lastFull) return@collect
                lastFull = full
                latest = full
                if (structural != lastStructural) {
                    lastStructural = structural
                    notifications.updateLink(full)
                } else {
                    throttle.request()
                }
            }
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
        /** A battery-only change of the PC is shown in the persistent notification at most this often (design 5.2). */
        private const val BATTERY_NOTIFICATION_MILLIS = 30_000L

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
