package app.kinrelay.android.share

import android.app.Activity
import android.net.Uri
import app.kinrelay.android.DebugHooks
import app.kinrelay.android.graph
import app.kinrelay.android.link.LinkService
import app.kinrelay.android.link.SendSource
import app.kinrelay.android.transfer.ShareSource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * What the no-UI send activities share: start the service while the activity is still on screen (a visible
 * activity may always start it), hand the text or the files to the sender, and finish. The sender runs on the app's scope,
 * so it goes on after the activity is gone, and it tells the user how it went.
 */
object SendEntry {
    /** [text] is null when there was nothing to send; [nothingToSend] is then what the user is told. */
    fun run(
        activity: Activity,
        text: String?,
        tapAtMillis: Long,
        source: SendSource,
        nothingToSend: String,
        sensitive: Boolean = false,
    ) {
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
        graph.scope.launch { graph.sender.send(text, tapAtMillis, source, sensitive) }
    }

    /**
     * A web page from the share sheet (2B.1): the text was exactly one URL, so it goes as a `link` with the page title ([title], the share's
     * subject, or null). Same shape as [run]: start the service, hand the link to the sender on the app's scope, finish at once.
     */
    fun runLink(activity: Activity, url: String, title: String?, tapAtMillis: Long) {
        val graph = activity.graph
        graph.scope.launch {
            try {
                graph.link.ensureRunning(activity.applicationContext)
            } finally {
                withContext(Dispatchers.Main) { activity.finish() }
            }
        }
        graph.scope.launch { graph.sender.sendLink(url, title, tapAtMillis, SendSource.SHARE_LINK) }
    }

    /**
     * Files from the share sheet. The system lets this activity read them only while it lives, and it finishes at once, so the same
     * URIs are passed to [LinkService] with `FLAG_GRANT_READ_URI_PERMISSION`: the service is long-lived, and access given to it
     * stays after this activity is gone (android.md; the finding is in docs/results/phase-1-3-android.md, R5). Only then is the
     * transfer created, on the app's scope, and the activity finishes.
     */
    fun runFiles(activity: Activity, uris: List<Uri>, mimeHint: String?) {
        val graph = activity.graph
        graph.log.i("share", "files shared: ${uris.size} (type $mimeHint)")
        if (DebugHooks.grantUrisToService(activity)) LinkService.startWithGrant(activity.applicationContext, uris) else LinkService.start(activity.applicationContext)
        graph.scope.launch(Dispatchers.IO) {
            try {
                graph.transfers.enqueueUpload(uris.map { ShareSource(it.toString(), mimeHint = mimeHint) })
            } finally {
                // Finished only after the files were looked at, so even the first look has the activity's own access.
                withContext(Dispatchers.Main) { activity.finish() }
            }
        }
    }
}
