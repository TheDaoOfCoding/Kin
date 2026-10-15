package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.clipboard.ClipboardPort
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.UrlRules

/**
 * What the phone does with `clipboard` and `text` from the PC (the ack has already gone out):
 * - `clipboard`: set the clipboard, silently. Not stored, not shown. The log gets a length and a hash.
 * - `text`: set the clipboard and post "Text from <PC>" with a preview; a link gets an Open action.
 *   History comes in 1.4, so for now the de-duplication memory is all that remembers it.
 * - `text` with `sensitive` (the PC saw a password manager): the clipboard is set and marked sensitive, the notification has no
 *   preview and no Open action, and nothing about the text is logged, not even its hash. It is never stored.
 */
class IncomingHandler(
    private val clipboard: ClipboardPort,
    private val alerts: Alerts,
    private val guard: ClipboardGuard,
    private val log: KinLog,
) {
    suspend fun onClipboard(text: String) {
        guard.notePeerSet(text)
        val written = write(text, sensitive = true)
        log.i("link", "clipboard from the PC ${ClipboardGuard.fingerprint(text)} written=$written")
        if (!written) tellNotCopied()
    }

    suspend fun onText(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean = false) {
        guard.notePeerSet(text)
        val written = write(text, sensitive = sensitive)
        if (sensitive) {
            alerts.textFrom(fromName, id, text = null, openUrl = null)
            log.i("link", "sensitive text from '$fromName' written=$written (its content is not logged)")
        } else {
            val openUrl = if (isUrl) UrlRules.httpUrl(text) else null
            alerts.textFrom(fromName, id, text, openUrl)
            log.i("link", "text from '$fromName' ${ClipboardGuard.fingerprint(text)} isUrl=$isUrl openable=${openUrl != null} written=$written")
        }
        if (!written) tellNotCopied()
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
