package app.kinrelay.android.protocol

/**
 * The `isUrl` rule of protocol.md: true only when the whole text, trimmed, is one absolute `http://` or
 * `https://` URL with no whitespace. The same check decides whether a received link may be opened
 * (only `http` and `https`; the `isUrl` flag from the peer is never trusted by itself).
 */
object UrlRules {
    /** The trimmed URL if [text] is one absolute http(s) URL, else null. */
    fun httpUrl(text: String): String? {
        val url = text.trim { isSpace(it) }
        val rest = when {
            url.startsWith("https://", ignoreCase = true) -> url.substring(8)
            url.startsWith("http://", ignoreCase = true) -> url.substring(7)
            else -> return null
        }
        if (url.any { isSpace(it) || Character.isISOControl(it) }) return null
        val authority = rest.takeWhile { it != '/' && it != '?' && it != '#' }
        val hostAndPort = authority.substringAfterLast('@')
        return if (hasHost(hostAndPort)) url else null
    }

    fun isUrl(text: String): Boolean = httpUrl(text) != null

    private fun hasHost(hostAndPort: String): Boolean {
        if (hostAndPort.startsWith('[')) {
            val close = hostAndPort.indexOf(']')
            return close > 1 && portIsValid(hostAndPort.substring(close + 1))
        }
        val colon = hostAndPort.lastIndexOf(':')
        val host = if (colon >= 0) hostAndPort.substring(0, colon) else hostAndPort
        val port = if (colon >= 0) hostAndPort.substring(colon) else ""
        return host.isNotEmpty() && portIsValid(port)
    }

    /** Empty, or `:` followed by ASCII digits only (a bare `:` is allowed, as in `http://host:/`). */
    private fun portIsValid(port: String): Boolean = port.isEmpty() || (port[0] == ':' && port.drop(1).all { it in '0'..'9' })

    /** Whitespace as Unicode defines it (Rust's `trim` does the same), which Kotlin's `isWhitespace` lacks for U+0085. */
    private fun isSpace(c: Char): Boolean = c.isWhitespace() || c == '\u0085'
}
