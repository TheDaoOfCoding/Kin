package app.kinrelay.android.links

import androidx.core.app.NotificationCompat

/**
 * What a link notification says, as plain values (the Android builder only puts them on screen, so the rules can be tested):
 * - the **title** is the page title; **the real address is always the second line**, in the collapsed and the expanded form, and it is worked
 *   out from the URL ([LinkDomain]), never from the title, so a title can't pass for the address;
 * - the **sub text** says what a tap does ("Open in YouTube", or "Open in browser");
 * - on the **lock screen** the title and the address are hidden unless the user turned "Show link previews on the lock screen" on: the
 *   public version says only that a link came from the PC.
 */
data class LinkNotificationContent(
    val title: String,
    /** The second line: the real address (with the warning mark when the URL is tricky). Always present, always the address. */
    val addressLine: String,
    /** The expanded text: the address on its own first line, then the URL (shortened), then where it came from. */
    val bigText: String,
    val subText: String,
    val publicTitle: String,
    val visibility: Int,
) {
    companion object {
        const val WARNING_MARK = "⚠"
        const val URL_SHOWN_CHARACTERS = 200

        fun of(pcName: String, link: ReceivedLink, openLabel: String, lockScreenPreviews: Boolean): LinkNotificationContent {
            val address = if (link.warn) "$WARNING_MARK ${link.domain}" else link.domain
            val shownUrl = LinkRules.displayUrl(link.url, URL_SHOWN_CHARACTERS)
            val title = link.title ?: address
            return LinkNotificationContent(
                title = title,
                // With no title the title *is* the address; the second line then says where the link came from instead of repeating it.
                addressLine = if (link.title == null) "Link from $pcName" else address,
                bigText = buildString {
                    append(address).append('\n').append(shownUrl).append('\n').append("From ").append(pcName)
                },
                subText = openLabel,
                publicTitle = "Link from $pcName",
                visibility = if (lockScreenPreviews) NotificationCompat.VISIBILITY_PUBLIC else NotificationCompat.VISIBILITY_PRIVATE,
            )
        }

        /** With two or more link notifications up a summary stacks them; with one there is none. */
        fun hasSummary(count: Int) = count >= 2

        /** The title of the group summary and the lines under it (one per link: its address, never its title, which is the page's own words). */
        fun summaryTitle(count: Int, pcName: String) = "$count links from $pcName"
    }
}
