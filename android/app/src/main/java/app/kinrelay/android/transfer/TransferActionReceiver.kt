package app.kinrelay.android.transfer

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.clipboard.AndroidClipboard
import app.kinrelay.android.graph
import app.kinrelay.android.links.LinkRules
import kotlinx.coroutines.launch

/**
 * The Cancel and Retry buttons of the transfer notifications, the swipe of the "texts from your PC" notification, and the Copy buttons of the
 * text and link notifications. Not exported: only our own `PendingIntent`s reach it. The work runs on the app's scope, so the receiver
 * returns at once (a copy holds the broadcast open for the moment it takes, `goAsync`).
 */
class TransferActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val graph = context.graph
        if (intent.action == ACTION_TEXTS_DISMISSED) {
            graph.notifications.textsSeen()
            return
        }
        // 2B.5: the swipe of the "alerts from your PC" notification: the next alert starts a new one.
        if (intent.action == ACTION_ALERTS_DISMISSED) {
            graph.alerts.seen()
            return
        }
        when (intent.action) {
            ACTION_COPY_LINK -> {
                // The URL (the intent's data) is checked again: it is what goes on the clipboard, and only an http or https address may.
                val url = intent.dataString?.let(LinkRules::acceptUrl) ?: return
                val pending = goAsync()
                graph.scope.launch {
                    try {
                        // Like Copy text: the clipboard sync must not send what the PC just gave us straight back to it.
                        graph.clipboardGuard.notePeerSet(url)
                        AndroidClipboard(context).write(url, sensitive = false)
                        graph.notifications.message("Link copied")
                        graph.notifications.linksSeen()
                    } catch (e: Exception) {
                        graph.log.w("links", "copy link failed: ${e.javaClass.simpleName}")
                        graph.notifications.message("Couldn't copy the link")
                    } finally {
                        pending.finish()
                    }
                }
                return
            }
            ACTION_COPY_TEXT -> {
                val messageId = intent.getStringExtra(EXTRA_MESSAGE_ID) ?: return
                val pending = goAsync()
                graph.scope.launch {
                    try {
                        // The newest text is in memory (a sensitive one only there); an older notification's text comes from the history.
                        val latest = graph.notifications.latestText(messageId)
                        val stored = if (latest?.text == null) graph.textOutbox.get(messageId)?.takeIf { !it.sensitive }?.body else null
                        val text = latest?.text ?: stored
                        if (text == null) {
                            graph.notifications.message("That text is no longer available. Look in History.")
                        } else {
                            graph.clipboardGuard.notePeerSet(text)
                            AndroidClipboard(context).write(text, sensitive = latest?.sensitive == true)
                            graph.notifications.message("Copied")
                        }
                    } catch (e: Exception) {
                        graph.log.w("link", "copy text failed: ${e.javaClass.simpleName}")
                        graph.notifications.message("Couldn't copy the text")
                    } finally {
                        pending.finish()
                    }
                }
                return
            }
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
        const val ACTION_ALERTS_DISMISSED = "app.kinrelay.android.ALERTS_DISMISSED"
        const val ACTION_COPY_LINK = "app.kinrelay.android.COPY_LINK"
        const val ACTION_COPY_TEXT = "app.kinrelay.android.COPY_TEXT"
        const val EXTRA_MESSAGE_ID = "messageId"
    }
}
