package app.kinrelay.android.log

/** Masks secrets that must never reach a log line: pairing tokens and private key material. */
object Redactor {
    private val rules = listOf(
        Regex("""(?i)([?&#]|\b)(tk|token)=[^&\s"]+""") to "$1$2=[redacted]",
        Regex("""(?i)"(token|privateKey|private_key)"\s*:\s*"[^"]*"""") to "\"$1\":\"[redacted]\"",
        Regex("""-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----""") to "[redacted private key]",
    )

    fun redact(text: String): String = rules.fold(text) { acc, (regex, replacement) -> regex.replace(acc, replacement) }
}
