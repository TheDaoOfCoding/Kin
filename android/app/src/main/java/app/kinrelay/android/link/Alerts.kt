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

    /**
     * A send didn't go out yet and was saved in the queue: it goes out by itself when the PC is back. [afterNoReply]: it was sent, but the
     * PC didn't ack in time (it may or may not have arrived; it is sent again with the same ID, which the PC recognises).
     */
    fun queued(pcName: String?, reason: String, afterNoReply: Boolean)

    /** A text that had been reported as queued has gone out now. */
    fun sentLater(pcName: String?)

    /** A send didn't work and won't be tried again: a notification, because the user may have left the screen already. */
    fun sendFailed(title: String, reason: String)

    /** A short message that needs no follow-up (a toast). */
    fun message(text: String)
}
