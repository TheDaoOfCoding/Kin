package app.kinrelay.android.notifmirror

import app.kinrelay.android.link.Clock
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageReceiver
import app.kinrelay.android.protocol.ReceiverRole
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** `notif.action` and `notif.dismiss` from the PC (design 3.7, 4.7, 4.8): the five steps in order, never a different button, never twice. */
class ActionHandlerTest {
    private class Live(val snapshot: NotifSnapshot)

    private class Port(var notifications: List<Live>?) : ActionPort<Live> {
        var snapshots = 0
        var finds = 0
        val fired = ArrayList<Triple<String, Int, String?>>()
        val cancelled = ArrayList<String>()
        var result = FireResult.SENT
        override fun active(): List<Live>? { finds++; return notifications }
        override fun key(sbn: Live) = sbn.snapshot.key
        override fun quick(sbn: Live) = QuickInfo(sbn.snapshot.packageName, sbn.snapshot.channelId, sbn.snapshot.currentUser)
        override fun snapshot(sbn: Live): NotifSnapshot { snapshots++; return sbn.snapshot }
        override fun fire(sbn: Live, index: Int, replyText: String?): FireResult { fired += Triple(sbn.snapshot.key, index, replyText); return result }
        override fun cancel(sbn: Live) { cancelled += sbn.snapshot.key }
    }

    private val key = "0|com.chat|42|null|10100"
    private val nid = Nid.of(key)
    private var now = 0L
    private val policy = MirrorPolicy(enabled = true, allowed = setOf("com.chat", "com.bank"), dismissOnPhone = true)

    private fun chat(
        actions: List<ActionSnap> = listOf(
            ActionSnap(0, "Mark as read", isActivity = false),
            ActionSnap(1, "Reply", remoteInputs = listOf(RemoteInputSnap("k", "Reply to Family")), isActivity = false),
            ActionSnap(2, "Open", isActivity = true),
        ),
        pkg: String = "com.chat", redacted: Boolean = false, notClearable: Boolean = false,
    ) = Live(NotifSnapshot(key = key, packageName = pkg, title = "Family", text = "Dinner?", actions = actions, redacted = redacted, notClearable = notClearable))

    private fun handler(port: Port, p: MirrorPolicy = policy, stats: MirrorStats = MirrorStats(), cancels: MutableList<String> = ArrayList()) =
        ActionHandler(port, { p }, MirrorFilter("app.kinrelay.android", null) { it == "com.bank" }, { null }, Clock { now }, stats) { cancels += it }

    private fun request(index: Int = 0, label: String = "Mark as read", text: String? = null, nid: String = this.nid) = NotifWire.ActionRequest("id-$index", nid, index, label, text)

    private fun error(outcome: ActionOutcome) = (outcome as ActionOutcome.Error).code

    @Test
    fun aButtonIsFiredAndAcked() = runTest {
        val port = Port(listOf(chat()))
        assertEquals(ActionOutcome.Ack, handler(port).onAction(request(0)))
        assertEquals(listOf(Triple(key, 0, null)), port.fired)
    }

    @Test
    fun aReplyIsFiredWithItsTextAndTheLabelIsTheRemoteInputsLabel() = runTest {
        val port = Port(listOf(chat()))
        assertEquals(ActionOutcome.Ack, handler(port).onAction(request(1, "Reply to Family", text = "On my way")))
        assertEquals(listOf(Triple(key, 1, "On my way")), port.fired)
        // The remote input's label, not the action's title.
        assertEquals("stale", error(handler(Port(listOf(chat()))).onAction(request(1, "Reply", text = "x"))))
    }

    @Test
    fun theFiveStepsAnswerInOrder() = runTest {
        assertEquals("not_found", error(handler(Port(listOf(chat()))).onAction(request(0, nid = Nid.of("other")))))
        assertEquals("disabled", error(handler(Port(listOf(chat())), policy.copy(enabled = false)).onAction(request(0))))
        assertEquals("not_found", error(handler(Port(listOf(chat())), policy.copy(allowed = setOf("com.bank"))).onAction(request(0))))
        assertEquals("disabled", error(handler(Port(null)).onAction(request(0))))
        assertEquals("stale", error(handler(Port(listOf(chat()))).onAction(request(0, "Archive"))))
        assertEquals("stale", error(handler(Port(listOf(chat()))).onAction(request(7, "x"))))
        assertEquals("bad_request", error(handler(Port(listOf(chat()))).onAction(request(0, text = "reply to a button"))))
        assertEquals("bad_request", error(handler(Port(listOf(chat()))).onAction(request(1, "Reply"))))
        val cancelled = Port(listOf(chat())).apply { result = FireResult.CANCELED }
        assertEquals("action_failed", error(handler(cancelled).onAction(request(0))))
        val failed = Port(listOf(chat())).apply { result = FireResult.FAILED }
        assertEquals("action_failed", error(handler(failed).onAction(request(0))))
    }

