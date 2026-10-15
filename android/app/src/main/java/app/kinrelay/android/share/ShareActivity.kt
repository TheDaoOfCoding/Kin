package app.kinrelay.android.share

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import app.kinrelay.android.clipboard.AndroidClipboard
import app.kinrelay.android.graph
import app.kinrelay.android.link.SendSource

/**
 * `ACTION_SEND` and `ACTION_SEND_MULTIPLE`, any MIME type: the Android share sheet's "Kin" (and, through the direct-share shortcut, the
 * paired PC in the top row). No UI: files go to the transfer engine (the service gets read access to them first, see [SendEntry.runFiles]),
 * text goes to the sender, and the activity finishes. With both a file and a text, the file is sent.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        val tapAt = SystemClock.elapsedRealtime()
        super.onCreate(savedInstanceState)
        val shared = intent
        val uris = if (shared?.action == Intent.ACTION_SEND || shared?.action == Intent.ACTION_SEND_MULTIPLE) ShareIntents.streams(shared) else emptyList()
        when {
            uris.isNotEmpty() -> SendEntry.runFiles(this, uris, shared?.type)
            shared?.action == Intent.ACTION_SEND && shared.hasExtra(Intent.EXTRA_TEXT) ->
                SendEntry.run(
                    this, shared.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString(), tapAt, SendSource.SHARE, nothingToSend = "Nothing to send.",
                    // A sender (a password manager) may mark the shared text sensitive the way it marks a clip.
                    sensitive = shared.clipData?.description?.let(AndroidClipboard::isSensitive) == true,
                )
            else -> {
                graph.log.w("share", "a share with nothing Kin can send was ignored (action ${shared?.action}, type ${shared?.type}, refused=${shared?.let(ShareIntents::hasRefusedStreams)})")
                SendEntry.run(this, null, tapAt, SendSource.SHARE, nothingToSend = "Kin can send text and files.")
            }
        }
    }
}
