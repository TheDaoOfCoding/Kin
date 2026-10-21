package app.kinrelay.android.alerts

import android.os.SystemClock
import app.kinrelay.android.AppGraph
import app.kinrelay.android.link.Clock

/**
 * Alerts as the rest of the app sees them (2B.5): one per process, made by [AppGraph]. It owns the phone's alert bucket, the notification and the plain [AlertInbound] over them. It starts nothing
 * by itself (no `LinkService` start, no timer, no polling): an alert is one frame on the open socket and one notification.
 */
class AlertHub(private val graph: AppGraph) {
    /** The phone's bucket: 10 alerts per 60 s, per process (the receiver asks it for each new valid alert). */
    val budget = AlertBudget(Clock { SystemClock.elapsedRealtime() })

    private val notifications by lazy { AlertNotifications(graph.appContext) }

    val inbound = AlertInbound(
        object : AlertPorts {
            override suspend fun record(id: String, pcName: String, alert: AlertMessage) {
                val peer = graph.pairedDevice() ?: return
                graph.textOutbox.recordIncomingAlert(peer.deviceId, pcName, id, alert.text, alert.title)
            }

            override fun show(pcName: String, alert: AlertMessage): Boolean = notifications.show(pcName, alert)
        },
        paused = { graph.pause.paused() },
        onQuiet = { graph.pause.noteQuietArrival() },
        log = graph.log,
    )

    suspend fun onAlert(id: String, pcName: String, alert: AlertMessage) = inbound.onAlert(id, pcName, alert)

    /** The PC's low battery (decided by [app.kinrelay.android.presence.PcBatteryNudge], which already honoured the phone's pause and "show the PC's status"). */
    fun showPcBattery(pcName: String, level: Int) = notifications.showPcBattery(pcName, level)

    /** The user opened the app or swiped the notification away. */
    fun seen() = notifications.seen()

    /** The debug state (`ALERT_STATE`): counts only. */
    fun debugLine(): String = inbound.stats.line()
}
