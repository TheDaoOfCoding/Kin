package app.kinrelay.android.notifmirror

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringSetPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.mirrorStore by preferencesDataStore(name = "kin_mirror")

/**
 * What the phone keeps about notification mirroring, and the only thing it keeps (design 4.12): the master switch, the packages the user ticked,
 * the packages ticked for notifications hidden on the lock screen, "Dismiss on the phone too" and "the setup was shown". A DataStore of its own,
 * not Room, so this milestone doesn't touch the database version. No notification content, no `nid`, no history.
 */
class MirrorPolicyStore(context: Context) {
    private val store = context.applicationContext.mirrorStore

    val policy: Flow<MirrorPolicy> = store.data.map { p ->
        MirrorPolicy(
            enabled = p[ENABLED] == true,
            allowed = p[ALLOWED].orEmpty(),
            hiddenAllowed = p[HIDDEN_ALLOWED].orEmpty(),
            dismissOnPhone = p[DISMISS_ON_PHONE] != false,
        )
    }

    suspend fun current(): MirrorPolicy = policy.first()

    val setupShown: Flow<Boolean> = store.data.map { it[SETUP_SHOWN] == true }

    suspend fun setEnabled(enabled: Boolean) = store.edit { it[ENABLED] = enabled }

    /** Replaces the allow list. Hidden-on-lock-screen choices of apps that are no longer allowed are dropped too. */
    suspend fun setAllowed(packages: Set<String>) = store.edit {
        it[ALLOWED] = packages
        it[HIDDEN_ALLOWED] = it[HIDDEN_ALLOWED].orEmpty().intersect(packages)
    }

    suspend fun setHiddenAllowed(packages: Set<String>) = store.edit { it[HIDDEN_ALLOWED] = packages }

    /** Ticks or unticks one app inside a single `edit`, so two quick taps can't undo each other (the list on screen can be a step behind the store). */
    suspend fun toggleAllowed(packageName: String, on: Boolean) = store.edit {
        val allowed = it[ALLOWED].orEmpty().let { set -> if (on) set + packageName else set - packageName }
        it[ALLOWED] = allowed
        it[HIDDEN_ALLOWED] = it[HIDDEN_ALLOWED].orEmpty().intersect(allowed)
    }

    suspend fun toggleHiddenAllowed(packageName: String, on: Boolean) = store.edit {
        it[HIDDEN_ALLOWED] = it[HIDDEN_ALLOWED].orEmpty().let { set -> if (on) set + packageName else set - packageName }
    }

    suspend fun setDismissOnPhone(on: Boolean) = store.edit { it[DISMISS_ON_PHONE] = on }

    suspend fun setSetupShown(shown: Boolean) = store.edit { it[SETUP_SHOWN] = shown }

    private companion object {
        val ENABLED = booleanPreferencesKey("mirror_enabled")
        val ALLOWED = stringSetPreferencesKey("mirror_allowed")
        val HIDDEN_ALLOWED = stringSetPreferencesKey("mirror_hidden_allowed")
        val DISMISS_ON_PHONE = booleanPreferencesKey("mirror_dismiss_on_phone")
        val SETUP_SHOWN = booleanPreferencesKey("mirror_setup_shown")
    }
}
