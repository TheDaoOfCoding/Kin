package app.kinrelay.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test

/** The skin and theme live in the settings store: Warm and the phone's own light or dark by default, and what is chosen is kept. */
class AppearanceSettingsTest {
    /** A DataStore in memory: the real one writes a file, and renaming over it fails on Windows under test. What is stored is the same preferences. */
    private class MemoryStore : DataStore<Preferences> {
        private val state = MutableStateFlow<Preferences>(emptyPreferences())
        override val data: Flow<Preferences> = state

        override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences =
            transform(state.value).also { state.value = it }
    }

    private fun store(): DataStore<Preferences> = MemoryStore()

    @Test
    fun theDefaultIsWarmWithTheSystemTheme() = runBlocking {
        val settings = KinSettings(store())
        assertEquals(Appearance(KinSkin.WARM, KinThemeMode.SYSTEM), settings.appearance.first())
    }

    @Test
    fun aChoiceIsKeptAndTheOtherHalfIsLeftAlone() = runBlocking {
        val store = store()
        val settings = KinSettings(store)
        settings.setSkin(KinSkin.SIMPLE)
        assertEquals(Appearance(KinSkin.SIMPLE, KinThemeMode.SYSTEM), settings.appearance.first())
        settings.setThemeMode(KinThemeMode.DARK)
        assertEquals(Appearance(KinSkin.SIMPLE, KinThemeMode.DARK), settings.appearance.first())
        // A new KinSettings over the same store (the app was restarted) reads the same choice.
        assertEquals(Appearance(KinSkin.SIMPLE, KinThemeMode.DARK), KinSettings(store).appearance.first())
        settings.setSkin(KinSkin.WARM)
        settings.setThemeMode(KinThemeMode.LIGHT)
        assertEquals(Appearance(KinSkin.WARM, KinThemeMode.LIGHT), settings.appearance.first())
    }

    @Test
    fun aValueThisVersionDoesNotKnowReadsAsTheDefault() = runBlocking {
        val store = store()
        store.edit {
            it[stringPreferencesKey("skin")] = "neon"
            it[stringPreferencesKey("theme_mode")] = "twilight"
        }
        assertEquals(Appearance(), KinSettings(store).appearance.first())
    }

    @Test
    fun theOtherSettingsAreNotTouched() = runBlocking {
        val settings = KinSettings(store())
        settings.setSkin(KinSkin.SIMPLE)
        settings.setDeviceName("Pixel")
        assertEquals("Pixel", settings.deviceName())
        assertEquals(Appearance(KinSkin.SIMPLE, KinThemeMode.SYSTEM), settings.appearance.first())
    }
}
