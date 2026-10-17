package app.kinrelay.android.notifmirror

import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.MessageIds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicLong

/** What the sender needs of a connection: it is ready, its peer's latest `hello` lists `notif`, and a frame can be put on the wire (best effort, no ack). */
interface MirrorLink {
    val isReady: Boolean
    val peerUnderstandsNotif: Boolean

    /** Puts the frame on the socket. False when it could not be queued (the socket is closed). Nothing is kept or re-sent. */
    fun sendEphemeral(frame: String): Boolean
}

/** An app icon as a PNG (design 4.6). [hash] is the `notif.icon` hash. */
class IconData(val hash: String, val png: ByteArray)

/** One post to send. [icon] is the app's icon (sent once per connection before the first post that uses its hash). [key] stays on the phone. */
class PostRequest(val packageName: String, val key: String, val post: WirePost, val icon: IconData? = null)

/** What [MirrorSender.post] did with a post at once (a coalesced one is sent later, unless something replaces it). */
enum class PostResult { SENT, COALESCED, IDENTICAL, DROPPED_OFFLINE, DROPPED_PACKAGE, DROPPED_BUDGET }

/** The state the PC's Settings shows (design 3.3, `notif.status`). */
data class MirrorStatus(val enabled: Boolean, val accessGranted: Boolean, val apps: Int, val otpHidden: Boolean)

/** Counters for the log line and the debug screen: counts and app names only, never content. */
class MirrorStats {
    val sent = AtomicLong()
    val removed = AtomicLong()
    val identical = AtomicLong()
    val actions = AtomicLong()
    val dismissed = AtomicLong()
    private val drops = java.util.concurrent.ConcurrentHashMap<String, AtomicLong>()

    fun drop(reason: String) {
        drops.computeIfAbsent(reason) { AtomicLong() }.incrementAndGet()
    }

    fun dropped(reason: String): Long = drops[reason]?.get() ?: 0L

    fun total(): Long = sent.get() + removed.get() + identical.get() + actions.get() + dismissed.get() + drops.values.sumOf { it.get() }

    fun line(): String {
        val reasons = drops.entries.sortedBy { it.key }.joinToString(", ") { "${it.key}: ${it.value.get()}" }
        return "mirror: sent ${sent.get()}, removed ${removed.get()}, dropped {$reasons}"
    }

    private var lastLogged = Long.MIN_VALUE / 2
    private var lastTotal = 0L

    /** At most one line a minute, and only when something changed since the last: no timer, no wake-up while idle (design 4.11, 4.12). */
    @Synchronized
    fun maybeLog(now: Long, log: KinLog, everyMillis: Long = 60_000L) {
        val total = total()
        if (total == lastTotal || now - lastLogged < everyMillis) return
        lastLogged = now
        lastTotal = total
        log.i("mirror", line())
    }
}

/** A token bucket on the injected clock: [capacity] tokens, one more every [refillMillis]. */
class TokenBucket(private val capacity: Int, private val refillMillis: Long, private val clock: Clock) {
    private var tokens = capacity.toDouble()
    private var at = clock.elapsedMillis()

    @Synchronized
    fun tryTake(): Boolean {
        val now = clock.elapsedMillis()
        tokens = minOf(capacity.toDouble(), tokens + (now - at).toDouble() / refillMillis)
        at = now
        if (tokens < 1.0) return false
        tokens -= 1.0
        return true
    }
}

/**
 * **The only thing that writes `notif.*` to the socket** (design 4.1). One lock, no queue on disk: a post is sent now, held for at most a
 * second (the last state of a `nid` goes out when the second ends), or dropped. The rules of 3.5:
 * - the peer's latest `hello` lists `notif` and the connection is ready, checked per message; otherwise the message is dropped and counted (3.6);
 * - an icon goes out before the first post that uses its hash on this connection;
 * - a `notif.remove` cancels a pending, not-yet-sent post for the same `nid` and always goes out (up to 50 per 10 s);
 * - at most one post per `nid` per second (trailing), an identical post is not sent, 20 posts per package per minute, and a bucket of 10 for all;
 * - the status goes out on each connection at once and when it changes, at most once per 5 s.
 *
 * The coalescing window is a coroutine `delay`, not an alarm: nothing runs while the phone is idle.
 */
