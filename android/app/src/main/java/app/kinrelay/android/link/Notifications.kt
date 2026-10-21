package app.kinrelay.android.link

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.net.toUri
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R
import app.kinrelay.android.clipboard.ClipboardSendActivity
import app.kinrelay.android.links.AndroidLinks
import app.kinrelay.android.links.LinkNotifications
import app.kinrelay.android.links.LinkRules
import app.kinrelay.android.links.ReceivedLink
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.notify.NotificationLedger
import app.kinrelay.android.notify.TextArrivals
import app.kinrelay.android.notify.TrailingThrottle
import app.kinrelay.android.pause.PauseReceiver
import app.kinrelay.android.transfer.TransferActionReceiver

/**
 * The service notification (with its "Send clipboard" action), "Ping from <name>", the texts from the PC, send problems, and the toasts.
 * It is the real [Alerts].
 *
 * Texts from the PC share **one** notification that is updated ("3 texts from Desk PC"), at most twice a second, with the last update
 * always posted ([TrailingThrottle]): a burst of 60 texts is one notification, not 60 (Android drops new notifications beyond 50 per app).
 * Send problems keep at most [MAX_PROBLEMS] notifications ([NotificationLedger]), each under its own ID, on the arrivals channel (default
 * importance), because a "not sent" notice that makes no sound is a notice nobody sees.
 */
