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
 * text goes to the sender, and the activity finishes. With both a file and a text, the file is sent. A text that is exactly one web link
 * goes as a `link` (with the page title from the share's subject), anything else as a `text`.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        val tapAt = SystemClock.elapsedRealtime()
        super.onCreate(savedInstanceState)
        val shared = intent
        val uris = if (shared?.action == Intent.ACTION_SEND || shared?.action == Intent.ACTION_SEND_MULTIPLE) ShareIntents.streams(shared) else emptyList()
        when {
            uris.isNotEmpty() -> SendEntry.runFiles(this, uris, shared?.type)
            shared?.action == Intent.ACTION_SEND && shared.hasExtra(Intent.EXTRA_TEXT) -> {
                val text = shared.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
                // A sender (a password manager) may mark the shared text sensitive the way it marks a clip.
                val sensitive = shared.clipData?.description?.let(AndroidClipboard::isSensitive) == true
                val url = ShareIntents.linkOf(text, sensitive)
                if (url != null) {
                    // A text that is exactly one web link is sent as a `link`, with the page title when the share has one (2B.1).
                    SendEntry.runLink(this, url, ShareIntents.titleOf(shared.getCharSequenceExtra(Intent.EXTRA_SUBJECT), shared.getCharSequenceExtra(Intent.EXTRA_TITLE)), tapAt)
                } else {
                    SendEntry.run(this, text, tapAt, SendSource.SHARE, nothingToSend = "Nothing to send.", sensitive = sensitive)
                }
            }
            else -> {
                graph.log.w("share", "a share with nothing Kin can send was ignored (action ${shared?.action}, type ${shared?.type}, refused=${shared?.let(ShareIntents::hasRefusedStreams)})")
                SendEntry.run(this, null, tapAt, SendSource.SHARE, nothingToSend = "Kin can send text and files.")
            }
        }
    }
}
