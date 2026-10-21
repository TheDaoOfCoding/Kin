package app.kinrelay.android.dnd

/** What the follower tells the controller to do (the vectors' `do`: `request:priority`, `request:alarms`, `release`, `leave`, `override`). */
sealed interface DndAction {
    val word: String

    /** Ask for [level] (Priority or Alarms): the filter was `ALL`. The caller persists `we_hold` **before** it asks. */
    data class Request(val level: DndLevel) : DndAction {
        override val word: String get() = "request:${level.word}"
    }

    /** Give back what Kin holds (the caller's job is to remove only Kin's own silence). */
    data object Release : DndAction {
        override val word: String get() = "release"
    }

    /** The PC went into Focus but the phone is already in some Do Not Disturb: nothing is requested and nothing is held. */
    data object Leave : DndAction {
        override val word: String get() = "leave"
    }

    /** The system reported a filter that is not the one Kin requested while Kin holds: the user changed it. Kin lets go and does not re-apply until the next rising edge. */
    data object Override : DndAction {
        override val word: String get() = "override"
    }
}

/**
 * PC Focus to phone Do Not Disturb, the rules of design 4.2 as a pure state machine over events (vectors `alerts.json` `dndFollow[]`). Every event carries the monotonic time [now] (ms) and returns
 * the actions to do, in order; the caller (`DndController`) does them and, for the 60 s grace, asks [graceDeadline] and calls [onTick] at that moment.
 *
 * ```
 * WANT = switch AND access AND not paused AND the PC's hello lists presence AND the PC's snapshot says focus
 * rising edge of WANT while Kin does not hold:  filter ALL -> Request(follow) and hold; any other filter -> Leave (nothing is held, nothing is done)
 * WANT false while Kin holds                 -> Release
 * a close while Kin holds                    -> the PC's Focus is kept for 60 s ([GRACE_MILLIS]); a snapshot clears the timer; when it ends the Focus is forgotten
 * a filter report that is not the requested one while Kin holds -> Override (hold = false, no re-apply until the next rising edge of WANT)
 * start (a process start: the memory is gone) while Kin holds   -> Release
 * ```
 *
 * It never produces a [DndAction.Request] unless the filter is `ALL` and Kin does not hold (T4 of the design: Kin never weakens or takes over the user's own Do Not Disturb). With two PCs (2D) the
 * phone would hold while **any** peer says focus and release when none does; with one PC the one connection is the whole input.
 */
