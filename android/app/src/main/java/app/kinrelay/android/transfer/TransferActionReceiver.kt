package app.kinrelay.android.transfer

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph

/**
 * The Cancel and Retry buttons of the transfer notifications. Not exported: only our own `PendingIntent`s reach it. The work runs on
 * the app's scope, so the receiver returns at once.
 */
class TransferActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(TransferNotifications.EXTRA_TRANSFER_ID) ?: return
        val transfers = context.graph.transfers
        when (intent.action) {
            TransferNotifications.ACTION_CANCEL -> transfers.cancel(id)
            TransferNotifications.ACTION_RETRY -> transfers.retry(id)
        }
    }
}
