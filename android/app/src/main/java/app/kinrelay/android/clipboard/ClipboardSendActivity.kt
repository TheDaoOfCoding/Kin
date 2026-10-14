package app.kinrelay.android.clipboard

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import app.kinrelay.android.graph
import app.kinrelay.android.link.SendSource
import app.kinrelay.android.share.SendEntry

/**
 * Sends the phone's clipboard to the PC, once. Since Android 10 only the focused app may read the clipboard,
 * so this transparent activity waits until its window has focus, reads, hands the text to the sender and
 * closes. The tile and the notification button start it (see docs/android.md, "Why sending the clipboard is manual").
 */
class ClipboardSendActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private var tapAt = 0L
    private var handled = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        tapAt = SystemClock.elapsedRealtime()
        // If the window never gets focus (a locked screen, a dialog on top), try once anyway instead of hanging.
        handler.postDelayed({ readAndSend("no focus after $FOCUS_WAIT_MILLIS ms") }, FOCUS_WAIT_MILLIS)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) readAndSend("focused")
    }

    private fun readAndSend(why: String) {
        if (handled) return
        handled = true
        handler.removeCallbacksAndMessages(null)
        val text = AndroidClipboard.read(this)
        graph.log.i("clipboard", "clipboard read ($why): ${text?.let(ClipboardGuard::fingerprint) ?: "nothing"}")
        SendEntry.run(this, text, tapAt, SendSource.CLIPBOARD, nothingToSend = "There's no text on the clipboard to send.")
    }

    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        super.onDestroy()
    }

    companion object {
        private const val FOCUS_WAIT_MILLIS = 1_500L

        fun intent(context: Context): Intent = Intent(context, ClipboardSendActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION or Intent.FLAG_ACTIVITY_EXCLUDE_FROM_RECENTS)
    }
}
