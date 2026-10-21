package app.kinrelay.android.dnd

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.SystemClock
import androidx.core.content.ContextCompat
import app.kinrelay.android.AppGraph
import app.kinrelay.android.features.FeatureIds
import app.kinrelay.android.notifmirror.MirrorListenerService
import app.kinrelay.android.presence.Cancel
import app.kinrelay.android.presence.CoroutineTimers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** What Settings shows about Do Not Disturb (design 4.2, 4.3): the two switches, "How much", the access, and whether Kin holds the phone silent right now. Nothing here is history. */
data class DndView(
    val followOn: Boolean = false,
    /** `priority` (default) or `alarms`. */
    val followFilter: String = DndWire.PRIORITY,
    val shareOn: Boolean = false,
    /** Do Not Disturb access, or notification access (which also grants it), is in place. */
    val access: Boolean = false,
    /** Kin holds the phone silent now (the persisted record). */
    val holding: Boolean = false,
    /** `rule` (Kin's own "Do Not Disturb (Kin)" rule, Android 15+) or `global` (Android 10 to 14), while holding. */
    val route: String? = null,
    /** The filter Kin asked for (`INTERRUPTION_FILTER_*`), while holding: what Settings says is held (the "How much" setting may have changed since). */
    val heldFilter: Int = DndFilter.ALL,
    /** The phone's own filter now (`INTERRUPTION_FILTER_*`). */
    val filter: Int = DndFilter.ALL,
    /** The PC says it is in Focus (live, memory only). */
    val pcFocus: Boolean = false,
    val connected: Boolean = false,
)

/**
 * PC Focus to phone Do Not Disturb, and phone Do Not Disturb to the PC, as the rest of the app sees it (2B.5, design 4). One per process, made by [AppGraph]; **it starts nothing by itself** (no
 * `LinkService` start, no polling, no timer but the one-shot of the 60 s grace). It owns the [DndFollower] (the rules), the [DndApplier] (the one writer of the phone's filter) and the reader: the
 * current filter once per ready session, and a dynamic receiver for `ACTION_INTERRUPTION_FILTER_CHANGED` **only while a session is ready and one of the two switches is on** (the broadcast needs no
 * access, which is why DND sharing works for someone who never turned on notification mirroring).
 *
 * Every event goes through one inbox and is handled one at a time (the follower is not thread-safe, and an action suspends on the store), so two events can never interleave.
 */
class DndController(private val graph: AppGraph) {
    private val context: Context = graph.appContext
    private val nm = context.getSystemService(NotificationManager::class.java)
    private val timers = CoroutineTimers(graph.scope)
    private val log = graph.log

    private val ports = AndroidDndPorts()
    val applier = DndApplier(ports, graph.dndStore, log)

    /** A release that failed (no access) is owed until it works; it is tried on the access-changed broadcast, on the watchdog alarm and at the start of the service (2B.5 review round 2, R44). */
    private val releases = DndReleaseGuard(
        release = { applier.release() }, onChange = { reevaluateReader() }, userChanged = { applier.userChangedWhileOwed(it) },
        persist = { applier.setOwed(it) },
    )

    private var follower: DndFollower? = null
    private val inbox = Channel<suspend () -> Unit>(Channel.UNLIMITED)
    private var graceTimer: Cancel? = null
    private var receiver: BroadcastReceiver? = null

    /** The watchdog alarm (see [DndWatchdog]): what makes a held silence end on time even when the phone slept through the timers. */
    private val alarm = DndAlarm(context)
    private var alarmSet = true // unknown after a process start: the first look cancels whatever an earlier process left

    /** `elapsedRealtime` of the last sign of life of the PC: the connection made, a snapshot, or a probe that found it answering. */
    @Volatile private var lastAliveAt = 0L

    @Volatile private var probing = false

    @Volatile private var followOn = false

    @Volatile private var shareOn = false

    @Volatile private var sessionReady = false

    @Volatile private var pcFocus = false

    private val mutableView = MutableStateFlow(DndView())
    val view: StateFlow<DndView> = mutableView

    private fun now(): Long = SystemClock.elapsedRealtime()

