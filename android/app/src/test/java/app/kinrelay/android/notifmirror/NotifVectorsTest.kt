package app.kinrelay.android.notifmirror

import app.kinrelay.android.link.Clock
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.MessageReceiver
import app.kinrelay.android.protocol.ReceiverRole
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import app.kinrelay.android.testing.string
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Base64

/**
 * Runs `protocol/notif.json` (changelog 56) on the phone's side: `nid[]`, `iconHash[]`, `icon[]`, `clean[]`, `post[]`, `code[]`, the phone's half of
 * `rate[]`, `coalesce[]` and `sendNotif[]`. `toastIds[]`, `toastArguments[]`, the PC's half of `rate[]` and `syncDiff[]` are the PC's own (toast names, click
 * arguments, its bucket and its removal of toasts), which this app has no code for. The PC's receiving rules (`icon[]`, `post[]`) run on [NotifInbound]
 * (`ReceiverRole.PC`), a stand-in for the desktop's receiver, so that the same cases also prove what the phone sends is accepted. The `notif-*` cases of
 * `messages.json` run in `MessageVectorsTest`.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class NotifVectorsTest {
    private val vectors = Vectors.load("notif.json")
    private val limits = vectors.getValue("limits").jsonObject

    private fun limit(name: String) = limits.getValue(name).jsonPrimitive.int

    @Test
    fun nidsAreTheFirst32HexCharactersOfSha256OfTheKey() {
        val cases = vectors.cases("nid")
        assertTrue(cases.size >= 9)
        for (case in cases) assertEquals(case.getValue("name").str(), case.getValue("nid").str(), Nid.of(case.getValue("key").str()))
    }

    @Test
    fun iconHashesAreTheFirst32HexCharactersOfSha256OfThePngBytes() {
        val cases = vectors.cases("iconHash")
        assertTrue(cases.size >= 4)
        for (case in cases) {
            val bytes = Base64.getUrlDecoder().decode(case.getValue("png").str())
            assertEquals(case.getValue("name").str(), case.getValue("hash").str(), NotifWire.iconHash(bytes))
            assertEquals(case.getValue("name").str(), case.getValue("bytes").jsonPrimitive.int, bytes.size)
        }
    }

    private fun fixture(path: String): ByteArray = NotifVectorsTest::class.java.classLoader!!.getResourceAsStream(path)!!.readBytes()

    private fun pngOf(case: JsonObject): ByteArray? = case["pngFile"]?.let { fixture(it.str()) } ?: case.string("png")?.let { Base64.getUrlDecoder().decode(it) }

    private fun pcReceiver(): MessageReceiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PC).also {
        it.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Phone","platform":"android","appVersion":"0.2.0","capabilities":["notif"]}""")
    }

    @Test
    fun iconFramesAreAcceptedDroppedOrRefusedAsTheSpecSays() {
        val cases = vectors.cases("icon")
        assertTrue(cases.size >= 17)
        for (case in cases) {
            val name = case.getValue("name").str()
            val png = case["pngFile"]?.let { fixture(it.str()) }
            val frame = buildJsonObject {
                put("type", "notif.icon"); put("id", "icon-$name".take(60))
                case.string("hash")?.let { put("hash", it) }
                if (png != null) put("png", Base64.getUrlEncoder().withoutPadding().encodeToString(png)) else case.string("png")?.let { put("png", it) }
            }
            val outcome = pcReceiver().onText(frame.toString())
            val got = when (outcome.action) {
                is Action.NotifReceived -> "accepted"
                Action.NotifIgnored -> "dropped"
                else -> if (outcome.replies.single().contains("bad_request")) "bad_request" else "other:${outcome.replies}"
            }
            assertEquals(name, case.getValue("expect").str(), got)
        }
    }

    @Test
    fun theFramesThePhoneBuildsForIconsAreAccepted() {
        for (case in vectors.cases("icon").filter { it.getValue("expect").str() == "accepted" }) {
            val bytes = pngOf(case) ?: continue
            val outcome = pcReceiver().onText(NotifWire.icon("phone-icon", bytes))
            assertTrue(case.getValue("name").str(), outcome.action is Action.NotifReceived)
        }
    }

    @Test
    fun theCleaningLimitsAreTheOnesTheSenderUses() {
        val byField = mapOf(
            "app" to NotifWire.APP_MAX, "title" to NotifWire.TITLE_MAX, "text" to NotifWire.TEXT_MAX, "bigText" to NotifWire.BIG_TEXT_MAX,
            "subText" to NotifWire.SUB_TEXT_MAX, "conversation" to NotifWire.CONVERSATION_MAX, "messageSender" to NotifWire.SENDER_MAX,
            "messageText" to NotifWire.MESSAGE_TEXT_MAX, "line" to NotifWire.LINE_MAX, "hint" to NotifWire.HINT_MAX, "label" to NotifWire.LABEL_MAX,
        )
        for ((field, max) in byField) assertEquals("limit of $field", limit(field), max)
        assertEquals(limit("messages"), NotifWire.MAX_MESSAGES)
        assertEquals(limit("lines"), NotifWire.MAX_LINES)
        assertEquals(limit("actions"), NotifWire.MAX_ACTIONS)
        assertEquals(limit("nids"), NotifWire.MAX_SYNC_NIDS)
        assertEquals(limit("actionText"), NotifWire.REPLY_TEXT_MAX)
        val cases = vectors.cases("clean")
        assertTrue(cases.size >= 60)
        for (case in cases) {
            val field = case.getValue("field").str()
            assertEquals(case.getValue("name").str(), case.string("expected"), NotifWire.cleanText(case.getValue("input").str(), byField.getValue(field)))
        }
    }

    @Test
    fun postsAreNormalisedAsTheSpecSaysAndTheSendersFramesStayWithinTheLimits() {
        val cases = vectors.cases("post")
        assertTrue(cases.size >= 79)
        for (case in cases) {
            val name = case.getValue("name").str()
            val receiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PC)
            receiver.ownCapabilities = case.getValue("own").jsonArray.map { it.str() }.toSet()
            val caps = case.getValue("peer").jsonArray.map { it.str() }
            receiver.onText(
                """{"type":"hello","id":"prime","protocol":1,"name":"Phone","platform":"android","appVersion":"0.2.0","capabilities":[${caps.joinToString(",") { "\"$it\"" }}]}""",
            )
            val text = when {
                case["frame"] != null -> case.getValue("frame").toString()
                case["raw"] != null -> case.getValue("raw").str()
                else -> {
                    val s = case.getValue("synthesize").jsonObject
                    val base = s.getValue("base").jsonObject.toMutableMap()
                    base[s.getValue("field").str()] = JsonPrimitive(s.getValue("fill").str().repeat(s.getValue("fillCount").jsonPrimitive.int))
                    JsonObject(base).toString()
                }
            }
            val outcome = receiver.onText(text)
            val expect = case.getValue("expect").jsonObject
            when (expect.getValue("reply").str()) {
                "none" -> {
                    assertTrue("$name: no reply, got ${outcome.replies}", outcome.replies.isEmpty())
                    assertEquals("$name: action", expect.getValue("action").str(), outcome.action.category)
                    expect["notif"]?.let { assertEquals("$name: normalized post", it.jsonObject, (outcome.action as Action.NotifReceived).notif) }
                }
                else -> {
                    val reply = kotlinx.serialization.json.Json.parseToJsonElement(outcome.replies.single()).jsonObject
                    assertEquals("$name: code", expect.getValue("code").str(), reply.getValue("code").str())
                    assertEquals("$name: ref", expect["ref"]?.takeIf { it !is JsonNull }?.str(), reply["ref"]?.str())
                }
            }
        }
    }

    @Test
    fun theCodeCheckIsTheOneTheSenderUses() {
        val cases = vectors.cases("code")
        assertTrue(cases.size >= 20)
        for (case in cases) assertEquals(case.getValue("name").str(), case.getValue("valid").jsonPrimitive.content == "true", NotifWire.isValidCode(case.getValue("input").str()))
        assertEquals(limit("codeMin"), 4)
        assertEquals(limit("codeMax"), 10)
    }

    @Test
    fun aLabelInToastFormIsWhatThePcHas() {
        val cases = vectors.cases("toastLabel")
        assertTrue(cases.size >= 10)
        for (case in cases) {
            val name = case.getValue("name").str()
            val expected = case.getValue("expected").str()
            assertEquals(name, expected, NotifWire.toastForm(case.getValue("input").str()))
            assertEquals("$name is idempotent", expected, NotifWire.toastForm(expected))
        }
    }

    @Test
    fun aSenderMaySendOnlyToAPeerWhoseLatestHelloListsNotif() {
        val cases = vectors.cases("sendNotif")
        assertTrue(cases.size >= 9)
        for (case in cases) {
            val receiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PHONE)
            for ((i, hello) in case.getValue("hellos").jsonArray.withIndex()) {
                val frame = buildJsonObject {
                    put("type", "hello"); put("id", "h$i"); put("protocol", 1); put("name", "PC"); put("platform", "windows"); put("appVersion", "0.2.0")
                    hello.jsonObject.forEach { (key, value) -> put(key, value) }
                }
                assertTrue(receiver.onText(frame.toString()).action is Action.HandshakeOk)
            }
            val expected = case.getValue("expected").str() == "send"
            assertEquals(case.getValue("name").str(), expected, receiver.peer?.understandsNotif == true)
        }
    }

    // --- the sender: `coalesce[]` and the phone's half of `rate[]`, on a virtual clock ---

    private class RecordingLink(private val now: () -> Long) : MirrorLink {
        val sent = ArrayList<Pair<Long, JsonObject>>()
        override val isReady = true
        override val peerUnderstandsNotif = true
        override fun sendEphemeral(frame: String): Boolean {
            sent += now() to kotlinx.serialization.json.Json.parseToJsonElement(frame).jsonObject
            return true
        }
    }

    private fun nidOf(index: Int) = "%032x".format(index)

    @Test
    fun theSendersCoalescingFollowsTheVectors() = runTest {
        val cases = vectors.cases("coalesce")
        assertTrue(cases.size >= 10)
        for (case in cases) {
            val name = case.getValue("name").str()
            val start = testScheduler.currentTime
            val link = RecordingLink { testScheduler.currentTime - start }
            val sender = MirrorSender(backgroundScope, Clock { testScheduler.currentTime }, { link })
            for (event in case.getValue("events").jsonArray.map { it.jsonObject }) {
                val at = start + event.getValue("t").jsonPrimitive.long
                if (at > testScheduler.currentTime) advanceTimeBy(at - testScheduler.currentTime)
                runCurrent()
                val nid = event.getValue("nid").str()
                if (event.getValue("op").str() == "remove") sender.remove(nid)
                else sender.post(PostRequest("pkg", "key-$nid", WirePost(nid = nid, app = "App", title = event.getValue("content").str())))
            }
            advanceTimeBy(5_000)
            runCurrent()
            val got = link.sent.map { (t, frame) ->
                buildJsonObject {
                    put("t", t)
                    if (frame.getValue("type").str() == "notif.remove") put("op", "remove") else put("op", "post")
                    put("nid", frame.getValue("nid").str())
                    if (frame.getValue("type").str() == "notif.post") put("content", frame.getValue("title").str())
                }
            }
            assertEquals("$name: sent", case.getValue("expect").jsonObject.getValue("sent").jsonArray, JsonArray(got))
            // Let the next case start clean (its own sender; leftover timers of this one are in the background scope).
            advanceTimeBy(70_000)
        }
    }

    @Test
    fun thePhonesBudgetPackageCapAndRemoveLimitFollowTheVectors() = runTest {
        val cases = vectors.cases("rate").filter { it.getValue("limiter").str() == "phone" }
        assertTrue(cases.size >= 6)
        for (case in cases) {
            val name = case.getValue("name").str()
            val start = testScheduler.currentTime
            val link = RecordingLink { testScheduler.currentTime - start }
            val sender = MirrorSender(backgroundScope, Clock { testScheduler.currentTime }, { link })
            val outcomes = ArrayList<String>()
            for ((i, event) in case.getValue("events").jsonArray.map { it.jsonObject }.withIndex()) {
                val at = start + event.getValue("t").jsonPrimitive.long
                if (at > testScheduler.currentTime) advanceTimeBy(at - testScheduler.currentTime)
                runCurrent()
                val nid = event.string("nid") ?: nidOf(i)
                if (event.getValue("op").str() == "remove") {
                    outcomes += if (sender.remove(nid)) "remove_sent" else "remove_dropped"
                } else {
                    outcomes += when (sender.post(PostRequest(event.getValue("package").str(), "key-$nid", WirePost(nid = nid, app = "App", title = "t$i")))) {
                        PostResult.SENT -> "sent"
                        PostResult.DROPPED_PACKAGE -> "dropped_package"
                        PostResult.DROPPED_BUDGET -> "dropped_budget"
                        else -> "other"
                    }
                }
            }
            assertEquals("$name: outcomes", case.getValue("expect").jsonArray.map { it.str() }, outcomes)
            advanceTimeBy(120_000)
        }
    }

    @Test
    fun theArraysThatAreThePcsOwnExist() {
        // toastIds[], toastArguments[], syncDiff[] and the PC's half of rate[] are the PC's: toast names, click arguments, its bucket and its removal of toasts.
        for (name in listOf("toastIds", "toastArguments", "syncDiff")) assertTrue(name, vectors.cases(name).isNotEmpty())
        assertFalse(vectors.cases("rate").none { it.getValue("limiter").str() == "pc" })
    }
}