    @Test
    fun anActiveNotificationOfAnAppThatIsNotTickedAnswersLikeAMissingOneAndIsNeverRead() = runTest {
        val port = Port(listOf(chat()))
        val notTicked = handler(port, policy.copy(allowed = setOf("com.bank"))).onAction(request(0))
        val missing = handler(Port(listOf(chat()))).onAction(request(0, nid = Nid.of("other")))
        assertEquals(missing, notTicked)
        assertEquals("not_found", error(notTicked))
        assertEquals(0, port.snapshots)
        assertTrue(port.fired.isEmpty())
    }

    @Test
    fun withMirroringOffNothingIsLookedUpOrRead() = runTest {
        val port = Port(listOf(chat()))
        assertEquals("disabled", error(handler(port, policy.copy(enabled = false)).onAction(request(0))))
        assertEquals(0, port.finds)
        assertEquals(0, port.snapshots)
    }

    @Test
    fun aDismissOfAnAppThatIsNotTickedReadsNothing() = runTest {
        val port = Port(listOf(chat()))
        assertFalse(handler(port, policy.copy(allowed = setOf("com.bank"))).onDismiss(nid))
        assertEquals(0, port.snapshots)
        assertTrue(port.cancelled.isEmpty())
    }

    @Test
    fun aDifferentButtonIsNeverPressedWhenTheAppChangedTheNotification() = runTest {
        // The PC showed "Mark as read" at index 0; the app now has "Delete" there.
        val changed = chat(actions = listOf(ActionSnap(0, "Delete", isActivity = false)))
        val port = Port(listOf(changed))
        assertEquals("stale", error(handler(port).onAction(request(0, "Mark as read"))))
        assertTrue(port.fired.isEmpty())
    }

    @Test
    fun theLabelIsComparedInToastForm() = runTest {
        val live = chat(actions = listOf(
            ActionSnap(0, "Mute ", isActivity = false),
            ActionSnap(1, "Reply", remoteInputs = listOf(RemoteInputSnap("k", " To Alice")), isActivity = false),
            ActionSnap(2, "  ", isActivity = false),
        ))
        val port = Port(listOf(live))
        // The PC sends the toast form ("Mute", "To Alice"); the phone's own label has the separators.
        assertEquals(ActionOutcome.Ack, handler(port).onAction(request(0, "Mute")))
        assertEquals(ActionOutcome.Ack, handler(port).onAction(request(1, "To Alice", text = "hi")))
        // A label that is empty in toast form was never offered: a button for it is stale, whatever the request says.
        assertEquals("stale", error(handler(port).onAction(request(2, ""))))
        assertEquals(2, port.fired.size)
    }

    @Test
    fun anActivityActionIsNeverFiredFromTheBackground() = runTest {
        val port = Port(listOf(chat()))
        assertEquals("action_failed", error(handler(port).onAction(request(2, "Open"))))
        assertTrue(port.fired.isEmpty())
    }

    @Test
    fun aFlaggedOrRedactedNotificationHasNoActions() = runTest {
        assertEquals("disabled", error(handler(Port(listOf(chat(pkg = "com.bank")))).onAction(request(0))))
        assertEquals("disabled", error(handler(Port(listOf(chat(redacted = true)))).onAction(request(0))))
    }

    @Test
    fun theRecomputedNidFindsANotificationAfterARestart() = runTest {
        // keyOf knows nothing (the process was restarted), so every active key is hashed again.
        val port = Port(listOf(Live(NotifSnapshot("0|com.chat|1|null|10100", "com.chat", title = "x")), chat()))
        assertEquals(ActionOutcome.Ack, handler(port).onAction(request(0)))
        assertEquals(key, port.fired.single().first)
    }

    @Test
    fun theKeyFromMemoryIsUsedFirst() = runTest {
        val port = Port(listOf(chat()))
        val h = ActionHandler(port, { policy }, MirrorFilter("x", null), { key }, Clock { now }, MirrorStats())
        assertEquals(ActionOutcome.Ack, h.onAction(request(0)))
    }

