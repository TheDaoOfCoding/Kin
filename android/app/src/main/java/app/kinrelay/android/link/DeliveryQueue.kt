package app.kinrelay.android.link

import app.kinrelay.android.log.KinLog
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch

/**
 * Runs what the PC sent (set the clipboard, show a notification) one after the other, in the order the
 * messages arrived. Two clipboard messages in a row must not overtake each other, or the older text
 * would end up on the clipboard.
 */
class DeliveryQueue(private val log: KinLog) {
    private val tasks = Channel<suspend () -> Unit>(Channel.UNLIMITED)

    /** Starts running the tasks. Until then they wait. */
    fun start(scope: CoroutineScope) {
        scope.launch {
            for (task in tasks) {
                try {
                    task()
                } catch (e: Exception) {
                    log.w("link", "delivering a message failed: ${e.javaClass.simpleName}: ${e.message}")
                }
            }
        }
    }

    fun submit(task: suspend () -> Unit) {
        tasks.trySend(task)
    }

    /** Tasks already queued still run; nothing new is accepted. */
    fun close() {
        tasks.close()
    }
}
