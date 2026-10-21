package app.kinrelay.android.links

import android.icu.text.IDNA

/**
 * UTS 46 ToASCII as a browser does it (non-transitional, `CheckBidi`, `CheckJoiners`, no STD3 rules, no hyphen or length checks): the ASCII
 * (punycode) form of a host, or null when a browser refuses the host. A seam so the JVM unit tests can run the same algorithm on ICU4J
 * (`android.icu` is only a stub there); the app uses [AndroidUts46].
 */
fun interface Uts46 {
    fun toAscii(host: String): String?
}

/** `android.icu.text.IDNA` (API 24+, the app's minSdk is 29). `java.net.IDN` is IDNA 2003 and gives other answers (`faß.de` → `fass.de`). */
object AndroidUts46 : Uts46 {
    private val idna: IDNA by lazy {
        IDNA.getUTS46Instance(IDNA.NONTRANSITIONAL_TO_ASCII or IDNA.CHECK_BIDI or IDNA.CHECK_CONTEXTJ)
    }

    /** The errors the WHATWG URL Standard does not count (it sets `CheckHyphens` and `VerifyDnsLength` to false). */
    private val ignored = setOf(
        IDNA.Error.EMPTY_LABEL, IDNA.Error.LABEL_TOO_LONG, IDNA.Error.DOMAIN_NAME_TOO_LONG,
        IDNA.Error.LEADING_HYPHEN, IDNA.Error.TRAILING_HYPHEN, IDNA.Error.HYPHEN_3_4,
    )

    override fun toAscii(host: String): String? {
        val info = IDNA.Info()
        val out = StringBuilder()
        idna.nameToASCII(host, out, info)
        if (info.errors.any { it !in ignored }) return null
        return out.toString()
    }
}

/**
 * The address of a URL as a browser reads it (protocol.md, "Showing the address of a link"; the vectors are `display[]` of `protocol/links.json`).
 * The input has passed the openable-URL rule, so the authority is only `host[:port]` (no `@`, `\` or `%` in it).
 */
object LinkDomain {
    class Address(
        /** What to show: the host a browser connects to (ASCII lower case; punycode, dotted-decimal IPv4 and compressed IPv6 as a browser makes them), with the port unless it is the default. */
        val display: String,
        /** The address is easy to misread or odd: non-ASCII as written, a punycode label, an IP address, or a host a browser refuses. */
        val warn: Boolean,
    )

    /** The address [url] (already accepted by [LinkRules.acceptUrl]) leads to; null if [url] isn't an `http(s)` URL with a plain authority. */
    fun of(url: String, uts46: Uts46 = AndroidUts46): Address? {
        val https = url.startsWith("https://", ignoreCase = true)
        val rest = when {
            https -> url.substring(8)
            url.startsWith("http://", ignoreCase = true) -> url.substring(7)
            else -> return null
        }
        val authority = rest.takeWhile { it != '/' && it != '?' && it != '#' }
        if (authority.any { it == '@' || it == '\\' || it == '%' }) return null
        val host: String
        val port: String?
        if (authority.startsWith('[')) {
            val close = authority.indexOf(']')
            if (close < 2) return null
            host = authority.substring(0, close + 1)
            port = authority.substring(close + 1).removePrefix(":").takeIf { it.isNotEmpty() }
        } else {
            host = authority.substringBefore(':')
            port = if (':' in authority) authority.substringAfter(':') else null
        }
        if (host.isEmpty() || host.all { it == '.' }) return null

        val shown = browserHost(host, uts46) ?: return Address(asWritten(host), warn = true)
        val defaultPort = if (https) 443 else 80
        val number = port?.trimStart('0')?.takeIf { it.isNotEmpty() }?.toIntOrNull()
        val display = if (number != null && number != defaultPort) "${shown.text}:$number" else shown.text
        val warn = shown.isIp || host.any { it.code > 0x7F } || shown.text.split('.').any { it.startsWith("xn--", ignoreCase = true) }
        return Address(display, warn)
    }

    private class Shown(val text: String, val isIp: Boolean)

    /** The WHATWG host parser (special schemes) of [host] (brackets included for IPv6), or null when a browser refuses it. */
    private fun browserHost(host: String, uts46: Uts46): Shown? {
        if (host.startsWith('[')) {
            if (!host.endsWith(']')) return null
            val pieces = parseIpv6(host.substring(1, host.length - 1)) ?: return null
            return Shown("[" + serializeIpv6(pieces) + "]", isIp = true)
        }
        // Plain ASCII with no punycode label needs no UTS 46 beyond lower-casing (the common case never loads ICU).
        val plain = host.all { it.code <= 0x7F } && host.split('.').none { it.startsWith("xn--", ignoreCase = true) }
        val ascii = if (plain) host.lowercase() else uts46.toAscii(host) ?: return null
        if (ascii.isEmpty() || ascii.any { isForbiddenDomainCodePoint(it) }) return null
        if (endsInANumber(ascii)) {
            val v4 = parseIpv4(ascii) ?: return null
            return Shown(serializeIpv4(v4), isIp = true)
        }
        return Shown(ascii.lowercase(), isIp = false) // already lower case from UTS 46; plain ASCII hosts pass through the same way
    }

    /** What is shown for a host a browser refuses: as written, ASCII lower case, every character outside U+0021 to U+007E as U+FFFD, no port. */
    private fun asWritten(host: String): String {
        val out = StringBuilder(host.length)
        var i = 0
        while (i < host.length) {
            val cp = host.codePointAt(i)
            i += Character.charCount(cp)
            when (cp) {
                in 'A'.code..'Z'.code -> out.append((cp + 32).toChar())
                in 0x21..0x7E -> out.append(cp.toChar())
                else -> out.append('�')
            }
        }
        return out.toString()
    }

