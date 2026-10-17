package app.kinrelay.android.notifmirror

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Random

/** `NotifSnapshot` to the wire (design 3.3 to 3.8, 4.5, 4.7, 4.9, 4.10): the limits, the cleaning, the order of messages, the buttons, the code, the flags. */
class NotifEncoderTest {
    private fun snap(
        title: String? = "Alice", text: String? = "Hi", bigText: String? = null, subText: String? = null, importance: Int = 3,
        onlyAlertOnce: Boolean = false, matchesInterruptionFilter: Boolean = true, messages: List<MessageSnap> = emptyList(),
        lines: List<String> = emptyList(), actions: List<ActionSnap> = emptyList(), conversationTitle: String? = null,
        group: Boolean = false, redacted: Boolean = false, contact: Boolean = false,
    ) = NotifSnapshot(
        key = "0|com.chat|1|null|10100", packageName = "com.chat", onlyAlertOnce = onlyAlertOnce, importance = importance,
        matchesInterruptionFilter = matchesInterruptionFilter, title = title, text = text, bigText = bigText, subText = subText,
        conversationTitle = conversationTitle, isGroupConversation = group, messages = messages, lines = lines, personHasContactUri = contact,
        actions = actions, redacted = redacted,
    )

    private fun encode(n: NotifSnapshot, sensitive: Boolean = false, isUpdate: Boolean = false, appLabel: String = "Chat", iconHash: String? = null) =
        NotifEncoder.encode(n, appLabel, sensitive, isUpdate, iconHash)

    private fun reply(index: Int, label: String? = "Reply", semantic: Int = ActionSnap.SEMANTIC_NONE, isActivity: Boolean? = false, freeForm: Boolean = true) =
        ActionSnap(index, "Reply", semantic, listOf(RemoteInputSnap("k$index", label, freeForm)), isActivity = isActivity)

    private fun button(index: Int, title: String? = "Mark as read", isActivity: Boolean? = false, hasIntent: Boolean = true, contextual: Boolean = false) =
        ActionSnap(index, title, hasIntent = hasIntent, isActivity = isActivity, contextual = contextual)

    @Test
    fun anOrdinaryNotificationKeepsItsTextAndHasTheNidOfItsKey() {
        val post = encode(snap(subText = "3 new"), iconHash = "a".repeat(32))
        assertEquals(Nid.of("0|com.chat|1|null|10100"), post.nid)
        assertEquals("Chat", post.app)
        assertEquals("Alice", post.title)
        assertEquals("Hi", post.text)
        assertEquals("3 new", post.subText)
        assertEquals("a".repeat(32), post.iconHash)
        assertFalse(post.silent)
        assertEquals(3, post.importance)
    }

    @Test
    fun stringsAreCleanedAndCutToTheLimits() {
        val x = "x".repeat(2000)
        val post = encode(snap(title = "Bad\u202ETitle$x", text = x, bigText = "$x!", subText = x), appLabel = x)
        assertEquals(40, post.app.length)
        assertTrue(post.title.orEmpty().startsWith("Bad_Title"))
        assertEquals(150, post.title?.length)
        assertEquals(500, post.text!!.length)
        assertEquals(1000, post.bigText!!.length)
        assertEquals(80, post.subText!!.length)
    }

    @Test
    fun anEmojiAtTheCutIsNotSplit() {
        val emoji = "😀"
        val post = encode(snap(text = "a".repeat(499) + emoji))
        assertEquals(500, post.text?.let { it.codePointCount(0, it.length) })
        assertTrue(post.text.orEmpty().endsWith(emoji))
    }

    @Test
    fun theBigTextIsSentOnlyWhenItAddsSomething() {
        assertNull(encode(snap(text = "Same", bigText = "Same")).bigText)
        assertEquals("Same and more", encode(snap(text = "Same", bigText = "Same and more")).bigText)
    }

    @Test
    fun messagingStyleKeepsTheLastFiveOldestFirstWithOwnMessagesAsYou() {
        val messages = (1..8).map { MessageSnap(if (it % 2 == 0) null else "Bob", "m$it", it * 1000L) }.shuffled(Random(7))
        val post = encode(snap(messages = messages, conversationTitle = "Family", group = true, title = null, text = null))
        assertEquals(listOf("m4", "m5", "m6", "m7", "m8"), post.messages.map { it.text })
        assertEquals(listOf(null, "Bob", null, "Bob", null), post.messages.map { it.sender })
        assertEquals("Family", post.conversation)
        assertTrue(post.group)
        assertEquals("Family", post.title)
        assertNull("the text and the big text are what messages are for", post.text)
    }

    @Test
    fun aOneToOneConversationIsTitledByTheLastSender() {
        val post = encode(snap(title = "ignored", messages = listOf(MessageSnap("Alice", "Hi", 1), MessageSnap(null, "Hello", 2), MessageSnap("Alice", "Ok", 3))))
        assertEquals("Alice", post.title)
        assertNull(post.conversation)
        assertFalse(post.group)
    }

