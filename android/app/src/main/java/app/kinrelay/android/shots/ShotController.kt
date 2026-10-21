package app.kinrelay.android.shots

import android.Manifest
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.MediaStore
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.kinrelay.android.AppGraph
import app.kinrelay.android.BuildConfig
import app.kinrelay.android.R
import app.kinrelay.android.features.FeatureIds
import app.kinrelay.android.link.Clock
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.transfer.ShareSource
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** What Settings and Home show about auto-send: the switches, whether Android lets Kin read the photos now, and what is missing. */
data class ShotsUi(
    val available: Boolean = BuildConfig.AUTO_SEND_SHOTS,
    val screenshots: Boolean = false,
    val camera: Boolean = false,
    /** Android lets Kin read all photos right now (checked at the point of use, never cached: Android 14's "Select photos" is a session-only grant that does not work for a background watcher). */
    val permissionGranted: Boolean = false,
    val watching: Boolean = false,
    /** The hold notification can really be seen (notifications allowed and on, the *File transfers* channel not blocked). Without it there is no Cancel, so nothing is sent. */
    val notificationsOk: Boolean = true,
) {
    val anyOn: Boolean get() = screenshots || camera

    /** A switch is on but Android does not give Kin all photos: "Needs access to all photos". */
    val needsAccess: Boolean get() = anyOn && !permissionGranted

    /** A switch is on but the 5-second hold (with its Cancel) can't be shown: "Needs notifications". */
    val needsNotifications: Boolean get() = anyOn && !notificationsOk
}

/**
 * Auto-send screenshots and camera photos (design 5.6), opt-in. One per process, made by [AppGraph]; **it starts nothing by itself**. A [ContentObserver] on `MediaStore.Images` is registered
 * **only while** a switch is on, Android lets Kin read the photos, a PC is paired and `LinkService` runs, and unregistered otherwise: **no polling, no timer**. The observer only signals
 * [ShotWatcherCore]; what is sent and when is decided there and by [ShotPicker].
 */
class ShotController(private val graph: AppGraph) {
    private val context: Context = graph.appContext
    private val resolver: ContentResolver = context.contentResolver
    private val clock = Clock { SystemClock.elapsedRealtime() }
    private val notifications = context.getSystemService(NotificationManager::class.java)
    private val handler = Handler(Looper.getMainLooper())

    @Volatile private var screenshotsOn = false

    @Volatile private var cameraOn = false

    private var observer: ContentObserver? = null

    @Volatile private var core: ShotWatcherCore? = null
    private val coreLock = Mutex()

    private val mutableUi = MutableStateFlow(ShotsUi())
    val ui: StateFlow<ShotsUi> = mutableUi

    /** The counts live here, not in the core, so reading them (the debug state) never creates the core. */
    val stats = ShotStats()

    /** The core is made on first use from the stored baseline; reading the store suspends (no `runBlocking`, so never on the main thread). */
    private suspend fun core(): ShotWatcherCore = core ?: coreLock.withLock {
        core ?: ShotWatcherCore(
            graph.scope, clock, ports,
            settings = { ShotSettings(screenshotsOn, cameraOn) },
            paused = { graph.pause.paused() || graph.presence.peerPaused() },
            initialBaseline = graph.shotsStore.baseline(),
            initialBaselineTimeSeconds = graph.shotsStore.baselineTime(),
            stats = stats,
        ).also { core = it }
    }

    /** Called once when the app starts: follows the switches, the link and the pause. Reacts only. */
    fun start() {
        // A hold or "Sending..." notification of an earlier run of the process can't be acted on any more (its hold is gone): take it away.
        runCatching { notifications.activeNotifications.filter { it.tag == TAG }.forEach { notifications.cancel(TAG, it.id) } }
        graph.scope.launch {
            combine(graph.features.flow(FeatureIds.SHOTS_SCREENSHOTS), graph.features.flow(FeatureIds.SHOTS_CAMERA)) { s, c -> s to c }.collect { (s, c) ->
                screenshotsOn = s
                cameraOn = c
                reevaluate()
            }
        }
        graph.scope.launch {
            graph.link.state.collect { reevaluate() }
        }
    }

