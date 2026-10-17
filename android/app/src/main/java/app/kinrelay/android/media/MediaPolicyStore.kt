package app.kinrelay.android.media

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.mediaStore by preferencesDataStore(name = "kin_media")

/**
 * The user's choices about media (design 4.1, decisions 1 to 3 of the user), all off by default:
 * - [share]: show this phone's media on the PC (one switch for every app);
 * - [show]: show the PC's media on this phone;
 * - [showOnLockScreen]: the PC-media notification is `VISIBILITY_PUBLIC` instead of private;
 * - [useVolumeKeys]: the phone's volume keys control the PC's volume while a PC session is shown.
 */
data class MediaPolicy(
    val share: Boolean = false,
    val show: Boolean = false,
    val showOnLockScreen: Boolean = false,
    val useVolumeKeys: Boolean = false,
)

/** A DataStore of its own (`kin_media`), like the mirror's. **No content**: no title, artist, album, app name, `sid`, art or position is ever stored. */
class MediaPolicyStore(context: Context) {
    private val store = context.applicationContext.mediaStore

    val policy: Flow<MediaPolicy> = store.data.map { p ->
        MediaPolicy(
            share = p[SHARE] == true,
            show = p[SHOW] == true,
            showOnLockScreen = p[LOCK_SCREEN] == true,
            useVolumeKeys = p[VOLUME_KEYS] == true,
        )
    }

    suspend fun current(): MediaPolicy = policy.first()

    suspend fun setShare(on: Boolean) = store.edit { it[SHARE] = on }

    suspend fun setShow(on: Boolean) = store.edit { it[SHOW] = on }

    suspend fun setShowOnLockScreen(on: Boolean) = store.edit { it[LOCK_SCREEN] = on }

    suspend fun setUseVolumeKeys(on: Boolean) = store.edit { it[VOLUME_KEYS] = on }

    private companion object {
        val SHARE = booleanPreferencesKey("media_share")
        val SHOW = booleanPreferencesKey("media_show")
        val LOCK_SCREEN = booleanPreferencesKey("media_lock_screen")
        val VOLUME_KEYS = booleanPreferencesKey("media_volume_keys")
    }
}
