package app.kinrelay.android.media

import app.kinrelay.android.log.KinLog
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** Counters for the log line and the debug state: **counts only**, never a title, artist, album, app name, `sid`, art or position (design 4.11). */
class MediaStats {
    val statesSent = AtomicLong()
    val imagesSent = AtomicLong()
    val volumesSent = AtomicLong()
    val clearsSent = AtomicLong()
    val statusesSent = AtomicLong()
    val cmdsSent = AtomicLong()
    val statesShown = AtomicLong()

    /** Commands from the PC that were run, by name (`play`, `pause`, ... `volume`), and what happened to the others, by error code. */
    private val cmdsIn = ConcurrentHashMap<String, AtomicLong>()
    private val cmdErrors = ConcurrentHashMap<String, AtomicLong>()
    private val drops = ConcurrentHashMap<String, AtomicLong>()

    @Volatile var lastCmdIn: String = "none"

    fun cmdIn(name: String) {
        cmdsIn.computeIfAbsent(name) { AtomicLong() }.incrementAndGet()
        lastCmdIn = name
    }

    fun cmdError(code: String) {
        cmdErrors.computeIfAbsent(code) { AtomicLong() }.incrementAndGet()
    }

    fun drop(reason: String) {
        drops.computeIfAbsent(reason) { AtomicLong() }.incrementAndGet()
    }

    fun dropped(reason: String): Long = drops[reason]?.get() ?: 0L

    fun cmdsIn(name: String): Long = cmdsIn[name]?.get() ?: 0L

    fun totalCmdsIn(): Long = cmdsIn.values.sumOf { it.get() }

    fun cmdsInLine(): String = cmdsIn.entries.sortedBy { it.key }.joinToString(",") { "${it.key}:${it.value.get()}" }

    fun cmdErrorsLine(): String = cmdErrors.entries.sortedBy { it.key }.joinToString(",") { "${it.key}:${it.value.get()}" }

    fun dropsLine(names: List<String> = DROP_REASONS): String = names.joinToString(",") { "$it:${dropped(it)}" }

    fun total(): Long = statesSent.get() + imagesSent.get() + volumesSent.get() + clearsSent.get() + statusesSent.get() + cmdsSent.get() + statesShown.get() +
        totalCmdsIn() + drops.values.sumOf { it.get() } + cmdErrors.values.sumOf { it.get() }

    fun line(): String {
        val reasons = drops.entries.sortedBy { it.key }.joinToString(", ") { "${it.key}: ${it.value.get()}" }
        return "media: sent ${statesSent.get()} states, ${imagesSent.get()} images, ${cmdsSent.get()} cmds, shown ${statesShown.get()} states, ${totalCmdsIn()} cmds in, dropped {$reasons}"
    }

    private var lastLogged = Long.MIN_VALUE / 2
    private var lastTotal = 0L

    /** At most one line a minute, and only when something changed since the last: no timer, no wake-up while idle (design 4.8, 4.11). */
    @Synchronized
    fun maybeLog(now: Long, log: KinLog, everyMillis: Long = 60_000L) {
        val total = total()
        if (total == lastTotal || now - lastLogged < everyMillis) return
        lastLogged = now
        lastTotal = total
        log.i("media", line())
    }

    companion object {
        const val OFFLINE = "offline"
        const val PEER_OFF = "peer_off"
        const val RATE = "rate"
        const val VIEWER_OFF = "viewer_off"
        const val NO_SHARE = "peer_no_share"
        const val SHARE_OFF = "share_off"
        const val BAD_IMAGE = "bad_image"

        /** Dropped because this device or the peer is paused (2B.4). */
        const val PAUSED = "paused"

        /** A `media.*` message arrived while a side's latest `hello` doesn't list `media`. */
        const val NOT_LISTED = "not_listed"
        val DROP_REASONS = listOf(OFFLINE, PEER_OFF, RATE, SHARE_OFF, VIEWER_OFF, NO_SHARE, BAD_IMAGE, NOT_LISTED, PAUSED)
    }
}
