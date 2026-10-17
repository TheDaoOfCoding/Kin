package app.kinrelay.android.notifmirror

import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import app.kinrelay.android.AppGraph
import app.kinrelay.android.BuildConfig
import app.kinrelay.android.link.Clock
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.protocol.Frames
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/** A live notification and the ranking it came with. */
class SbnItem(val sbn: StatusBarNotification, val ranking: NotificationListenerService.Ranking?)

/**
 * Notification mirroring as the rest of the app sees it (design 4.1): the user's choices, the sender, the engine, and the hooks the link and the
 * listener service call. One per process, made by [AppGraph]. **It starts nothing by itself:** it only reacts (a notification, a PC message, a
 * setting), so an idle phone spends nothing on it.
 */
class MirrorController(private val graph: AppGraph) {
    private val context: Context = graph.appContext
    private val clock = Clock { SystemClock.elapsedRealtime() }

    val store = MirrorPolicyStore(context)
    val icons by lazy { AndroidIcons(context) }

    private val labels = HashMap<String, String?>()

    private fun labelOf(packageName: String): String? = synchronized(labels) {
        labels.getOrPut(packageName) {
            try {
                context.packageManager.getApplicationInfo(packageName, 0).loadLabel(context.packageManager).toString()
            } catch (_: PackageManager.NameNotFoundException) {
                null
            } catch (_: SecurityException) {
                null
            }
        }
    }

    val filter = MirrorFilter(
        ownPackage = context.packageName,
        debugChannel = if (BuildConfig.DEBUG) DEBUG_CHANNEL else null,
        flagged = { pkg -> AppCatalog.isFlagged(pkg, labelOf(pkg)) },
    )

    val sender = MirrorSender(graph.scope, clock, link = { graph.link.runtime?.currentSession }, log = graph.log)

    private val sbnPort = object : SbnPort<SbnItem> {
        override fun quick(sbn: SbnItem) = SbnMapper.quick(sbn.sbn)
        override fun key(sbn: SbnItem) = sbn.sbn.key
        override fun snapshot(sbn: SbnItem) = SbnMapper.snapshot(sbn.sbn, sbn.ranking)
        override fun appLabel(packageName: String) = labelOf(packageName) ?: packageName
        override fun icon(packageName: String, sbn: SbnItem) = icons.forPackage(packageName, sbn.sbn)
    }

    val engine = MirrorEngine(sbnPort, policy = { store.current() }, filter = filter, sender = sender, clock = clock, log = graph.log)

    private val actionPort = object : ActionPort<SbnItem> {
        override fun active(): List<SbnItem>? = MirrorListenerService.activeItems()
        override fun key(sbn: SbnItem) = sbn.sbn.key
        override fun quick(sbn: SbnItem) = SbnMapper.quick(sbn.sbn)
        override fun snapshot(sbn: SbnItem) = SbnMapper.snapshot(sbn.sbn, sbn.ranking)

        override fun fire(sbn: SbnItem, index: Int, replyText: String?): FireResult {
            val action = sbn.sbn.notification.actions?.getOrNull(index) ?: return FireResult.FAILED
            val pending = action.actionIntent ?: return FireResult.FAILED
            return try {
                if (replyText != null) {
                    val input = action.remoteInputs?.firstOrNull { it.allowFreeFormInput } ?: return FireResult.FAILED
                    val intent = Intent()
                    RemoteInput.addResultsToIntent(arrayOf(input), intent, Bundle().apply { putCharSequence(input.resultKey, replyText) })
                    RemoteInput.setResultsSource(intent, RemoteInput.SOURCE_FREE_FORM_INPUT)
                    pending.send(context, 0, intent)
                } else {
                    pending.send()
                }
                FireResult.SENT
            } catch (_: PendingIntent.CanceledException) {
                FireResult.CANCELED
            } catch (_: Exception) {
                FireResult.FAILED
            }
        }

        override fun cancel(sbn: SbnItem) {
            MirrorListenerService.cancel(sbn.sbn.key)
        }
    }

