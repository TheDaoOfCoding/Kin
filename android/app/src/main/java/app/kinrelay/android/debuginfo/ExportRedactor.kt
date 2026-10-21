package app.kinrelay.android.debuginfo

import app.kinrelay.android.log.Redactor

/**
 * What is taken out of the exported logs before they leave the phone (the same rules as the desktop's export, `export.rs`): **IPv4 addresses**
 * (the PC's address on the home network, with its port) become `<ip>`; **the PC's name** (every PC the log mentions) becomes `<pc>` and
 * **this phone's name** becomes `<phone>`; **a full device ID** is cut to its first 8 characters. Tokens and private keys are
 * removed first ([Redactor]); those never reach the log in the first place, this is the second lock.
 *
 * It is applied to the whole text of every exported file. The names come from the database (the paired PC, the phone's own name); a PC
 * from an earlier pairing is found in the log lines that name it (`connected to '<name>'` and so on).
 */
class ExportRedactor(pcNames: Collection<String> = emptyList(), phoneNames: Collection<String> = emptyList()) {
    // Longer names first, so "Pixel 8 Pro" doesn't leave "Pro" behind a replaced "Pixel 8".
    private val pc = names(pcNames)
    private val phone = names(phoneNames)

    fun apply(text: String): String {
        // The echo lines of the debug build's test hooks (`debug: CMD ...`) carry file names and nothing a developer needs from an export; a
        // release build has no such line.
        var out = Redactor.redact(text).lineSequence().filterNot { DEBUG_COMMAND.containsMatchIn(it) }.joinToString("\n")
        out = NAMED_IN_QUOTES.replace(out) { "${it.groupValues[1]} '<pc>'" }
        out = PAIRED_WITH.replace(out, "paired with <pc> (")
        for (name in phone) out = out.replace(name, "<phone>", ignoreCase = true)
        for (name in pc) out = out.replace(name, "<pc>", ignoreCase = true)
        out = IPV4.replace(out, "<ip>")
        out = IPV6_BRACKETED.replace(out, "<ip>")
        out = DEVICE_ID.replace(out) { it.value.take(SHORT_ID_CHARS) + "…" }
        return out
    }

    private fun names(all: Collection<String>): List<String> =
        all.map { it.trim() }.filter { it.codePointCount(0, it.length) >= MIN_NAME_CHARS }.distinct().sortedByDescending { it.length }

    private companion object {
        const val MIN_NAME_CHARS = 2
        const val SHORT_ID_CHARS = 8

        /** A device ID is 43 characters of base64url (the unpadded SHA-256 of the key). */
        val DEVICE_ID = Regex("(?<![A-Za-z0-9_-])[A-Za-z0-9_-]{43}(?![A-Za-z0-9_-])")
        val IPV4 = Regex("""\b(?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?\b""")
        val IPV6_BRACKETED = Regex("""\[[0-9A-Fa-f:.%a-z0-9]*:[0-9A-Fa-f:.%a-z0-9]*](?::\d{1,5})?""")

        /** The lines of `link` and `share` that name a PC between quotes. */
        val NAMED_IN_QUOTES = Regex("""(connected to|text from|ping received from|unpairing from|shortcut for) '[^']*'""")
        val PAIRED_WITH = Regex("""paired with .*? \(""")

        /** A log line written by a debug hook: `<time> <level> debug: CMD ...`. */
        val DEBUG_COMMAND = Regex("""^\S+ [IWE] debug: CMD """)
    }
}
