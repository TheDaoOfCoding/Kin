package app.kinrelay.android.protocol

/**
 * The openable-URL rule (protocol.md, "Text messages", "Links: `isUrl` and openable URLs"), applied after trimming. It decides both
 * `isUrl` on a text the phone sends and whether a received link gets an Open action: the `isUrl` flag from the peer is never trusted
 * by itself. A text is an openable URL when
 * - the scheme is `http` or `https` (any case), followed by `://`;
 * - there is no whitespace or control character anywhere;
 * - the host is not empty and not only dots, or it is a non-empty `[IPv6]`;
 * - a port, if present, is 1 to 65535 in ASCII digits (a bare `:` is refused);
 * - the whole thing is at most 8 KiB (UTF-8).
 *
 * No URL library: they disagree on exactly these points. The shared vectors are `protocol/urls.json`.
 */
object UrlRules {
    const val MAX_BYTES = 8 * 1024

    /** The trimmed URL if [text] is one openable URL, else null. */
    fun httpUrl(text: String): String? {
        val url = text.trim { isWhitespace(it) }
        val rest = when {
            url.startsWith("https://", ignoreCase = true) -> url.substring(8)
            url.startsWith("http://", ignoreCase = true) -> url.substring(7)
            else -> return null
        }
        if (url.any { isWhitespace(it) || isControl(it) }) return null
        if (Utf8.size(url) > MAX_BYTES) return null
        val authority = rest.takeWhile { it != '/' && it != '?' && it != '#' }
        val hostPart = authority.substringAfterLast('@')
        return if (hostAndPortAreValid(hostPart)) url else null
    }

    fun isUrl(text: String): Boolean = httpUrl(text) != null

    private fun hostAndPortAreValid(hostPart: String): Boolean {
        if (hostPart.startsWith('[')) {
            // An IPv6 literal: at least one character between the brackets, no other bracket inside, then nothing or `:` and a port.
            val close = hostPart.indexOf(']')
            if (close < 2) return false
            val inside = hostPart.substring(1, close)
            if ('[' in inside) return false
            val after = hostPart.substring(close + 1)
            return after.isEmpty() || (after[0] == ':' && portIsValid(after.substring(1)))
        }
        // Split at the first `:`: the host is before it, the port after it.
        val colon = hostPart.indexOf(':')
        val host = if (colon >= 0) hostPart.substring(0, colon) else hostPart
        if (host.all { it == '.' }) return false // empty, or only dots
        return colon < 0 || portIsValid(hostPart.substring(colon + 1))
    }

    /** ASCII digits only, value 1 to 65535, leading zeros allowed. Empty (a bare `:`) is refused. */
    private fun portIsValid(port: String): Boolean {
        if (port.isEmpty() || !port.all { it in '0'..'9' }) return false
        val significant = port.trimStart('0')
        return significant.isNotEmpty() && significant.length <= 5 && significant.toInt() in 1..65535
    }

    /** The Unicode `White_Space` set, written out (the spec lists it): Kotlin's `isWhitespace` leaves out U+00A0, U+2007 and U+202F. */
    fun isWhitespace(c: Char): Boolean {
        val code = c.code
        return code in 0x0009..0x000D || code == 0x0020 || code == 0x0085 || code == 0x00A0 || code == 0x1680 ||
            code in 0x2000..0x200A || code == 0x2028 || code == 0x2029 || code == 0x202F || code == 0x205F || code == 0x3000
    }

    /** U+0000 to U+001F and U+007F to U+009F. */
    private fun isControl(c: Char): Boolean = c.code <= 0x1F || c.code in 0x7F..0x9F
}