    @Test
    fun aMediaOnlyMessageBecomesAPlaceholderAndAnEmptyOneIsNotSent() {
        val post = encode(
            snap(
                messages = listOf(
                    MessageSnap("Alice", null, 1, "image/jpeg"), MessageSnap("Alice", "", 2, "application/pdf"),
                    MessageSnap("Alice", null, 3), MessageSnap("Alice", "Hi", 4),
                ),
            ),
        )
        assertEquals(listOf("[Photo]", "[Attachment]", "Hi"), post.messages.map { it.text })
    }

    @Test
    fun inboxLinesKeepTheLastFive() {
        val post = encode(snap(lines = (1..8).map { "l$it" }))
        assertEquals(listOf("l4", "l5", "l6", "l7", "l8"), post.lines)
    }

    @Test
    fun silentIsSetForLowImportanceAnUpdateOfAnAlertOnceNotificationAndDoNotDisturb() {
        assertTrue(encode(snap(importance = 2)).silent)
        assertFalse(encode(snap(importance = 4)).silent)
        assertEquals(4, encode(snap(importance = 4)).importance)
        assertTrue(encode(snap(onlyAlertOnce = true), isUpdate = true).silent)
        assertFalse("the first post of an alert-once notification still alerts", encode(snap(onlyAlertOnce = true)).silent)
        assertFalse("an update of an ordinary notification still alerts", encode(snap(), isUpdate = true).silent)
        assertTrue(encode(snap(matchesInterruptionFilter = false)).silent)
        assertEquals("importance is held to 2..5", 5, encode(snap(importance = 9)).importance)
    }

    @Test
    fun aSensitiveNotificationSendsOnlyTheAppName() {
        val secret = "my secret title and 482913"
        val post = encode(
            snap(
                title = secret, text = "Your verification code is 482913", bigText = secret, messages = listOf(MessageSnap("Bob", secret, 1)), lines = listOf(secret),
                actions = listOf(ActionSnap(0, "Reply", remoteInputs = listOf(RemoteInputSnap("k", "Reply", false, false))), ActionSnap(1, "Mark as read", isActivity = false)),
            ),
            sensitive = true,
        )
        assertTrue(post.sensitive)
        assertEquals("the app name is the only text, so the post is not empty for the receiver", "Chat", post.title)
        assertNull(post.text)
        assertNull(post.bigText)
        assertNull(post.subText)
        assertNull(post.code)
        assertNull(post.reply)
        assertTrue(post.messages.isEmpty() && post.lines.isEmpty() && post.actions.isEmpty())
        val frame = NotifWire.post("x", post)
        assertFalse(frame.contains("secret"))
        assertFalse(frame.contains("482913"))
        assertFalse(frame.contains("Reply"))
    }

    @Test
    fun aRedactedNotificationHasNoContentAndNoCode() {
        val withReply = listOf(ActionSnap(0, "Reply", remoteInputs = listOf(RemoteInputSnap("k"))))
        val post = encode(snap(title = "Messages", text = "Sensitive notification content hidden", actions = withReply, redacted = true))
        assertTrue(post.redacted)
        assertEquals("Messages", post.title)
        assertNull(post.text)
        assertNull(post.code)
        assertNull(post.reply)
        assertTrue(post.actions.isEmpty())
        assertNull(encode(snap(title = null, text = null, redacted = true)).title)
    }

    @Test
    fun aOneTimeCodeIsSentWhenTheDetectorFindsOne() {
        assertEquals("482913", encode(snap(title = "Example", text = "Your verification code is 482913")).code)
        assertNull(encode(snap(title = "Example", text = "Your order #482913 has shipped")).code)
        assertNull("a friend's message", encode(snap(title = "a friend's message", text = "what's the code 482913?", contact = true)).code)
        assertEquals(
            "a code in the last message of a conversation", "482913",
            encode(snap(title = null, text = null, messages = listOf(MessageSnap("Bank", "Your verification code is 482913", 1)))).code,
        )
    }

    @Test
    fun theFrameStaysUnder16KiBByDroppingTheBigTextThenLinesThenMessages() {
        val huge = "x".repeat(NotifWire.BIG_TEXT_MAX)
        val post = encode(snap(text = huge, bigText = huge + "!", lines = List(5) { "y".repeat(150) }))
        assertTrue(post.frameSize() <= NotifWire.SENDER_FRAME_TARGET)
        val messages = List(5) { MessageSnap("s$it", "z".repeat(300), it.toLong()) }
        val lines = List(5) { "y".repeat(150) }
        val full = WirePost("a".repeat(32), "App", title = "t", bigText = huge, messages = messages.map { WireMessage(it.sender, it.text!!) }, lines = lines)
        // The order of the drops, one stage at a time: a limit that the post fits once the big text is gone leaves the lines and the messages alone ...
        val withoutBigText = full.copy(bigText = null)
        full.fit(withoutBigText.frameSize()).let {
            assertNull("the big text goes first", it.bigText)
            assertEquals(5, it.lines.size)
            assertEquals(5, it.messages.size)
        }
        // ... a limit that needs the inbox lines too drops them (the oldest first, one stays) and still leaves every message ...
        val withOneLine = withoutBigText.copy(lines = lines.takeLast(1))
        full.fit(withOneLine.frameSize()).let {
            assertNull(it.bigText)
            assertEquals("the lines go second", 1, it.lines.size)
            assertEquals(5, it.messages.size)
        }
        // ... and only a tighter one drops the oldest messages (the newest stays).
        val withOneMessage = withOneLine.copy(messages = full.messages.takeLast(1))
        full.fit(withOneMessage.frameSize()).let {
            assertNull(it.bigText)
            assertEquals(1, it.lines.size)
            assertEquals("the messages go third", withOneMessage.messages, it.messages)
        }
        // The body text is cut last, and only once nothing else is left to drop.
        val withText = full.copy(text = "t".repeat(400))
        withText.fit(withOneMessage.copy(text = "t".repeat(400)).frameSize() - 1).let {
            assertEquals(1, it.messages.size)
            assertTrue(it.text!!.length < 400)
        }
    }

