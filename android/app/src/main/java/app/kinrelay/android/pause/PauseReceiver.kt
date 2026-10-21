package app.kinrelay.android.pause

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph
import kotlinx.coroutines.launch

/** "Pause 1 h" and "Resume", the second action of the persistent notification. Not exported: only our own `PendingIntent` reaches it. */
class PauseReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_TOGGLE) return
        val graph = context.graph
        val pending = goAsync()
        graph.scope.launch {
            try {
                if (graph.pause.paused()) graph.pause.resume() else graph.pause.set(PauseChoice.HOUR)
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_TOGGLE = "app.kinrelay.android.pause.TOGGLE"
    }
}
