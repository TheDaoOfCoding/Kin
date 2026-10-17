package app.kinrelay.android.shots

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.shotsStore by preferencesDataStore(name = "kin_shots")

/**
 * The two switches of auto-send (design section 4: both **off** by default) and **one number**: the baseline, the highest `_ID` of `MediaStore.Images` at the moment the feature was turned on
 * (and later moved up past everything that was sent, cancelled or skipped while paused). No name, no path, no image is ever stored (design 5.8). Only [app.kinrelay.android.features.Features]
 * reads the two switches as a feature switch.
 */
class ShotsStore(context: Context) {
    private val store = context.applicationContext.shotsStore

    val screenshots: Flow<Boolean> = store.data.map { it[SCREENSHOTS] == true }.distinctUntilChanged()
    val camera: Flow<Boolean> = store.data.map { it[CAMERA] == true }.distinctUntilChanged()

    suspend fun setScreenshots(on: Boolean) = store.edit { it[SCREENSHOTS] = on }

    suspend fun setCamera(on: Boolean) = store.edit { it[CAMERA] = on }

    suspend fun baseline(): Long = store.data.first()[BASELINE] ?: 0L

    suspend fun setBaseline(id: Long) = store.edit { if (id > (it[BASELINE] ?: 0L)) it[BASELINE] = id }

    /** The second (wall clock) at which the baseline was taken, or 0: a number too, so a copied old screenshot (an old date taken) is told from a new one. */
    suspend fun baselineTime(): Long = store.data.first()[BASELINE_TIME] ?: 0L

    suspend fun setBaselineTime(seconds: Long) = store.edit { it[BASELINE_TIME] = seconds }

    suspend fun clearBaseline() = store.edit {
        it.remove(BASELINE)
        it.remove(BASELINE_TIME)
    }

    private companion object {
        val BASELINE_TIME = longPreferencesKey("shots_baseline_time")
        val SCREENSHOTS = booleanPreferencesKey("shots_screenshots")
        val CAMERA = booleanPreferencesKey("shots_camera")
        val BASELINE = longPreferencesKey("shots_baseline")
    }
}