    @Test
    fun fitNeverCutsThroughASurrogatePair() {
        // 101 emoji are 202 chars; half is 101, which ends between the two halves of a pair.
        val text = "😀".repeat(101)
        val fitted = WirePost("a".repeat(32), "App", title = "t", text = text).fit(150)
        val cut = fitted.text!!
        assertFalse(cut.last().isHighSurrogate())
        assertTrue(cut.length < text.length)
    }

    @Test
    fun theFrameIsValidJsonWithOnlyThePresentFields() {
        val frame = Json.parseToJsonElement(NotifWire.post("id1", encode(snap()))).jsonObject
        assertEquals(setOf("type", "id", "nid", "app", "title", "text"), frame.keys)
        assertEquals("notif.post", frame.getValue("type").toString().trim('"'))
    }

    @Test
    fun theReplyIsTheActionWithAFreeFormRemoteInputAndTheReplySemanticWins() {
        val picked = NotifEncoder.pickActions(listOf(button(0), reply(1, "First"), reply(2, "Second", semantic = ActionSnap.SEMANTIC_REPLY)))
        assertEquals(WireReply(2, "Second"), picked.reply)
        assertEquals(listOf(WireAction(0, "Mark as read")), picked.actions)
        assertEquals(WireReply(1, "First"), NotifEncoder.pickActions(listOf(button(0), reply(1, "First"))).reply)
    }

    @Test
    fun anActionThatOpensAnActivityIsNotOfferedOnApi31AndLater() {
        val picked = NotifEncoder.pickActions(listOf(button(0, "Open", isActivity = true), button(1, "Mark as read", isActivity = false), button(2, "Mute", isActivity = null)))
        assertEquals(listOf(WireAction(1, "Mark as read"), WireAction(2, "Mute")), picked.actions)
        assertNull("a reply that opens an activity can't be fired either", NotifEncoder.pickActions(listOf(reply(0, isActivity = true))).reply)
    }

    @Test
    fun atMostThreeButtonsAndEveryOneHasATitleAndAnIntentAndNoRemoteInput() {
        val picked = NotifEncoder.pickActions(
            listOf(
                button(0, null), button(1, ""), button(2, hasIntent = false), button(3, contextual = true),
                ActionSnap(4, "Choose", remoteInputs = listOf(RemoteInputSnap("k", null, false, true))),
                button(5, "A"), button(6, "B"), button(7, "C"), button(8, "D"),
            ),
        )
        assertEquals(listOf(WireAction(5, "A"), WireAction(6, "B"), WireAction(7, "C")), picked.actions)
        assertNull(picked.reply)
    }

    @Test
    fun anActionPastIndex31IsNotOfferedAndLabelsAreCut() {
        val picked = NotifEncoder.pickActions(listOf(button(32, "Too far"), button(31, "L".repeat(50))))
        assertEquals(listOf(WireAction(31, "L".repeat(30))), picked.actions)
    }

    @Test
    fun hintsAndButtonLabelsAreInToastFormAndAnEmptyOneIsNotOffered() {
        val picked = NotifEncoder.pickActions(listOf(reply(0, "To\u2028Alice"), button(1, "\u2028\u2029"), button(2, "Mu\u2029te\uFFFF")))
        assertEquals("To Alice", picked.reply?.hint)
        assertEquals(listOf(WireAction(2, "Mu te")), picked.actions)
        assertNull("a hint of only separators is no hint", NotifEncoder.pickActions(listOf(reply(0, "\u2028\u2029"))).reply?.hint)
    }

    @Test
    fun theReplyHintIsTheCleanedRemoteInputLabel() {
        assertEquals("Reply to _Family", NotifEncoder.pickActions(listOf(reply(0, "Reply to \u202EFamily"))).reply?.hint)
        assertNull(NotifEncoder.pickActions(listOf(reply(0, null))).reply?.hint)
        assertNull("a remote input with choices only is not a reply", NotifEncoder.pickActions(listOf(reply(0, freeForm = false))).reply)
    }
}
