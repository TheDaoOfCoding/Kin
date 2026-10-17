package app.kinrelay.android.shots

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph

/** The Cancel button of the "Sending screenshot to <PC> in 5 s" notification. Not exported: only our own `PendingIntent` reaches it. */
class ShotActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_CANCEL) return
        val id = intent.getLongExtra(EXTRA_ID, -1L)
        if (id >= 0) context.graph.shots.cancel(id)
    }

    companion object {
        const val ACTION_CANCEL = "app.kinrelay.android.shots.CANCEL"
        const val EXTRA_ID = "id"
    }
}
