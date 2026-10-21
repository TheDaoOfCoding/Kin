package app.kinrelay.android.protocol

/**
 * The wire rules of the `link` message (protocol.md, "Links: `link`"; vectors `protocol/links.json` and the `link-*` cases of `messages.json`).
 * What the phone then shows (the real address, the lock-screen rule) is in `app.kinrelay.android.links`.
 */
object LinkWire {
    const val MAX_TITLE_CODE_POINTS = 300
    const val MAX_SOURCE_APP_CODE_POINTS = 64

    /** The capability a `hello` lists when the app understands the `link` message (protocol.md, "Versioning"). */
    const val CAPABILITY_LINK = "link"

    /**
     * Whether [url] is a valid `link.url`: the openable-URL rule ([UrlRules]: `http` or `https`, no whitespace or control character, a real host,
     * a valid port, at most 8 KiB) applied to the string **exactly as it is sent, with no trimming**. A sender trims what the user gave it first.
     */
    fun isLinkUrl(url: String): Boolean = UrlRules.httpUrl(url) == url

    /**
     * A `title` or `sourceApp` as the receiver keeps it, cut to [maxCodePoints] code points; null when nothing is left. In order: (1) every
     * `Bidi_Control` character of the file-name rules becomes `_`; (2) every control character (U+0000 to U+001F, U+007F to U+009F: tab, CR
     * and LF included) becomes a space; (3) runs of U+0020 become one space and the ends are trimmed; (4) the first [maxCodePoints] code points
     * stay (never cutting inside a code point), trailing spaces trimmed again; (5) empty is no text. Other whitespace (no-break space) and
     * other invisible characters are kept, as in file names.
     */
    fun cleanText(input: String, maxCodePoints: Int): String? {
        val replaced = StringBuilder(input.length)
        var i = 0
        while (i < input.length) {
            val cp = input.codePointAt(i)
            i += Character.charCount(cp)
            when {
                cp <= 0xFFFF && FileNames.isBidiControl(cp.toChar()) -> replaced.append('_')
                cp <= 0x1F || cp in 0x7F..0x9F -> replaced.append(' ')
                else -> replaced.appendCodePoint(cp)
            }
        }
        val collapsed = StringBuilder(replaced.length)
        var previousSpace = false
        for (c in replaced) {
            if (c == ' ') {
                if (previousSpace) continue
                previousSpace = true
            } else {
                previousSpace = false
            }
            collapsed.append(c)
        }
        var text = collapsed.toString().trim(' ')
        if (text.codePointCount(0, text.length) > maxCodePoints) {
            text = text.substring(0, text.offsetByCodePoints(0, maxCodePoints)).trimEnd(' ')
        }
        return text.ifEmpty { null }
    }

    fun cleanTitle(input: String): String? = cleanText(input, MAX_TITLE_CODE_POINTS)

    fun cleanSourceApp(input: String): String? = cleanText(input, MAX_SOURCE_APP_CODE_POINTS)
}

/** A valid `link` as the receiver keeps it: the URL exactly as sent, and the cleaned title and source app (null = none). */
data class LinkMessage(val url: String, val title: String?, val sourceApp: String?)