class MirrorSender(
    private val scope: CoroutineScope,
    private val clock: Clock,
    private val link: () -> MirrorLink?,
    private val log: KinLog = KinLog.None,
    private val newId: () -> String = MessageIds::newId,
    val stats: MirrorStats = MirrorStats(),
) {
    private class NidState(var lastSentAt: Long = NEVER, var lastHash: String? = null, var pending: PostRequest? = null, var timer: Job? = null)

    private class Announced(val key: String, val packageName: String)

    private val lock = Any()
    private var connection: MirrorLink? = null
    private val iconsSent = HashSet<String>()
    private val states = LinkedHashMap<String, NidState>()
    private val announced = LinkedHashMap<String, Announced>()
    private val bucket = TokenBucket(BUCKET_CAPACITY, BUCKET_REFILL_MILLIS, clock)
    private val packageTimes = HashMap<String, ArrayDeque<Long>>()
    private val removeTimes = ArrayDeque<Long>()
    private var lastStatus: MirrorStatus? = null
    private var lastStatusAt = NEVER
    private var statusTimer: Job? = null
    private var pendingStatus: MirrorStatus? = null

    /** The connection to use, or null: not connected, no accepted `hello`, or a peer without `notif` (design 3.1 and 3.6). Changing connections starts a clean per-connection state. */
    private fun usable(): MirrorLink? {
        val c = link()?.takeIf { it.isReady && it.peerUnderstandsNotif }
        if (c != null && c !== connection) {
            connection = c
            iconsSent.clear()
            states.values.forEach { it.timer?.cancel() }
            states.clear()
            packageTimes.clear()
            lastStatus = null
            statusTimer?.cancel()
            statusTimer = null
            pendingStatus = null
        }
        return c
    }

    fun post(request: PostRequest): PostResult = synchronized(lock) {
        val c = usable() ?: run {
            stats.drop(OFFLINE)
            return PostResult.DROPPED_OFFLINE
        }
        val nid = request.post.nid
        remember(nid, request.key, request.packageName)
        val state = states.getOrPut(nid) { NidState() }
        trimStates(nid)
        val now = clock.elapsedMillis()
        if (state.lastSentAt != NEVER && now - state.lastSentAt < COALESCE_MILLIS) {
            state.pending = request
            if (state.timer == null) {
                val wait = COALESCE_MILLIS - (now - state.lastSentAt)
                state.timer = scope.launch {
                    delay(wait)
                    firePending(nid)
                }
            }
            return PostResult.COALESCED
        }
        sendNow(c, state, request, now)
    }

    private fun firePending(nid: String) = synchronized(lock) {
        val state = states[nid] ?: return@synchronized
        state.timer = null
        val request = state.pending ?: return@synchronized
        state.pending = null
        val c = usable() ?: run {
            stats.drop(OFFLINE)
            return@synchronized
        }
        sendNow(c, state, request, clock.elapsedMillis())
        stats.maybeLog(clock.elapsedMillis(), log)
    }

    private fun sendNow(c: MirrorLink, state: NidState, request: PostRequest, now: Long): PostResult {
        val hash = request.post.contentHash()
        if (hash == state.lastHash) {
            stats.identical.incrementAndGet()
            return PostResult.IDENTICAL
        }
        val times = packageTimes.getOrPut(request.packageName) { ArrayDeque() }
        while (times.isNotEmpty() && now - times.first() >= PACKAGE_WINDOW_MILLIS) times.removeFirst()
        // The package cap comes first: a post over it costs no token of the shared budget.
        if (times.size >= PACKAGE_LIMIT) {
            stats.drop(RATE)
            return PostResult.DROPPED_PACKAGE
        }
        if (!bucket.tryTake()) {
            stats.drop(RATE)
            return PostResult.DROPPED_BUDGET
        }
        val icon = request.icon
        if (icon != null && request.post.iconHash == icon.hash && icon.hash !in iconsSent) {
            if (c.sendEphemeral(NotifWire.icon(newId(), icon.png))) iconsSent += icon.hash
        }
        if (!c.sendEphemeral(NotifWire.post(newId(), request.post))) {
            stats.drop(OFFLINE)
            return PostResult.DROPPED_OFFLINE
        }
        state.lastSentAt = now
        state.lastHash = hash
        times.addLast(now)
        stats.sent.incrementAndGet()
        stats.maybeLog(now, log)
        return PostResult.SENT
    }

    /**
     * The notification [nid] is gone from the phone. A pending post for it is cancelled and the remove always goes out (up to 50 per 10 s), and the
     * next post of that `nid` is never taken for "identical" to one before the remove.
     */
    fun remove(nid: String): Boolean = synchronized(lock) {
        states.remove(nid)?.let { it.timer?.cancel() }
        announced.remove(nid)
        val c = usable() ?: run {
            stats.drop(OFFLINE)
            return false
        }
        val now = clock.elapsedMillis()
        while (removeTimes.isNotEmpty() && now - removeTimes.first() >= REMOVE_WINDOW_MILLIS) removeTimes.removeFirst()
        if (removeTimes.size >= REMOVE_LIMIT) {
            stats.drop(RATE)
            return false
        }
        removeTimes.addLast(now)
        val sent = c.sendEphemeral(NotifWire.remove(newId(), nid))
        if (sent) stats.removed.incrementAndGet() else stats.drop(OFFLINE)
        stats.maybeLog(now, log)
        sent
    }

    /** True when a post could go out now: a ready connection whose peer lists `notif`. Cheap, so the listener can skip reading a notification that has nowhere to go. */
    fun isOnline(): Boolean = synchronized(lock) { usable() != null }

    /** Drops what is remembered about [nid] without telling the PC (the PC itself asked for the dismissal). */
    fun forget(nid: String) = synchronized(lock) {
        states.remove(nid)?.let { it.timer?.cancel() }
        announced.remove(nid)
        Unit
    }

    /** The user turned mirroring off, or took apps off the list: a remove for every announced notification of those apps ([only] null: all). Returns how many were announced. */
    fun removeAnnounced(only: ((String) -> Boolean)? = null): Int {
        val nids = synchronized(lock) { announced.entries.filter { only == null || only(it.value.packageName) }.map { it.key } }
        nids.forEach { remove(it) }
        return nids.size
    }

    /** True when [nid] was announced and has not been removed since (this process only). */
    fun isAnnounced(nid: String): Boolean = synchronized(lock) { nid in announced }

    /** The notification key behind a `nid` this phone announced, from memory only (never stored). */
    fun keyOf(nid: String): String? = synchronized(lock) { announced[nid]?.key }

    /** The packages of the announced notifications and how many (for the debug state, counts only). */
    fun announcedCount(): Int = synchronized(lock) { announced.size }

    /** The per-`nid` state of a long-running process stays bounded: the oldest ones with nothing pending go first (at worst a post is sent again, still under the rate limits). */
    private fun trimStates(keep: String) {
        if (states.size <= STATE_LIMIT) return
        val it = states.entries.iterator()
        while (states.size > STATE_LIMIT && it.hasNext()) {
            val e = it.next()
            if (e.key != keep && e.value.timer == null && e.value.pending == null) it.remove()
        }
    }

    private fun remember(nid: String, key: String, packageName: String) {
        announced.remove(nid)
        announced[nid] = Announced(key, packageName)
        while (announced.size > ANNOUNCED_LIMIT) announced.remove(announced.keys.first())
    }

    /**
     * A new connection is ready (both `hello`s accepted): the status goes out, then the sync with the `nid`s that are active, allowed and not skipped
     * (design 3.6). No content is sent: what was on the phone before is not shown. Does nothing when the peer does not list `notif`.
     */
    fun onConnected(status: MirrorStatus, activeNids: List<String>) = synchronized(lock) {
        val c = usable() ?: return@synchronized
        sendStatusNow(c, status)
        c.sendEphemeral(NotifWire.sync(newId(), activeNids.take(NotifWire.MAX_SYNC_NIDS)))
    }

    /** The status changed: it goes out when it differs from the last one sent, at most once per 5 s (a change inside the window goes out when the window ends). */
    fun statusChanged(status: MirrorStatus) = synchronized(lock) {
        val c = usable() ?: return@synchronized
        if (status == lastStatus && pendingStatus == null) return@synchronized
        val now = clock.elapsedMillis()
        if (lastStatusAt == NEVER || now - lastStatusAt >= STATUS_WINDOW_MILLIS) {
            sendStatusNow(c, status)
            return@synchronized
        }
        pendingStatus = status
        if (statusTimer == null) {
            val wait = STATUS_WINDOW_MILLIS - (now - lastStatusAt)
            statusTimer = scope.launch {
                delay(wait)
                synchronized(lock) {
                    statusTimer = null
                    val next = pendingStatus ?: return@synchronized
                    pendingStatus = null
                    usable()?.let { sendStatusNow(it, next) }
                }
            }
        }
    }

    private fun sendStatusNow(c: MirrorLink, status: MirrorStatus) {
        pendingStatus = null
        c.sendEphemeral(NotifWire.status(newId(), status.enabled, status.accessGranted, status.apps, status.otpHidden))
        lastStatus = status
        lastStatusAt = clock.elapsedMillis()
    }

    companion object {
        const val COALESCE_MILLIS = 1_000L
        const val BUCKET_CAPACITY = 10
        const val BUCKET_REFILL_MILLIS = 500L
        const val PACKAGE_LIMIT = 20
        const val PACKAGE_WINDOW_MILLIS = 60_000L
        const val REMOVE_LIMIT = 50
        const val REMOVE_WINDOW_MILLIS = 10_000L
        const val STATUS_WINDOW_MILLIS = 5_000L
        const val ANNOUNCED_LIMIT = 200
        const val STATE_LIMIT = 500
        const val OFFLINE = "offline"
        const val RATE = "rate"
        private const val NEVER = Long.MIN_VALUE
    }
}
