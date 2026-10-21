package app.kinrelay.android.presence

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.presenceStore by preferencesDataStore(name = "kin_presence")

/**
 * The defaults of the presence and ring switches (design section 4): "Share this phone's battery with the PC" ([share]), "Show the PC's status" ([show]), "Let my PC ring this phone"
 * ([ringAllow]) and the ring volume choice ([ringLoud], default Loud). All on by default (user decisions: battery sharing on, ring on). **Only [app.kinrelay.android.features.Features]
 * reads these keys as a feature switch**; the rest of the app asks it. No battery level, no pause time of the peer is ever stored: the one number that is, [savedAlarmIndex], exists
 * only while the phone is ringing (the alarm volume to put back).
 */
class PresenceStore(context: Context) {
    private val store = context.applicationContext.presenceStore

    val share: Flow<Boolean> = store.data.map { it[SHARE] != false }.distinctUntilChanged()
    val show: Flow<Boolean> = store.data.map { it[SHOW] != false }.distinctUntilChanged()
    val ringAllow: Flow<Boolean> = store.data.map { it[RING_ALLOW] != false }.distinctUntilChanged()
    val ringLoud: Flow<Boolean> = store.data.map { it[RING_LOUD] != false }.distinctUntilChanged()

    suspend fun setShare(on: Boolean) = store.edit { it[SHARE] = on }

    suspend fun setShow(on: Boolean) = store.edit { it[SHOW] = on }

    suspend fun setRingAllow(on: Boolean) = store.edit { it[RING_ALLOW] = on }

    suspend fun setRingLoud(on: Boolean) = store.edit { it[RING_LOUD] = on }

    suspend fun ringLoudNow(): Boolean = ringLoud.first()

    /** The user's alarm-stream volume index saved **before** it was raised for a ring, or null when nothing is saved (so nothing to restore). */
    suspend fun savedAlarmIndex(): Int? = store.data.first()[SAVED_ALARM]

    suspend fun saveAlarmIndex(index: Int) = store.edit { it[SAVED_ALARM] = index }

    suspend fun clearSavedAlarmIndex() = store.edit { it.remove(SAVED_ALARM) }

    private companion object {
        val SHARE = booleanPreferencesKey("presence_share_battery")
        val SHOW = booleanPreferencesKey("presence_show_pc")
        val RING_ALLOW = booleanPreferencesKey("ring_allow")
        val RING_LOUD = booleanPreferencesKey("ring_loud")
        val SAVED_ALARM = intPreferencesKey("ring_saved_alarm_index")
    }
}
