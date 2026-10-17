package app.kinrelay.android.notifmirror

import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog

/**
 * What the engine needs from Android, for a notification of type [T] (`StatusBarNotification` in the app; a fake in the unit tests, which have
 * no Android classes). [quick] reads only the cheap fields; [snapshot] reads the rest and is called only after the first three skip rules passed.
 */
interface SbnPort<T> {
    fun quick(sbn: T): QuickInfo
    fun key(sbn: T): String
    fun snapshot(sbn: T): NotifSnapshot
    fun appLabel(packageName: String): String

    /** The app icon, drawn once per app version and cached; null when it can't be had. Only called when something will be sent. */
    fun icon(packageName: String, sbn: T): IconData?
}

/**
 * The listener's decisions, one notification at a time and in order (the service feeds it from one coroutine): the skip rules, the encoding, the
 * remove rules (design 4.4). It never keeps content: only the counters and, in [MirrorSender], the `nid` to key map in memory.
 */
class MirrorEngine<T>(
    private val port: SbnPort<T>,
    private val policy: suspend () -> MirrorPolicy,
    private val filter: MirrorFilter,
    private val sender: MirrorSender,
    private val clock: Clock,
    private val log: KinLog = KinLog.None,
) {
    private val stats get() = sender.stats
    private val selfCancelled = java.util.concurrent.ConcurrentHashMap<String, Long>()

    suspend fun onPosted(sbn: T) {
        val current = policy()
        if (!current.enabled) return
        filter.pre(port.quick(sbn), current)?.let {
            stats.drop(it.code)
            return
        }
        if (!sender.isOnline()) {
            // Nothing can be shown on the PC: no point in reading the notification. Dropped, not queued (a stale toast is worse than none).
            stats.drop(MirrorSender.OFFLINE)
            return
        }
        val snapshot = port.snapshot(sbn)
        when (val decision = filter.post(snapshot, current)) {
            is FilterDecision.Skip -> {
                stats.drop(decision.reason.code)
                // It was mirrored and no longer qualifies (it became ongoing, or was hidden): the toast goes.
                val nid = Nid.of(snapshot.key)
                if (sender.isAnnounced(nid)) sender.remove(nid)
            }
            is FilterDecision.Send -> {
                val nid = Nid.of(snapshot.key)
                // A flagged or hidden notification goes out as its app name only: no icon either.
                val icon = if (decision.sensitive) null else port.icon(snapshot.packageName, sbn)
                val post = NotifEncoder.encode(snapshot, port.appLabel(snapshot.packageName), decision.sensitive, sender.isAnnounced(nid), icon?.hash)
                sender.post(PostRequest(snapshot.packageName, snapshot.key, post, icon))
            }
        }
        stats.maybeLog(clock.elapsedMillis(), log)
    }

    suspend fun onRemoved(sbn: T, reason: Int) {
        if (reason in IGNORED_REMOVE_REASONS) return
        val current = policy()
        if (!current.enabled) return
        if (filter.pre(port.quick(sbn), current) != null) return
        val key = port.key(sbn)
        val nid = Nid.of(key)
        val now = clock.elapsedMillis()
        selfCancelled.entries.removeAll { now - it.value > SELF_CANCEL_MEMORY_MILLIS }
        if (selfCancelled.remove(nid) != null) {
            // Kin cancelled it because the PC dismissed it: the PC knows, nothing is echoed back.
            sender.forget(nid)
            return
        }
        if (sender.isAnnounced(nid) || wouldHaveBeenSent(sbn, current)) sender.remove(nid)
        stats.maybeLog(now, log)
    }

    /** After a restart nothing is remembered: a removed notification that would have been mirrored still gets its `notif.remove`. */
    private fun wouldHaveBeenSent(sbn: T, current: MirrorPolicy): Boolean =
        sender.isOnline() && filter.post(port.snapshot(sbn), current) is FilterDecision.Send

    /** Kin is about to cancel [nid] because the PC dismissed it: the `onNotificationRemoved` that follows is not echoed (for 5 s). */
    fun noteSelfCancel(nid: String) {
        selfCancelled[nid] = clock.elapsedMillis()
    }

    /** The `nid`s of the notifications that are active, allowed and not skipped (what `notif.sync` lists). A one-off pass; nothing is cached. */
    suspend fun activeNids(active: List<T>): List<String> {
        val current = policy()
        if (!current.enabled) return emptyList()
        return active.asSequence()
            .filter { filter.pre(port.quick(it), current) == null }
            .map { port.snapshot(it) }
            .filter { filter.post(it, current) is FilterDecision.Send }
            .map { Nid.of(it.key) }
            .take(NotifWire.MAX_SYNC_NIDS)
            .toList()
    }

    companion object {
        /**
         * 13 (`REASON_GROUP_OPTIMIZATION`) and 16 (`REASON_UNAUTOBUNDLED`) are the system regrouping, and it re-posts. 10 and 11 (`REASON_LISTENER_CANCEL`
         * and `_CANCEL_ALL`) are not ignored: another listener (a watch) dismissing a notification must remove the toast, and Kin's own cancel is
         * recognised by the 5-second memory of [noteSelfCancel].
         */
        val IGNORED_REMOVE_REASONS = setOf(13, 16)
        const val SELF_CANCEL_MEMORY_MILLIS = 5_000L
    }
}