    /** Android lets Kin read all photos right now. Evaluated each time, never cached. */
    fun permissionGranted(): Boolean {
        val permission = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_IMAGES else Manifest.permission.READ_EXTERNAL_STORAGE
        return ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
    }

    /** The permission to ask for when the user turns auto-send on (never at start-up). */
    fun permissionToRequest(): String =
        if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_IMAGES else Manifest.permission.READ_EXTERNAL_STORAGE

    /**
     * Registers or unregisters the observer from the facts (switch on, permission held, a PC paired, the service running). Called when any of them may have changed, including when the user comes back
     * from the system's permission screen.
     */
    fun reevaluate() {
        if (!BuildConfig.AUTO_SEND_SHOTS) {
            unregister()
            publish(false)
            return
        }
        graph.scope.launch {
            val paired = graph.pairedDevice() != null
            // Without a visible hold (notifications off) nothing could be cancelled, so nothing is watched either.
            val want = (screenshotsOn || cameraOn) && permissionGranted() && holdVisible() && paired && graph.link.runtime != null
            val c = if (want) core() else null
            synchronized(this@ShotController) {
                if (c != null) register(c) else unregister()
            }
            publish(want)
        }
    }

    private fun publish(watching: Boolean) {
        mutableUi.value = ShotsUi(
            available = BuildConfig.AUTO_SEND_SHOTS, screenshots = screenshotsOn, camera = cameraOn, permissionGranted = permissionGranted(), watching = watching,
            notificationsOk = holdVisible(),
        )
    }

    /**
     * The 5-second hold can really be seen right now: Android lets Kin post notifications (the `POST_NOTIFICATIONS` permission, and the app's notifications on) and the hold's own
     * channel (*Screenshot about to be sent*) isn't blocked. Evaluated each time. Without it there is no Cancel button, so an image is never sent (it is skipped as `no_hold`) and Settings says "Needs notifications".
     */
    fun holdVisible(): Boolean {
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return false
        if (!notifications.areNotificationsEnabled()) return false
        NotificationChannels.ensure(context)
        val channel = notifications.getNotificationChannel(NotificationChannels.SHOT_HOLD)
        return channel == null || channel.importance != NotificationManager.IMPORTANCE_NONE
    }