    /** A forbidden host code point or one of the extra forbidden domain code points (C0 controls, `%`, DEL; the others are the rule's own). */
    private fun isForbiddenDomainCodePoint(c: Char): Boolean =
        c.code <= 0x20 || c.code == 0x7F || c in "#/:<>?@[\\]^|%"

    // --- IPv4 ---

    private fun endsInANumber(host: String): Boolean {
        val parts = host.split('.').toMutableList()
        if (parts.last().isEmpty()) {
            if (parts.size == 1) return false
            parts.removeAt(parts.size - 1)
        }
        val last = parts.last()
        if (last.isNotEmpty() && last.all { it in '0'..'9' }) return true
        return parseIpv4Number(last) != null
    }

    /** A saturated value (never overflows); null if [text] is not an IPv4 number (empty, or a character outside its radix). */
    private fun parseIpv4Number(text: String): Long? {
        if (text.isEmpty()) return null
        var input = text
        var radix = 10
        if (input.length >= 2 && input[0] == '0' && (input[1] == 'x' || input[1] == 'X')) {
            input = input.substring(2)
            radix = 16
        } else if (input.length >= 2 && input[0] == '0') {
            input = input.substring(1)
            radix = 8
        }
        if (input.isEmpty()) return 0
        var value = 0L
        for (c in input) {
            val digit = Character.digit(c, radix)
            if (c.code > 0x7F || digit < 0) return null
            value = minOf(value * radix + digit, 1L shl 40)
        }
        return value
    }

    private fun parseIpv4(host: String): Long? {
        val parts = host.split('.').toMutableList()
        if (parts.last().isEmpty() && parts.size > 1) parts.removeAt(parts.size - 1)
        if (parts.size > 4) return null
        val numbers = parts.map { parseIpv4Number(it) ?: return null }
        if (numbers.dropLast(1).any { it > 255 }) return null
        var limit = 1L
        repeat(5 - numbers.size) { limit *= 256 }
        if (numbers.last() >= limit) return null
        var ipv4 = numbers.last()
        numbers.dropLast(1).forEachIndexed { index, n ->
            var factor = 1L
            repeat(3 - index) { factor *= 256 }
            ipv4 += n * factor
        }
        return ipv4
    }

    private fun serializeIpv4(address: Long): String =
        listOf(24, 16, 8, 0).joinToString(".") { ((address shr it) and 0xFF).toString() }

    // --- IPv6 ---

    private fun parseIpv6(input: String): IntArray? {
        val address = IntArray(8)
        var pieceIndex = 0
        var compress = -1
        var p = 0
        fun at(i: Int): Char = if (i < input.length) input[i] else '\u0000'
        fun isHex(c: Char) = c in '0'..'9' || c in 'a'..'f' || c in 'A'..'F'
        if (at(p) == ':') {
            if (at(p + 1) != ':') return null
            p += 2
            pieceIndex++
            compress = pieceIndex
        }
        while (p < input.length) {
            if (pieceIndex == 8) return null
            if (at(p) == ':') {
                if (compress != -1) return null
                p++
                pieceIndex++
                compress = pieceIndex
                continue
            }
            var value = 0
            var length = 0
            while (length < 4 && p < input.length && isHex(at(p))) {
                value = value * 16 + Character.digit(at(p), 16)
                p++
                length++
            }
            if (at(p) == '.') {
                if (length == 0) return null
                p -= length
                if (pieceIndex > 6) return null
                var numbersSeen = 0
                while (p < input.length) {
                    var ipv4Piece = -1
                    if (numbersSeen > 0) {
                        if (at(p) == '.' && numbersSeen < 4) p++ else return null
                    }
                    if (p >= input.length || at(p) !in '0'..'9') return null
                    while (p < input.length && at(p) in '0'..'9') {
                        val number = at(p) - '0'
                        ipv4Piece = when (ipv4Piece) {
                            -1 -> number
                            0 -> return null
                            else -> ipv4Piece * 10 + number
                        }
                        if (ipv4Piece > 255) return null
                        p++
                    }
                    address[pieceIndex] = address[pieceIndex] * 0x100 + ipv4Piece
                    numbersSeen++
                    if (numbersSeen == 2 || numbersSeen == 4) pieceIndex++
                }
                if (numbersSeen != 4) return null
                break
            } else if (at(p) == ':') {
                p++
                if (p >= input.length) return null
            } else if (p < input.length) {
                return null
            }
            address[pieceIndex] = value
            pieceIndex++
        }
        if (compress != -1) {
            var swaps = pieceIndex - compress
            pieceIndex = 7
            while (pieceIndex != 0 && swaps > 0) {
                val other = compress + swaps - 1
                val tmp = address[pieceIndex]
                address[pieceIndex] = address[other]
                address[other] = tmp
                pieceIndex--
                swaps--
            }
        } else if (pieceIndex != 8) {
            return null
        }
        return address
    }

    private fun serializeIpv6(address: IntArray): String {
        // The first longest run of zero pieces (at least two) becomes `::`.
        var compress = -1
        var bestLength = 1
        var i = 0
        while (i < 8) {
            if (address[i] != 0) { i++; continue }
            var j = i
            while (j < 8 && address[j] == 0) j++
            if (j - i > bestLength) { compress = i; bestLength = j - i }
            i = j
        }
        val out = StringBuilder()
        var ignoreZeros = false
        for (index in 0 until 8) {
            if (ignoreZeros && address[index] == 0) continue
            ignoreZeros = false
            if (compress == index) {
                out.append(if (index == 0) "::" else ":")
                ignoreZeros = true
                continue
            }
            out.append(Integer.toHexString(address[index]))
            if (index != 7) out.append(':')
        }
        return out.toString()
    }
}
