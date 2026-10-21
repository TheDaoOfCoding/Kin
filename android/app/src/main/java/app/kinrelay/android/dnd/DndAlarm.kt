package app.kinrelay.android.dnd

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph

/**
 * The one alarm of [DndWatchdog]: `ELAPSED_REALTIME_WAKEUP` with `setAndAllowWhileIdle` (inexact: no exact-alarm permission, and the system may batch it, which is fine for a safety net of minutes).
 * Setting it again replaces it; [cancel] removes it. An alarm survives the end of the process, and a process started by it releases a stale hold at start anyway.
 */
class DndAlarm(private val context: Context) {
    private val alarms = context.getSystemService(AlarmManager::class.java)

    private fun pending(): PendingIntent = PendingIntent.getBroadcast(
        context, REQUEST_CODE, Intent(context, DndWatchdogReceiver::class.java).setAction(ACTION_WATCHDOG), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    fun set(atElapsedMillis: Long) {
        runCatching { alarms.setAndAllowWhileIdle(AlarmManager.ELAPSED_REALTIME_WAKEUP, atElapsedMillis, pending()) }
    }

    fun cancel() {
        runCatching { alarms.cancel(pending()) }
    }

    companion object {
        const val ACTION_WATCHDOG = "app.kinrelay.android.DND_WATCHDOG"
        private const val REQUEST_CODE = 0x4b44
    }
}

/** The alarm of [DndAlarm] goes off: the controller looks at the hold. Not exported: only our own `PendingIntent` reaches it. */
class DndWatchdogReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != DndAlarm.ACTION_WATCHDOG) return
        val pending = goAsync()
        context.graph.dnd.onWatchdog { pending.finish() }
    }
}
