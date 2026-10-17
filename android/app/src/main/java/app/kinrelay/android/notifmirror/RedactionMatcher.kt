package app.kinrelay.android.notifmirror

import java.util.Locale

/**
 * Detects the Android 15+ redaction of one-time codes (design 4.9). There is no documented flag, so the one sign is the system's placeholder
 * text in the body, the big text or a message ([RedactedStrings]). API 34 and below never redact: always false there.
 */
object RedactionMatcher {
    /** The first Android version that hides one-time codes from notification listeners (API 35, Android 15). */
    const val FIRST_REDACTING_SDK = 35

    private val PLACEHOLDERS: Set<String> = RedactedStrings.ALL.map(::fold).toSet()

    /** The phone will hide codes from Kin (`notif.status.otpHidden`). */
    fun codesHidden(sdkInt: Int): Boolean = sdkInt >= FIRST_REDACTING_SDK

    fun isRedacted(sdkInt: Int, texts: List<String?>): Boolean =
        sdkInt >= FIRST_REDACTING_SDK && texts.any { it != null && fold(it) in PLACEHOLDERS }

    fun isPlaceholder(text: String): Boolean = fold(text) in PLACEHOLDERS

    private fun fold(text: String): String = text.trim().lowercase(Locale.ROOT)
}
