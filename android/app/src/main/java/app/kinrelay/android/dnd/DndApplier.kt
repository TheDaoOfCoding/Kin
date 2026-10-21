package app.kinrelay.android.dnd

import app.kinrelay.android.log.KinLog

/**
 * What Kin recorded about its own silence: whether it holds, which filter it asked for, and which model applied it (`rule` or `global`, for `DND_STATE` and the stuck-rule risk R44).
 * [releaseOwed] is the "a release failed and is still owed" flag, kept with the record so a process death cannot lose it (2B.6; it only means something while [weHold]).
 */
data class DndHold(val weHold: Boolean = false, val heldFilter: Int = DndFilter.ALL, val route: String? = null, val releaseOwed: Boolean = false) {
    companion object {
        val NONE = DndHold()
    }
}

/** Where `we_hold` lives between runs. The app's is [DndStore] (DataStore `kin_dnd`); the tests use memory. */
interface DndHoldStore {
    suspend fun load(): DndHold

    suspend fun save(hold: DndHold)
}

/** What the applier needs from Android. The real one is [AndroidDndPorts]; the tests fake the `NotificationManager` and the listener. */
interface DndPorts {
    /** `NotificationManager.isNotificationPolicyAccessGranted` (the emulator reports it true for an app that has notification access too: spike S5). */
    fun policyAccess(): Boolean

    /** The notification listener is bound (notification access granted and `onListenerConnected` has run). */
    fun listenerConnected(): Boolean

    /** `NotificationManager.getCurrentInterruptionFilter()` (needs no access). */
    fun currentFilter(): Int

    /** `NotificationManager.setInterruptionFilter` (route 1). Throws `SecurityException` without Do Not Disturb access. */
    fun setFilter(filter: Int)

    /** `NotificationListenerService.requestInterruptionFilter` through the bound listener (route 2). False when not connected or refused. */
    fun requestViaListener(filter: Int): Boolean

    /** The app targets Android 15+ on a phone that runs it: neither route changes the **global** filter, they activate or deactivate **Kin's own rule** ("Do Not Disturb (Kin)", most restrictive wins). */
    val ruleModel: Boolean
}

/**
 * The one writer of the phone's Do Not Disturb (design 4.1), the part that has no Android in it. **It only ever adds silence and only removes its own**:
 * - the route is the narrowest access already granted ([path]): Do Not Disturb access (`setInterruptionFilter`; on the emulator this is also true while only notification access is given), else
 *   the listener (`requestInterruptionFilter`), else nothing (Settings asks for Do Not Disturb access, never for notification access);
 * - **`we_hold` is persisted before the request** (a crash right after must leave a record to release at the next start);
 * - [release] asks for `ALL`: on the rule model this only deactivates Kin's own rule and leaves the user's; on Android 10 to 14 (a global filter) it restores `ALL` **only if the filter is still the one Kin
 *   set** (the user changed it otherwise: it is theirs now). If the call fails (the access went away) the record stays and is tried again at the next start;
 * - it never calls anything with `ALL` unless it holds.
 */
