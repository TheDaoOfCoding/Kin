package app.kinrelay.android.links

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.core.net.toUri
import app.kinrelay.android.graph

/**
 * The tap on a link notification. It has no UI: it opens the link by the plan ([LinkOpener]: the app that owns it if there is one, else the
 * default browser) and finishes. It is an activity of ours (not a broadcast or a service) because Android 12 and later refuse a notification
 * that starts an activity through a trampoline, and the plan needs code at tap time (the fallback to the browser when no app owns the link).
 * Not exported: only our own `PendingIntent`s start it, and the URL is checked again (`http` or `https` only).
 */
class LinkOpenActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val url = intent?.dataString
        val graph = applicationContext.graph
        if (url == null || !AndroidLinks(this).let { LinkOpener(it, it).open(url) }) {
            graph.log.w("links", "a tapped link could not be opened (${if (url == null) "no address" else "no app or browser took it"})")
            Toast.makeText(this, "No app can open this link. Use Copy link.", Toast.LENGTH_LONG).show()
        }
        // The notification went with the tap (setAutoCancel); its group summary is refreshed by the ledger.
        graph.notifications.linksSeen()
        finish()
    }

    companion object {
        /** The intent a notification holds: the URL as data, so that a different link is a different `PendingIntent`. */
        fun intent(context: Context, url: String): Intent =
            Intent(context, LinkOpenActivity::class.java).setData(url.toUri()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_HISTORY)
    }
}
