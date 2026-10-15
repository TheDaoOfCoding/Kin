package app.kinrelay.android.transfer

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph

/**
 * The Cancel and Retry buttons of the transfer notifications, and the swipe of the "texts from your PC" notification. Not exported: only our
 * own `PendingIntent`s reach it. The work runs on the app's scope, so the receiver returns at once.
 */
class TransferActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val graph = context.graph
        if (intent.action == ACTION_TEXTS_DISMISSED) {
            graph.notifications.textsSeen()
            return
        }
        val id = intent.getStringExtra(TransferNotifications.EXTRA_TRANSFER_ID) ?: return
        when (intent.action) {
            TransferNotifications.ACTION_CANCEL -> graph.transfers.cancel(id)
            // The notification's Retry is the user asking: an upload may go out under a new ID if the PC has used the old one up.
            TransferNotifications.ACTION_RETRY -> graph.transfers.retry(id)
        }
    }

    companion object {
        const val ACTION_TEXTS_DISMISSED = "app.kinrelay.android.TEXTS_DISMISSED"
    }
}
