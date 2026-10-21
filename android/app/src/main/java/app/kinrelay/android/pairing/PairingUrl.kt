package app.kinrelay.android.pairing

import app.kinrelay.android.identity.DeviceIds
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

data class PairingAddress(val host: String, val port: Int) {
    override fun toString() = "$host:$port"
}

/** The result of parsing a pairing URL: `{ version, addresses, id, token, name }`. */
data class PairingTarget(
    val version: Int,
    val addresses: List<PairingAddress>,
    val id: String,
    val token: String,
    val name: String?,
)

enum class PairingErrorCategory { NOT_PAIRING_URL, UNSUPPORTED_VERSION, MISSING_PARAM, INVALID_PARAM, DUPLICATE_PARAM }

sealed interface PairingParse {
    data class Ok(val target: PairingTarget) : PairingParse
    data class Error(val category: PairingErrorCategory, val param: String? = null) : PairingParse
}

/**
 * Parses the text of a scanned QR code or a link, exactly as protocol.md, "Parsing rules".
 * No URL library: they disagree on custom schemes, `+` and repeated parameters.
 */
object PairingUrl {
    private const val MAX_NAME_CODE_POINTS = 64
    private val ASCII_DIGITS = Regex("[0-9]+")

    fun parse(input: String): PairingParse {
        val text = input.trim { it == ' ' || it == '\t' || it == '\r' || it == '\n' }
        val query = parameterText(text) ?: return PairingParse.Error(PairingErrorCategory.NOT_PAIRING_URL)
        val params = split(query)

        fun single(name: String, required: Boolean = true): Pair<String?, PairingParse.Error?> {
            val values = params[name].orEmpty()
            if (values.size > 1) return null to PairingParse.Error(PairingErrorCategory.DUPLICATE_PARAM, name)
            if (values.isEmpty()) {
                return null to if (required) PairingParse.Error(PairingErrorCategory.MISSING_PARAM, name) else null
            }
            val decoded = decode(values[0])
            return decoded to if (decoded == null) PairingParse.Error(PairingErrorCategory.INVALID_PARAM, name) else null
        }

        fun invalid(name: String) = PairingParse.Error(PairingErrorCategory.INVALID_PARAM, name)

        val (v, vError) = single("v")
        vError?.let { return it }
        if (!ASCII_DIGITS.matches(v!!)) return invalid("v")
        if (v != "1") return PairingParse.Error(PairingErrorCategory.UNSUPPORTED_VERSION)

        val (a, aError) = single("a")
        aError?.let { return it }
        val addresses = parseAddresses(a!!) ?: return invalid("a")

        val (id, idError) = single("id")
        idError?.let { return it }
        if (!DeviceIds.isDeviceId(id!!)) return invalid("id")

        val (token, tokenError) = single("tk")
        tokenError?.let { return it }
        if (!DeviceIds.isToken(token!!)) return invalid("tk")

        val (rawName, nameError) = single("n", required = false)
        nameError?.let { return it }
        val name = rawName?.takeIf { it.isNotEmpty() }?.let(::cutName)

        return PairingParse.Ok(PairingTarget(1, addresses, id, token, name))
    }

    /** The raw parameter text for either form, or null if this isn't a pairing URL. */
    private fun parameterText(text: String): String? {
        if (text.startsWith("kinrelay://", ignoreCase = true)) {
            val rest = text.substring("kinrelay://".length)
            val hostEnd = rest.indexOfAny(charArrayOf('/', '?', '#')).let { if (it < 0) rest.length else it }
            if (!rest.substring(0, hostEnd).equals("pair", ignoreCase = true)) return null
            var after = rest.substring(hostEnd)
            if (after.startsWith("/")) after = after.substring(1)
            return when {
                after.isEmpty() || after.startsWith("#") -> ""
                after.startsWith("?") -> after.substring(1).substringBefore('#')
                else -> null
            }
        }
        if (text.startsWith("https://", ignoreCase = true)) {
            val rest = text.substring("https://".length)
            val authorityEnd = rest.indexOfAny(charArrayOf('/', '?', '#')).let { if (it < 0) rest.length else it }
            if (authorityEnd == 0) return null
            val afterAuthority = rest.substring(authorityEnd)
            val path = afterAuthority.substringBefore('?').substringBefore('#')
            if (path != "/pair" && path != "/pair/") return null
            val hash = afterAuthority.indexOf('#')
            return if (hash < 0) "" else afterAuthority.substring(hash + 1)
        }
        return null
    }

    private fun split(query: String): Map<String, List<String>> {
        val params = LinkedHashMap<String, MutableList<String>>()
        for (piece in query.split('&')) {
            if (piece.isEmpty()) continue
            val eq = piece.indexOf('=')
            val name = if (eq < 0) piece else piece.substring(0, eq)
            val value = if (eq < 0) "" else piece.substring(eq + 1)
            params.getOrPut(name) { mutableListOf() }.add(value)
        }
        return params
    }

    /** `%XX` becomes a byte, everything else is itself; the bytes must be valid UTF-8. Null if not. */
    private fun decode(value: String): String? {
        val bytes = ByteArrayOutputStream()
        var i = 0
        while (i < value.length) {
            val c = value[i]
            if (c == '%') {
                if (i + 2 >= value.length) return null
                val hi = Character.digit(value[i + 1], 16)
                val lo = Character.digit(value[i + 2], 16)
                if (hi < 0 || lo < 0 || value[i + 1].code > 127 || value[i + 2].code > 127) return null
                bytes.write(hi * 16 + lo)
                i += 3
            } else {
                var end = i + 1
                if (Character.isHighSurrogate(c) && end < value.length && Character.isLowSurrogate(value[end])) end++
                bytes.write(value.substring(i, end).toByteArray(Charsets.UTF_8))
                i = end
            }
        }
        return try {
            Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes.toByteArray()))
                .toString()
        } catch (_: java.nio.charset.CharacterCodingException) {
            null
        }
    }

    private fun parseAddresses(value: String): List<PairingAddress>? {
        val entries = value.split(',')
        if (entries.size !in 1..3) return null
        return entries.map { parseAddress(it) ?: return null }
    }

    private fun parseAddress(entry: String): PairingAddress? {
        val parts = entry.split(':')
        if (parts.size != 2) return null
        val octets = parts[0].split('.')
        if (octets.size != 4) return null
        val numbers = octets.map { octet ->
            if (!ASCII_DIGITS.matches(octet) || octet.length > 3 || (octet.length > 1 && octet[0] == '0')) return null
            octet.toInt().takeIf { it <= 255 } ?: return null
        }
        if (!isAllowedRange(numbers)) return null
        val portText = parts[1]
        if (!ASCII_DIGITS.matches(portText) || portText.length > 5 || (portText.length > 1 && portText[0] == '0')) return null
        val port = portText.toInt()
        if (port !in 1..65535) return null
        return PairingAddress(parts[0], port)
    }

    /** RFC 1918 plus the shared address space 100.64.0.0/10. */
    private fun isAllowedRange(o: List<Int>): Boolean =
        o[0] == 10 ||
            (o[0] == 172 && o[1] in 16..31) ||
            (o[0] == 192 && o[1] == 168) ||
            (o[0] == 100 && o[1] in 64..127)

    private fun cutName(name: String): String {
        if (name.codePointCount(0, name.length) <= MAX_NAME_CODE_POINTS) return name
        return name.substring(0, name.offsetByCodePoints(0, MAX_NAME_CODE_POINTS))
    }
}
