package app.kinrelay.android.media

import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.MessageIds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** An image to send as `media.image` (design 4.3): [hash] is the first 32 hex of SHA-256 of [bytes], [kind] `art` or `icon`. Kept in memory only. */
class MediaImageData(val hash: String, val kind: String, val bytes: ByteArray)

/**
 * **The only thing that writes `media.*` to the socket** (design 4.1). One lock, nothing on disk. The rules of 3.1, 3.6 and 3.7, checked for every message:
 * - a source sends `media.state`, `media.image`, `media.volume` and `media.clear` only if its own `share` is on, the connection is ready, the peer's latest `hello` lists `media`
 *   **and the peer's latest `media.status` says `show: true`** ([MediaGate]); otherwise the message is dropped and counted;
 * - a `media.state` goes out at most every 250 ms (the last state of a burst goes when the interval ends), an identical one (same fields, position within 1,500 ms of the
 *   extrapolation) is not sent, and a bucket of 10 refilled 1 per 250 ms is the budget; a `media.volume` is coalesced the same way;
 * - an image goes out **before the first `media.state` that uses its hash** on this connection, once per hash per connection, at most 20 per 60 s;
 * - "no session" is a `media.clear` after 1.5 s, cancelled if a session comes back (a track change often destroys and recreates a session);
 * - a viewer's `media.cmd` goes only to a peer that lists `media` and whose status says `share: true`, `seek` and `volume` coalesced (trailing 150 ms, last wins), at most 20 per 10 s;
 * - `media.status` goes to any peer that lists `media`, after each accepted `hello` and when a value changes, at most once per 5 s.
 * It never starts `LinkService`; with no connection it drops. The waits are coroutine `delay`s: nothing runs while the phone is idle.
 */