class DndApplier(
    private val ports: DndPorts,
    private val store: DndHoldStore,
    private val log: KinLog = KinLog.None,
) {
    enum class Path { POLICY, LISTENER, NONE }

    /** The narrowest route that works now (design 4.2.1, decided by spike S5). */
    fun path(): Path = when {
        ports.policyAccess() -> Path.POLICY
        ports.listenerConnected() -> Path.LISTENER
        else -> Path.NONE
    }

    val hasAccess: Boolean get() = path() != Path.NONE

    /** Persists the hold, then asks for [level] (Priority or Alarms). False (and nothing held) when there is no access or the system refused. */
    suspend fun request(level: DndLevel): Boolean {
        val filter = DndWire.filterOf(level) ?: return false
        val path = path()
        if (path == Path.NONE) {
            log.w("dnd", "dnd: no access, nothing requested")
            return false
        }
        // Before the request, never after: a crash between the two must leave the record.
        store.save(DndHold(weHold = true, heldFilter = filter, route = if (ports.ruleModel) ROUTE_RULE else ROUTE_GLOBAL))
        val ok = try {
            call(path, filter)
        } catch (e: Exception) {
            log.w("dnd", "dnd: the request was refused (${e.javaClass.simpleName})")
            false
        }
        if (!ok) {
            store.save(DndHold.NONE)
            return false
        }
        log.i("dnd", "dnd: asked for ${level.word} (${if (ports.ruleModel) "Kin's own rule" else "the global filter"}, route ${path.name.lowercase()})")
        return true
    }

    /** Gives back what Kin holds. True when the record was cleared (a failed call keeps it: the access is gone, and the next start tries again). */
    suspend fun release(): Boolean {
        val hold = store.load()
        if (!hold.weHold) return true
        val path = path()
        if (path == Path.NONE) {
            log.w("dnd", "dnd: cannot release now (no access); it is tried again at the next start")
            return false
        }
        return try {
            // The rule model: ALL only deactivates Kin's own rule. A global filter: only if it is still what Kin set.
            if (ports.ruleModel || ports.currentFilter() == hold.heldFilter) call(path, DndFilter.ALL)
            store.save(DndHold.NONE)
            log.i("dnd", "dnd: released")
            true
        } catch (e: Exception) {
            log.w("dnd", "dnd: the release was refused (${e.javaClass.simpleName}); it is tried again at the next start")
            false
        }
    }

    /** Writes the "release owed" flag next to the record (2B.6). With no record there is nothing owed, so nothing is written. */
    suspend fun setOwed(owed: Boolean) {
        val hold = store.load()
        if (!hold.weHold || hold.releaseOwed == owed) return
        store.save(hold.copy(releaseOwed = owed))
    }

    /**
     * The user changed the filter while Kin held: Kin lets go and does not re-apply. On the rule model Kin's own rule may still be active (the user's change was a different rule), so it is
     * deactivated (it touches nothing of theirs); on a global filter the user's change is the filter now and nothing is done. True when the record was cleared. When deactivating Kin's rule
     * failed (the access is gone) the record **stays** and the caller hands it to the release guard: Kin's own rule is still on and is Kin's to take off (2B.5 review round 3).
     */
    suspend fun overridden(): Boolean {
        val hold = store.load()
        if (ports.ruleModel && hold.weHold) {
            val path = path()
            val ok = path != Path.NONE && try {
                call(path, DndFilter.ALL)
            } catch (e: Exception) {
                false
            }
            if (!ok) {
                log.w("dnd", "dnd: the user changed Do Not Disturb, but Kin's own rule could not be switched off now; it is tried again")
                return false
            }
        }
        store.save(DndHold.NONE)
        log.i("dnd", "dnd: the user changed Do Not Disturb; Kin lets go")
        return true
    }

    /**
     * A filter was reported while a release is owed (it failed, the record stayed, the follower already let go). Any filter other than the one Kin set is the user's doing: the record is cleared so
     * the retry can never switch off **their** Do Not Disturb (2B.5 review round 3, R44). The same filter changes nothing (Kin's silence is still on and still owed). True when the record is
     * clear afterwards.
     */
    suspend fun userChangedWhileOwed(filter: Int): Boolean {
        val hold = store.load()
        if (!hold.weHold) return true
        if (filter == hold.heldFilter) return false
        return overridden()
    }

    private fun call(path: Path, filter: Int): Boolean = when (path) {
        Path.POLICY -> {
            ports.setFilter(filter)
            true
        }
        Path.LISTENER -> ports.requestViaListener(filter)
        Path.NONE -> false
    }

    companion object {
        const val ROUTE_RULE = "rule"
        const val ROUTE_GLOBAL = "global"
    }
}
