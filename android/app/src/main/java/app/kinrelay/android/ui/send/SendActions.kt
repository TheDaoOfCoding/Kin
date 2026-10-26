package app.kinrelay.android.ui.send

import android.content.Context
import android.net.Uri
import android.os.SystemClock
import app.kinrelay.android.AppGraph
import app.kinrelay.android.clipboard.AndroidClipboard
import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.link.LinkService
import app.kinrelay.android.link.SendSource
import app.kinrelay.android.transfer.ShareSource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * What the in-app Send button does. The sending itself is the machinery the share sheet, the text menu and the tile already use
 * ([app.kinrelay.android.link.OutgoingSender], [app.kinrelay.android.transfer.TransferEngine]); this is only the entry point from inside the
 * app. Everything runs on the app's scope, so a send goes on when the sheet closes, and the sender tells the user how it went.
 */
class SendActions(private val graph: AppGraph, private val context: Context) {
    /** The compose box: one web link goes as a `link`, anything else as a `text`. Both are explicit, so the loop guard never skips them. */
    fun sendTyped(raw: String) {
        val tapAt = SystemClock.elapsedRealtime()
        when (val plan = SendPlan.typed(raw)) {
            SendPlan.Typed.Empty -> graph.notifications.message("Nothing to send.")
            is SendPlan.Typed.Link -> graph.scope.launch { graph.sender.sendLink(plan.url, null, tapAt, SendSource.TYPED) }
            is SendPlan.Typed.Plain -> graph.scope.launch { graph.sender.send(plan.text, tapAt, SendSource.TYPED) }
        }
    }

    /**
     * The Paste tile. The app is on screen, so it may read the clipboard now (Android 10+ lets only the focused app do that). It goes as a
     * [SendSource.CLIPBOARD] send like the tile's, which is what makes the sender skip a text the PC set a moment ago and mark a password
     * manager's clip sensitive.
     */
    fun paste() {
        val tapAt = SystemClock.elapsedRealtime()
        val content = AndroidClipboard.read(context)
        val plan = SendPlan.paste(content)
        // A sensitive clip is logged by nothing at all, not even its hash.
        val shown = when {
            content == null -> "nothing"
            content.sensitive -> "sensitive"
            else -> ClipboardGuard.fingerprint(content.text)
        }
        graph.log.i("clipboard", "clipboard read (in-app paste): $shown")
        when (plan) {
            SendPlan.Paste.Empty -> graph.notifications.message("There's no text on the clipboard to send.")
            is SendPlan.Paste.Send -> graph.scope.launch { graph.sender.send(plan.text, tapAt, plan.source, plan.sensitive) }
        }
    }

    /**
     * Files or photos the user picked. A picker hands read access to this activity only, and a queued upload may run long after the user is
     * gone from here, so the URIs go to [LinkService] with `FLAG_GRANT_READ_URI_PERMISSION` first, exactly as a share does
     * ([app.kinrelay.android.share.SendEntry.runFiles]); a file under 50 MB is also copied into the cache when its upload has to wait.
     */
    fun sendFiles(uris: List<Uri>) {
        if (uris.isEmpty()) return
        graph.log.i("share", "files picked in the app: ${uris.size}")
        LinkService.startWithGrant(graph.appContext, uris)
        graph.scope.launch(Dispatchers.IO) {
            graph.transfers.enqueueUpload(uris.map { ShareSource(it.toString()) })
        }
    }
}
