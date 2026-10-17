package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.clipboard.ClipboardPort
import app.kinrelay.android.links.LinkRules
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.UrlRules

/**
 * What the phone does with `clipboard` and `text` from the PC (the ack has already gone out):
 * - `clipboard`: set the clipboard, silently. Not stored, not shown. The log gets a length and a hash.
 * - `text`: set the clipboard and post "Text from <PC>" with a preview; a link gets an Open action.
 *   History comes in 1.4, so for now the de-duplication memory is all that remembers it.
 * - `text` with `sensitive` (the PC saw a password manager): the clipboard is set and marked sensitive, the notification has no
 *   preview and no Open action, and nothing about the text is logged, not even its hash. It is never stored.
 * - A `text` is put on the clipboard when it arrives unless the user turned "Copy received text automatically" off ([copyAutomatically]);
 *   the notification always has a Copy button.
 * - `link` (2B.1): checked again (`http` or `https` only, the real address worked out from the URL), then a notification with the title,
 *   the address, a tap that opens it and a Copy link button. A link is **never** copied or opened by itself.
 */
class IncomingHandler(
    private val clipboard: ClipboardPort,
    private val alerts: Alerts,
    private val guard: ClipboardGuard,
    private val log: KinLog,
    /** "Copy received text automatically" (default on, the behaviour since 1.2). */
    private val copyAutomatically: suspend () -> Boolean = { true },
    /** "Show link previews on the lock screen" (default off). */
    private val lockScreenLinks: suspend () -> Boolean = { false },
) {
    suspend fun onClipboard(text: String) {
        guard.notePeerSet(text)
        val written = write(text, sensitive = true)
        log.i("link", "clipboard from the PC ${ClipboardGuard.fingerprint(text)} written=$written")
        if (!written) tellNotCopied()
    }

    suspend fun onText(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean = false) {
        val automatic = copyAutomatically()
        if (automatic) guard.notePeerSet(text)
        val written = automatic && write(text, sensitive = sensitive)
        if (sensitive) {
            alerts.textFrom(fromName, id, text = null, openUrl = null, copyText = text, sensitive = true, copiedAlready = written)
            log.i("link", "sensitive text from '$fromName' written=$written automatic=$automatic (its content is not logged)")
        } else {
            val openUrl = if (isUrl) UrlRules.httpUrl(text) else null
            // A text that is a web address is shown like a link (2B.1 review): the real address first, with the warning mark when it is tricky, then
            // the URL; never the raw text alone. The phone checks the rule again and never trusts `isUrl`. Copy still copies the text as it came.
            val shown = openUrl?.let(::shownLikeALink) ?: text
            alerts.textFrom(fromName, id, shown, openUrl, copyText = text, sensitive = false, copiedAlready = written)
            log.i("link", "text from '$fromName' ${ClipboardGuard.fingerprint(text)} isUrl=$isUrl openable=${openUrl != null} written=$written automatic=$automatic")
        }
        if (automatic && !written) tellNotCopied()
    }

    /** A `link` from the PC, already acked. Returns the link that was shown, or null when it is not one the phone accepts (nothing is shown). */
    suspend fun onLink(fromName: String, id: String, url: String, title: String?, sourceApp: String?): app.kinrelay.android.links.ReceivedLink? {
        val link = LinkRules.accept(url, title, sourceApp)
        if (link == null) {
            log.w("link", "a link from '$fromName' was refused: not an http or https address")
            return null
        }
        alerts.linkFrom(fromName, id, link, lockScreenPreviews = lockScreenLinks())
        // protocol.md: no URL in a log (a URL can carry a token), the length and the scheme at most.
        log.i("link", "link from '$fromName' scheme=${link.url.substringBefore(':').lowercase()} urlChars=${link.url.length} titleChars=${link.title?.length ?: 0} warn=${link.warn}")
        return link
    }

    /** "[⚠ ]address" on the first line and the URL (shortened, invisible characters made visible) on the next, as the link notification shows them. */
    private fun shownLikeALink(url: String): String {
        val link = LinkRules.accept(url, null) ?: return url
        val address = if (link.warn) "${app.kinrelay.android.links.LinkNotificationContent.WARNING_MARK} ${link.domain}" else link.domain
        return address + "\n" + LinkRules.displayUrl(link.url, app.kinrelay.android.links.LinkNotificationContent.URL_SHOWN_CHARACTERS)
    }

    /** The ack has gone out already, so without this the PC would say "sent" and Paste on the phone would give the old text. */
    private fun tellNotCopied() =
        alerts.sendFailed("Couldn't copy the text from your PC", "It may be too long for the phone's clipboard.")

    private suspend fun write(text: String, sensitive: Boolean): Boolean = try {
        clipboard.write(text, sensitive)
        true
    } catch (e: Exception) {
        log.w("link", "could not set the clipboard: ${e.javaClass.simpleName}: ${e.message}")
        false
    }
}
