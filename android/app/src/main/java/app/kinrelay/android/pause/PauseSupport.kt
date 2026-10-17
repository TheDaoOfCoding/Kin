package app.kinrelay.android.pause

import android.app.StatusBarManager
import android.content.ComponentName
import android.content.Context
import android.graphics.drawable.Icon
import android.os.Build
import android.text.format.DateFormat
import app.kinrelay.android.R
import java.time.ZoneId
import java.util.Date
import java.util.Locale

/** Small Android helpers for the Pause screens, tile and notification. */
object PauseSupport {
    /** "Until 14:30" / "Until tomorrow 08:00" / "Until resumed", in the phone's time format and language. */
    fun sentence(context: Context, state: PauseState, nowMillis: Long = System.currentTimeMillis()): String =
        PauseWords.untilSentence(state, nowMillis, ZoneId.systemDefault(), clock = { time(context, it) }, dayName = { day(it) })

    /** "14:30" or "tomorrow 08:00" without the "Until": for the notification line. */
    fun words(context: Context, state: PauseState, nowMillis: Long = System.currentTimeMillis()): String =
        PauseWords.until(state, nowMillis, ZoneId.systemDefault(), clock = { time(context, it) }, dayName = { day(it) })

    fun time(context: Context, millis: Long): String = DateFormat.getTimeFormat(context).format(Date(millis))

    private fun day(millis: Long): String = java.text.SimpleDateFormat("EEE", Locale.getDefault()).format(Date(millis))

    /**
     * "Add the Pause tile" (Settings): on Android 13+ the system asks the user to add it with one tap (`StatusBarManager.requestAddTileService`); below that nothing can be requested and the
     * Settings row shows the steps. Returns true when a request was made.
     */
    fun requestAddTile(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < 33) return false
        val manager = context.getSystemService(StatusBarManager::class.java) ?: return false
        return try {
            manager.requestAddTileService(
                ComponentName(context, PauseTile::class.java), context.getString(R.string.pause_tile_label),
                Icon.createWithResource(context, R.drawable.ic_stat_kin), context.mainExecutor,
            ) { }
            true
        } catch (_: Exception) {
            false
        }
    }
}
