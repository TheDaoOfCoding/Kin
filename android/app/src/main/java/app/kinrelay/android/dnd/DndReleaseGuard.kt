package app.kinrelay.android.dnd

/**
 * A silence Kin could not give back (2B.5 review round 2, risk R44). When a release fails for lack of access the applier keeps its `we_hold` record, but nothing used to look at it again until the
 * next app start or "Release now": the phone stayed silent. This flag is the "a release is still owed" memory next to that record, and the controller retries on every chance it has:
 * the access-changed broadcast, the next watchdog alarm and the start of the service.
 *
 * [release] is the applier's: true when the record is clear afterwards (also when there was nothing to release). [onChange] tells the controller that the receiver and the alarm may need to be set
 * or taken away (they exist for as long as a release is owed, whatever the switches say). [persist] writes the flag next to the `we_hold` record (2B.6: a process that dies while a release is owed
 * must still know it is owed); [restore] reads it back at the start.
 *
 * The retry alarm backs off (2B.6): when access is there but the call keeps being refused, the alarm would otherwise repeat for ever. Each alarm-driven retry that fails moves [retryStep] up;
 * [DndWatchdog.nextWake] turns it into 5, 10, 20 and 40 minutes and then no alarm. Anything that is not the alarm (the access-changed broadcast, a service or app start, "Release now") starts the
 * ladder again.
 */
class DndReleaseGuard(
    private val release: suspend () -> Boolean,
    private val onChange: () -> Unit = {},
    /** The applier's `userChangedWhileOwed`: true when a reported filter was the user's and the record is clear. */
    private val userChanged: suspend (Int) -> Boolean = { false },
    /** The applier's `setOwed`: writes the flag with the record. */
    private val persist: suspend (Boolean) -> Unit = {},
) {
    @Volatile var pending: Boolean = false
        private set

    /** How many alarm-driven retries have failed in a row (see [DndWatchdog.nextWake]). */
    @Volatile var retryStep: Int = 0
        private set

    /** A process start: the flag that was persisted with the record. Nothing is written back (it is what was read). */
    fun restore(owed: Boolean) {
        if (pending == owed) return
        pending = owed
        retryStep = 0
        onChange()
    }

    /** Releases now (the follower said so, or "Release now"); a failure leaves a release owed. */
    suspend fun attempt(fromAlarm: Boolean = false): Boolean {
        val ok = release()
        retryStep = if (!ok && fromAlarm) retryStep + 1 else 0
        set(!ok)
        return ok
    }

    /** A chance to pay the debt (access came back, an alarm went off, the service started). Does nothing when nothing is owed. */
    suspend fun retry(fromAlarm: Boolean = false): Boolean {
        if (!pending) return true
        return attempt(fromAlarm)
    }

    /** The record is not an unpaid release any more: a new request replaced it, or the user's change cleared it. */
    suspend fun settled() = set(false)

    /** A release is owed: the user's change was seen, but Kin's own rule could not be switched off yet (the record stays). */
    suspend fun owe() = set(true)

    /**
     * The system reported [filter]. While a release is owed the follower no longer holds, so it cannot see the user's change: a filter other than the one Kin set means the user took over, and the
     * debt is cancelled (the retry must never switch off their Do Not Disturb, and the alarm stops).
     */
    suspend fun onFilter(filter: Int) {
        if (!pending) return
        if (userChanged(filter)) settled()
    }

    private suspend fun set(value: Boolean) {
        if (pending == value) return
        pending = value
        if (!value) retryStep = 0
        onChange()
        persist(value)
    }
}
