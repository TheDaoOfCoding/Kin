package app.kinrelay.android.protocol

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Parses protocol JSON as protocol.md asks: RFC 8259 strictly, nesting at most 64, no unpaired
 * surrogates, duplicate keys resolved last-wins, into a generic tree.
 *
 * kotlinx.serialization alone is not enough: it accepts unquoted literals, `NaN`, leading zeros
 * and any nesting depth. So the raw text is checked by [Validator] first and kotlinx only builds
 * the tree from text that already passed.
 */
object StrictJson {
    const val MAX_DEPTH = 64

    class InvalidJson(message: String) : Exception(message)

    private val json = Json { isLenient = false }

    fun parse(text: String): JsonElement {
        Validator(text).validate()
        val tree = try {
            json.parseToJsonElement(text)
        } catch (e: Exception) {
            throw InvalidJson("rejected by the JSON reader: ${e.message}")
        }
        rejectUnpairedSurrogates(tree)
        return tree
    }

    private fun rejectUnpairedSurrogates(element: JsonElement) {
        when (element) {
            is JsonObject -> element.forEach { (key, value) ->
                checkString(key)
                rejectUnpairedSurrogates(value)
            }
            is JsonArray -> element.forEach(::rejectUnpairedSurrogates)
            is JsonPrimitive -> if (element.isString) checkString(element.content)
        }
    }

    private fun checkString(text: String) {
        var i = 0
        while (i < text.length) {
            val c = text[i]
            if (Character.isHighSurrogate(c)) {
                if (i + 1 >= text.length || !Character.isLowSurrogate(text[i + 1])) throw InvalidJson("unpaired surrogate")
                i++
            } else if (Character.isLowSurrogate(c)) {
                throw InvalidJson("unpaired surrogate")
            }
            i++
        }
    }

    /** A one-pass RFC 8259 checker with a depth counter. It builds nothing. */
    private class Validator(private val text: String) {
        private var pos = 0

        fun validate() {
            skipWhitespace()
            value(0)
            skipWhitespace()
            if (pos != text.length) fail("data after the JSON value")
        }

        private fun value(depth: Int) {
            if (pos >= text.length) fail("unexpected end")
            when (text[pos]) {
                '{' -> container(depth, '}') { objectMember(depth + 1) }
                '[' -> container(depth, ']') { value(depth + 1) }
                '"' -> string()
                't' -> literal("true")
                'f' -> literal("false")
                'n' -> literal("null")
                '-', in '0'..'9' -> number()
                else -> fail("unexpected character at $pos")
            }
        }

        private inline fun container(depth: Int, close: Char, element: () -> Unit) {
            if (depth + 1 > MAX_DEPTH) fail("nesting deeper than $MAX_DEPTH")
            pos++
            skipWhitespace()
            if (peek() == close) { pos++; return }
            while (true) {
                skipWhitespace()
                element()
                skipWhitespace()
                when (peek()) {
                    ',' -> pos++
                    close -> { pos++; return }
                    else -> fail("expected ',' or '$close' at $pos")
                }
            }
        }

        private fun objectMember(depth: Int) {
            if (peek() != '"') fail("object keys must be quoted strings")
            string()
            skipWhitespace()
            if (peek() != ':') fail("expected ':' at $pos")
            pos++
            skipWhitespace()
            value(depth)
        }

        private fun string() {
            pos++ // opening quote
            while (true) {
                if (pos >= text.length) fail("unterminated string")
                val c = text[pos++]
                when {
                    c == '"' -> return
                    c < ' ' -> fail("control character in string")
                    c == '\\' -> {
                        if (pos >= text.length) fail("unterminated escape")
                        when (text[pos++]) {
                            '"', '\\', '/', 'b', 'f', 'n', 'r', 't' -> Unit
                            'u' -> repeat(4) {
                                if (pos >= text.length || !isHex(text[pos])) fail("bad \\u escape")
                                pos++
                            }
                            else -> fail("bad escape")
                        }
                    }
                }
            }
        }

        private fun number() {
            if (peek() == '-') pos++
            when (peek()) {
                '0' -> pos++
                in '1'..'9' -> digits()
                else -> fail("bad number")
            }
            if (peek() == '.') {
                pos++
                if (peek() !in '0'..'9') fail("digits expected after '.'")
                digits()
            }
            if (peek() == 'e' || peek() == 'E') {
                pos++
                if (peek() == '+' || peek() == '-') pos++
                if (peek() !in '0'..'9') fail("digits expected in exponent")
                digits()
            }
        }

        private fun digits() {
            while (peek() in '0'..'9') pos++
        }

        private fun literal(word: String) {
            if (!text.startsWith(word, pos)) fail("bad literal at $pos")
            pos += word.length
        }

        private fun skipWhitespace() {
            while (pos < text.length && (text[pos] == ' ' || text[pos] == '\t' || text[pos] == '\n' || text[pos] == '\r')) pos++
        }

        private fun peek(): Char = if (pos < text.length) text[pos] else '\u0000'

        private fun isHex(c: Char) = c in '0'..'9' || c in 'a'..'f' || c in 'A'..'F'

        private fun fail(message: String): Nothing = throw InvalidJson(message)
    }
}