class DndFollower(
    private var switchOn: Boolean = false,
    private var access: Boolean = false,
    private var paused: Boolean = false,
    follow: DndLevel = DndLevel.PRIORITY,
    private var filter: DndLevel = DndLevel.ALL,
    weHold: Boolean = false,
    /** The level the persisted record says Kin asked for (2B.6). A follower rebuilt around a hold must expect that, not whatever "How much" says now (the user may have changed it since). */
    heldLevel: DndLevel? = null,
) {
    private var follow: DndLevel = if (follow == DndLevel.ALARMS) DndLevel.ALARMS else DndLevel.PRIORITY

    /** Kin holds its silence now (persisted by the controller before each request). */
    var weHold: Boolean = weHold
        private set

    private var requested: DndLevel? = if (weHold) (heldLevel ?: this.follow) else null
    private var peerListsPresence = false
    private var focus = false
    private var wantPrev = false
    private var graceEnd: Long? = null

    /**
     * A live connection to the PC (its `hello` was accepted or a snapshot arrived on it, and it has not closed). Kin never **starts** holding without one: the PC's Focus remembered through the 60 s
     * grace is there only to keep a hold that already exists (2B.5 review).
     */
    private var connected = false

    /** Whether a live connection is known (for the controller's watchdog). */
    val isConnected: Boolean get() = connected

    /** The monotonic time at which the 60 s grace ends (the caller calls [onTick] then), or null when no grace runs. */
    fun graceDeadline(): Long? = graceEnd

    /** What the phone's filter is as far as the follower knows (the last report). */
    val currentFilter: DndLevel get() = filter

    private fun want(): Boolean = switchOn && access && !paused && peerListsPresence && focus

    /** The grace timer that is due at or before [now] fires first, in every event. */
    private fun fire(now: Long): List<DndAction> {
        val end = graceEnd ?: return emptyList()
        if (now < end) return emptyList()
        graceEnd = null
        focus = false
        peerListsPresence = false
        return evaluate()
    }

    private fun evaluate(): List<DndAction> {
        // No live connection and nothing held: whatever Focus is remembered counts for nothing, and the edge is not used up (the next snapshot of a real connection is the rising edge).
        val want = want() && (weHold || connected)
        val rising = want && !wantPrev
        wantPrev = want
        if (want) {
            if (!rising || weHold) return emptyList()
            return if (filter == DndLevel.ALL) {
                weHold = true
                requested = follow
                listOf(DndAction.Request(follow))
            } else {
                listOf(DndAction.Leave)
            }
        }
        if (weHold) {
            weHold = false
            requested = null
            if (!connected) {
                // Released with no connection (a pause during the grace): the remembered Focus served the hold only, so it goes with it. A resume inside the 60 s must not look like a rising edge.
                focus = false
                peerListsPresence = false
                graceEnd = null
            }
            return listOf(DndAction.Release)
        }
        return emptyList()
    }

    private inline fun event(now: Long, change: () -> Unit): List<DndAction> {
        val fired = fire(now)
        change()
        return fired + evaluate()
    }

    /** A `hello` of the PC was accepted; [listsPresence] is whether it lists `presence` (a later `hello` can take it away: Kin releases). */
    fun onConnect(now: Long, listsPresence: Boolean): List<DndAction> = event(now) {
        connected = true
        peerListsPresence = listsPresence
    }

    /** A `presence` snapshot of the PC: [focus] is true only when it carried `focus: true` (absent, false and null are "the PC left Focus"). A snapshot clears the grace timer. */
    fun onSnapshot(now: Long, focus: Boolean): List<DndAction> {
        val fired = fire(now)
        graceEnd = null
        connected = true // a snapshot can only have come over a live connection
        this.focus = focus
        return fired + evaluate()
    }

    /** The connection closed. While Kin holds, the PC's Focus counts for [GRACE_MILLIS] more (a Wi-Fi blip must not flap the phone); otherwise it is forgotten at once. */
    fun onClose(now: Long): List<DndAction> {
        val fired = fire(now)
        connected = false
        if (weHold) {
            if (graceEnd == null) graceEnd = now + GRACE_MILLIS
            return fired
        }
        focus = false
        peerListsPresence = false
        return fired + evaluate()
    }

    fun onPause(now: Long, on: Boolean): List<DndAction> = event(now) { paused = on }

    fun onSwitch(now: Long, on: Boolean): List<DndAction> = event(now) { switchOn = on }

    fun onAccess(now: Long, granted: Boolean): List<DndAction> = event(now) { access = granted }

    /**
     * The access as the system says it **now**, set before an event is evaluated and with no event of its own (like [setFilter]): the remembered access can be stale when it was revoked while no
     * receiver was registered (both switches off), and then a switch turned on, a snapshot or a connection would spend the rising edge on a request that cannot be made (2B.5 review round 2).
     */
    fun setAccess(granted: Boolean) {
        access = granted
    }

    /** The "How much" setting changed. It applies at the next rising edge (a hold in progress is not changed under the user). */
    fun onFollowLevel(level: DndLevel) {
        follow = if (level == DndLevel.ALARMS) DndLevel.ALARMS else DndLevel.PRIORITY
    }

    /**
     * The system reported the phone's filter. A report that differs from the one Kin requested while Kin holds is the user's doing: [DndAction.Override]. (A report of the one Kin requested, or any
     * report while Kin does not hold, only updates what the follower knows.)
     */
    fun onFilter(now: Long, level: DndLevel): List<DndAction> {
        val fired = fire(now)
        filter = level
        if (weHold && level != requested) {
            weHold = false
            requested = null
            return fired + DndAction.Override
        }
        return fired
    }

    /** The phone's filter was read (before the first event of a session): it only updates what the follower knows, it never overrides. */
    fun setFilter(level: DndLevel) {
        filter = level
    }

    /** The PC was unlinked: it is gone, so nothing is held for it and nothing it says counts any more. */
    fun onUnlink(now: Long): List<DndAction> = event(now) {
        connected = false
        peerListsPresence = false
        focus = false
        graceEnd = null
    }

    /** A process start (`LinkService`, `AppGraph`): the memory of the connection is gone, so a persisted hold is released now (the crash and update case). The PC in Focus is a fresh edge later. */
    fun onStart(now: Long): List<DndAction> {
        val fired = fire(now)
        connected = false
        focus = false
        peerListsPresence = false
        graceEnd = null
        wantPrev = false
        return fired + evaluate()
    }

    /** Time only (the grace timer was due). */
    fun onTick(now: Long): List<DndAction> = fire(now)

    /**
     * The user tapped "Release now" in Settings: Kin lets go whatever the PC says, and does not ask again until the next rising edge of WANT (the PC leaving Focus and coming back). Returns
     * [DndAction.Release] when Kin held (the controller releases through the applier either way: its record may outlive the follower's memory).
     */
    fun onReleaseNow(): List<DndAction> {
        if (!weHold) return emptyList()
        weHold = false
        requested = null
        return listOf(DndAction.Release)
    }

    /** A request the system refused (the access went away between the check and the call): nothing was applied, so nothing is held. The edge stays handled: it is not retried in a loop. */
    fun onRequestFailed() {
        weHold = false
        requested = null
    }

    companion object {
        /** A close that happens while Kin holds is forgiven for this long (design 4.2): with the 240 s dead-link detection it bounds a stuck state to about 5 minutes awake (about 9 with the phone asleep: the watchdog alarm is inexact). */
        const val GRACE_MILLIS = 60_000L
    }
}
