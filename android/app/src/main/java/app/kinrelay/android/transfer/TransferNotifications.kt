package app.kinrelay.android.transfer

import android.Manifest
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
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
import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.notify.NotificationLedger
import app.kinrelay.android.notify.TrailingThrottle

/**
 * What the user sees of a transfer: a progress notification with Cancel, and the result ("Photo from Desk PC" with Open and Share
 * for a download). Progress is updated at most twice a second by the runners ([ProgressThrottle]).
 *
 * - The result has its **own notification ID**, so it is a new notification and never an update of the progress one: Android drops
 *   updates of an app that posts too fast, and a dropped result would leave "Receiving..." on screen for ever. The progress notification
 *   is cancelled when the result is posted.
 * - **Bounded and grouped (1.4).** Android drops *new* notifications once an app has 50 up, so a long batch of transfers could lose its
 *   results. Each of the two groups (what the phone sends and the progress of everything: channel *transfers*; what arrived: channel
 *   *arrivals*) keeps at most [MAX_SLOTS] transfers on screen ([NotificationLedger]: a new one beyond the cap pushes out the oldest), and
 *   shows a summary notification ("N file transfers") when there are two or more, so they are one stack.
 */
class TransferNotifications(context: Context) : TransferNotifier {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val main = Handler(Looper.getMainLooper())

    /** Transfers that have a progress notification up (so that the first update of a retried transfer takes the old result away). */
    private val showingProgress: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

    private val sendLedger = NotificationLedger(MAX_SLOTS, now = SystemClock::elapsedRealtime)
    private val arrivalLedger = NotificationLedger(MAX_SLOTS, now = SystemClock::elapsedRealtime)
    private val lock = Any()

    // A summary is the one notification that is *updated* (everything else is a new notification under its own tag). Android sheds updates
    // of an app that posts more than about five a second, and the update that is shed in a burst of 15 files is the last, so the count
    // would stay low for good (seen on the emulator: "4 files from your PC" with 10 on screen). The summaries are posted at most once a
    // second each, and the last request is always posted ([TrailingThrottle]).
    private fun summaryPoster(ledger: NotificationLedger, group: String, channel: String, summaryId: Int, what: String) = TrailingThrottle(
        intervalMillis = SUMMARY_UPDATE_MILLIS,
        now = SystemClock::elapsedRealtime,
        schedule = { delay, task -> main.postDelayed(task, delay) },
        action = { synchronized(lock) { renderSummary(ledger, group, channel, summaryId, what) } },
    )

    private val sendSummary = summaryPoster(sendLedger, GROUP_SENDS, NotificationChannels.TRANSFERS, SENDS_SUMMARY_ID, "file transfers")
    private val arrivalSummary = summaryPoster(arrivalLedger, GROUP_ARRIVALS, NotificationChannels.ARRIVALS, ARRIVALS_SUMMARY_ID, "files from your PC")

    init {
        NotificationChannels.ensure(this.context)
    }