    val actions = ActionHandler(
        actionPort, policy = { store.current() }, filter = filter, keyOf = sender::keyOf, clock = clock,
        stats = sender.stats, onSelfCancel = engine::noteSelfCancel,
    )

    /** Counts of what the last debug action did (no content): the answer to each `notif.action`. */
    @Volatile var lastActionResult: String = "none"
        private set

    /** The connected PC's latest `hello` lists `notif` (it will show what is sent): Settings says "Your PC needs Kin 0.2.0 ..." while this is false. */
    val pcAcceptsNotif = MutableStateFlow(false)

    /** The service binding state, set by [MirrorListenerService]. */
    @Volatile var listenerConnected: Boolean = false
        private set

    fun accessGranted(): Boolean = context.getSystemService(NotificationManager::class.java)
        .isNotificationListenerAccessGranted(ComponentName(context, MirrorListenerService::class.java))

    suspend fun status(): MirrorStatus {
        val policy = store.current()
        val access = accessGranted()
        return MirrorStatus(enabled = policy.enabled && access, accessGranted = access, apps = policy.allowed.size, otpHidden = RedactionMatcher.codesHidden(Build.VERSION.SDK_INT))
    }

    /** Called once when the app starts: follows the user's choices (what to take down when mirroring or an app is turned off). */
    fun start() {
        graph.scope.launch { graph.link.state.collect { if (it !is LinkState.Connected) pcAcceptsNotif.value = false } }
        graph.scope.launch {
            var last: MirrorPolicy? = null
            store.policy.distinctUntilChanged().collect { now ->
                val before = last
                last = now
                if (before == null) return@collect
                val removedApps = before.allowed - now.allowed
                when {
                    before.enabled && !now.enabled -> sender.removeAnnounced()
                    removedApps.isNotEmpty() -> sender.removeAnnounced { it in removedApps }
                }
                if (now.allowed != before.allowed) graph.log.i("mirror", "mirror: ${now.allowed.size} apps allowed")
                if (now.enabled != before.enabled) graph.log.i("mirror", "mirror: ${if (now.enabled) "on" else "off"}")
                sender.statusChanged(status())
            }
        }
    }

    /** The service was bound or lost (access granted or revoked). */
    fun listenerChanged(connected: Boolean) {
        listenerConnected = connected
        graph.log.i("mirror", "mirror: listener ${if (connected) "connected" else "disconnected"}")
        graph.scope.launch { sender.statusChanged(status()) }
    }

    /** The PC's `hello` is accepted (also a later `hello` that changes its capabilities): status, then the sync with what is active now. */
    fun onLinkReady(connection: LinkConnection) {
        pcAcceptsNotif.value = connection.peerUnderstandsNotif
        graph.scope.launch {
            val active = MirrorListenerService.activeItems().orEmpty()
            sender.onConnected(status(), engine.activeNids(active))
        }
    }

    /** `notif.dismiss` from the PC. Dropped silently when mirroring is off (design 3.1). */
    fun onDismiss(nid: String) {
        graph.scope.launch {
            val done = actions.onDismiss(nid)
            graph.log.i("mirror", "mirror: dismiss from the PC cancelled=$done")
        }
    }

    /** `notif.action` from the PC: fired, then `ack` or `error`. [reply] gets the frame to send and remember as the response. */
    suspend fun onAction(request: NotifWire.ActionRequest): String {
        val outcome = actions.onAction(request)
        lastActionResult = when (outcome) {
            ActionOutcome.Ack -> "ack"
            is ActionOutcome.Error -> outcome.code
        }
        graph.log.i("mirror", "mirror: action from the PC result=$lastActionResult")
        return when (outcome) {
            ActionOutcome.Ack -> Frames.ack(request.id)
            is ActionOutcome.Error -> Frames.error(outcome.code, outcome.message, request.id)
        }
    }

    companion object {
        /** The channel of Kin's own debug test notifications that a debug build still mirrors (design 4.14). */
        const val DEBUG_CHANNEL = "kin_debug_mirror"
    }
}
