package app.kinrelay.android.ring

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.kinrelay.android.R
import app.kinrelay.android.notify.NotificationChannels

/**
 * The Stop notification of a ring (design 5.4): channel `kin_ring` (high, no sound or vibration of its own), `CATEGORY_ALARM`, ongoing, a **Stop** action, and a full-screen intent to
 * [RingActivity] (`VISIBILITY_PUBLIC`: a stop button must be reachable on the lock screen; the text says only "<PC> is ringing your phone"). On Android 14+ the full-screen intent is limited to
 * calling and alarm apps (the system revokes it from a Play install), so when `canUseFullScreenIntent()` is false the notification still posts, as a heads-up with its Stop button for 60 s.
 * Ringing never depends on any of this: the sound is the service's.
 */
object RingNotification {
    private const val ID = 40

    /** A safety net only (the ring hides it when it stops): the 2-minute cap plus a margin, so a restarted ring never loses its Stop button to an early timeout. */
    private const val TIMEOUT_MILLIS = RingMachine.CONTINUOUS_CAP_MILLIS + 5_000L

    fun show(context: Context, pcName: String): Boolean {
        val manager = context.getSystemService(NotificationManager::class.java)
        NotificationChannels.ensure(context)
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return false
        val screen = PendingIntent.getActivity(
            context, 41, RingActivity.intent(context),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stop = PendingIntent.getBroadcast(
            context, 42, Intent(context, RingActionReceiver::class.java).setAction(RingActionReceiver.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val public = NotificationCompat.Builder(context, NotificationChannels.RING)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("$pcName is ringing your phone")
            .build()
        val notification = NotificationCompat.Builder(context, NotificationChannels.RING)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("$pcName is ringing your phone")
            .setContentText("Tap Stop to end the ring.")
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setPublicVersion(public)
            .setOngoing(true)
            // No setSilent(true): androidx puts such a notification in the muted "silent" group, which suppresses the heads-up and the full-screen intent. The kin_ring channel has no sound or vibration of its own.
            .setOnlyAlertOnce(true)
            .setTimeoutAfter(TIMEOUT_MILLIS)
            .setContentIntent(screen)
            .setFullScreenIntent(screen, true)
            .addAction(R.drawable.ic_stat_kin, "Stop", stop)
            .build()
        notification.flags = notification.flags or Notification.FLAG_NO_CLEAR
        return try {
            manager.notify(ID, notification)
            true
        } catch (_: SecurityException) {
            false
        }
    }

    fun hide(context: Context) {
        context.getSystemService(NotificationManager::class.java).cancel(ID)
        RingActivity.finishIfOpen()
    }
}