    @Test
    fun atMostTenActionsPerTenSeconds() = runTest {
        val port = Port(listOf(chat()))
        val h = handler(port)
        repeat(10) { assertEquals(ActionOutcome.Ack, h.onAction(request(0))) }
        assertEquals("rate_limited", error(h.onAction(request(0))))
        now += 10_000
        assertEquals(ActionOutcome.Ack, h.onAction(request(0)))
    }

    @Test
    fun noErrorMessageHoldsContent() = runTest {
        val port = Port(listOf(chat()))
        val errors = listOf(
            handler(port).onAction(request(0, "Secret label ZZZ")), handler(port).onAction(request(1, "Reply to Family", text = null)),
            handler(Port(listOf(chat())), policy.copy(enabled = false)).onAction(request(0)),
        ).map { (it as ActionOutcome.Error).message }
        for (message in errors) assertFalse(message, message.contains("ZZZ") || message.contains("Family"))
    }

    // --- dismiss ---

    @Test
    fun aDismissCancelsAClearableNotificationWhenTheSettingIsOn() = runTest {
        val port = Port(listOf(chat()))
        val cancels = ArrayList<String>()
        assertTrue(handler(port, cancels = cancels).onDismiss(nid))
        assertEquals(listOf(key), port.cancelled)
        assertEquals(listOf(nid), cancels)
    }

    @Test
    fun aDismissIsIgnoredWhenTheSettingIsOffOrItCannotBeCleared() = runTest {
        for ((p, live) in listOf(
            policy.copy(dismissOnPhone = false) to chat(), policy.copy(enabled = false) to chat(), policy.copy(allowed = emptySet()) to chat(),
            policy to chat(notClearable = true),
        )) {
            val port = Port(listOf(live))
            assertFalse(handler(port, p).onDismiss(nid))
            assertTrue(port.cancelled.isEmpty())
        }
        assertFalse(handler(Port(null)).onDismiss(nid))
        assertFalse(handler(Port(listOf(chat()))).onDismiss(Nid.of("nope")))
    }

    // --- the receiver in the app: the answer comes after the action fired, and a repeat never fires it twice ---

    @Test
    fun theReceiverAnswersALaterAckAndARepeatedIdGetsThatAnswerWithoutFiringAgain() = runTest {
        val dedup = DedupMemory({ 0L })
        val receiver = MessageReceiver(dedup, ReceiverRole.PHONE, notifAction = { null })
        receiver.onText("""{"type":"hello","id":"h","protocol":1,"name":"PC","platform":"windows","appVersion":"0.2.0","capabilities":["notif"]}""")
        val frame = """{"type":"notif.action","id":"a1","nid":"$nid","i":0,"label":"Mark as read"}"""
        val first = receiver.onText(frame)
        assertTrue(first.replies.isEmpty())
        val action = first.action as Action.NotifAction
        // The app fires, then answers and remembers the answer.
        val port = Port(listOf(chat()))
        val answer = when (val o = handler(port).onAction(action.request)) {
            ActionOutcome.Ack -> Frames.ack(action.request.id)
            is ActionOutcome.Error -> Frames.error(o.code, o.message, action.request.id)
        }
        receiver.recordResponse(action.request.id, answer)
        assertEquals("""{"type":"ack","ref":"a1"}""", answer)
        val again = receiver.onText(frame)
        assertEquals(listOf(answer), again.replies)
        assertEquals(Action.IgnoreDuplicate, again.action)
        assertEquals(1, port.fired.size)
    }

    @Test
    fun anErrorIsTheIdsOneResponseToo() = runTest {
        val dedup = DedupMemory({ 0L })
        val receiver = MessageReceiver(dedup, ReceiverRole.PHONE, notifAction = { null })
        receiver.onText("""{"type":"hello","id":"h","protocol":1,"name":"PC","platform":"windows","appVersion":"0.2.0","capabilities":["notif"]}""")
        val frame = """{"type":"notif.action","id":"a2","nid":"$nid","i":9,"label":"x"}"""
        val action = receiver.onText(frame).action as Action.NotifAction
        val outcome = handler(Port(listOf(chat()))).onAction(action.request) as ActionOutcome.Error
        val answer = Frames.error(outcome.code, outcome.message, "a2")
        receiver.recordResponse("a2", answer)
        assertEquals(listOf(answer), receiver.onText(frame).replies)
        assertTrue(answer.contains("stale"))
    }
}
