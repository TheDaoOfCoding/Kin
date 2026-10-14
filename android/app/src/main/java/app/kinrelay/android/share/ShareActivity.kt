package app.kinrelay.android.share

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import app.kinrelay.android.graph
import app.kinrelay.android.link.SendSource

/**
 * `ACTION_SEND` with `text/plain`: the Android share sheet's "Kin". No UI: the text goes to the service's
 * sender and the activity finishes. Files and photos come with 1.3.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        val tapAt = SystemClock.elapsedRealtime()
        super.onCreate(savedInstanceState)
        val text = if (intent?.action == Intent.ACTION_SEND) intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString() else null
        if (text == null) graph.log.w("share", "a share without text was ignored (type ${intent?.type}); files come with 1.3")
        SendEntry.run(this, text, tapAt, SendSource.SHARE, nothingToSend = "Kin can only send text for now.")
    }
}