    override fun progress(transfer: TransferEntity, files: List<TransferFileEntity>, bytesDone: Long, waiting: Boolean) {
        if (!canPost()) return
        // A transfer that starts again (Retry) replaces its "PC not reachable" or "Couldn't send" notification.
        if (showingProgress.add(transfer.id)) manager.cancel(transfer.id, RESULT_ID)
        val up = transfer.direction == TransferDirection.UP
        val total = files.sumOf { it.size }
        val percent = if (total > 0) (bytesDone * 100 / total).toInt().coerceIn(0, 100) else 0
        val finished = files.count { it.finished }
        val title = if (up) "Sending to ${transfer.peerName}" else "Receiving from ${transfer.peerName}"
        val text = when {
            waiting -> "Waiting for ${transfer.peerName}…"
            files.size == 1 -> "$percent%"
            else -> "${minOf(finished + 1, files.size)} of ${files.size} files · $percent%"
        }
        val builder = base(NotificationChannels.TRANSFERS, GROUP_SENDS, title, text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .setProgress(100, percent, waiting)
            .addAction(R.drawable.ic_stat_kin, "Cancel", broadcast(ACTION_CANCEL, transfer.id))
        synchronized(lock) {
            claim(sendLedger, transfer.id, title)
            manager.notify(transfer.id, PROGRESS_ID, builder.build())
            updateSummary(sends = true)
        }
    }

    override fun uploaded(transfer: TransferEntity, files: List<TransferFileEntity>) {
        val what = if (files.size == 1) files[0].name else "${files.size} files"
        if (!canPost()) {
            message("Sent $what to ${transfer.peerName}")
            return
        }
        val builder = base(NotificationChannels.TRANSFERS, GROUP_SENDS, "Sent to ${transfer.peerName}", what)
            .setAutoCancel(true)
            .setTimeoutAfter(RESULT_TIMEOUT_MILLIS)
            .setSilent(true)
        postSendResult(transfer.id, "Sent to ${transfer.peerName}", builder)
    }

    override fun received(transfer: TransferEntity, files: List<TransferFileEntity>) {
        val saved = files.filter { it.finished && it.localUri != null }
        if (saved.isEmpty() || !canPost()) {
            endProgress(transfer.id)
            return
        }
        val title = receivedTitle(transfer.peerName, saved)
        val builder = base(NotificationChannels.ARRIVALS, GROUP_ARRIVALS, title, saved.joinToString(", ") { it.finalName ?: it.name })
            .setAutoCancel(true)
            .setContentIntent(open(saved.first(), transfer.id))
            .addAction(R.drawable.ic_stat_kin, "Open", open(saved.first(), transfer.id))
            .addAction(R.drawable.ic_stat_kin, "Share", share(saved, transfer.id))
        synchronized(lock) {
            // A new slot in the arrivals group, under its own ID; the progress notification of the send group goes after it.
            claim(arrivalLedger, transfer.id, title)
            manager.notify(transfer.id, RESULT_ID, builder.build())
            sendLedger.release(transfer.id)
            endProgress(transfer.id)
            updateSummary(sends = false)
            updateSummary(sends = true)
        }
    }

    override fun queued(transfer: TransferEntity, reason: String) {
        val up = transfer.direction == TransferDirection.UP
        val title = if (up) "PC not reachable" else "Waiting for ${transfer.peerName}"
        val text = if (up) "Couldn't send to ${transfer.peerName}. $reason Your files are saved and go out when it is back." else "Your PC's files will download when it is back. $reason"
        if (!canPost()) {
            message("$title. $reason")
            return
        }
        val builder = base(NotificationChannels.TRANSFERS, GROUP_SENDS, title, text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(!up)
            .addAction(R.drawable.ic_stat_kin, "Retry", broadcast(ACTION_RETRY, transfer.id))
            .addAction(R.drawable.ic_stat_kin, "Discard", broadcast(ACTION_CANCEL, transfer.id))
        postSendResult(transfer.id, title, builder)
    }

    override fun failed(transfer: TransferEntity, message: String) {
        val up = transfer.direction == TransferDirection.UP
        val title = if (up) "Couldn't send to ${transfer.peerName}" else "Couldn't receive from ${transfer.peerName}"
        if (!canPost()) {
            message("$title. $message")
            return
        }
        val builder = base(NotificationChannels.TRANSFERS, GROUP_SENDS, title, message)
            .setStyle(NotificationCompat.BigTextStyle().bigText(message))
            .setAutoCancel(true)
        postSendResult(transfer.id, title, builder)
    }

    override fun clear(transferId: String) {
        synchronized(lock) {
            endProgress(transferId)
            manager.cancel(transferId, RESULT_ID)
            sendLedger.release(transferId)
            arrivalLedger.release(transferId)
            updateSummary(sends = true)
            updateSummary(sends = false)
        }
    }

    /** The result goes out first, under its own ID; then the progress notification goes. */
    private fun postSendResult(transferId: String, title: String, builder: NotificationCompat.Builder) {
        synchronized(lock) {
            claim(sendLedger, transferId, title)
            manager.notify(transferId, RESULT_ID, builder.build())
            endProgress(transferId)
            updateSummary(sends = true)
        }
    }

    private fun endProgress(transferId: String) {
        showingProgress.remove(transferId)
        manager.cancel(transferId, PROGRESS_ID)
    }

    /**
     * Forgets the slots whose notification is gone (swiped away, timed out), so they no longer count against the cap or in a summary. The
     * system's list lags a `notify` that was just made, so a slot is only forgotten when it has been missing for more than
     * [NotificationLedger.MISSING_GRACE_MILLIS] ([NotificationLedger.retainOnly]).
     */
    private fun pruneGone(ledger: NotificationLedger) {
        runCatching {
            val live = manager.activeNotifications.filter { it.id == PROGRESS_ID || it.id == RESULT_ID }.mapNotNull { it.tag }.toSet()
            ledger.retainOnly(live)
        }
    }

    /** Takes a slot for [key]; a transfer pushed out by the cap loses its notifications. */
    private fun claim(ledger: NotificationLedger, key: String, title: String) {
        if (key !in ledger) pruneGone(ledger)
        ledger.claim(key, title).forEach { evicted ->
            showingProgress.remove(evicted)
            manager.cancel(evicted, PROGRESS_ID)
            manager.cancel(evicted, RESULT_ID)
        }
    }

    /** Asks for the summary of a group to be posted (at most once a second, the last request always: see [summaryPoster]). */
    private fun updateSummary(sends: Boolean) {
        (if (sends) sendSummary else arrivalSummary).request()
    }

    /** With two or more notifications in a group, a summary stacks them; with fewer there is none. */
    private fun renderSummary(ledger: NotificationLedger, group: String, channel: String, summaryId: Int, what: String) {
        pruneGone(ledger)
        if (ledger.size < 2) {
            manager.cancel(SUMMARY_TAG, summaryId)
            return
        }
        val style = NotificationCompat.InboxStyle().setBigContentTitle("${ledger.size} $what")
        ledger.titles.takeLast(SUMMARY_LINES).forEach { style.addLine(it) }
        val summary = NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle("${ledger.size} $what")
            .setStyle(style)
            .setGroup(group)
            .setGroupSummary(true)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setSilent(true)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(NotificationCompat.Builder(context, channel).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle("${ledger.size} $what").build())
            .build()
        manager.notify(SUMMARY_TAG, summaryId, summary)
    }

    override fun message(text: String) {
        main.post { Toast.makeText(context, text, Toast.LENGTH_SHORT).show() }
    }

    /** The title builder shared by every notification: private on the lock screen (names are private), with a bare public version. */
    private fun base(channel: String, group: String, title: String, text: String): NotificationCompat.Builder =
        NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(openApp())
            .setGroup(group)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(NotificationCompat.Builder(context, channel).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle(title).build())

    private fun canPost() = Build.VERSION.SDK_INT < 33 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun broadcast(action: String, transferId: String): PendingIntent {
        val intent = Intent(context, TransferActionReceiver::class.java).setAction(action).putExtra(EXTRA_TRANSFER_ID, transferId)
        return PendingIntent.getBroadcast(context, (action + transferId).hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** Open: view the first saved file with whatever app handles it. The URI is MediaStore's, so the viewer gets a read grant. */
    private fun open(file: TransferFileEntity, transferId: String): PendingIntent {
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(file.localUri!!.toUri(), file.mime)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        return PendingIntent.getActivity(context, ("open$transferId").hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** Share: the system chooser with every saved file of the transfer. */
    private fun share(files: List<TransferFileEntity>, transferId: String): PendingIntent {
        val uris = files.map { it.localUri!!.toUri() }
        val send = if (uris.size == 1) {
            Intent(Intent.ACTION_SEND).setType(files[0].mime).putExtra(Intent.EXTRA_STREAM, uris[0])
        } else {
            Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*").putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
        }
        send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        send.clipData = ClipData.newRawUri("files", uris[0]).also { clip -> uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) } }
        val chooser = Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
        return PendingIntent.getActivity(context, ("share$transferId").hashCode(), chooser, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    companion object {
        const val ACTION_CANCEL = "app.kinrelay.android.transfer.CANCEL"
        const val ACTION_RETRY = "app.kinrelay.android.transfer.RETRY"
        const val EXTRA_TRANSFER_ID = "transferId"

        /** The progress notification of a transfer, and (own ID, so it is never an update of the progress one) its result. */
        private const val PROGRESS_ID = 10
        private const val RESULT_ID = 11
        private const val SENDS_SUMMARY_ID = 20
        private const val ARRIVALS_SUMMARY_ID = 21
        private const val SUMMARY_TAG = "summary"
        private const val GROUP_SENDS = "app.kinrelay.android.group.TRANSFERS"
        private const val GROUP_ARRIVALS = "app.kinrelay.android.group.ARRIVALS"
        private const val SUMMARY_LINES = 5
        private const val RESULT_TIMEOUT_MILLIS = 8_000L
        private const val SUMMARY_UPDATE_MILLIS = 1_000L

        /** At most this many transfers of each group are on screen (the oldest goes first). Android drops new notifications beyond 50 per app. */
        const val MAX_SLOTS = 10

        /** "Photo from Desk PC", "Video from Desk PC", "File from Desk PC", or "3 files from Desk PC". */
        fun receivedTitle(pcName: String, saved: List<TransferFileEntity>): String {
            if (saved.size > 1) return "${saved.size} files from $pcName"
            val mime = saved.first().mime
            val what = when {
                mime.startsWith("image/") -> "Photo"
                mime.startsWith("video/") -> "Video"
                else -> "File"
            }
            return "$what from $pcName"
        }
    }
}