class Notifications(
    context: Context,
    /** This phone is paused (2B.4): the persistent notification's second action says "Resume" instead of "Pause 1 h". */
    private val paused: () -> Boolean = { false },
) : Alerts {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val arrivals = TextArrivals()
    private val problems = NotificationLedger(MAX_PROBLEMS)
    private val links = LinkNotifications(this.context, AndroidLinks(this.context))
    private var problemCounter = 0

    private val textPoster = TrailingThrottle(
        intervalMillis = TEXT_UPDATE_MILLIS,
        now = { SystemClock.elapsedRealtime() },
        schedule = { delay, task -> main.postDelayed(task, delay) },
        action = ::postTexts,
    )

    init {
        NotificationChannels.ensure(this.context)
    }

    fun link(text: String): Notification =
        NotificationCompat.Builder(context, NotificationChannels.LINK)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("Kin")
            .setContentText(text)
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setContentIntent(openApp())
            .addAction(R.drawable.ic_stat_kin, "Send clipboard", sendClipboard())
            // The second (and last) action (2B.4): pause for an hour, or resume while paused. Pausing is harmless, so it works on the lock screen; **resuming asks for the unlock** (it turns auto-send,
            // Do Not Disturb following and alerts back on). The Pause tile does the same (2B.5 review: one rule for both). The notification is rebuilt at every change of the pause, so the flag follows it.
            .addAction(
                paused().let { isPaused ->
                    NotificationCompat.Action.Builder(R.drawable.ic_stat_kin, if (isPaused) "Resume" else "Pause 1 h", togglePause()).setAuthenticationRequired(isPaused).build()
                },
            )
            .build()

    private fun togglePause(): PendingIntent = PendingIntent.getBroadcast(
        context, 4, Intent(context, PauseReceiver::class.java).setAction(PauseReceiver.ACTION_TOGGLE),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    fun updateLink(text: String) {
        if (canPost()) manager.notify(LINK_ID, link(text))
    }

    fun pingFrom(name: String) {
        if (!canPost()) return
        val notification = NotificationCompat.Builder(context, NotificationChannels.ARRIVALS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("Ping from $name")
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(PING_ID, notification)
    }

    override fun textFrom(pcName: String, messageId: String, text: String?, openUrl: String?, copyText: String?, sensitive: Boolean, copiedAlready: Boolean) {
        if (!canPost()) return
        arrivals.add(pcName, text?.let { preview(it, LONG_PREVIEW, singleLine = false) }, openUrl, TextArrivals.Latest(messageId, copyText, sensitive), copiedAlready)
        textPoster.request()
    }

    override fun linkFrom(pcName: String, messageId: String, link: ReceivedLink, lockScreenPreviews: Boolean) {
        if (!canPost()) return
        links.show(pcName, messageId, link, lockScreenPreviews)
    }

    /** A link was tapped (it went with the tap): the group summary forgets it. */
    fun linksSeen() = links.refresh()

    /** The newest text of the "Text from <PC>" notification, kept in memory only (a sensitive text is never stored anywhere else). */
    fun latestText(messageId: String): TextArrivals.Latest? = arrivals.latest(messageId)

    /** The user opened the app or swiped the notification away: the next text starts a new "Text from <PC>". */
    fun textsSeen() {
        if (!arrivals.isEmpty()) arrivals.reset()
    }

    private fun postTexts() {
        val state = arrivals.state()
        if (state.count == 0 || !canPost()) return
        val title = TextArrivals.title(state)
        val builder = NotificationCompat.Builder(context, NotificationChannels.ARRIVALS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setAutoCancel(true)
            .setOnlyAlertOnce(state.count > 1)
            .setContentIntent(openApp())
            .setDeleteIntent(dismissed())
            .setNumber(state.count)
            // The setting "Copy received text automatically" says whether the text is on the clipboard already.
            .also { if (state.copied) it.setSubText("Copied") }
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            // The text is private: the lock screen shows only the title.
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(
                NotificationCompat.Builder(context, NotificationChannels.ARRIVALS).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle(title).build()
            )
        val shown = state.lines
        if (state.count == 1) {
            // No text means sensitive text: the title is all there is.
            shown.firstOrNull()?.let { builder.setContentText(preview(it, SHORT_PREVIEW)).setStyle(NotificationCompat.BigTextStyle().bigText(it)) }
        } else {
            val style = NotificationCompat.InboxStyle().setBigContentTitle(title)
            shown.forEach { style.addLine(it?.let { line -> preview(line, SHORT_PREVIEW) } ?: TextArrivals.SENSITIVE_LINE) }
            if (state.count > shown.size) style.setSummaryText("and ${state.count - shown.size} earlier")
            builder.setStyle(style).setContentText(shown.lastOrNull()?.let { preview(it, SHORT_PREVIEW) } ?: TextArrivals.SENSITIVE_LINE)
        }
        // Copy: the newest text, by its message ID (the text itself is not in the PendingIntent, and a sensitive one is only in memory).
        state.latest?.takeIf { it.text != null }?.let { latest ->
            builder.addAction(R.drawable.ic_stat_kin, if (state.count > 1) "Copy latest" else "Copy", copyText(latest.id))
        }
        state.openUrl?.let { url ->
            val view = Intent(Intent.ACTION_VIEW, LinkRules.intentUrl(url).toUri()).addCategory(Intent.CATEGORY_BROWSABLE).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            val pending = PendingIntent.getActivity(
                context, url.hashCode(), view, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(R.drawable.ic_stat_kin, "Open", pending)
        }
        manager.notify(TEXT_ID, builder.build())
    }

    override fun sent(pcName: String?, arrivalMillis: Long) = message("Sent to ${pcName ?: "your PC"}")

    override fun queued(pcName: String?, reason: String, afterNoReply: Boolean) {
        val pc = pcName ?: "your PC"
        message(if (afterNoReply) "No reply from $pc yet. Saved: Kin sends it again when $pc is back." else "Saved. Kin sends it when $pc is back. $reason")
    }

    override fun sentLater(pcName: String?) = message("Sent to ${pcName ?: "your PC"}")

    override fun sendFailed(title: String, reason: String) {
        if (!canPost()) {
            message("$title. $reason")
            return
        }
        // Its own ID each time: a problem is never an update of an earlier one (and so never rate-limited away), and at most MAX_PROBLEMS stay.
        val key = "problem-${problemCounter++}"
        val evicted = synchronized(problems) {
            val live = runCatching { manager.activeNotifications.filter { it.id == PROBLEM_ID }.mapNotNull { it.tag }.toSet() }.getOrNull()
            if (live != null) problems.retainOnly(live)
            problems.claim(key, title)
        }
        evicted.forEach { manager.cancel(it, PROBLEM_ID) }
        // A problem ("Not sent: <PC> offline") needs to be seen: it goes on the arrivals channel (default importance, so it makes a sound and
        // can appear on screen). The low-importance transfers channel is for progress, and showed these in the shade only, in silence.
        val notification = NotificationCompat.Builder(context, NotificationChannels.ARRIVALS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setContentText(reason)
            .setStyle(NotificationCompat.BigTextStyle().bigText(reason))
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(key, PROBLEM_ID, notification)
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

    private fun dismissed(): PendingIntent = PendingIntent.getBroadcast(
        context, 2, Intent(context, TransferActionReceiver::class.java).setAction(TransferActionReceiver.ACTION_TEXTS_DISMISSED),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun copyText(messageId: String): PendingIntent = PendingIntent.getBroadcast(
        context, 3,
        Intent(context, TransferActionReceiver::class.java).setAction(TransferActionReceiver.ACTION_COPY_TEXT).putExtra(TransferActionReceiver.EXTRA_MESSAGE_ID, messageId),
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

        /** At most this many send-problem notifications stay up (the oldest goes first). */
        const val MAX_PROBLEMS = 5
        private const val TEXT_UPDATE_MILLIS = 500L
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
