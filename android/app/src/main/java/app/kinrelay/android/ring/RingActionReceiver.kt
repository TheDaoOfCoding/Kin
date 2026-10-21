package app.kinrelay.android.ring

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph

/** The Stop button of the ring notification. Not exported: only our own `PendingIntent` reaches it. */
class RingActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == ACTION_STOP) context.graph.ring.stopByUser()
    }

    companion object {
        const val ACTION_STOP = "app.kinrelay.android.ring.STOP"
    }
}
