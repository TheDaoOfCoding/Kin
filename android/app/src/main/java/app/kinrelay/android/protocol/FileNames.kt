package app.kinrelay.android.protocol

/**
 * File name cleaning of protocol.md, "File names": exactly its six steps in order. "Characters" are code points;
 * the 200 and 16 limits are UTF-8 bytes, and a cut falls only on a code point boundary. The result is a *request*
 * on Android: MediaStore may still pick another name, and the app then keeps the one MediaStore chose.
 */
object FileNames {
    const val MAX_NAME_BYTES = 200
    const val MAX_EXTENSION_BYTES = 16
    private const val FALLBACK = "file"

    private val RESERVED = buildSet {
        addAll(listOf("CON", "PRN", "AUX", "NUL"))
        for (n in 0..9) { add("COM$n"); add("LPT$n") }
        for (code in intArrayOf(0xB9, 0xB2, 0xB3)) { add("COM${code.toChar()}"); add("LPT${code.toChar()}") }
    }

    /** `Bidi_Control` characters, which would let a name read differently from what it is (U+202E in `photo<RLO>gpj.exe`). */
    private fun isBidiControl(c: Char): Boolean =
        c.code.let { it == 0x061C || it == 0x200E || it == 0x200F || it in 0x202A..0x202E || it in 0x2066..0x2069 }

    private fun isForbidden(c: Char): Boolean =
        c in "<>:\"/\\|?*" || c.code <= 0x1F || c.code in 0x7F..0x9F || isBidiControl(c)

    /** At most 100 candidates: the plain name and `(1)` to `(99)`; when all of those are taken the file fails (`internal`). */
    const val MAX_SUFFIX = 99

    /** The cleaned name for [input], free of every name in [existing] (compared case-insensitively). */
    fun clean(input: String, existing: Collection<String>): String = cleanLimited(input, existing, Int.MAX_VALUE)!!

    /** [clean], but null when the name and every suffix up to [maxSuffix] are taken. */
    private fun cleanLimited(input: String, existing: Collection<String>, maxSuffix: Int): String? {
        // 1. Path: keep what follows the last / or \.
        var name = input.substring(maxOf(input.lastIndexOf('/'), input.lastIndexOf('\\')) + 1)
        // 2. Forbidden characters, control characters and bidi controls become _.
        name = buildString(name.length) { for (c in name) append(if (isForbidden(c)) '_' else c) }
        // 3. Trailing dots and spaces go.
        name = name.trimEnd('.', ' ')
        // 4. Nothing left.
        if (name.isEmpty()) name = FALLBACK
        // 5. Reserved Windows names, with any extension.
        if (isReserved(name)) name = "_$name"
        // 6. Collision and length, together.
        return withSuffixAndLimit(name, existing.mapTo(HashSet()) { it.lowercase() }, maxSuffix)
    }

    /** The part before the first `.`, trailing spaces removed, is a reserved device name (ASCII case-insensitive). */
    private fun isReserved(name: String): Boolean {
        val base = name.substringBefore('.').trimEnd(' ')
        return asciiUpper(base) in RESERVED
    }

    private fun asciiUpper(text: String): String =
        buildString(text.length) { for (c in text) append(if (c in 'a'..'z') (c - 32) else c) }

    private fun withSuffixAndLimit(cleaned: String, takenLowercase: Set<String>, maxSuffix: Int): String? {
        val dot = cleaned.lastIndexOf('.')
        val extCandidate = if (dot > 0) cleaned.substring(dot) else ""
        val ext = if (extCandidate.isNotEmpty() && Utf8.size(extCandidate) <= MAX_EXTENSION_BYTES) extCandidate else ""
        val stem = cleaned.substring(0, cleaned.length - ext.length)
        val extBytes = Utf8.size(ext)
        var n = 0
        while (n <= maxSuffix) {
            val suffix = if (n == 0) "" else " ($n)"
            val room = MAX_NAME_BYTES - Utf8.size(suffix) - extBytes
            var cut = prefixWithin(stem, room)
            if (cut.length != stem.length) {
                // The cut removed something: tidy the cut stem (a, b, c of the spec).
                cut = cut.trimEnd('.', ' ')
                if (cut.isEmpty()) cut = FALLBACK
                if (isReserved(cut)) cut = "_$cut"
            }
            val candidate = cut + suffix + ext
            if (candidate.lowercase() !in takenLowercase) return candidate
            n++
        }
        return null
    }

    /** The longest prefix of whole code points whose UTF-8 size is at most [maxBytes]. */
    private fun prefixWithin(text: String, maxBytes: Int): String {
        if (maxBytes <= 0) return ""
        var bytes = 0
        var i = 0
        while (i < text.length) {
            val cp = text.codePointAt(i)
            val size = when {
                cp < 0x80 -> 1
                cp < 0x800 -> 2
                cp < 0x10000 -> 3
                else -> 4
            }
            if (bytes + size > maxBytes) break
            bytes += size
            i += Character.charCount(cp)
        }
        return text.substring(0, i)
    }

    /** How a create call went (the `createNew[]` vectors). */
    sealed interface Created {
        data class Name(val name: String) : Created
        data object AlreadyExists : Created
        data class Failed(val reason: String) : Created
    }

    sealed interface CreateResult {
        data class Ok(val name: String) : CreateResult

        /** Any create failure that wasn't "already exists" fails the file with `internal`. */
        data class Failed(val reason: String) : CreateResult
    }

    /**
     * Create-new semantics: clean the name, try to create it, and when the create says "already exists" add the name
     * to the taken set and take the next suffix. "Already exists" is never a failure by itself, but the search stops after
     * [MAX_SUFFIX] suffixes: a system that keeps refusing every name must not keep the transfer (and its lock) forever, and the file
     * then fails with `internal`. [create] gets a candidate name.
     */
    fun createNew(input: String, existing: Collection<String>, create: (String) -> Created): CreateResult {
        val taken = existing.toMutableSet()
        while (true) {
            val candidate = cleanLimited(input, taken, MAX_SUFFIX)
                ?: return CreateResult.Failed("every name up to suffix ($MAX_SUFFIX) is taken")
            when (val result = create(candidate)) {
                is Created.Name -> return CreateResult.Ok(result.name)
                Created.AlreadyExists -> taken += candidate
                is Created.Failed -> return CreateResult.Failed(result.reason)
            }
        }
    }
}
