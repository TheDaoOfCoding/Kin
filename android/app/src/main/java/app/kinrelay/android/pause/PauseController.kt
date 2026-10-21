package app.kinrelay.android.pause

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.presence.Cancel
import app.kinrelay.android.presence.Timers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import java.time.ZoneId
import java.util.concurrent.atomic.AtomicInteger

/** Where the pause lives between runs. The app's is [PauseStore] (DataStore `kin_pause`); the tests use memory. */
interface PauseStorage {
    suspend fun current(): PauseState

    suspend fun set(state: PauseState)
}

/** Why [PauseController.changes] emitted: the user (or a debug command) set or resumed the pause, or a timed pause ended by itself (the presence sender tells the peer at once). */
enum class PauseChange { CHANGED, EXPIRED }

/** What the screens show: the state as it is now, whether it is in force, and how many manual arrivals were handled quietly since the pause began (in memory, reset on resume). */
data class PauseView(val state: PauseState, val paused: Boolean, val receivedWhilePaused: Int)

/**
 * This device's Pause (design 3.5 and 5.3): one persisted state, a master gate read at each entry of an automatic feature (**`paused()` is a wall-clock comparison**, so nothing is started
 * or stopped by pausing and no timer is needed to resume locally). The only timer is the one-shot [expiry], which tells the peer (through [changes]) when the pause ends. A second pause
 * replaces the first; Resume clears it. Unlinking the last peer clears it.
 */
class PauseController(
    private val storage: PauseStorage,
    private val timers: Timers,
    private val log: KinLog,
    private val now: () -> Long = System::currentTimeMillis,
    private val zone: () -> ZoneId = { ZoneId.systemDefault() },
) {
    private val loaded = CompletableDeferred<Unit>()
    private val mutableState = MutableStateFlow(PauseState.NONE)
    private val mutableView = MutableStateFlow(PauseView(PauseState.NONE, paused = false, receivedWhilePaused = 0))
    private val quiet = AtomicInteger()
    private var expiry: Cancel? = null

    /** The persisted state (after [load]). */
    val state: StateFlow<PauseState> = mutableState

    /** The state with `paused` evaluated now; republished when the state changes and when a pause expires. */
    val view: StateFlow<PauseView> = mutableView

    private val mutableChanges = MutableSharedFlow<PauseChange>(extraBufferCapacity = 8)

    /** Emits whenever the pause was set, resumed or expired: the presence sender tells the peer, the media hub and the notification re-evaluate. */
    val changes: SharedFlow<PauseChange> = mutableChanges

    /** Reads the persisted pause (at service start and after a reboot) and arms the expiry. The peer is told in the first `presence`. */
    suspend fun load() {
        var saved = PauseState.NONE
        try {
            saved = storage.current()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The store can't be read: the gates must still answer (they wait for this load), so Kin starts not paused.
            log.w("pause", "pause: the stored pause could not be read (${e.javaClass.simpleName}); starting not paused")
        } finally {
            mutableState.value = saved
            loaded.complete(Unit)
        }
        afterChange(emit = false)
    }

    /**
     * The pause as it is **now**: [view] is only republished when something changes or the expiry timer fires, and a timer that was set before a deep sleep may not have run yet, so the tile, Home
     * and the notification read this (the wall clock again) instead of trusting the last published `paused`. Doesn't wait for the first load (not paused before it).
     */
    fun viewNow(): PauseView {
        val st = mutableState.value
        val on = PauseGate.paused(st, now())
        return PauseView(st, on, if (on) quiet.get() else 0)
    }

    /** Publishes [viewNow] (a screen came back, the tile started listening): what a pause that ended during deep sleep shows is corrected at once. */
    fun refresh() = publish()

    /** True while paused. The gate every automatic feature checks next to its own switch; cheap, wall-clock. */
    fun paused(): Boolean = PauseGate.paused(stateNow(), now())

    /** [paused] without waiting for the first load (false before it): for UI text that must not block the main thread. */
    fun pausedIfLoaded(): Boolean = loaded.isCompleted && paused()

    /** The state; waits (briefly) for the first load so a gate asked in the first milliseconds of a process can't say "not paused" by mistake. */
    fun stateNow(): PauseState {
        if (!loaded.isCompleted) runBlocking { loaded.await() }
        return mutableState.value
    }

    suspend fun set(choice: PauseChoice) = apply(PauseGate.stateFor(choice, now(), zone()), "pause $choice")

    suspend fun resume() = apply(PauseState.NONE, "resume")

    /** Sets a state as it is (debug commands and tests: a `pausedUntil` in the past or far ahead). */
    suspend fun setRaw(state: PauseState) = apply(state, "set")

    /** The pause as `presence` says it. */
    fun wire() = PauseGate.wire(stateNow(), now())

    /** A manual message handled quietly while paused (3.5): counted in memory ("3 received while paused"). */
    fun noteQuietArrival() {
        quiet.incrementAndGet()
        publish()
    }

    /** Unlinking the last peer clears the pause. */
    suspend fun onLastPeerUnlinked() {
        if (stateNow() != PauseState.NONE) apply(PauseState.NONE, "unlink")
    }

    private suspend fun apply(next: PauseState, why: String) {
        if (!loaded.isCompleted) loaded.await()
        storage.set(next)
        mutableState.value = next
        if (next == PauseState.NONE) quiet.set(0)
        // The kind of pause only, never its end.
        val kind = if (next.untilResumed) "until resumed" else if (next == PauseState.NONE) "off" else "timed"
        log.i("pause", "pause: $why ($kind)")
        afterChange(emit = true)
    }

    private fun afterChange(emit: Boolean) {
        expiry?.cancel()
        expiry = null
        val st = mutableState.value
        val ahead = st.untilMillis - now()
        if (!st.untilResumed && ahead > 0 && PauseGate.paused(st, now())) {
            expiry = timers.after(ahead) {
                // The pause ended by itself: the sender says so (one `presence` without `pause`), the screens and gates re-read the clock.
                quiet.set(0)
                publish()
                mutableChanges.tryEmit(PauseChange.EXPIRED)
            }
        }
        publish()
        if (emit) mutableChanges.tryEmit(PauseChange.CHANGED)
    }

    private fun publish() {
        mutableView.value = viewNow()
    }
}