    /** Called once when the app starts: loads the switches and the persisted hold, releases a hold left by a crash or an update, then follows the switches, the pause and the link. Reacts only. */
    fun start() {
        graph.scope.launch {
            for (job in inbox) {
                try {
                    job()
                } catch (e: kotlinx.coroutines.CancellationException) {
                    throw e
                } catch (e: Exception) {
                    log.w("dnd", "dnd: an event failed (${e.javaClass.simpleName})")
                }
            }
        }
        // The first job of the inbox makes the follower, so every later event finds it.
        inbox.trySend {
            val hold = graph.dndStore.load()
            followOn = graph.features.enabled(FeatureIds.DND_FOLLOW)
            shareOn = graph.features.enabled(FeatureIds.DND_SHARE)
            val f = DndFollower(
                switchOn = followOn, access = applier.hasAccess, paused = graph.pause.paused(),
                follow = DndWire.levelFor(graph.dndStore.followFilterNow()), filter = DndWire.levelOf(ports.currentFilter()), weHold = hold.weHold,
                heldLevel = DndWire.levelOf(hold.heldFilter),
            )
            follower = f
            // A release that was owed when the last process died is still owed (it is persisted with the record); the receiver and the alarm follow it.
            releases.restore(hold.weHold && hold.releaseOwed)
            lastAliveAt = now()
            // A process start: a hold that outlived the process is released now (the crash, update and force-stop cases; spike S5 found Kin's rule survives `am force-stop`).
            execute(f, f.onStart(now()))
            reschedule(f)
            publish()
        }
        graph.scope.launch {
            graph.features.flow(FeatureIds.DND_FOLLOW).collect { on ->
                followOn = on
                post { f -> freshen(f); execute(f, f.onSwitch(now(), on)); reschedule(f) }
                reevaluateReader()
            }
        }
        graph.scope.launch {
            graph.features.flow(FeatureIds.DND_SHARE).collect { on ->
                shareOn = on
                reevaluateReader()
                graph.presence.sender.onDndChanged()
                publish()
            }
        }
        graph.scope.launch { graph.dndStore.followFilter.collect { word -> post { f -> f.onFollowLevel(DndWire.levelFor(word)) } } }
        graph.scope.launch { graph.dndStore.hold.collect { publish() } }
        graph.scope.launch {
            graph.pause.changes.collect {
                // The filter is read again first: during a grace the receiver is gone, so the user may have turned their own Do Not Disturb on, and a resume must not replace it (2B.5 review).
                post { f -> freshen(f); execute(f, f.onPause(now(), graph.pause.paused())); reschedule(f) }
                graph.presence.sender.onDndChanged()
            }
        }
    }

    /**
     * What the system says now, set on the follower before an event is evaluated: the filter (the receiver may not have been registered) and the access (it can be revoked while no receiver listens,
     * both switches off; 2B.5 review round 2).
     */
    private fun freshen(f: DndFollower) {
        f.setFilter(DndWire.levelOf(ports.currentFilter()))
        f.setAccess(applier.hasAccess)
    }

    /** Runs [block] on the follower after the previous event is done. */
    private fun post(block: suspend (DndFollower) -> Unit) {
        inbox.trySend {
            follower?.let { block(it) }
            publish()
        }
    }

    // ------------------------------------------------------------------ what happens to the link (PresenceHub tells us)

    /** The PC's `hello` was accepted: whether it lists `presence`, and the session is ready (the filter is read once, and the receiver registered if it is wanted). */
    fun onConnect(listsPresence: Boolean) {
        sessionReady = true
        lastAliveAt = now()
        post { f ->
            freshen(f)
            execute(f, f.onConnect(now(), listsPresence))
            reschedule(f)
        }
        reevaluateReader()
        graph.presence.sender.onDndChanged()
    }

    /** A `presence` snapshot of the PC: [focus] is true only when it carried `focus: true`. The first snapshot of a connection is a rising edge when it says so. */
    fun onSnapshot(focus: Boolean) {
        pcFocus = focus
        lastAliveAt = now()
        post { f ->
            freshen(f)
            execute(f, f.onSnapshot(now(), focus))
            reschedule(f)
        }
    }

    /** The connection closed (or the link was stopped). While Kin holds, the PC's Focus is kept for 60 s; the receiver goes away at once. */
    fun onClose() {
        sessionReady = false
        post { f ->
            execute(f, f.onClose(now()))
            reschedule(f)
        }
        reevaluateReader()
    }

    /** The PC was unlinked: Kin releases at once. */
    fun onUnlink() {
        sessionReady = false
        pcFocus = false
        post { f ->
            execute(f, f.onUnlink(now()))
            reschedule(f)
        }
        reevaluateReader()
    }

