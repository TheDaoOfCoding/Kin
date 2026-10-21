package app.kinrelay.android.notifmirror

import app.kinrelay.android.notifmirror.NotifWire.cleanText

/** The reply and buttons offered to the PC, chosen from a notification's actions (design 4.7). */
data class PickedActions(val reply: WireReply?, val actions: List<WireAction>)

/**
 * `NotifSnapshot` to the wire fields of `notif.post` (design 3.3 to 3.8, 4.5, 4.7, 4.9, 4.10): the limits, the cleaning, the order of
 * the messages, the media placeholders, the buttons, the code, and the three flags. Pure.
 */
object NotifEncoder {
    /**
     * [appLabel] is the app's name; [sensitive] comes from [MirrorFilter]; [isUpdate] says this `nid` was announced before on this connection
     * (an update of a notification that has `FLAG_ONLY_ALERT_ONCE` is silent); [iconHash] is the hash of the app icon sent with `notif.icon`.
     */
    fun encode(n: NotifSnapshot, appLabel: String, sensitive: Boolean, isUpdate: Boolean, iconHash: String?): WirePost {
        val nid = Nid.of(n.key)
        val app = cleanText(appLabel, NotifWire.APP_MAX) ?: cleanText(n.packageName, NotifWire.APP_MAX) ?: "App"
        val importance = n.importance.coerceIn(2, 5)
        val silent = importance == 2 || (isUpdate && n.onlyAlertOnce) || !n.matchesInterruptionFilter
        val base = WirePost(nid = nid, app = app, iconHash = iconHash, importance = importance, silent = silent)

        // A flagged app or a hidden-on-the-lock-screen notification: only the app name leaves the phone (the PC shows "Private notification").
        // The title is the app name so the post is not "empty" for the receiver (an empty post is `bad_request`); it is no content.
        if (sensitive) return base.copy(title = app, sensitive = true)

        if (n.redacted) return base.copy(title = cleanText(n.title, NotifWire.TITLE_MAX), redacted = true)

        val messages = wireMessages(n)
        val lastOther = messages.lastOrNull { it.sender != null }?.sender
        val conversation = cleanText(n.conversationTitle, NotifWire.CONVERSATION_MAX)
        val title = if (messages.isNotEmpty()) {
            cleanText(n.conversationTitle, NotifWire.TITLE_MAX) ?: lastOther ?: cleanText(n.title, NotifWire.TITLE_MAX)
        } else {
            cleanText(n.title, NotifWire.TITLE_MAX)
        }
        val text = if (messages.isNotEmpty()) null else cleanText(n.text, NotifWire.TEXT_MAX)
        val bigText = if (messages.isNotEmpty()) null else cleanText(n.bigText, NotifWire.BIG_TEXT_MAX)?.takeIf { it != text }
        val lines = if (messages.isNotEmpty()) emptyList() else n.lines.takeLast(NotifWire.MAX_LINES).mapNotNull { cleanText(it, NotifWire.LINE_MAX) }

        val code = OtpDetector.detect(
            n.title, n.text ?: lastMessageText(n), n.bigText, hasContactSender = n.personHasContactUri,
        )?.takeIf(NotifWire::isValidCode)
        val picked = pickActions(n.actions)

        return base.copy(
            title = title,
            text = text,
            bigText = bigText,
            subText = cleanText(n.subText, NotifWire.SUB_TEXT_MAX),
            conversation = conversation.takeIf { messages.isNotEmpty() },
            group = n.isGroupConversation && messages.isNotEmpty(),
            messages = messages,
            lines = lines,
            code = code,
            reply = picked.reply,
            actions = picked.actions,
        ).fit()
    }

    /** MessagingStyle: the last 5 by time, oldest first. Own messages have no sender; a message with no text is "[Photo]" or "[Attachment]", or not sent. */
    fun wireMessages(n: NotifSnapshot): List<WireMessage> =
        n.messages.sortedBy { it.timestamp }.takeLast(NotifWire.MAX_MESSAGES).mapNotNull { m ->
            val text = cleanText(m.text, NotifWire.MESSAGE_TEXT_MAX)
                ?: m.mime?.let { if (it.startsWith("image/")) "[Photo]" else "[Attachment]" }
                ?: return@mapNotNull null
            WireMessage(sender = m.sender?.let { cleanText(it, NotifWire.SENDER_MAX) ?: "Someone" }, text = text)
        }

    private fun lastMessageText(n: NotifSnapshot): String? = n.messages.maxByOrNull { it.timestamp }?.text

    /**
     * The reply and the buttons (design 4.7). **Reply:** the first action with a free-form `RemoteInput` and a `PendingIntent` (the reply semantic wins
     * over an earlier one). **Buttons:** at most 3 others with a title, no remote input at all, a `PendingIntent` that is not known to start an activity
     * (Android blocks an activity started from a listener in the background, silently), and not a contextual action. An index is 0 to 31 (the
     * PC's limit). A reply whose `PendingIntent` is known to start an activity is not offered either: the phone must never claim an action it can't fire.
     */
    fun pickActions(actions: List<ActionSnap>): PickedActions {
        val usable = actions.filter { it.hasIntent && it.index in 0..NotifWire.MAX_ACTION_INDEX && !it.contextual }
        val replyCandidates = usable.filter { a -> a.isActivity != true && a.remoteInputs.any { it.freeForm } }
        val replyAction = replyCandidates.firstOrNull { it.semantic == ActionSnap.SEMANTIC_REPLY } ?: replyCandidates.firstOrNull()
        val reply = replyAction?.let { a ->
            val input = a.remoteInputs.first { it.freeForm }
            WireReply(a.index, NotifWire.cleanLabel(input.label, NotifWire.HINT_MAX))
        }
        val buttons = usable.asSequence()
            .filter { it !== replyAction && it.remoteInputs.isEmpty() && it.isActivity != true }
            .mapNotNull { a -> NotifWire.cleanLabel(a.title, NotifWire.LABEL_MAX)?.let { WireAction(a.index, it) } }
            .take(NotifWire.MAX_ACTIONS)
            .toList()
        return PickedActions(reply, buttons)
    }
}
