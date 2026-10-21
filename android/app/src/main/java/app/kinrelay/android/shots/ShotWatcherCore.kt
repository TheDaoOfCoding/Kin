package app.kinrelay.android.shots

import app.kinrelay.android.link.Clock
import app.kinrelay.android.media.MediaBucket
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.CancellationException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** The switches as the feature resolver gives them. */
data class ShotSettings(val screenshots: Boolean, val camera: Boolean)

/** How an upload ended: sent, or dropped because the PC could not take it now (never queued). */
enum class ShotUpload { SENT, OFFLINE }

/** What the core needs from the phone. The real ports query MediaStore, post the hold notification and use the transfer engine; the tests record the calls. */
interface ShotPorts {
    /** Android lets Kin read all photos right now (checked at the point of use, never cached). */
    fun mayRead(): Boolean

    /** The highest `_ID` in `MediaStore.Images` now (the baseline taken when the feature is turned on). */
    fun maxId(): Long

    /** The rows with an `_ID` above [afterId], oldest first, at most [ShotWatcherCore.PAGE] of them. */
    fun queryNewer(afterId: Long): List<ShotRow>

    fun nowSeconds(): Long

    /** A ready connection to a PC whose latest `hello` lists `presence` exists right now (auto-send never queues). */
    fun readyForUpload(): Boolean

    /**
     * The silent "Sending screenshot (or photo) to <PC> in 5 s . Cancel" notification for [id]. True only when the hold is **really visible** (notifications allowed and on, the channel not
     * blocked): without it there is no Cancel, so the image is not sent.
     */
    fun showHold(id: Long, kind: ShotKind): Boolean

    /** Replaces the hold with a "Sending..." notification that has no Cancel (the image has left the hold). */
    fun showSending(id: Long)

    /** The result in the same notification for a moment ("Sent", "Not sent: PC away"), or null to just take it away. */
    fun showResult(id: Long, text: String?)

    /** Sends [row]'s image with `origin`. Never queued: [ShotUpload.OFFLINE] when the PC doesn't answer the first request in 5 s. */
    suspend fun upload(row: ShotRow, kind: ShotKind): ShotUpload

    /** Stores the baseline number (only that number: no name, path or image). */
    fun saveBaseline(id: Long)

    /** Stores the second at which the baseline was taken (a number: images taken before it are not new). */
    fun saveBaselineTime(seconds: Long)
}

/** Counts only (design 5.8): `shots: seen N, sent N, skipped {...}`. No names, paths or sizes. */
class ShotStats {
    val seen = AtomicLong()
    val sent = AtomicLong()
    val cancelled = AtomicLong()
    private val skipped = ConcurrentHashMap<String, AtomicLong>()

    fun skip(reason: String) {
        skipped.computeIfAbsent(reason) { AtomicLong() }.incrementAndGet()
    }

    fun skipped(reason: String): Long = skipped[reason]?.get() ?: 0L

    fun line(): String =
        "shots: seen ${seen.get()}, sent ${sent.get()}, cancelled ${cancelled.get()}, skipped {old: ${skipped("old")}, own: ${skipped("own")}, other: ${skipped("other")}, paused: ${skipped("paused")}, " +
            "rate: ${skipped("rate")}, offline: ${skipped("offline")}, stale: ${skipped("stale")}, off: ${skipped("off")}, no_hold: ${skipped("no_hold")}, not_image: ${skipped("not_image")}}"
}

/**
 * The logic of the screenshot watcher (design 5.6), with no Android in it. `onChange` is a bare signal from the `ContentObserver` (**no polling, no timer** while idle): it is coalesced 500 ms, then
 * the rows above the baseline are decided by [ShotPicker]. A candidate waits **5 s** with a Cancel before it leaves the phone. Auto-sent images are **never queued**: with no ready connection
 * after the hold they are dropped and counted.
 *
 * **The baseline** moves up past every row with a **final** decision (sent, cancelled, or skipped for any reason), in `_ID` order, and **stops at the first row that is still pending** (an
 * `IS_PENDING` row, or an image waiting out its hold). A pass reads the rows page by page (the query returns [PAGE] at most), so a long run of skipped images can't stall the watcher.
 *
 * **A hold has one owner at a time:** whoever removes it from the table first (the 5 s job claiming it, or the Cancel button) wins, under one short lock. The long work (the upload) holds no lock
 * the Cancel button needs, so a Cancel is never swallowed by an upload and never removes a "Sent" notification.
 */
