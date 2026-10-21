package app.kinrelay.android.presence

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.PowerManager
import android.os.SystemClock
import androidx.core.content.ContextCompat
import app.kinrelay.android.AppGraph
import app.kinrelay.android.features.FeatureIds
import app.kinrelay.android.link.Clock
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.pause.PauseChange
import app.kinrelay.android.pause.PeerPauseView
import app.kinrelay.android.protocol.Frames
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** What Home and the notification show about the PC (design 5.2). Nothing of it is stored: it is memory until Kin quits. */
data class PeerPresence(
    val connected: Boolean = false,
    val peerListsPresence: Boolean = false,
    /** The PC's battery from its latest snapshot on this connection (a laptop), or null. */
    val battery: Battery? = null,
    /** The last battery the PC said, kept until Kin quits (for a "last known" line while it is away). */
    val lastKnownBattery: Battery? = null,
    val peerPaused: Boolean = false,
    val peerPausedUntilResumed: Boolean = false,
    /** Wall-clock time (epoch ms) when the PC's pause ends by itself, or null. */
    val peerPausedUntilWall: Long? = null,
    /** "Show the PC's status" is on. */
    val showOn: Boolean = true,
    /** "Share this phone's battery with the PC" is on. */
    val shareOn: Boolean = true,
)

/**
 * Presence as the rest of the app sees it (design 5.1 and 5.2): the phone's battery and pause to the PC, and the PC's pause and laptop battery to this phone. One per process, made by
 * [AppGraph]; **it starts nothing by itself** (no `LinkService` start, no polling, no timer but the one-shots of [PresenceSender] and the expiry of the PC's pause). The battery receiver
 * (`ACTION_BATTERY_CHANGED` and `ACTION_POWER_SAVE_MODE_CHANGED`, which the system broadcasts) is registered **only while a session is ready and "Share this phone's battery" is on**.
 */
class PresenceHub(private val graph: AppGraph) {
    private val context: Context = graph.appContext
    private val clock = Clock { SystemClock.elapsedRealtime() }
    private val timers: Timers = CoroutineTimers(graph.scope)
    private val log = graph.log
    private val power = context.getSystemService(PowerManager::class.java)

    val stats = PresenceStats()
    private val peerPause = PeerPauseView(clock)

    @Volatile private var shareOn = true

    @Volatile private var showOn = true

    private var connection: LinkConnection? = null
    private var limiter = PresenceReceiveLimiter(clock)
    private var peerBattery: Battery? = null
    private var lastKnown: Battery? = null
    private var expiryTimer: Cancel? = null
    private var receiver: BroadcastReceiver? = null

    val sender = PresenceSender(
        clock, timers,
        link = { graph.link.runtime?.currentSession },
        battery = { if (shareOn) readBattery() else null },
        pause = { graph.pause.wire() },
        pauseKey = { graph.pause.stateNow().takeIf { graph.pause.paused() } },
        dnd = { graph.dnd.sharedDnd() },
        stats = stats,
    )

    private val mutableView = MutableStateFlow(PeerPresence())
    val view: StateFlow<PeerPresence> = mutableView

    /** True while the PC says it is paused. Automatic features that send to the PC stop meanwhile (design 3.5). */
    fun peerPaused(): Boolean = peerPause.isPaused()

    /** Called once when the app starts: follows the user's switches, this device's pause and the link. Reacts only. */
    fun start() {
        graph.scope.launch {
            graph.features.flow(FeatureIds.PRESENCE_SHARE).collect {
                shareOn = it
                reevaluateReceiver()
                sender.onBatteryChanged()
                publish()
            }
        }
        graph.scope.launch {
            graph.features.flow(FeatureIds.PRESENCE_SHOW).collect {
                showOn = it
                publish()
            }
        }
        graph.scope.launch {
            graph.pause.changes.collect {
                if (it == PauseChange.EXPIRED) sender.onPauseExpired() else sender.onPauseChanged()
                publish()
            }
        }
        graph.scope.launch {
            graph.link.state.collect { if (it !is LinkState.Connected) onClosed() }
        }
    }

    /** The PC's `hello` was accepted (the first, or a later one that changed its capabilities): the first snapshot goes out and the battery receiver is looked at. */
    fun onHelloAccepted(c: LinkConnection) {
        synchronized(this) {
            if (c !== connection) {
                connection = c
                limiter = PresenceReceiveLimiter(clock)
                peerPause.onClosed()
                peerBattery = null
            }
        }
        sender.onReady()
        reevaluateReceiver()
        publish()
        // 2B.5: the session is ready: the phone's filter is read once and the DND follower learns whether this PC lists `presence` (a later `hello` can take it away).
        graph.dnd.onConnect(c.peerListsPresence)
    }