    /** Every `LinkService` start: whatever Kin still holds, with no connection yet, is released now (the follower does the same at the app's start). */
    fun onServiceStart() {
        post { f ->
            val actions = f.onStart(now())
            execute(f, actions)
            // A release that failed in an earlier run of the service (or of this process) is owed still: the follower holds nothing, so it has no action for it.
            if (actions.isEmpty()) releases.retry()
            reschedule(f)
        }
    }

    /**
     * The user came back from the system's Do Not Disturb access screen, or the access changed (the broadcast), or the app started: look at the access again. Cheap and idempotent. A release owed
     * from a time the access was missing is paid first, so a new request can't meet the old record.
     */
    fun refreshAccess() {
        post { f ->
            freshen(f)
            releases.retry()
            execute(f, f.onAccess(now(), applier.hasAccess))
            reschedule(f)
        }
    }

    /** "Release now" in Settings: Kin gives back its silence whatever the PC says. */
    fun releaseNow() {
        post { f ->
            val actions = f.onReleaseNow()
            execute(f, actions)
            if (actions.isEmpty()) releases.attempt()
            reschedule(f)
        }
    }

    /**
     * The watchdog alarm went off ([DndAlarm]); [done] is called when the work is finished (the receiver's `goAsync`). A due grace ends, a connected PC that has been silent for too long is asked
     * whether it still answers, and a hold with no connection and no grace is released: see [DndWatchdog.due].
     */
    fun onWatchdog(done: () -> Unit) {
        inbox.trySend {
            var finished = true
            try {
                val f = follower
                if (f != null) {
                    releases.retry(fromAlarm = true) // the owed release of a time the access was missing (round 2); a failure moves the backoff up (2B.6)
                    val t = now()
                    when (DndWatchdog.due(t, f.weHold, f.graceDeadline(), f.isConnected, lastAliveAt)) {
                        DndWatchdog.Due.NOTHING -> Unit
                        DndWatchdog.Due.GRACE_END -> execute(f, f.onTick(t))
                        DndWatchdog.Due.RELEASE -> {
                            log.w("dnd", "dnd: Kin held the silence with no connection and no grace; released by the watchdog")
                            execute(f, f.onStart(t))
                        }
                        DndWatchdog.Due.PROBE -> if (!probing) {
                            probing = true
                            finished = false
                            probe(done)
                        }
                    }
                    reschedule(f)
                    publish()
                }
            } finally {
                if (finished) done()
            }
        }
    }

    /**
     * The PC has been silent for [DndWatchdog.PROBE_AFTER_MILLIS] while Kin holds: a new TLS connection to it (no frame, nothing shown on the PC). An answer is a sign of life; no answer ends the
     * connection (the follower's 60 s grace starts) and, when there is no connection to end, starts that grace here.
     */
    private fun probe(done: () -> Unit) {
        graph.scope.launch {
            val alive = try {
                graph.link.runtime?.currentSession?.checkAlive() ?: false
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                false
            }
            inbox.trySend {
                try {
                    probing = false
                    if (alive) {
                        lastAliveAt = now()
                        follower?.let { reschedule(it) }
                    } else {
                        log.i("dnd", "dnd: the PC does not answer while Kin holds the silence; the 60 s grace starts")
                        follower?.let { f ->
                            execute(f, f.onClose(now()))
                            reschedule(f)
                        }
                    }
                    publish()
                } finally {
                    done()
                }
            }
        }
    }

    /** The phone's filter as `presence.dnd` says it: only while the user shares it and this phone is not paused (a paused device leaves it out); null is "off, or not shared". */
    fun sharedDnd(): String? = if (shareOn && !graph.pause.paused()) DndWire.wire(ports.currentFilter()) else null

    // ------------------------------------------------------------------ the reader