class ShotWatcherCore(
    private val scope: CoroutineScope,
    clock: Clock,
    private val ports: ShotPorts,
    private val settings: () -> ShotSettings,
    /** This device is paused, or the peer is (3.5): nothing is sent, and the baseline moves. */
    private val paused: () -> Boolean,
    initialBaseline: Long,
    initialBaselineTimeSeconds: Long = 0,
    val stats: ShotStats = ShotStats(),
) {
    /** Serialises the passes over the rows (never held for an upload). */
    private val mutex = Mutex()

    /** Guards [holds], [coalesce] and the baseline: short, never held for I/O that waits. */
    private val lock = Any()
    private var coalesce: Job? = null
    private val holds = HashMap<Long, Job>()

    /** The `IS_PENDING` rows of the last pass (the observer fires again when one clears): the baseline never moves past them either. */
    private val pendingRows = HashSet<Long>()
    private val handled = LinkedHashSet<Long>()
    private val bucket = MediaBucket(RATE_CAPACITY, RATE_REFILL_MILLIS, clock)

    @Volatile var baseline: Long = initialBaseline
        private set

    @Volatile private var baselineTime: Long = initialBaselineTimeSeconds

    /** A change in the image collection (the observer's signal). Coalesced: one pass 500 ms after the first signal of a burst. */
    fun onChange() {
        synchronized(lock) {
            if (coalesce?.isActive == true) return
            coalesce = scope.launch {
                delay(COALESCE_MILLIS)
                process()
            }
        }
    }

    /** A switch turned on (or the feature enabled): the baseline moves up to the newest image now, so nothing that already exists is ever sent. */
    suspend fun retakeBaseline() {
        mutex.withLock {
            baselineTime = ports.nowSeconds()
            ports.saveBaselineTime(baselineTime)
            advance(ports.maxId())
        }
    }

    /**
     * The Cancel button of the hold notification: the hold goes and its job stops **now** (this doesn't wait for a pass or an upload), and the image never comes back (the baseline moves). With no
     * hold (it was claimed by the 5 s job a moment ago, or is gone) it does nothing: the "Sending..." or "Sent" notification stays.
     */
    fun cancel(id: Long) {
        val job = synchronized(lock) { holds.remove(id) } ?: return
        job.cancel()
        ports.showResult(id, null)
        stats.cancelled.incrementAndGet()
        advance(id)
    }

    /** Takes every hold away (the feature went off, the PC was unlinked): nothing is sent. An upload that already left its hold is not touched. */
    fun dropHolds() {
        val dropped = synchronized(lock) { holds.toMap().also { holds.clear() } }
        dropped.forEach { (id, job) ->
            job.cancel()
            ports.showResult(id, null)
        }
    }

    /** The number of images waiting out their hold (debug state). */
    val holding: Int get() = synchronized(lock) { holds.size }

    private suspend fun process() {
        mutex.withLock {
            val on = settings()
            if (!ports.mayRead() || (!on.screenshots && !on.camera)) return
            var cursor = baseline
            var frontier = baseline
            var blocked = false
            synchronized(lock) { pendingRows.clear() }
            var pages = 0
            while (pages++ < MAX_PAGES) {
                val rows = ports.queryNewer(cursor).filter { it.id > cursor }.sortedBy { it.id }
                if (rows.isEmpty()) break
                for (row in rows) {
                    cursor = row.id
                    val final = decide(row, on)
                    if (!final) blocked = true else if (!blocked) frontier = row.id
                }
                if (rows.size < PAGE) break
            }
            advance(frontier)
        }
    }

    /** Decides one row. True when the decision is **final** (the baseline may move past it); false while the row is still pending (`IS_PENDING`, or waiting out its hold). */
    private fun decide(row: ShotRow, on: ShotSettings): Boolean {
        if (isHeld(row.id)) return false
        if (row.id in handled || row.id <= baseline) return true
        val config = ShotConfig(on.screenshots, on.camera, paused(), baseline, handled, baselineTime)
        val decision = ShotPicker.decide(row, ports.nowSeconds(), config)
        if (decision is ShotDecision.Skip) {
            if (decision.reason == ShotPicker.SKIP_PENDING) {
                // A pending row older than the freshness window can only be stale when it clears: it is decided now, so a row stuck "pending" can't hold the baseline for ever.
                if (ports.nowSeconds() - row.dateAddedSeconds <= ShotPicker.STALE_SECONDS) {
                    synchronized(lock) { pendingRows += row.id }
                    return false
                }
                finalSkip(row.id, ShotPicker.SKIP_STALE)
                return true
            }
            finalSkip(row.id, decision.reason)
            return true
        }
        stats.seen.incrementAndGet()
        remember(row.id)
        if (!bucket.tryTake()) {
            stats.skip(ShotPicker.SKIP_RATE)
            return true
        }
        if (!startHold(row, (decision as ShotDecision.Send).kind)) {
            stats.skip(ShotPicker.SKIP_NO_HOLD)
            return true
        }
        return false
    }

    private fun finalSkip(id: Long, reason: String) {
        stats.seen.incrementAndGet()
        stats.skip(reason)
        remember(id)
    }

    private fun isHeld(id: Long): Boolean = synchronized(lock) { id in holds }

    /** False when the hold can't be seen by the user: the image is then not sent (there would be no Cancel). */
    private fun startHold(row: ShotRow, kind: ShotKind): Boolean {
        if (!ports.showHold(row.id, kind)) return false
        val job = scope.launch(start = CoroutineStart.LAZY) {
            delay(HOLD_MILLIS)
            finish(row, kind)
        }
        synchronized(lock) { holds[row.id] = job }
        job.start()
        return true
    }

    private suspend fun finish(row: ShotRow, kind: ShotKind) {
        // The claim: the hold is ours only if the Cancel button hasn't taken it. From here on the notification can't be cancelled (it says "Sending...").
        if (synchronized(lock) { holds.remove(row.id) } == null) return
        ports.showSending(row.id)
        try {
            val on = settings()
            val stillWanted = ports.mayRead() && (if (kind == ShotKind.SCREENSHOT) on.screenshots else on.camera)
            when {
                !stillWanted -> {
                    stats.skip(ShotPicker.SKIP_OFF)
                    ports.showResult(row.id, null)
                }
                paused() -> {
                    stats.skip(ShotPicker.SKIP_PAUSED)
                    ports.showResult(row.id, null)
                }
                !ports.readyForUpload() -> {
                    stats.skip(ShotPicker.SKIP_OFFLINE)
                    ports.showResult(row.id, "Not sent: PC away")
                }
                else -> when (ports.upload(row, kind)) {
                    ShotUpload.SENT -> {
                        stats.sent.incrementAndGet()
                        ports.showResult(row.id, "Sent")
                    }
                    ShotUpload.OFFLINE -> {
                        stats.skip(ShotPicker.SKIP_OFFLINE)
                        ports.showResult(row.id, "Not sent: PC away")
                    }
                }
            }
        } catch (e: CancellationException) {
            ports.showResult(row.id, null)
            throw e
        } catch (e: Exception) {
            stats.skip(ShotPicker.SKIP_OFFLINE)
            ports.showResult(row.id, "Not sent: PC away")
        }
        advance(row.id)
    }

    private fun remember(id: Long) {
        handled += id
        while (handled.size > HANDLED_KEPT) handled.remove(handled.first())
    }

    /**
     * The baseline only ever goes up (monotonic), and never past an image still pending (waiting out its hold, or `IS_PENDING`): it stops one below the oldest of them, so a cancelled newer image
     * can't carry the baseline over an older one that is still going to be decided.
     */
    private fun advance(id: Long) {
        val target = synchronized(lock) {
            val oldestPending = minOf(holds.keys.minOrNull() ?: Long.MAX_VALUE, pendingRows.minOrNull() ?: Long.MAX_VALUE)
            val limit = if (oldestPending != Long.MAX_VALUE) minOf(id, oldestPending - 1) else id
            if (limit <= baseline) return
            baseline = limit
            limit
        }
        ports.saveBaseline(target)
    }

    companion object {
        const val COALESCE_MILLIS = 500L
        const val HOLD_MILLIS = 5_000L
        const val RATE_CAPACITY = 5
        const val RATE_REFILL_MILLIS = 20_000L

        /** The most rows one query returns (the ports honour it): a pass asks again from the last row until a page is not full. */
        const val PAGE = 50

        /** At most this many pages in one pass (a restored library of thousands of newer images can't hold the watcher for ever). */
        private const val MAX_PAGES = 40
        private const val HANDLED_KEPT = 50
    }
}
