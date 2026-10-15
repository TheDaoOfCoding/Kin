package app.kinrelay.android.data

import android.content.Context
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

private val Context.dataStore by preferencesDataStore(name = "kin_settings")

/** A snapshot of the settings and counters the debug screen shows. */
data class Stats(
    val connects: Int = 0,
    val disconnects: Int = 0,
    val kills: Int = 0,
    val lastRttMillis: Double? = null,
    /** Tap to ack of the last text sent to the PC, in ms (targets.md priority 2). */
    val lastArrivalMillis: Long? = null,
    val pingIntervalSeconds: Int = KinSettings.DEFAULT_PING_INTERVAL_SECONDS,
    /** The last ping round trips (ms) and tap-to-ack times (ms), oldest first, at most [KinSettings.LATENCY_KEPT] of each. */
    val recentRtts: List<Double> = emptyList(),
    val recentArrivals: List<Long> = emptyList(),
    /** The name this phone sends in `hello`, or null for the system's. */
    val deviceName: String? = null,
    /** The reliability setup (notifications, battery, OEM steps) was shown and finished or skipped. */
    val setupDone: Boolean = false,
)

class KinSettings(context: Context) {
    private val store = context.applicationContext.dataStore

    val stats: Flow<Stats> = store.data.map { prefs ->
        Stats(
            connects = prefs[CONNECTS] ?: 0,
            disconnects = prefs[DISCONNECTS] ?: 0,
            kills = prefs[KILLS] ?: 0,
            lastRttMillis = prefs[LAST_RTT],
            lastArrivalMillis = prefs[LAST_ARRIVAL],
            pingIntervalSeconds = prefs[PING_INTERVAL] ?: DEFAULT_PING_INTERVAL_SECONDS,
            recentRtts = prefs[RTTS]?.split(',')?.mapNotNull { it.toDoubleOrNull() }.orEmpty(),
            recentArrivals = prefs[ARRIVALS]?.split(',')?.mapNotNull { it.toLongOrNull() }.orEmpty(),
            deviceName = prefs[DEVICE_NAME]?.takeIf { it.isNotBlank() },
            setupDone = prefs[SETUP_DONE] == true,
        )
    }

    suspend fun pingIntervalSeconds(): Int = store.data.first()[PING_INTERVAL] ?: DEFAULT_PING_INTERVAL_SECONDS

    suspend fun setPingIntervalSeconds(seconds: Int) = store.edit { it[PING_INTERVAL] = seconds }

    suspend fun countConnect() = increment(CONNECTS)

    suspend fun countDisconnect() = increment(DISCONNECTS)

    suspend fun setLastRtt(millis: Double) = store.edit {
        it[LAST_RTT] = millis
        it[RTTS] = ((it[RTTS]?.split(',').orEmpty() + millis.toString()).takeLast(LATENCY_KEPT)).joinToString(",")
    }

    suspend fun setLastArrival(millis: Long) = store.edit {
        it[LAST_ARRIVAL] = millis
        it[ARRIVALS] = ((it[ARRIVALS]?.split(',').orEmpty() + millis.toString()).takeLast(LATENCY_KEPT)).joinToString(",")
    }

    /** The device name the user chose (blank clears it: the system name is used again). */
    suspend fun setDeviceName(name: String) = store.edit { if (name.isBlank()) it.remove(DEVICE_NAME) else it[DEVICE_NAME] = name.trim() }

    suspend fun deviceName(): String? = store.data.first()[DEVICE_NAME]?.takeIf { it.isNotBlank() }

    suspend fun setSetupDone(done: Boolean) = store.edit { it[SETUP_DONE] = done }

    /** How the previous run of the service ended. */
    enum class PreviousRun {
        CLEAN, KILLED, REBOOT;

        companion object {
            fun classify(wasRunning: Boolean, storedBootCount: Int?, bootCount: Int): PreviousRun = when {
                !wasRunning -> CLEAN
                storedBootCount != null && storedBootCount != bootCount -> REBOOT
                else -> KILLED
            }
        }
    }

    /**
     * Marks the service as running and says how the previous run ended. A run that never reached
     * a clean stop was killed (by the OS or a crash), and counts as a kill, unless the phone has
     * rebooted since ([bootCount] is the system boot counter): a reboot is not a kill.
     */
    suspend fun markServiceStarted(bootCount: Int): PreviousRun {
        var previous = PreviousRun.CLEAN
        store.edit {
            previous = PreviousRun.classify(it[SERVICE_RUNNING] == true, it[BOOT_COUNT], bootCount)
            if (previous == PreviousRun.KILLED) it[KILLS] = (it[KILLS] ?: 0) + 1
            it[SERVICE_RUNNING] = true
            it[BOOT_COUNT] = bootCount
        }
        return previous
    }

    suspend fun markServiceStopped() = store.edit { it[SERVICE_RUNNING] = false }

    private suspend fun increment(key: Preferences.Key<Int>) = store.edit { it[key] = (it[key] ?: 0) + 1 }

    companion object {
        const val DEFAULT_PING_INTERVAL_SECONDS = 240
        const val LATENCY_KEPT = 20
        private val CONNECTS = intPreferencesKey("connects")
        private val DISCONNECTS = intPreferencesKey("disconnects")
        private val KILLS = intPreferencesKey("kills")
        private val LAST_RTT = doublePreferencesKey("last_rtt_ms")
        private val LAST_ARRIVAL = longPreferencesKey("last_arrival_ms")
        private val PING_INTERVAL = intPreferencesKey("ws_ping_interval_s")
        private val SERVICE_RUNNING = booleanPreferencesKey("service_running")
        private val BOOT_COUNT = intPreferencesKey("boot_count")
        private val RTTS = stringPreferencesKey("recent_rtts")
        private val ARRIVALS = stringPreferencesKey("recent_arrivals")
        private val DEVICE_NAME = stringPreferencesKey("device_name")
        private val SETUP_DONE = booleanPreferencesKey("setup_done")
    }
}
