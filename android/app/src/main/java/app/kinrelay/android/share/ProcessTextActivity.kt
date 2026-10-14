package app.kinrelay.android.share

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import app.kinrelay.android.link.SendSource

/** `ACTION_PROCESS_TEXT`: "Send to PC" in the text selection menu. No UI, nothing is returned to the app. */
class ProcessTextActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        val tapAt = SystemClock.elapsedRealtime()
        super.onCreate(savedInstanceState)
        val text = intent?.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()
        SendEntry.run(this, text, tapAt, SendSource.PROCESS_TEXT, nothingToSend = "Nothing selected to send.")
    }
}
