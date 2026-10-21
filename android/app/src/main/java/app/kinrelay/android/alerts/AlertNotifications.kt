package app.kinrelay.android.alerts

import android.Manifest
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.notify.TrailingThrottle
import app.kinrelay.android.presence.PcBatteryNudge
import app.kinrelay.android.transfer.TransferActionReceiver

/**
 * The notifications of 2B.5 on the `kin_alerts` channel ("From your PC", default importance):
 * - **Alerts** (design 4.4): title = the alert's title or the PC's name, text = the alert, sub text = the PC's name (and "· 3 h ago" when it waited more than 2 minutes). `VISIBILITY_PRIVATE`
 *   with a public version "Alert from <PC>". **Several alerts share one updating notification** (at most twice a second, the last update always posted), never one each (Android drops new
 *   notifications beyond 50 per app). A tap opens History. **No buttons**, no clipboard, nothing opened.
 * - **The PC's low battery**: one notification "Desk PC battery is 14%" (its own slot, the same channel).
 * Android's own Do Not Disturb applies as to any notification: Kin never overrides it.
 */
class AlertNotifications(context: Context) {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val arrivals = AlertArrivals()

    init {
        // Idempotent: the channel exists before the first post, whatever was made first.
        NotificationChannels.ensure(this.context)
    }

    private val poster = TrailingThrottle(
        intervalMillis = UPDATE_MILLIS,
        now = { SystemClock.elapsedRealtime() },
        schedule = { delay, task -> main.postDelayed(task, delay) },
        action = ::post,
    )

    fun show(pcName: String, alert: AlertMessage): Boolean {
        if (!canPost()) return false
        arrivals.add(pcName, AlertArrivals.Entry(alert.title, alert.text, alert.waitedMs))
        poster.request()
        return true
    }

    /** The user opened the app or swiped the notification away: the next alert starts a new one. */
    fun seen() {
        if (!arrivals.isEmpty()) arrivals.reset()
    }

    fun showPcBattery(pcName: String, level: Int) {
        if (!canPost()) return
        val title = PcBatteryNudge.title(pcName, level)
        val notification = NotificationCompat.Builder(context, NotificationChannels.ALERTS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setContentText(PcBatteryNudge.TEXT)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .setContentIntent(openHistory())
            .build()
        manager.notify(TAG, BATTERY_ID, notification)
    }

    private fun post() {
        val state = arrivals.state()
        if (state.count == 0 || !canPost()) return
        val title = AlertArrivals.title(state)
        val builder = NotificationCompat.Builder(context, NotificationChannels.ALERTS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setAutoCancel(true)
            .setOnlyAlertOnce(state.count > 1)
            .setContentIntent(openHistory())
            .setDeleteIntent(dismissed())
            .setNumber(state.count)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            // The text is private (a script's output can hold anything): the lock screen shows only who it is from.
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(
                NotificationCompat.Builder(context, NotificationChannels.ALERTS).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle(AlertArrivals.publicTitle(state)).build(),
            )
        if (state.count == 1) {
            val entry = state.entries.last()
            builder.setContentText(entry.text).setStyle(NotificationCompat.BigTextStyle().bigText(entry.text)).setSubText(AlertArrivals.subText(state.pcName, entry.waitedMs))
        } else {
            val style = NotificationCompat.InboxStyle().setBigContentTitle(title)
            state.entries.forEach { style.addLine(AlertArrivals.line(it)) }
            if (state.count > state.entries.size) style.setSummaryText("and ${state.count - state.entries.size} earlier")
            builder.setStyle(style).setContentText(AlertArrivals.line(state.entries.last())).setSubText(state.pcName)
        }
        manager.notify(TAG, ALERTS_ID, builder.build())
    }

    private fun canPost() = Build.VERSION.SDK_INT < 33 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    /** A tap opens the app on the History tab (the alert is a row there). */
    private fun openHistory(): PendingIntent = PendingIntent.getActivity(
        context, REQUEST_OPEN_HISTORY,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP).putExtra(MainActivity.EXTRA_TAB, MainActivity.TAB_HISTORY),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun dismissed(): PendingIntent = PendingIntent.getBroadcast(
        context, REQUEST_DISMISSED, Intent(context, TransferActionReceiver::class.java).setAction(TransferActionReceiver.ACTION_ALERTS_DISMISSED),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private companion object {
        /** A tag of its own: these two never collide with the untagged IDs of the other notifications. */
        const val TAG = "kin_alerts"
        const val ALERTS_ID = 1
        const val BATTERY_ID = 2
        const val REQUEST_OPEN_HISTORY = 51
        const val REQUEST_DISMISSED = 52
        const val UPDATE_MILLIS = 500L
    }
}
