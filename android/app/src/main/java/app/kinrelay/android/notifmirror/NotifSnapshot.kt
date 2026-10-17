package app.kinrelay.android.notifmirror

import java.security.MessageDigest

/**
 * Everything the mirroring rules need to know about one notification, as plain data, so the rules are testable on the JVM with no Android
 * classes (design 2B.2, section 4.1). [SbnMapper] fills it from a `StatusBarNotification` and its `Ranking`; nothing in here is kept after
 * the notification is handled.
 */
data class NotifSnapshot(
    val key: String,
    val packageName: String,
    val channelId: String? = null,
    /** The notification belongs to the current user (not a work profile). */
    val currentUser: Boolean = true,
    val ongoing: Boolean = false,
    val foregroundService: Boolean = false,
    /** `!sbn.isClearable` (covers `FLAG_NO_CLEAR`). */
    val notClearable: Boolean = false,
    val groupSummary: Boolean = false,
    val localOnly: Boolean = false,
    val onlyAlertOnce: Boolean = false,
    val category: String? = null,
    /** A `MediaStyle` notification, or one with `EXTRA_MEDIA_SESSION`. */
    val media: Boolean = false,
    val progressMax: Int = 0,
    val progressIndeterminate: Boolean = false,
    /** `Ranking.getImportance()`, 1 (min) to 5 (max); 3 when unknown. */
    val importance: Int = 3,
    val suspended: Boolean = false,
    /** `Notification.visibility`: 1 public, 0 private, -1 secret. */
    val visibility: Int = VISIBILITY_PRIVATE,
    /** `Ranking.getLockscreenVisibilityOverride()`; [VISIBILITY_NO_OVERRIDE] (-1000) when the user did not choose. */
    val lockscreenOverride: Int = VISIBILITY_NO_OVERRIDE,
    /** `Ranking.matchesInterruptionFilter()`: false when Do Not Disturb blocks this one. */
    val matchesInterruptionFilter: Boolean = true,
    val title: String? = null,
    val text: String? = null,
    val bigText: String? = null,
    val subText: String? = null,
    val conversationTitle: String? = null,
    val isGroupConversation: Boolean = false,
    /** MessagingStyle messages in the order Android keeps them (any order: the encoder sorts by time). */
    val messages: List<MessageSnap> = emptyList(),
    /** InboxStyle lines. */
    val lines: List<String> = emptyList(),
    /** Some sender of the MessagingStyle has a contact URI (a saved contact): a friend's "what's the code 1234?" is not a code. */
    val personHasContactUri: Boolean = false,
    val actions: List<ActionSnap> = emptyList(),
    /** Android replaced the content with its "sensitive notification content hidden" placeholder (API 35+, [RedactionMatcher]). */
    val redacted: Boolean = false,
) {
    /** The visibility that counts: the user's lock-screen choice for this notification wins over the app's. */
    val effectiveVisibility: Int get() = if (lockscreenOverride != VISIBILITY_NO_OVERRIDE) lockscreenOverride else visibility

    val hasContent: Boolean
        get() = !title.isNullOrBlank() || !text.isNullOrBlank() || !bigText.isNullOrBlank() || messages.isNotEmpty() || lines.isNotEmpty()

    companion object {
        const val VISIBILITY_PUBLIC = 1
        const val VISIBILITY_PRIVATE = 0
        const val VISIBILITY_SECRET = -1
        const val VISIBILITY_NO_OVERRIDE = -1000
    }
}

/** One MessagingStyle message. [sender] null is the user's own message. [mime] is the data URI's type (never the data itself). */
data class MessageSnap(val sender: String?, val text: String?, val timestamp: Long = 0L, val mime: String? = null)

/**
 * One `Notification.Action`. [index] is its position in `Notification.actions`: the number the PC sends back. [isActivity] is null when the
 * phone can't tell (API 29 and 30 have no `PendingIntent.isActivity`).
 */
data class ActionSnap(
    val index: Int,
    val title: String?,
    val semantic: Int = SEMANTIC_NONE,
    val remoteInputs: List<RemoteInputSnap> = emptyList(),
    val hasIntent: Boolean = true,
    val isActivity: Boolean? = null,
    /** A contextual (system-generated) action: not the app's own button. */
    val contextual: Boolean = false,
) {
    companion object {
        const val SEMANTIC_NONE = 0
        const val SEMANTIC_REPLY = 1
    }
}

data class RemoteInputSnap(val resultKey: String, val label: String? = null, val freeForm: Boolean = true, val hasChoices: Boolean = false)

/** The `nid` of a notification key: the first 32 hex characters of SHA-256 of its UTF-8. Raw keys never leave the phone. */
object Nid {
    fun of(key: String): String = Hashing.sha256Hex(key.toByteArray(Charsets.UTF_8)).substring(0, 32)

    private val FORMAT = Regex("[0-9a-f]{32}")

    fun isValid(value: String): Boolean = FORMAT.matches(value)
}

object Hashing {
    fun sha256Hex(bytes: ByteArray): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(bytes)
        val out = StringBuilder(digest.size * 2)
        for (b in digest) {
            out.append(HEX[(b.toInt() shr 4) and 0xF]).append(HEX[b.toInt() and 0xF])
        }
        return out.toString()
    }

    private val HEX = "0123456789abcdef".toCharArray()
}
