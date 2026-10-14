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
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.net.toUri
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R
import app.kinrelay.android.clipboard.ClipboardSendActivity

/**
 * The service notification (with its "Send clipboard" action), "Ping from <name>", "Text from <name>",
 * send failures, and the toasts. It is the real [Alerts].
 */
class Notifications(context: Context) : Alerts {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val main = Handler(Looper.getMainLooper())

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
        manager.createNotificationChannel(
            NotificationChannel(TEXT_CHANNEL, "Text from your PC", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "Text and links sent from your PC. They are also copied to the clipboard"
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(PROBLEM_CHANNEL, "Send problems", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "Something you sent to your PC didn't arrive"
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
            .addAction(R.drawable.ic_stat_kin, "Send clipboard", sendClipboard())
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

    override fun textFrom(pcName: String, messageId: String, text: String, openUrl: String?) {
        if (!canPost()) return
        val title = "Text from $pcName"
        val builder = NotificationCompat.Builder(context, TEXT_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setContentText(preview(text, SHORT_PREVIEW))
            .setStyle(NotificationCompat.BigTextStyle().bigText(preview(text, LONG_PREVIEW, singleLine = false)))
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            // The text is private: the lock screen shows only the title.
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(
                NotificationCompat.Builder(context, TEXT_CHANNEL).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle(title).build()
            )
        if (openUrl != null) {
            val view = Intent(Intent.ACTION_VIEW, openUrl.toUri()).addCategory(Intent.CATEGORY_BROWSABLE).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            val pending = PendingIntent.getActivity(
                context, messageId.hashCode(), view, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(R.drawable.ic_stat_kin, "Open", pending)
        }
        manager.notify(messageId, TEXT_ID, builder.build())
    }

    override fun sent(pcName: String?, arrivalMillis: Long) = message("Sent to ${pcName ?: "your PC"}")

    override fun sendFailed(title: String, reason: String) {
        if (!canPost()) {
            message("$title. $reason")
            return
        }
        val notification = NotificationCompat.Builder(context, PROBLEM_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setContentText(reason)
            .setStyle(NotificationCompat.BigTextStyle().bigText(reason))
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(PROBLEM_ID, notification)
    }

    override fun message(text: String) {
        main.post { Toast.makeText(context, text, Toast.LENGTH_SHORT).show() }
    }

    private fun canPost() = Build.VERSION.SDK_INT < 33 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun sendClipboard(): PendingIntent = PendingIntent.getActivity(
        context, 1, ClipboardSendActivity.intent(context),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    companion object {
        const val LINK_ID = 1
        private const val PING_ID = 2
        private const val TEXT_ID = 3
        private const val PROBLEM_ID = 4
        private const val LINK_CHANNEL = "link"
        private const val PING_CHANNEL = "pings"
        private const val TEXT_CHANNEL = "text"
        private const val PROBLEM_CHANNEL = "problems"
        private const val SHORT_PREVIEW = 100
        private const val LONG_PREVIEW = 600

        /** A preview of at most [limit] characters (cut on a code point), with line breaks turned into spaces if [singleLine]. */
        fun preview(text: String, limit: Int, singleLine: Boolean = true): String {
            val flat = if (singleLine) text.replace(Regex("\\s+"), " ").trim() else text.trim()
            if (flat.codePointCount(0, flat.length) <= limit) return flat
            return flat.substring(0, flat.offsetByCodePoints(0, limit)) + "…"
        }
    }
}
