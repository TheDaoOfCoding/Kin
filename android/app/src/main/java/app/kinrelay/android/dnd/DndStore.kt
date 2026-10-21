package app.kinrelay.android.dnd

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.dndStore by preferencesDataStore(name = "kin_dnd")

/**
 * The 2B.5 switches and Kin's own record of its silence (DataStore `kin_dnd`; design 4.1 and 4.2):
 * - [follow] "Silence this phone while my PC is in Focus" (default **off**) and [followFilter] "How much" (`priority`, the default, or `alarms`; never total silence);
 * - [share] "Tell my PC when this phone is on Do Not Disturb" (default **off**);
 * - `we_hold`, `held_filter`, `held_route` and `release_owed`: **no content, no history**. They exist only while Kin holds the phone silent (written before the request, cleared on release).
 * **Only [app.kinrelay.android.features.Features] reads [follow] and [share] as feature switches**; the rest of the app asks it.
 */
class DndStore(context: Context) : DndHoldStore {
    private val store = context.applicationContext.dndStore

    val follow: Flow<Boolean> = store.data.map { it[FOLLOW] == true }.distinctUntilChanged()
    val share: Flow<Boolean> = store.data.map { it[SHARE] == true }.distinctUntilChanged()
    val followFilter: Flow<String> = store.data.map { it[FOLLOW_FILTER] ?: DndWire.PRIORITY }.distinctUntilChanged()
    val hold: Flow<DndHold> = store.data.map(::holdOf).distinctUntilChanged()

    suspend fun setFollow(on: Boolean) = store.edit { it[FOLLOW] = on }

    suspend fun setShare(on: Boolean) = store.edit { it[SHARE] = on }

    /** Anything but `alarms` is stored as `priority`: the setting can never ask for total silence. */
    suspend fun setFollowFilter(word: String) = store.edit { it[FOLLOW_FILTER] = if (word == DndWire.ALARMS) DndWire.ALARMS else DndWire.PRIORITY }

    suspend fun followFilterNow(): String = followFilter.first()

    override suspend fun load(): DndHold = holdOf(store.data.first())

    override suspend fun save(hold: DndHold) {
        store.edit {
            if (hold.weHold) {
                it[WE_HOLD] = true
                it[HELD_FILTER] = hold.heldFilter
                if (hold.route != null) it[HELD_ROUTE] = hold.route else it.remove(HELD_ROUTE)
                if (hold.releaseOwed) it[RELEASE_OWED] = true else it.remove(RELEASE_OWED)
            } else {
                it.remove(WE_HOLD)
                it.remove(HELD_FILTER)
                it.remove(HELD_ROUTE)
                it.remove(RELEASE_OWED)
            }
        }
    }

    private fun holdOf(p: androidx.datastore.preferences.core.Preferences): DndHold =
        DndHold(weHold = p[WE_HOLD] == true, heldFilter = p[HELD_FILTER] ?: DndFilter.ALL, route = p[HELD_ROUTE], releaseOwed = p[WE_HOLD] == true && p[RELEASE_OWED] == true)

    private companion object {
        val FOLLOW = booleanPreferencesKey("dnd_follow")
        val SHARE = booleanPreferencesKey("dnd_share")
        val FOLLOW_FILTER = stringPreferencesKey("dnd_follow_filter")
        val WE_HOLD = booleanPreferencesKey("we_hold")
        val HELD_FILTER = intPreferencesKey("held_filter")
        val HELD_ROUTE = stringPreferencesKey("held_route")
        /** 2B.6, additive: a store written by 0.1.x or 0.2.0 before this key has none, which reads as "nothing owed". */
        val RELEASE_OWED = booleanPreferencesKey("release_owed")
    }
}