    @Synchronized
    private fun reevaluateReader() {
        // Also while a release is owed, whatever the switches say: the access coming back is what pays it.
        val want = (sessionReady && (followOn || shareOn)) || releases.pending
        if (want && receiver == null) {
            val r = object : BroadcastReceiver() {
                override fun onReceive(c: Context, intent: Intent) {
                    // The access can change while the session is up (the user turns it on in Settings while the PC is already in Focus): the follower looks at it again.
                    if (intent.action == NotificationManager.ACTION_NOTIFICATION_POLICY_ACCESS_GRANTED_CHANGED) refreshAccess() else onFilterChanged()
                }
            }
            val filter = IntentFilter().apply {
                addAction(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED)
                addAction(NotificationManager.ACTION_NOTIFICATION_POLICY_ACCESS_GRANTED_CHANGED)
            }
            ContextCompat.registerReceiver(context, r, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
            receiver = r
        } else if (!want) {
            receiver?.let { runCatching { context.unregisterReceiver(it) } }
            receiver = null
        }
    }

    val receiverRegistered: Boolean @Synchronized get() = receiver != null

    /** The system said the filter changed (the user, a schedule or Kin's own request): the follower hears it, and the PC is told within 1 s when the user shares it. */
    private fun onFilterChanged() {
        val raw = ports.currentFilter()
        val level = DndWire.levelOf(raw)
        post { f ->
            val actions = f.onFilter(now(), level)
            execute(f, actions)
            // No hold, but a release owed: the follower cannot see the user's change, the guard does (round 3).
            if (actions.isEmpty()) releases.onFilter(raw)
            reschedule(f)
        }
        graph.presence.sender.onDndChanged()
    }

    // ------------------------------------------------------------------ doing what the follower says

    private suspend fun execute(f: DndFollower, actions: List<DndAction>) {
        for (action in actions) {
            when (action) {
                is DndAction.Request -> if (applier.request(action.level)) releases.settled() else f.onRequestFailed()
                DndAction.Release -> releases.attempt()
                DndAction.Override -> if (applier.overridden()) releases.settled() else releases.owe()
                DndAction.Leave -> log.i("dnd", "dnd: the PC is in Focus but this phone is already in Do Not Disturb; nothing is changed")
            }
        }
    }

    /**
     * The 60 s grace is one one-shot timer, set when the follower says so and called off when it doesn't. While Kin holds, the watchdog alarm backs it (and the link's own dead-link detection) up,
     * because a coroutine `delay` does not run while the phone sleeps: see [DndWatchdog].
     */
    private fun reschedule(f: DndFollower) {
        graceTimer?.cancel()
        graceTimer = null
        val end = f.graceDeadline()
        if (end != null) graceTimer = timers.after(maxOf(0L, end - now())) { post { g -> execute(g, g.onTick(now())); reschedule(g) } }
        val wake = DndWatchdog.nextWake(f.weHold, end, lastAliveAt, releases.pending, now(), applier.hasAccess, releases.retryStep)
        if (wake != null) {
            alarm.set(maxOf(wake, now() + MIN_ALARM_LEAD_MILLIS))
            alarmSet = true
        } else if (alarmSet) {
            alarm.cancel()
            alarmSet = false
        }
    }

    private suspend fun publish() {
        val hold = graph.dndStore.load()
        mutableView.value = DndView(
            followOn = followOn, followFilter = graph.dndStore.followFilterNow(), shareOn = shareOn, access = applier.hasAccess,
            holding = hold.weHold, route = hold.route, heldFilter = hold.heldFilter, filter = ports.currentFilter(), pcFocus = pcFocus, connected = sessionReady,
        )
    }

    /** The debug state (`DND_STATE`): the switches, the access, the hold and the filter as numbers. Never a history. */
    suspend fun debugLine(): String {
        val hold = graph.dndStore.load()
        return "follow=$followOn followFilter=${graph.dndStore.followFilterNow()} share=$shareOn access=${applier.hasAccess} path=${applier.path()} weHold=${hold.weHold} heldFilter=${hold.heldFilter} " +
            "route=${hold.route} filter=${ports.currentFilter()} pcFocus=$pcFocus sessionReady=$sessionReady receiver=$receiverRegistered ruleModel=${ports.ruleModel} " +
            "followerFilter=${follower?.currentFilter?.word} followerHolds=${follower?.weHold} graceMs=${follower?.graceDeadline()?.let { it - now() }}"
    }

    private companion object {
        /** An alarm is never set closer than this (a due time already passed while a probe runs must not make the alarm go off at once, again and again). */
        const val MIN_ALARM_LEAD_MILLIS = 1_000L
    }

    private inner class AndroidDndPorts : DndPorts {
        override fun policyAccess(): Boolean = nm.isNotificationPolicyAccessGranted

        override fun listenerConnected(): Boolean = MirrorListenerService.isConnected()

        override fun currentFilter(): Int = nm.currentInterruptionFilter

        override fun setFilter(filter: Int) = nm.setInterruptionFilter(filter)

        override fun requestViaListener(filter: Int): Boolean = MirrorListenerService.requestFilter(filter)

        override val ruleModel: Boolean get() = Build.VERSION.SDK_INT >= 35
    }
}
