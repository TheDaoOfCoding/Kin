package app.kinrelay.android.link

/** What the app tells the user about texts and sends. The real one posts notifications and toasts. */
interface Alerts {
    /**
     * A `text` arrived: "Text from <pc>" with a preview. [text] is null for sensitive text: no preview at all. [openUrl] is set only for
     * a link that is safe to open.
     */
    fun textFrom(pcName: String, messageId: String, text: String?, openUrl: String?)

    /** A send worked. [arrivalMillis] is the time from the tap to the PC's ack. */
    fun sent(pcName: String?, arrivalMillis: Long)

    /** A send didn't work: a notification, because the user may have left the screen already. */
    fun sendFailed(title: String, reason: String)

    /** A short message that needs no follow-up (a toast). */
    fun message(text: String)
}
