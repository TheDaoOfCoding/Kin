package app.kinrelay.android.share

import android.app.Activity
import app.kinrelay.android.graph
import app.kinrelay.android.link.SendSource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * What the no-UI send activities share: start the service while the activity is still on screen (a visible
 * activity may always start it), hand the text to the sender, and finish. The sender runs on the app's scope,
 * so it goes on after the activity is gone, and it tells the user how it went.
 */
object SendEntry {
    /** [text] is null when there was nothing to send; [nothingToSend] is then what the user is told. */
    fun run(activity: Activity, text: String?, tapAtMillis: Long, source: SendSource, nothingToSend: String) {
        val graph = activity.graph
        if (text == null) {
            graph.notifications.message(nothingToSend)
            activity.finish()
            return
        }
        graph.scope.launch {
            try {
                graph.link.ensureRunning(activity.applicationContext)
            } finally {
                withContext(Dispatchers.Main) { activity.finish() }
            }
        }
        graph.scope.launch { graph.sender.send(text, tapAtMillis, source) }
    }
}