    /** The connection ended: the PC's pause is forgotten (nothing is paused as far as this side knows) and only the last battery stays, for the "last known" line. */
    fun onClosed() {
        synchronized(this) {
            connection = null
            peerPause.onClosed()
            peerBattery = null
            expiryTimer?.cancel()
            expiryTimer = null
        }
        sender.onClosed()
        unregisterReceiver()
        publish()
        // 2B.5: a closed connection starts the 60 s grace of a held Do Not Disturb (the follower decides); the filter receiver goes away.
        graph.dnd.onClose()
    }

    fun onDropped() {
        stats.droppedNotListed.incrementAndGet()
    }

    /** A valid `presence`: it replaces the PC's previous snapshot on this connection. Ephemeral. Bucket of 10, refilled 1 per 3 s: over it is dropped and counted, with one `error rate_limited` per 10 s. */
    fun onPresence(snapshot: PresenceSnapshot) {
        val verdict = limiter.onPresence()
        if (!verdict.accepted) {
            stats.droppedRate.incrementAndGet()
            if (verdict.sendError) connection?.sendEphemeral(Frames.error("rate_limited", "Too many presence messages.", null))
            return
        }
        stats.received.incrementAndGet()
        val before = peerPause.isPaused()
        synchronized(this) {
            peerPause.onPresence(snapshot.pause)
            peerBattery = snapshot.battery
            snapshot.battery?.let { lastKnown = it }
            scheduleExpiry()
        }
        publish()
        if (before && !peerPause.isPaused()) peerPauseEnded()
        // 2B.5: the PC's Focus (the follower applies its own switch, access, pause and the rising-edge rules) and the PC's low battery (the phone's own nudge: one per discharge episode).
        graph.dnd.onSnapshot(snapshot.focus)
        val c = connection
        if (c != null) {
            graph.pcBatteryNudge.onPcBattery(c.peerDeviceId, snapshot.battery, showOn, graph.pause.paused())?.let { graph.alerts.showPcBattery(c.peerName ?: "Your PC", it) }
        }
    }

    /** One `delay` until the PC's pause ends by itself, so what waited for it (media) carries on without a message. */
    private fun scheduleExpiry() {
        expiryTimer?.cancel()
        expiryTimer = null
        val left = peerPause.remainingMillis() ?: return
        expiryTimer = timers.after(left + 1) {
            publish()
            peerPauseEnded()
        }
    }

    private fun peerPauseEnded() {
        graph.media.onPeerPauseEnded()
    }

    // ------------------------------------------------------------------ the battery

    /** The battery now (a sticky read: no receiver needed), or null when the phone has none. */
    fun readBattery(): Battery? {
        val intent = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) ?: return null
        val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
        val plugged = intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0)
        return BatteryMap.fromExtras(level, scale, plugged, saver = power?.isPowerSaveMode)
    }

    /** Registered only while a session is ready and sharing is on: the system broadcasts a change, so nothing is polled. */
    @Synchronized
    private fun reevaluateReceiver() {
        val want = shareOn && connection?.isReady == true
        if (want && receiver == null) {
            val r = object : BroadcastReceiver() {
                override fun onReceive(c: Context, intent: Intent) {
                    sender.onBatteryChanged()
                }
            }
            val filter = IntentFilter().apply {
                addAction(Intent.ACTION_BATTERY_CHANGED)
                addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
            }
            ContextCompat.registerReceiver(context, r, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
            receiver = r
        } else if (!want) {
            unregisterReceiver()
        }
    }

    @Synchronized
    private fun unregisterReceiver() {
        receiver?.let { runCatching { context.unregisterReceiver(it) } }
        receiver = null
    }

    val receiverRegistered: Boolean @Synchronized get() = receiver != null

    // ------------------------------------------------------------------ what the screens show

    private fun publish() {
        val c = connection
        val remaining = peerPause.remainingMillis()
        mutableView.value = PeerPresence(
            connected = c?.isReady == true,
            peerListsPresence = c?.peerListsPresence == true,
            battery = peerBattery,
            lastKnownBattery = lastKnown,
            peerPaused = peerPause.isPaused(),
            peerPausedUntilResumed = peerPause.hasNoEnd(),
            peerPausedUntilWall = remaining?.let { System.currentTimeMillis() + it },
            showOn = showOn,
            shareOn = shareOn,
        )
    }

    /** The debug state (`PRESENCE_STATE`): counts and the PC's battery and pause as numbers. No content beyond that, never logged. */
    fun debugLine(): String {
        val v = view.value
        return "connected=${v.connected} listsPresence=${v.peerListsPresence} ${stats.line()} pcBattery=${v.battery?.let { "${it.level}/${it.charging}" }} " +
            "peerPaused=${v.peerPaused} share=$shareOn show=$showOn receiver=$receiverRegistered"
    }

}