class MediaSender(
    private val scope: CoroutineScope,
    private val clock: Clock,
    private val link: () -> MediaLink?,
    private val log: KinLog = KinLog.None,
    private val newId: () -> String = MessageIds::newId,
    val stats: MediaStats = MediaStats(),
    /** The peer says it is paused (2B.4, design 3.5): an automatic push goes nowhere meanwhile (counted as `paused`). This device's own pause is `setOwnShare(false)`. */
    private val peerPaused: () -> Boolean = { false },
) {
    private class Latest(val snapshot: MediaSnapshot, val images: Map<String, MediaImageData>)

    private val lock = Any()
    private var connection: MediaLink? = null
    private var ownShare = false
    private var latest: Latest? = null
    private var shown = false
    private var clearJob: Job? = null
    private val imagesSent = HashSet<String>()
    private var stateBucket = MediaLimits.sourceState(clock)
    private var imageWindow = MediaLimits.image(clock)
    private var cmdWindow = MediaLimits.viewerCmd(clock)

    private val mutablePeerStatus = MutableStateFlow<MediaStatus?>(null)

    /** The peer's latest `media.status` on this connection, or null (before one, and after the connection changes). The Settings line reads it. */
    val peerStatus: StateFlow<MediaStatus?> = mutablePeerStatus

    private val stateThrottle = ValueThrottle<MediaSnapshot>(
        MediaLimits.STATE_THROTTLE_MILLIS, clock, scope, lock, same = { last, candidate, now -> last.sameAs(candidate, now) }, send = ::sendState,
    )
    private val volumeThrottle = ValueThrottle<MediaVolume>(MediaLimits.STATE_THROTTLE_MILLIS, clock, scope, lock, send = ::sendVolume)
    private val seekThrottle = ValueThrottle<MediaCmd>(MediaLimits.COMMAND_COALESCE_MILLIS, clock, scope, lock, same = { _, _, _ -> false }, send = ::sendCmdNow)
    private val volumeCmdThrottle = ValueThrottle<MediaCmd>(MediaLimits.COMMAND_COALESCE_MILLIS, clock, scope, lock, same = { _, _, _ -> false }, send = ::sendCmdNow)

    private var lastStatus: MediaStatus? = null
    private var lastStatusAt = Long.MIN_VALUE
    private var statusTimer: Job? = null
    private var pendingStatus: MediaStatus? = null

    /** The current connection if it is ready and lists `media`; a new connection starts a clean per-connection state (3.1: nothing about a peer is remembered across connections). */
    private fun usable(): MediaLink? {
        val raw = link()
        val c = raw?.takeIf { it.isReady && it.peerUnderstandsMedia }
        if (raw !== connection) {
            connection = raw
            imagesSent.clear()
            stateBucket = MediaLimits.sourceState(clock)
            imageWindow = MediaLimits.image(clock)
            cmdWindow = MediaLimits.viewerCmd(clock)
            mutablePeerStatus.value = null
            shown = false
            lastStatus = null
            statusTimer?.cancel()
            statusTimer = null
            pendingStatus = null
            stateThrottle.reset()
            volumeThrottle.reset()
            seekThrottle.reset()
            volumeCmdThrottle.reset()
        }
        return c
    }

    /** Whether a source may push right now. Counts the reason when not. */
    private fun mayPush(kind: PushKind, justTurnedOff: Boolean = false): MediaLink? {
        val c = usable() ?: run {
            stats.drop(MediaStats.OFFLINE)
            return null
        }
        if (kind != PushKind.CLEAR && peerPaused()) {
            stats.drop(MediaStats.PAUSED)
            return null
        }
        if (!MediaGate.mayPush(kind, ownShare, c.peerUnderstandsMedia, mutablePeerStatus.value, justTurnedOff)) {
            stats.drop(if (!ownShare && !justTurnedOff) MediaStats.SHARE_OFF else MediaStats.PEER_OFF)
            return null
        }
        return c
    }

    /** True when a source could push now (share on, a ready connection whose peer lists `media` and shows). Cheap: the source stops reading sessions when this is false. */
    fun isOpen(): Boolean = synchronized(lock) {
        val c = usable() ?: return false
        MediaGate.mayPush(PushKind.STATE, ownShare, c.peerUnderstandsMedia, mutablePeerStatus.value)
    }

    /** The peer's pause ended (it said so, or its end passed): the session that was held for it goes out now. */
    fun peerPauseEnded() {
        synchronized(lock) {
            if (!peerPaused()) resendLatest()
        }
    }

    /** Whether the connection is ready and the peer lists `media` (the status can go). */
    fun isOnline(): Boolean = synchronized(lock) { usable() != null }

    // ------------------------------------------------------------------ source: phone -> PC

    /** The user turned `share` on or off. Off while connected: one `media.clear` (if the viewer shows), then nothing (3.1). */
    fun setOwnShare(on: Boolean) {
        synchronized(lock) {
            if (on == ownShare) return
            ownShare = on
            if (!on) {
                clearJob?.cancel()
                clearJob = null
                stateThrottle.reset()
                volumeThrottle.reset()
                if (shown) {
                    val c = mayPush(PushKind.CLEAR, justTurnedOff = true)
                    if (c != null && c.sendEphemeral(MediaWire.clear(newId()))) stats.clearsSent.incrementAndGet()
                }
                shown = false
                latest = null
                // A viewer may drop its pictures with the session: the next session sends them again (a few KiB, once per session).
                imagesSent.clear()
            } else {
                resendLatest()
            }
            stats.maybeLog(clock.elapsedMillis(), log)
        }
    }

    /** The peer's `media.status` arrived: it is the gate. A viewer that now shows gets the current session; one that stopped showing gets nothing more. */
    fun onPeerStatus(status: MediaStatus) {
        synchronized(lock) {
            usable()
            val before = mutablePeerStatus.value
            mutablePeerStatus.value = status
            if (status.show && before?.show != true) {
                resendLatest()
            } else if (!status.show) {
                // The viewer removed what it showed: forget what was sent, so that its next `show` gets the session again from scratch. The session held for it goes too:
                // the source stops while the viewer doesn't show, so by the time it shows again that session may be long gone (a fresh read follows the next `show`).
                latest = null
                stateThrottle.reset()
                volumeThrottle.reset()
                clearJob?.cancel()
                clearJob = null
                shown = false
                imagesSent.clear()
            }
        }
    }

    /** The active session changed in a way that matters (or the first read): sent at once, or held for the 250 ms interval. Remembered, so that a viewer that starts showing later gets it. */
    fun update(snapshot: MediaSnapshot, images: List<MediaImageData>) {
        synchronized(lock) {
            clearJob?.cancel()
            clearJob = null
            latest = Latest(snapshot, images.associateBy { it.hash })
            if (mayPush(PushKind.STATE) == null) return
            stateThrottle.offer(snapshot)
        }
    }

    /** There is no candidate session any more: a `media.clear` follows after 1.5 s unless one comes back. Nothing to do when nothing was shown. */
    fun clear() {
        synchronized(lock) {
            latest = null
            if (!shown) {
                // Nothing was shown: a state that is still waiting for its interval must not go out for a session that is gone.
                stateThrottle.reset()
                return
            }
            if (clearJob != null) return
            clearJob = scope.launch {
                delay(MediaLimits.CLEAR_DEBOUNCE_MILLIS)
                synchronized(lock) {
                    clearJob = null
                    if (latest != null) return@synchronized
                    stateThrottle.reset()
                    volumeThrottle.reset()
                    val c = mayPush(PushKind.CLEAR)
                    if (c != null && c.sendEphemeral(MediaWire.clear(newId()))) stats.clearsSent.incrementAndGet()
                    shown = false
                    imagesSent.clear()
                    stats.maybeLog(clock.elapsedMillis(), log)
                }
            }
        }
    }

    /** The phone's volume changed (a `media.cmd volume` of its own, or a fresh read): coalesced, identical values not sent. */
    fun volume(volume: MediaVolume) {
        synchronized(lock) {
            if (mayPush(PushKind.VOLUME) == null) return
            volumeThrottle.offer(volume)
        }
    }

    private fun resendLatest() {
        val l = latest ?: return
        if (mayPush(PushKind.STATE) == null) return
        stateThrottle.reset()
        stateThrottle.offer(l.snapshot)
    }

    /** Called by the throttle under the lock. */
    private fun sendState(snapshot: MediaSnapshot) {
        val c = mayPush(PushKind.STATE) ?: return
        if (!stateBucket.tryTake()) {
            stats.drop(MediaStats.RATE)
            stateThrottle.forgetLast()
            return
        }
        val images = latest?.images.orEmpty()
        val now = clock.elapsedMillis()
        val artSent = snapshot.artHash?.let { hash -> images[hash]?.let { ensureImage(c, it) } } == true
        val iconSent = snapshot.iconHash?.let { hash -> images[hash]?.let { ensureImage(c, it) } } == true
        val frame = MediaWire.state(newId(), snapshot.toWire(now, withArt = artSent, withIcon = iconSent))
        if (frame == null || !c.sendEphemeral(frame)) {
            stats.drop(MediaStats.OFFLINE)
            stateThrottle.forgetLast()
            return
        }
        shown = true
        stats.statesSent.incrementAndGet()
        stats.maybeLog(now, log)
    }

    /** An image goes out once per hash per connection, before the state that uses it; false when the budget or the socket says no (the state then goes without the hash). */
    private fun ensureImage(c: MediaLink, image: MediaImageData): Boolean {
        if (image.hash in imagesSent) return true
        if (!imageWindow.tryTake()) {
            stats.drop(MediaStats.RATE)
            return false
        }
        if (!c.sendEphemeral(MediaWire.image(newId(), image.kind, image.bytes))) return false
        imagesSent += image.hash
        stats.imagesSent.incrementAndGet()
        return true
    }

    private fun sendVolume(volume: MediaVolume) {
        val c = mayPush(PushKind.VOLUME) ?: return
        if (c.sendEphemeral(MediaWire.volume(newId(), volume))) stats.volumesSent.incrementAndGet() else stats.drop(MediaStats.OFFLINE)
    }

    // ------------------------------------------------------------------ viewer: PC -> phone

    /** What a command did at once: [SENT] now, [HELD] for the 150 ms coalescing (it goes later, or a newer one replaces it), or [DROPPED] (counted). */
    enum class CmdResult { SENT, HELD, DROPPED }

    /**
     * A `media.cmd` from this phone's buttons. Only to a peer that lists `media` and whose latest status says `share: true`. `seek` and `volume` are coalesced (trailing 150 ms, last wins);
     * at most 20 commands per 10 s. Never re-sent, never queued: a command nobody saw is simply not visible to the user.
     */
    fun command(cmd: MediaCmd): CmdResult {
        synchronized(lock) {
            val c = usable() ?: run {
                stats.drop(MediaStats.OFFLINE)
                return CmdResult.DROPPED
            }
            if (!MediaGate.mayCommand(c.peerUnderstandsMedia, mutablePeerStatus.value)) {
                stats.drop(MediaStats.NO_SHARE)
                return CmdResult.DROPPED
            }
            return when (cmd.kind) {
                MediaCmdKind.SEEK -> hold(seekThrottle, cmd)
                MediaCmdKind.VOLUME -> hold(volumeCmdThrottle, cmd)
                else -> if (sendCmdNow(cmd)) CmdResult.SENT else CmdResult.DROPPED
            }
        }
    }

    private fun hold(throttle: ValueThrottle<MediaCmd>, cmd: MediaCmd): CmdResult {
        throttle.offer(cmd)
        return if (throttle.hasHeld()) CmdResult.HELD else CmdResult.SENT
    }

    /** Called under the lock, now or by a throttle. Checks the peer again: a held command may come due after the peer stopped sharing. */
    private fun sendCmdNow(cmd: MediaCmd): Boolean {
        val c = usable() ?: run {
            stats.drop(MediaStats.OFFLINE)
            return false
        }
        if (!MediaGate.mayCommand(c.peerUnderstandsMedia, mutablePeerStatus.value)) {
            stats.drop(MediaStats.NO_SHARE)
            return false
        }
        if (!cmdWindow.tryTake()) {
            stats.drop(MediaStats.RATE)
            return false
        }
        if (!c.sendEphemeral(MediaWire.cmd(newId(), cmd))) {
            stats.drop(MediaStats.OFFLINE)
            return false
        }
        stats.cmdsSent.incrementAndGet()
        stats.maybeLog(clock.elapsedMillis(), log)
        return true
    }

    // ------------------------------------------------------------------ both directions: media.status

    /** A new connection is ready (an accepted `hello`): the status goes out at once. Does nothing for a peer that doesn't list `media`. */
    fun onConnected(status: MediaStatus) {
        synchronized(lock) {
            val c = usable() ?: return
            if (!MediaGate.mayStatus(c.peerUnderstandsMedia)) return
            sendStatusNow(c, status)
        }
    }

    /** A switch changed: it goes out when it differs from the last one sent, at most once per 5 s (a change inside the window goes out when the window ends). */
    fun statusChanged(status: MediaStatus) {
        synchronized(lock) {
            val c = usable() ?: return
            if (status == lastStatus && pendingStatus == null) return
            val now = clock.elapsedMillis()
            if (lastStatusAt == Long.MIN_VALUE || now - lastStatusAt >= MediaLimits.STATUS_WINDOW_MILLIS) {
                sendStatusNow(c, status)
                return
            }
            pendingStatus = status
            if (statusTimer == null) {
                val wait = MediaLimits.STATUS_WINDOW_MILLIS - (now - lastStatusAt)
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
    }

    private fun sendStatusNow(c: MediaLink, status: MediaStatus) {
        pendingStatus = null
        if (c.sendEphemeral(MediaWire.status(newId(), status))) stats.statusesSent.incrementAndGet()
        lastStatus = status
        lastStatusAt = clock.elapsedMillis()
    }

    /** Sends [frame] (an `error` answering a `media.cmd`) on the current connection: not a `media.*` message, so no gate beyond "ready". */
    fun answer(frame: String): Boolean = synchronized(lock) { link()?.takeIf { it.isReady }?.sendEphemeral(frame) == true }

    /** Everything is forgotten without telling the peer (the connection closed, unpair). */
    fun reset() {
        synchronized(lock) {
            clearJob?.cancel()
            clearJob = null
            latest = null
            shown = false
            imagesSent.clear()
            stateThrottle.reset()
            volumeThrottle.reset()
            seekThrottle.reset()
            volumeCmdThrottle.reset()
            mutablePeerStatus.value = null
        }
    }
}
