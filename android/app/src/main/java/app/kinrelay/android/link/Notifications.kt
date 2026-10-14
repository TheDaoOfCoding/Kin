package app.kinrelay.android.link

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R

/** The service notification and the "Ping from <name>" notification. */
class Notifications(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    init {
        manager.createNotificationChannel(
            NotificationChannel(LINK_CHANNEL, "Connection status", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shows whether your PC is connected"
                setShowBadge(false)
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(PING_CHANNEL, "Pings", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "A ping sent from your PC"
            }
        )
    }

    fun link(text: String): Notification =
        NotificationCompat.Builder(context, LINK_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("Kin")
            .setContentText(text)
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setContentIntent(openApp())
            .build()

    fun updateLink(text: String) {
        if (canPost()) manager.notify(LINK_ID, link(text))
    }

    fun pingFrom(name: String) {
        if (!canPost()) return
        val notification = NotificationCompat.Builder(context, PING_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("Ping from $name")
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(PING_ID, notification)
    }

    private fun canPost() = Build.VERSION.SDK_INT < 33 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    companion object {
        const val LINK_ID = 1
        private const val PING_ID = 2
        private const val LINK_CHANNEL = "link"
        private const val PING_CHANNEL = "pings"
    }
}
