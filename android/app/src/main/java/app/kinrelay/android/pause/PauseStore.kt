package app.kinrelay.android.pause

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.pauseStore by preferencesDataStore(name = "kin_pause")

/** The persisted pause of this device (`kin_pause`): `paused_until_ms` (0 = none) and `until_resumed`. No content, nothing about the peer. Survives a restart and a reboot. */
class PauseStore(context: Context) : PauseStorage {
    private val store = context.applicationContext.pauseStore

    val state: Flow<PauseState> = store.data.map { PauseState(it[UNTIL] ?: 0L, it[UNTIL_RESUMED] == true) }.distinctUntilChanged()

    override suspend fun current(): PauseState = state.first()

    override suspend fun set(state: PauseState) {
        store.edit {
            it[UNTIL] = state.untilMillis
            it[UNTIL_RESUMED] = state.untilResumed
        }
    }

    private companion object {
        val UNTIL = longPreferencesKey("paused_until_ms")
        val UNTIL_RESUMED = booleanPreferencesKey("until_resumed")
    }
}
