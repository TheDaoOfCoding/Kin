package app.kinrelay.android.links

import app.kinrelay.android.protocol.LinkWire

/**
 * A `link` the phone accepted: the URL (exactly as sent, `http` or `https` only), a cleaned title or null, and the **real address** to show with it.
 * [domain] is what a browser would connect to, never taken from the title; [warn] says the address is easy to misread or odd (a non-ASCII
 * host, a punycode label, an IP address, a host a browser refuses: protocol.md, "Showing the address of a link") and the notification marks it.
 */
data class ReceivedLink(
    val url: String,
    val title: String?,
    val domain: String,
    val warn: Boolean,
    val sourceApp: String?,
)

/**
 * What the phone does with a link from the PC (protocol.md, "Links: `link`"): which URLs are accepted ([LinkWire]: the wire rules, shared with
 * the receiver), and how the address the user sees is worked out ([LinkDomain]). No URL library for the accept/reject decision: libraries
 * disagree on exactly the points that matter here (`https://good.com\@evil.com`, which the rule refuses).
 */
object LinkRules {
    /** The URL if [raw] is one `http` or `https` URL that passes the wire rule **exactly as it is** (no trimming), else null. Everything else is rejected. */
    fun acceptUrl(raw: String): String? = raw.takeIf(LinkWire::isLinkUrl)

    /**
     * The URL as an `Intent` should carry it: the scheme in lower case. Android matches an intent filter's scheme **case-sensitively**, so
     * `HTTPS://example.com` (valid for the protocol: the scheme may be in any case) finds no app and no browser. Only the scheme changes
     * (everything before the first `://`); the rest of the URL is exactly as it came, and what is shown, copied and checked is never this form.
     */
    fun intentUrl(url: String): String {
        val end = url.indexOf("://")
        return if (end <= 0) url else url.substring(0, end).lowercase(java.util.Locale.ROOT) + url.substring(end)
    }

    /** Builds a [ReceivedLink], or null if the URL is rejected. A title and a source app that are already clean stay as they are. */
    fun accept(rawUrl: String, rawTitle: String?, rawSourceApp: String? = null): ReceivedLink? {
        val url = acceptUrl(rawUrl) ?: return null
        val address = LinkDomain.of(url) ?: return null
        return ReceivedLink(
            url = url,
            title = rawTitle?.let(LinkWire::cleanTitle),
            domain = address.display,
            warn = address.warn,
            sourceApp = rawSourceApp?.let(LinkWire::cleanSourceApp),
        )
    }

    /**
     * The URL for the eyes: the bidirectional controls and other invisible characters in it become U+FFFD (they could reorder the text around
     * them), and it is cut to [limit] code points with an ellipsis. Only for showing. What is opened is always the URL itself.
     */
    fun displayUrl(url: String, limit: Int): String {
        val safe = StringBuilder(url.length)
        var i = 0
        while (i < url.length) {
            val cp = url.codePointAt(i)
            i += Character.charCount(cp)
            if (isBidiOrInvisible(cp) || cp in 0xD800..0xDFFF) safe.append('�') else safe.appendCodePoint(cp)
        }
        val text = safe.toString()
        if (text.codePointCount(0, text.length) <= limit) return text
        return text.substring(0, text.offsetByCodePoints(0, limit)) + "…"
    }

    /** `Bidi_Control` characters, the zero-width characters and the other format characters a URL could hide or reorder text with (for display only). */
    private fun isBidiOrInvisible(cp: Int): Boolean =
        cp == 0x061C || cp == 0x200E || cp == 0x200F || cp in 0x202A..0x202E || cp in 0x2066..0x2069 ||
            cp in 0x200B..0x200D || cp == 0x2060 || cp == 0xFEFF || cp == 0x00AD || cp in 0xE0000..0xE007F
}
