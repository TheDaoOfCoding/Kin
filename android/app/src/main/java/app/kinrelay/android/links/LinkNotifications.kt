package app.kinrelay.android.links

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.core.app.NotificationCompat
import androidx.core.net.toUri
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.notify.NotificationLedger
import app.kinrelay.android.notify.TrailingThrottle
import app.kinrelay.android.transfer.TransferActionReceiver

/**
 * The notifications of links from the PC (milestone 2B.1). One notification per link, on the *arrivals* channel:
 * - the page title, with the real address under it ([LinkNotificationContent]);
 * - a tap opens the link at once, in the app that owns it or else the browser ([LinkOpenActivity]; "Open in YouTube" is the sub text);
 * - **the only button is "Copy link"**: no "Open in browser" button, no auto-open and no "display over other apps" permission;
 * - several links in a row are **one group** with a summary ("3 links from Desk PC"), at most [MAX_LINKS] on screen (Android drops new
 *   notifications once an app has 50): a new one beyond the cap takes the place of the oldest ([NotificationLedger]);
 * - on the lock screen the title and the address are hidden unless the user allowed previews.
 * Posting is the caller's job to guard (the permission check is in [app.kinrelay.android.link.Notifications]).
 */
class LinkNotifications(context: Context, private val handlers: LinkHandlers) {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val ledger = NotificationLedger(MAX_LINKS, now = SystemClock::elapsedRealtime)
    private val lock = Any()
    private var pcName = ""

    /** The lock-screen setting of the newest link: the group summary follows it (a private summary would hide public links on the lock screen). */
    private var previewsOnLockScreen = false

    // The summary is the one notification here that is updated, and Android sheds updates of an app that posts too fast: at most once a second,
    // and the last request is always posted (the same rule as the file arrivals).
    private val summaryPoster = TrailingThrottle(
        intervalMillis = SUMMARY_UPDATE_MILLIS,
        now = SystemClock::elapsedRealtime,
        schedule = { delay, task -> main.postDelayed(task, delay) },
        action = { synchronized(lock) { renderSummary() } },
    )

    fun show(fromName: String, messageId: String, link: ReceivedLink, lockScreenPreviews: Boolean) {
        val label = LinkOpenPlan.label(handlers.nonBrowserApps(link.url))
        val content = LinkNotificationContent.of(fromName, link, label, lockScreenPreviews)
        val publicVersion = NotificationCompat.Builder(context, NotificationChannels.ARRIVALS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(content.publicTitle)
            .build()
        val notification = NotificationCompat.Builder(context, NotificationChannels.ARRIVALS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(content.title)
            .setContentText(content.addressLine)
            .setSubText(content.subText)
            .setStyle(NotificationCompat.BigTextStyle().setBigContentTitle(content.title).bigText(content.bigText))
            .setAutoCancel(true)
            .setContentIntent(open(link.url))
            .addAction(R.drawable.ic_stat_kin, "Copy link", copy(link.url))
            .setGroup(GROUP)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setVisibility(content.visibility)
            .setPublicVersion(publicVersion)
            .build()
        synchronized(lock) {
            pcName = fromName
            previewsOnLockScreen = lockScreenPreviews
            if (messageId !in ledger) pruneGone()
            ledger.claim(messageId, link.domain).forEach { evicted -> manager.cancel(evicted, LINK_ID) }
            manager.notify(messageId, LINK_ID, notification)
        }
        summaryPoster.request()
    }

    /** A link was tapped, copied or swiped: the summary and the cap forget what is gone. */
    fun refresh() = summaryPoster.request()

    private fun pruneGone() {
        runCatching {
            val live = manager.activeNotifications.filter { it.id == LINK_ID && it.tag != SUMMARY_TAG }.mapNotNull { it.tag }.toSet()
            ledger.retainOnly(live)
        }
    }

    /** With two or more links up, a summary stacks them; with fewer there is none. The lines are the real addresses, not the titles. */
    private fun renderSummary() {
        pruneGone()
        if (!LinkNotificationContent.hasSummary(ledger.size)) {
            manager.cancel(SUMMARY_TAG, LINK_ID)
            return
        }
        val title = LinkNotificationContent.summaryTitle(ledger.size, pcName)
        val style = NotificationCompat.InboxStyle().setBigContentTitle(title)
        ledger.titles.takeLast(SUMMARY_LINES).forEach { style.addLine(it) }
        val summary = NotificationCompat.Builder(context, NotificationChannels.ARRIVALS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setStyle(style)
            .setGroup(GROUP)
            .setGroupSummary(true)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setSilent(true)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .setVisibility(if (previewsOnLockScreen) NotificationCompat.VISIBILITY_PUBLIC else NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(NotificationCompat.Builder(context, NotificationChannels.ARRIVALS).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle(title).build())
            .build()
        manager.notify(SUMMARY_TAG, LINK_ID, summary)
    }

    /**
     * A tap: our own activity opens the link (not a service or a receiver: Android 12 refuses a notification that starts an activity through
     * a trampoline). The URL is the intent's data, so each link has its own `PendingIntent`.
     */
    private fun open(url: String): PendingIntent = PendingIntent.getActivity(
        context, 0, LinkOpenActivity.intent(context, url), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** The URL is the intent's data (extras don't tell two `PendingIntent`s apart, and a hash of the URL can collide), so each link has its own. */
    private fun copy(url: String): PendingIntent = PendingIntent.getBroadcast(
        context, 0,
        Intent(context, TransferActionReceiver::class.java).setAction(TransferActionReceiver.ACTION_COPY_LINK).setData(url.toUri()),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    companion object {
        const val LINK_ID = 5
        const val GROUP = "kin_links"
        const val SUMMARY_TAG = "links-summary"

        /** At most this many link notifications stay up (the oldest goes first). */
        const val MAX_LINKS = 10
        private const val SUMMARY_LINES = 5
        private const val SUMMARY_UPDATE_MILLIS = 1_000L
    }
}