    @Synchronized
    private fun register(c: ShotWatcherCore) {
        if (observer != null) return
        val o = object : ContentObserver(handler) {
            override fun onChange(selfChange: Boolean) {
                c.onChange()
            }

            override fun onChange(selfChange: Boolean, uris: MutableCollection<Uri>, flags: Int) {
                c.onChange()
            }
        }
        resolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, o)
        observer = o
    }

    @Synchronized
    private fun unregister() {
        observer?.let { runCatching { resolver.unregisterContentObserver(it) } }
        observer = null
        core?.dropHolds()
    }

    val watching: Boolean @Synchronized get() = observer != null

    /**
     * The user turned a switch on and Android gave the permission: the baseline is taken (the newest image now), **then** the switch is set, so nothing that already exists is ever sent. Returns
     * false when the PC can't keep a screenshot out of its clipboard history (its `hello` lacks `presence`): the phone refuses to turn auto-send on for a PC without it (design 3.10).
     */
    suspend fun turnOn(screenshots: Boolean?, camera: Boolean?): Boolean {
        if (!BuildConfig.AUTO_SEND_SHOTS || !permissionGranted() || !holdVisible()) return false
        if (graph.link.runtime?.currentSession?.peerListsPresence != true) return false
        core().retakeBaseline()
        screenshots?.let { graph.shotsStore.setScreenshots(it) }
        camera?.let { graph.shotsStore.setCamera(it) }
        return true
    }

    suspend fun turnOff(screenshots: Boolean?, camera: Boolean?) {
        screenshots?.let { graph.shotsStore.setScreenshots(it) }
        camera?.let { graph.shotsStore.setCamera(it) }
    }

    /** The Cancel button of the hold notification. Immediate (no waiting on a pass or an upload); with no core there is no hold to cancel. */
    fun cancel(id: Long) {
        core?.cancel(id)
    }

    /**
     * Unlinking (design 5.7): the baseline and every hold go, and the observer with them. Nothing is left that names a photo. Both switches go off too: a PC paired later starts from nothing, and
     * turning auto-send on again takes a **new** baseline (so photos taken while unlinked are never sent to the next PC).
     */
    suspend fun onUnlink() {
        core?.dropHolds()
        unregister()
        graph.shotsStore.setScreenshots(false)
        graph.shotsStore.setCamera(false)
        graph.shotsStore.clearBaseline()
        core = null
        publish(false)
    }

    // ------------------------------------------------------------------ the real ports

    private val ports = object : ShotPorts {
        private val projection = arrayOf(
            MediaStore.Images.Media._ID, MediaStore.MediaColumns.RELATIVE_PATH, MediaStore.MediaColumns.MIME_TYPE, MediaStore.MediaColumns.IS_PENDING,
            MediaStore.MediaColumns.DATE_ADDED, MediaStore.MediaColumns.SIZE, MediaStore.MediaColumns.OWNER_PACKAGE_NAME, MediaStore.MediaColumns.DATE_TAKEN,
        )

        override fun mayRead(): Boolean = permissionGranted()

        override fun maxId(): Long {
            val args = Bundle().apply {
                putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER, "${MediaStore.Images.Media._ID} DESC")
                putInt(ContentResolver.QUERY_ARG_LIMIT, 1)
            }
            return runCatching {
                resolver.query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, arrayOf(MediaStore.Images.Media._ID), args, null)?.use { c -> if (c.moveToFirst()) c.getLong(0) else 0L }
            }.getOrNull() ?: 0L
        }

        override fun queryNewer(afterId: Long): List<ShotRow> {
            val args = Bundle().apply {
                putString(ContentResolver.QUERY_ARG_SQL_SELECTION, "${MediaStore.Images.Media._ID} > ?")
                putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, arrayOf(afterId.toString()))
                putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER, "${MediaStore.Images.Media._ID} ASC")
                putInt(ContentResolver.QUERY_ARG_LIMIT, ShotWatcherCore.PAGE)
            }
            return runCatching {
                resolver.query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, projection, args, null)?.use { c ->
                    buildList {
                        while (c.moveToNext()) {
                            add(ShotRow(c.getLong(0), c.getString(1), c.getString(2), c.getInt(3) == 1, c.getLong(4), c.getLong(5), c.getString(6), if (c.isNull(7)) 0L else c.getLong(7)))
                        }
                    }
                }
            }.getOrNull().orEmpty()
        }

        override fun nowSeconds(): Long = System.currentTimeMillis() / 1000 + app.kinrelay.android.DebugHooks.shotsClockOffsetSeconds(context)

        override fun readyForUpload(): Boolean {
            val c = graph.link.runtime?.currentSession ?: return false
            return c.isReady && c.peerListsPresence
        }

        override fun showHold(id: Long, kind: ShotKind): Boolean {
            if (!holdVisible()) return false
            val pc = graph.link.runtime?.currentSession?.peerName ?: "your PC"
            return post(id, ShotHoldText.title(kind, pc, ShotWatcherCore.HOLD_MILLIS / 1000), Style.HOLD)
        }

        override fun showSending(id: Long) {
            post(id, "Sending…", Style.SENDING)
        }

        override fun showResult(id: Long, text: String?) {
            if (text == null) notifications.cancel(TAG, notificationId(id)) else post(id, text, Style.RESULT)
        }

        override suspend fun upload(row: ShotRow, kind: ShotKind): ShotUpload {
            val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, row.id)
            // A generic name, never the file's own: a screenshot's name says which app it came from (design 5.8). It carries only the time.
            val stem = (if (kind == ShotKind.CAMERA) "Photo " else "Screenshot ") + GENERIC_TIME.format(Instant.ofEpochSecond(row.dateAddedSeconds).atZone(ZoneId.systemDefault()))
            val ok = graph.transfers.sendNow(listOf(ShareSource(uri.toString(), mimeHint = row.mime, genericStem = stem)), kind.wire, SEND_WAIT_MILLIS)
            return if (ok) ShotUpload.SENT else ShotUpload.OFFLINE
        }

        override fun saveBaseline(id: Long) {
            graph.scope.launch { graph.shotsStore.setBaseline(id) }
        }

        override fun saveBaselineTime(seconds: Long) {
            graph.scope.launch { graph.shotsStore.setBaselineTime(seconds) }
        }
    }

    private fun notificationId(id: Long): Int = ShotNotificationIds.of(id)

    private enum class Style { HOLD, SENDING, RESULT }

    /** True when it was posted. */
    private fun post(id: Long, text: String, style: Style): Boolean {
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return false
        // The hold is on its own high-importance channel (a heads-up, with no sound and no vibration: the channel has none); sending and the result stay quiet on *File transfers*.
        // No setSilent on the hold: it makes the notification a silent group child, and a silenced child gets no heads-up.
        val builder = NotificationCompat.Builder(context, if (style == Style.HOLD) NotificationChannels.SHOT_HOLD else NotificationChannels.TRANSFERS)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(text)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            // Its own group key (the 2B.4 review): without one Android grouped the 5-second hold under the persistent "Connected" notification, and Cancel showed only after expanding it. One
            // key per screenshot, so each stands alone with its button visible.
            .setGroup("$GROUP_PREFIX$id")
        when (style) {
            Style.HOLD -> {
                // The full id is in the intent's data, which PendingIntent matching compares (extras are not), on top of the id-based request code: with FLAG_UPDATE_CURRENT a second screenshot
                // must never rewrite the first one's Cancel (2B.5 review round 3).
                val cancel = PendingIntent.getBroadcast(
                    context, notificationId(id),
                    Intent(context, ShotActionReceiver::class.java).setAction(ShotActionReceiver.ACTION_CANCEL).setData(Uri.parse("kin-shot://cancel/$id")).putExtra(ShotActionReceiver.EXTRA_ID, id),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
                // Round 2 (B2): a collapsed row in the shade shows no button, so a tap on the notification itself cancels too (the same single-owner `cancel` as the button), and says so.
                builder.setOngoing(true)
                    .setPriority(NotificationCompat.PRIORITY_HIGH)
                    .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_ALL)
                    .setContentText(ShotHoldText.TAP_TO_CANCEL)
                    .setContentIntent(cancel)
                    .addAction(R.drawable.ic_stat_kin, ShotHoldText.CANCEL_ACTION, cancel)
            }
            // The image has left the hold: no Cancel (a late tap would find nothing to cancel). A safety timeout, in case the result never comes.
            Style.SENDING -> builder.setSilent(true).setOngoing(true).setTimeoutAfter(SENDING_MILLIS)
            Style.RESULT -> builder.setSilent(true).setAutoCancel(true).setTimeoutAfter(RESULT_MILLIS)
        }
        return try {
            notifications.notify(TAG, notificationId(id), builder.build())
            true
        } catch (_: SecurityException) {
            false
        }
    }

    /** The debug state (`SHOTS_STATE`): the baseline, the counts and the skip reasons. No names, paths or sizes. */
    fun debugLine(): String =
        "screenshots=$screenshotsOn camera=$cameraOn permission=${permissionGranted()} notifications=${holdVisible()} watching=$watching baseline=${core?.baseline} holding=${core?.holding} ${stats.line()}"

    private companion object {
        const val TAG = "kin-shot"
        const val GROUP_PREFIX = "kin-shot-"
        const val SEND_WAIT_MILLIS = 5_000L
        const val RESULT_MILLIS = 4_000L
        const val SENDING_MILLIS = 60_000L
        val GENERIC_TIME: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH.mm.ss")
    }
}
