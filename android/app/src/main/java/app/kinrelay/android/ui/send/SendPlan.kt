package app.kinrelay.android.ui.send

import app.kinrelay.android.clipboard.ClipboardContent
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.link.SendSource
import app.kinrelay.android.share.ShareIntents

/** The decisions of the in-app Send button that need no Android: what a typed text becomes, what a paste does, what the words say. */
object SendPlan {
    /** What the compose box's text turns into. */
    sealed interface Typed {
        /** Empty or only whitespace: the Send button stays off. */
        data object Empty : Typed

        /** The text is exactly one web link (after trimming): it goes as a `link`, like a link from the share sheet. */
        data class Link(val url: String) : Typed

        /** Anything else, a sentence with a link in it included: it goes as a `text`, as typed. */
        data class Plain(val text: String) : Typed
    }

    fun typed(raw: String): Typed = when {
        raw.isBlank() -> Typed.Empty
        else -> ShareIntents.linkOf(raw, sensitive = false)?.let(Typed::Link) ?: Typed.Plain(raw)
    }

    /** What the Paste tile does with what it read from the clipboard. */
    sealed interface Paste {
        /** No text on the clipboard (or only whitespace): the user is told, nothing is queued. */
        data object Empty : Paste

        /** Send [text] as [SendSource.CLIPBOARD], so the loop guard skips what the PC just put there. A [sensitive] clip is never queued or logged. */
        data class Send(val text: String, val sensitive: Boolean) : Paste {
            val source: SendSource get() = SendSource.CLIPBOARD
        }
    }

    fun paste(content: ClipboardContent?): Paste =
        if (content == null || content.text.isBlank()) Paste.Empty else Paste.Send(content.text, content.sensitive)

    /** True when a send would go out at once. Everything else waits in the queue (or says why it can't). */
    fun connected(state: LinkState): Boolean = state is LinkState.Connected

    /** The line under "Send" on the Home button. */
    fun buttonSubtitle(state: LinkState): String =
        if (connected(state)) "Text, links, files or photos" else "It will wait until your PC is back"

    /** The line under the sheet's title. */
    fun sheetStatus(state: LinkState): String =
        if (connected(state)) "Connected, it arrives in a moment" else "Not connected yet, it will wait"
}
