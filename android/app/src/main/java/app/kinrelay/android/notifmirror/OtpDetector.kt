package app.kinrelay.android.notifmirror

import java.util.Locale

/**
 * Finds a one-time code in a notification (design 4.10). It runs on the phone only: the PC gets the result as `code` and shows "Copy <code>".
 * A wrong code is worse than none, so every doubt ends in "no code". Pure; the shared vectors (`otpDetect[]`) and the unit tests pin it.
 *
 * **Gate 1:** at least one keyword of [OtpKeywords] must be in the text. **Candidates:** 4 to 8 digits (or `NNN-NNN`, `NNNN NNNN`), or 5 to 8
 * upper-case letters and digits with both. **Choosing:** one candidate is the answer; several only when the nearest to a keyword is at least
 * [MARGIN] characters closer than the next.
 */
object OtpDetector {
    private const val MAX_CHARS = 400
    private const val PROMO_WINDOW = 30
    private const val MARGIN = 10

    private class Candidate(val value: String, val start: Int, val end: Int, val isYear: Boolean)

    private val RUN = Regex("[0-9]+(?:[ \\-][0-9]+)*")
    private val ALPHANUMERIC = Regex("(?<![A-Za-z0-9])[A-Z0-9]{5,8}(?![A-Za-z0-9])")
    private val UNIT_AFTER = Regex(
        "^ ?(?:usd|eur|gbp|jpy|cny|inr|rub|krw|chf|cad|aud|mxn|brl|pln|sek|nok|dkk|km|kg|min|mins|h|hr|hrs|m|cm|mm|mi|lb|lbs|mph|kph)(?![\\p{L}\\p{N}])",
        RegexOption.IGNORE_CASE,
    )
    private val CURRENCY_BEFORE = "$€£¥₹₽₩+("
    private val KEYWORDS: List<String> = OtpKeywords.ALL.map { it.lowercase(Locale.ROOT) }
    private val PROMO_WORDS = Regex("(?<![\\p{L}\\p{N}])(?:${OtpKeywords.PROMO.joinToString("|")})(?![\\p{L}\\p{N}])")

    fun detect(title: String?, text: String?, bigText: String? = null, hasContactSender: Boolean = false): String? {
        if (hasContactSender) return null
        val joined = listOfNotNull(title, text, bigText).filter { it.isNotEmpty() }.joinToString("\n")
        if (joined.isEmpty() || joined.length > MAX_CHARS) return null
        val folded = foldDigits(joined)
        val lower = CharArray(folded.length) { folded[it].lowercaseChar() }.concatToString()

        val keywords = keywordSpans(lower)
        if (keywords.isEmpty()) return null

        val candidates = (numericCandidates(folded) + alphanumericCandidates(folded))
            .filter { !isPromo(it, lower) }
        if (candidates.isEmpty()) return null
        // A year is a code only when nothing else is there.
        val usable = candidates.filter { !it.isYear }.ifEmpty { candidates }
        // The same value twice (the title and the text, or the text and the big text) is one candidate: the nearest copy counts.
        val byValue = LinkedHashMap<String, Int>()
        for (c in usable) {
            val distance = distanceToKeyword(c, keywords)
            byValue[c.value] = minOf(byValue[c.value] ?: Int.MAX_VALUE, distance)
        }
        if (byValue.size == 1) return byValue.keys.first()
        val ranked = byValue.entries.sortedBy { it.value }
        // "At least 10 characters closer" is read as more than 10: the spec's own example, "Verification code 123456 or 654321" (the second is exactly 10 farther), must give no code.
        return if (ranked[1].value - ranked[0].value > MARGIN) ranked[0].key else null
    }

    /** Unicode decimal digits (Arabic-Indic, Devanagari, full-width ...) become ASCII `0-9`. */
    fun foldDigits(input: String): String {
        val out = StringBuilder(input.length)
        var i = 0
        while (i < input.length) {
            val cp = input.codePointAt(i)
            i += Character.charCount(cp)
            if (cp > 0x7F && Character.getType(cp) == Character.DECIMAL_DIGIT_NUMBER.toInt()) {
                out.append(('0' + Character.digit(cp, 10)))
            } else {
                out.appendCodePoint(cp)
            }
        }
        return out.toString()
    }

    private fun isWordChar(c: Char): Boolean = Character.isLetterOrDigit(c) || Character.getType(c).let {
        it == Character.NON_SPACING_MARK.toInt() || it == Character.COMBINING_SPACING_MARK.toInt() || it == Character.ENCLOSING_MARK.toInt()
    }

    /** Scripts that are written without spaces between words: their keywords match inside a run of text. */
    private fun matchesAsSubstring(keyword: String): Boolean = keyword !in OtpKeywords.ALWAYS_WHOLE_WORD && keyword.any {
        val script = Character.UnicodeScript.of(it.code)
        script == Character.UnicodeScript.HAN || script == Character.UnicodeScript.HIRAGANA || script == Character.UnicodeScript.KATAKANA ||
            script == Character.UnicodeScript.HANGUL || script == Character.UnicodeScript.THAI
    }

