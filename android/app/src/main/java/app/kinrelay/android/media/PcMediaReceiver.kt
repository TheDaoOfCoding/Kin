package app.kinrelay.android.media

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph

/** The buttons and the swipe of the PC-media notification (design 4.5). Not exported: only Kin's own `PendingIntent`s reach it. */
class PcMediaReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action ?: return
        context.graph.media.onNotificationAction(action)
    }
}
