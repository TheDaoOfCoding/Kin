package app.kinrelay.android.clipboard

import app.kinrelay.android.protocol.Frames
import java.security.MessageDigest

/**
 * Loop prevention. When the PC sets the phone's clipboard (`clipboard`, or the copy that comes with `text`)
 * the phone remembers the hash of that text. A "send clipboard" of the same text within [windowMillis] is
 * the PC's own text coming back, so it is not sent. Sending the clipboard is manual on the phone, so this
 * only matters for the tile and the notification button, which the user can tap without looking.
 */
class ClipboardGuard(private val nowMillis: () -> Long, private val windowMillis: Long = WINDOW_MILLIS) {
    private var lastHash: String? = null
    private var lastAt = 0L

    @Synchronized
    fun notePeerSet(text: String) {
        lastHash = hash(text)
        lastAt = nowMillis()
    }

    /** True if [text] is what the peer just put on this clipboard. */
    @Synchronized
    fun isEcho(text: String): Boolean = lastHash != null && hash(text) == lastHash && nowMillis() - lastAt in 0..windowMillis

    /** For the debug state dump: what the guard remembers, without the text. */
    @Synchronized
    fun describe(): String = lastHash?.let { "sha=${it.take(8)} ageMs=${nowMillis() - lastAt}" } ?: "none"

    companion object {
        const val WINDOW_MILLIS = 60_000L

        /** SHA-256 of the UTF-8 text as the wire would carry it (unpaired surrogates already replaced), as hex. */
        fun hash(text: String): String =
            MessageDigest.getInstance("SHA-256").digest(Frames.sanitize(text).toByteArray(Charsets.UTF_8))
                .joinToString("") { "%02x".format(it) }

        /** What the log may say about a text: its length and a short hash, never the text. */
        fun fingerprint(text: String): String = "len=${text.codePointCount(0, text.length)} sha=${hash(text).take(8)}"
    }
}