    private fun keywordSpans(lower: String): List<IntRange> {
        val spans = ArrayList<IntRange>()
        for (keyword in KEYWORDS) {
            val substring = matchesAsSubstring(keyword)
            var from = 0
            while (true) {
                val at = lower.indexOf(keyword, from)
                if (at < 0) break
                val end = at + keyword.length
                val bounded = substring || ((at == 0 || !isWordChar(lower[at - 1])) && (end == lower.length || !isWordChar(lower[end])))
                if (bounded) spans += at until end
                from = at + 1
            }
        }
        return spans
    }

    private fun numericCandidates(text: String): List<Candidate> {
        val out = ArrayList<Candidate>()
        for (run in RUN.findAll(text)) {
            val groups = ArrayList<Triple<String, Int, Int>>()
            var at = run.range.first
            for (part in run.value.split(' ', '-')) {
                groups += Triple(part, at, at + part.length)
                at += part.length + 1
            }
            when {
                groups.size >= 3 -> Unit // "1234 5678 9012", a date, a phone number: not a code
                groups.size == 2 && ((groups[0].first.length == 3 && groups[1].first.length == 3) || (groups[0].first.length == 4 && groups[1].first.length == 4)) ->
                    addNumeric(out, text, groups[0].first + groups[1].first, groups[0].second, groups[1].third, run.value)
                else -> groups.forEach { (digits, start, end) -> if (digits.length in 4..8) addNumeric(out, text, digits, start, end, digits) }
            }
        }
        return out
    }

    private fun addNumeric(out: MutableList<Candidate>, text: String, value: String, start: Int, end: Int, @Suppress("UNUSED_PARAMETER") shown: String) {
        if (!standsAlone(text, start, end)) return
        out += Candidate(value, start, end, isYear = value.length == 4 && value.toInt() in 1900..2099)
    }

    /** The candidate is not a piece of a bigger number, an amount, a phone number, a percentage or a measurement. */
    private fun standsAlone(text: String, start: Int, end: Int): Boolean {
        if (start > 0) {
            val before = text[start - 1]
            if (touches(before) || before in CURRENCY_BEFORE) return false
            if (before in ".,:/-" && start >= 2 && text[start - 2].isAsciiDigit()) return false
        }
        if (end < text.length) {
            val after = text[end]
            if (touches(after) || after == '%') return false
            if (after in ".,:/" && end + 1 < text.length && text[end + 1].isAsciiDigit()) return false
        }
        if (UNIT_AFTER.containsMatchIn(text.substring(end, minOf(text.length, end + 8)))) return false
        return true
    }

    private fun Char.isAsciiDigit() = this in '0'..'9'

    /**
     * A character right next to the candidate that makes it part of a word or a bigger number: a digit, or a letter. Not the letters of scripts that are written
     * without spaces (Chinese, Japanese, Korean, Thai): `验证码482913，请勿泄露` has the code right against the text, and that is normal there.
     */
    private fun touches(c: Char): Boolean {
        if (c.isDigit()) return true
        if (!Character.isLetter(c)) return false
        return !unspaced(c)
    }

    private fun unspaced(c: Char): Boolean = Character.UnicodeScript.of(c.code).let {
        it == Character.UnicodeScript.HAN || it == Character.UnicodeScript.HIRAGANA || it == Character.UnicodeScript.KATAKANA ||
            it == Character.UnicodeScript.HANGUL || it == Character.UnicodeScript.THAI
    }

    private fun alphanumericCandidates(text: String): List<Candidate> =
        ALPHANUMERIC.findAll(text).filter { match ->
            val token = match.value
            val start = match.range.first
            val end = match.range.last + 1
            token.any { it in 'A'..'Z' } && token.any { it in '0'..'9' } && !insideUrl(text, start) &&
                (start == 0 || !touches(text[start - 1])) && (end == text.length || !touches(text[end]))
        }.map { Candidate(it.value, it.range.first, it.range.last + 1, isYear = false) }.toList()

    /** The whitespace-delimited word that holds [index] is a web address: its path is not a code. */
    private fun insideUrl(text: String, index: Int): Boolean {
        var from = index
        while (from > 0 && !text[from - 1].isWhitespace()) from--
        var to = index
        while (to < text.length && !text[to].isWhitespace()) to++
        val word = text.substring(from, to).lowercase(Locale.ROOT)
        return "://" in word || word.startsWith("www.")
    }

    /** A coupon, not a code: the token itself or a word within [PROMO_WINDOW] characters says save, off, discount, promo, coupon or sale. */
    private fun isPromo(c: Candidate, lower: String): Boolean {
        val own = lower.substring(c.start, c.end)
        if (OtpKeywords.PROMO.any { it in own }) return true
        val before = lower.substring(maxOf(0, c.start - PROMO_WINDOW), c.start)
        val after = lower.substring(c.end, minOf(lower.length, c.end + PROMO_WINDOW))
        return PROMO_WORDS.containsMatchIn(before) || PROMO_WORDS.containsMatchIn(after)
    }

    private fun distanceToKeyword(c: Candidate, keywords: List<IntRange>): Int = keywords.minOf { k ->
        when {
            k.last < c.start -> c.start - k.last - 1
            k.first >= c.end -> k.first - c.end
            else -> 0
        }
    }
}
