package app.kinrelay.android.protocol

import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import app.kinrelay.android.testing.string
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Runs the milestone 1.1 and 1.2 cases of `protocol/messages.json` through the real [MessageReceiver]. */
class MessageVectorsTest {
    private val vectors = Vectors.load("messages.json")

    private val validScope = listOf(
        "hello-*", "ack", "ack-from-pc", "error-*", "ping-*", "unpair-*", "nesting-depth-64-*",
        "brackets-inside-strings-do-not-count-as-nesting",
        // 1.2
        "text-*", "clipboard-*", "duplicate-keys-*",
        // 1.3
        "offer-*", "complete-*", "cancel-*",
        // 2B.1
        "link-*",
        // 2B.2 (the PC's side of the phone's `notif.*` is `NotifInbound`, which stands for the PC here; the real one is the desktop's)
        "notif-*",
    )
    private val invalidScope = listOf(
        "first-message-*", "hello-*", "ack-*", "error-*", "unpair-missing-id", "ping-*", "type-*",
        "unknown-type-is-checked-before-fields", "frame-*", "binary-frame*", "text-frame-1-MiB-plus-1-byte",
        "text-frame-multibyte-over-limit", "text-lone-*", "text-high-surrogate-followed-by-non-low-escape",
        "text-reversed-surrogate-pair-escape", "lone-surrogate-escape-*", "strict-*", "nesting-*",
        // 1.2
        "text-*", "clipboard-*",
        // 1.3
        "offer-*", "complete-*", "cancel-*",
        // 2B.1
        "link-*",
        // 2B.2
        "notif-*",
    )
    private val sequenceScope = listOf(
        "second-hello-after-handshake-is-a-normal-message", "invalid-hello-does-not-count-as-accepted",
        "garbage-before-hello-keeps-the-connection-open", "future-hello-gets-unsupported-version-not-a-timeout",
        "duplicate-ping-shows-one-toast",
        // 1.2
        "handshake-then-text-then-duplicate", "invalid-message-keeps-connection-open", "rejected-message-id-is-not-remembered",
        // 1.3
        "duplicate-offer-is-acked-not-restarted",
        // 1.4 (they use the steps `reconnect` and `transfer_over`, see protocol/README.md)
        "offer-reusing-a-text-id-is-bad-request",
        "offer-repeated-while-unfinished-is-acked-after-a-reconnect", "queue-resend-after-reconnect-is-idempotent",
        "queue-is-not-sent-before-hello",
        // 1.4 review (item 52): the same PC repeating an offer is acked in any state, any other reuse is `bad_request`; complete and cancel are re-sent
        "same-pc-repeat-after-done-is-acked", "same-pc-repeat-after-canceled-is-acked", "offer-from-another-device-is-bad-request",
        "same-pc-repeat-with-different-files-is-bad-request", "complete-re-sent-after-reconnect-is-acked-again",
        "cancel-re-sent-after-reconnect-is-acked-again",
        // 2B.1
        "link-duplicate-is-acked-not-redelivered", "link-rejected-id-is-not-remembered", "link-after-reconnect-is-idempotent",
        "link-sent-as-text-to-an-old-peer-is-a-text",
        // 2B.2
        "notif-dropped-when-feature-off-is-silent", "notif-capability-changes-with-a-new-hello", "notif-not-sent-without-capability-receiver-side",
        "notif-is-not-in-the-dedup-memory", "notif-action-is-acked-and-a-repeat-is-not-run-twice", "notif-action-is-not-resent-after-reconnect",
        "notif-post-after-reconnect-needs-the-hellos-again", "notif-sync-removes-the-rest", "notif-invalid-post-keeps-the-connection-open",
        "notif-status-then-icon-then-post",
    )

    /** The invalid-UTF-8 case can only be checked on the PC side: OkHttp never shows the bad bytes. */
    private val skipped = setOf("text-frame-invalid-utf8")

    /** The phone is the receiver for `to: phone`; for `to: pc` the receiver stands for the PC, which refuses `clipboard`. */
    private fun newReceiver(role: String = "phone") = MessageReceiver(DedupMemory({ 0L }), if (role == "phone") ReceiverRole.PHONE else ReceiverRole.PC)

    /** [peerCapabilities]: what the sender's `hello` lists (`peer` of a 2B.2 case); null sends a `hello` with no `capabilities`, as a 0.1.0 peer does. */
    private fun open(receiver: MessageReceiver, peerCapabilities: List<String>? = null) {
        val caps = peerCapabilities?.joinToString(",", prefix = ",\"capabilities\":[", postfix = "]") { "\"$it\"" }.orEmpty()
        val hello = receiver.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Peer","platform":"windows","appVersion":"0.1.0"$caps}""")
        assertTrue(hello.action is Action.HandshakeOk)
    }

    private fun input(case: JsonObject): Any? = when {
        case["frame"] != null -> case.getValue("frame").toString()
        case["raw"] != null -> case.getValue("raw").str()
        case["binaryHex"] != null -> BINARY
        case["synthesize"] != null -> {
            val s = case.getValue("synthesize").jsonObject
            val base = s.getValue("base").jsonObject.toMutableMap()
            base[s.getValue("field").str()] = JsonPrimitive(s.getValue("fill").str().repeat(s.getValue("fillCount").jsonPrimitive.int))
            val text = JsonObject(base).toString()
            assertEquals(s.getValue("totalBytes").jsonPrimitive.int, Utf8.size(text))
            text
        }
        else -> null
    }

    private fun feed(receiver: MessageReceiver, case: JsonObject): Outcome {
        val frame = input(case) ?: error("no input in ${case.getValue("name").str()}")
        return if (frame === BINARY) receiver.onBinary() else receiver.onText(frame as String)
    }

    private fun check(name: String, case: JsonObject, outcome: Outcome) {
        val expect = case.getValue("expect").jsonObject
        val closeExpected = expect["close"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.int
        assertEquals("$name: close", closeExpected, outcome.close)
        val replies = outcome.replies.map { Vectors.run { kotlinx.serialization.json.Json.parseToJsonElement(it).jsonObject } }
        when (expect.getValue("reply").str()) {
            "none" -> assertTrue("$name: no reply, got ${outcome.replies}", replies.isEmpty())
            "ack" -> {
                assertEquals("$name: one reply", 1, replies.size)
                assertEquals("$name: type", "ack", replies[0].getValue("type").str())
                val source = case["frame"]?.jsonObject
                    ?: case["synthesize"]?.jsonObject?.getValue("base")?.jsonObject
                    ?: StrictJson.parse(case.getValue("raw").str()).jsonObject
                val ref = expect.string("ref") ?: source.getValue("id").str()
                assertEquals("$name: ack ref", ref, replies[0].getValue("ref").str())
            }
            "error" -> {
                assertEquals("$name: one reply, got ${outcome.replies}", 1, replies.size)
                assertEquals("$name: type", "error", replies[0].getValue("type").str())
                assertEquals("$name: code", expect.getValue("code").str(), replies[0].getValue("code").str())
                assertEquals("$name: ref", expect["ref"]?.takeIf { it !is JsonNull }?.str(), replies[0]["ref"]?.str())
                assertTrue("$name: message is text", replies[0].getValue("message").jsonPrimitive.isString)
            }
        }
        expect["action"]?.let { assertEquals("$name: action", it.str(), outcome.action.category) }
        // `expect.link` is what the receiver shows and opens: the URL exactly as sent, the cleaned title and source app (absent = none).
        (outcome.action as? Action.DeliverLink)?.let {
            val link = expect.getValue("link").jsonObject
            assertEquals("$name: link url", link.getValue("url").str(), it.link.url)
            assertEquals("$name: link title", link.string("title"), it.link.title)
            assertEquals("$name: link sourceApp", link.string("sourceApp"), it.link.sourceApp)
        }
        // `expect.notif` (2B.2) is what the receiver ends up with for a valid `notif.*`.
        expect["notif"]?.let { notif ->
            val got: JsonObject = when (val action = outcome.action) {
                is Action.NotifReceived -> action.notif
                is Action.NotifDismiss -> buildJsonObject { put("nid", action.nid) }
                is Action.NotifAction -> buildJsonObject {
                    put("nid", action.request.nid); put("i", action.request.index); put("label", action.request.label)
                    action.request.text?.let { put("text", it) }
                }
                else -> error("$name: expected a notif action, got ${outcome.action}")
            }
            assertEquals("$name: notif", notif.jsonObject, got)
        }
        // `expect.sensitive` says how the receiver treats a delivered text (absent means false).
        (outcome.action as? Action.DeliverText)?.let {
            assertEquals("$name: sensitive", expect["sensitive"]?.jsonPrimitive?.boolean ?: false, it.sensitive)
        }
    }

    private fun runSingleCases(section: String, scope: List<String>) {
        var run = 0
        for (case in vectors.cases(section)) {
            val name = case.getValue("name").str()
            if (!Vectors.inScope(name, scope) || name in skipped) continue
            val receiver = newReceiver(case.getValue("to").str())
            // 2B.2 cases say what the receiver's own `hello` and the sender's list (`own`, `peer`).
            case["own"]?.let { own -> receiver.ownCapabilities = own.jsonArray.map { it.str() }.toSet() }
            if (case.getValue("state").str() == "open") open(receiver, case["peer"]?.jsonArray?.map { it.str() })
            check("$section/$name", case, feed(receiver, case))
            run++
        }
        println("messages.json $section: $run cases run")
        assertTrue("$section: ran $run", run > if (section == "valid") 30 else 80)
    }

    @Test
    fun validCases() = runSingleCases("valid", validScope)

    @Test
    fun invalidCases() = runSingleCases("invalid", invalidScope)

    @Test
    fun sequences() {
        var run = 0
        for (sequence in vectors.cases("sequences")) {
            val name = sequence.getValue("name").str()
            if (!Vectors.inScope(name, sequenceScope)) continue
            // The memory of IDs outlives a connection (`reconnect` makes new receivers over the same memory). `known` stands for the transfer
            // engine's store: the offers the phone started, with the PC that made them and their files (file ID to size). The engine's rule
            // (item 52) is in [TransferEngine.offerHistory]; this is the same rule over a map, since the real one needs a store and a link.
            val dedups = mapOf("phone" to DedupMemory({ 0L }), "pc" to DedupMemory({ 0L }))
            class Known(val device: String, val files: Map<String, Long>)
            val known = HashMap<String, Known>()
            var device = "first"
            fun history(offer: Offer): OfferHistory {
                val earlier = known[offer.id] ?: return OfferHistory.NEW
                return if (earlier.device == device && earlier.files == offer.files.associate { it.fileId to it.size }) OfferHistory.REPEAT else OfferHistory.REUSED
            }
            fun fresh() = mapOf(
                "phone" to MessageReceiver(dedups.getValue("phone"), ReceiverRole.PHONE, offerHistory = ::history),
                "pc" to MessageReceiver(dedups.getValue("pc"), ReceiverRole.PC),
            )
            var receivers = fresh()
            for ((i, step) in sequence.getValue("steps").jsonArray.withIndex()) {
                val case = step.jsonObject
                when (case["event"]?.str()) {
                    "reconnect" -> { case["device"]?.str()?.let { device = it }; receivers = fresh(); continue }
                    // A finished download keeps its ID for ever: nothing to change in the model.
                    "transfer_over" -> continue
                    null -> Unit
                    else -> error("unknown step event in $name#$i")
                }
                val fromRole = case.getValue("from").str()
                val receiver = receivers.getValue(if (fromRole == "phone") "pc" else "phone")
                // A `hello` sets the sender's own capabilities (the other side keeps them as its view of the sender, through its own receiver).
                case["frame"]?.jsonObject?.takeIf { it["type"]?.str() == "hello" }?.let { hello ->
                    receivers.getValue(fromRole).ownCapabilities =
                        (hello["capabilities"] as? kotlinx.serialization.json.JsonArray)?.mapNotNull { it.jsonPrimitive.contentOrNull }?.toSet().orEmpty()
                }
                val outcome = feed(receiver, case)
                check("sequence/$name#$i", case, outcome)
                (outcome.action as? Action.StartDownload)?.let { known[it.offer.id] = Known(device, it.offer.files.associate { f -> f.fileId to f.size }) }
            }
            run++
        }
        assertEquals(sequenceScope.size, run)
    }

    @Test
    fun helloTimeoutTimers() {
        val timers = vectors.cases("timers")
        assertTrue(timers.size >= 3)
        for (case in timers) {
            val expected = case.getValue("expect").jsonObject["close"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.int
            val accepted = case.getValue("state").str() == "open"
            assertEquals(case.getValue("name").str(), expected, CloseRules.helloTimeoutClose(accepted, case.getValue("elapsedSinceOpenMs").jsonPrimitive.long))
        }
    }

    @Test
    fun closeCodeActionsAndBackoffReset() {
        val cases = vectors.cases("closeCodes")
        assertTrue(cases.size >= 24)
        for (case in cases) {
            val name = case.getValue("name").str()
            val code = case["closeCode"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.int
            val upgrade = case["upgradeStatus"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.int
            val expected = when (val action = case.getValue("action").str()) {
                "reconnect_with_backoff" -> CloseAction.RECONNECT_WITH_BACKOFF
                "reconnect_on_user_action" -> CloseAction.RECONNECT_ON_USER_ACTION
                "stop_update_app" -> CloseAction.STOP_UPDATE_APP
                "stop_pair_again" -> CloseAction.STOP_PAIR_AGAIN
                else -> error("unknown action $action")
            }
            assertEquals("$name: action", expected, CloseRules.action(code, upgrade))
            case["resetsBackoff"]?.let {
                val acceptedFor = case["helloAcceptedForMs"]?.takeIf { v -> v !is JsonNull }?.jsonPrimitive?.long
                assertEquals("$name: resets the backoff", it.jsonPrimitive.boolean, CloseRules.resetsBackoff(acceptedFor))
            }
        }
    }

    @Test
    fun theLinkUrlTricksOfTheReviewAreAmongTheCasesTheLoopsRun() {
        // B1 (changelog 55): a backslash, user information or an escape in the authority is `bad_request` (the invalid loop checks the reply and `ref`);
        // a mixed-case scheme and host are kept as sent (the valid loop checks the URL byte for byte).
        val invalid = vectors.cases("invalid").map { it.getValue("name").str() }
        for (name in listOf("link-url-backslash-in-the-authority", "link-url-userinfo-lookalike", "link-url-percent-in-the-host")) assertTrue(name, name in invalid)
        assertTrue(vectors.cases("valid").any { it.getValue("name").str() == "link-scheme-and-host-case-are-kept" })
    }

    // --- cases of our own, for behaviour the vectors only touch ---

    @Test
    fun duplicateKeysInAPingTheLastValueWins() {
        val receiver = newReceiver().also(::open)
        val outcome = receiver.onText("""{"type":"text","id":"first","type":"ping","id":"last","sentAt":1}""")
        assertEquals("last", (outcome.action as Action.ShowPing).id)
        assertEquals("""{"type":"ack","ref":"last"}""", outcome.replies.single())
    }

    @Test
    fun aRepeatedPingGetsTheSameAckAndIsNotProcessedAgain() {
        val receiver = newReceiver().also(::open)
        val frame = """{"type":"ping","id":"p1","sentAt":5}"""
        assertTrue(receiver.onText(frame).action is Action.ShowPing)
        val again = receiver.onText(frame)
        assertEquals(Action.IgnoreDuplicate, again.action)
        assertEquals("""{"type":"ack","ref":"p1"}""", again.replies.single())
    }

    @Test
    fun aRejectedMessageIdIsNotRemembered() {
        val receiver = newReceiver().also(::open)
        assertTrue(receiver.onText("""{"type":"ping","id":"p2"}""").action is Action.Rejected)
        assertTrue(receiver.onText("""{"type":"ping","id":"p2","sentAt":1}""").action is Action.ShowPing)
    }

    @Test
    fun theReceiverUsesTheHelloNameOfThePeer() {
        val receiver = newReceiver()
        receiver.onText("""{"type":"hello","id":"h","protocol":1,"name":"Desk PC","platform":"windows","appVersion":"1"}""")
        assertEquals("Desk PC", receiver.peer?.name)
        receiver.onText("""{"type":"hello","id":"h2","protocol":1,"name":"Renamed","platform":"windows","appVersion":"1"}""")
        assertEquals("Renamed", receiver.peer?.name)
    }

    @Test
    fun anOversizeFrameIsMeasuredInBytesNotCharacters() {
        val receiver = newReceiver().also(::open)
        val chars = "é".repeat(524_288) // 1 MiB of bytes, 0.5 Mi characters
        val within = """{"type":"ping","id":"x","sentAt":1,"pad":"${chars.dropLast(100)}"}"""
        assertTrue(Utf8.size(within) <= MAX_FRAME_BYTES)
        assertTrue(receiver.onText(within).action is Action.ShowPing)
        val over = """{"type":"ping","id":"y","sentAt":1,"pad":"$chars"}"""
        val outcome = receiver.onText(over)
        assertEquals(1009, outcome.close)
        assertTrue(outcome.replies.isEmpty())
    }

    @Test
    fun sentFramesAreCompactAndStrictlyValid() {
        val hello = Frames.hello("0193a5c2-7b1e-7c3a-9d4f-5e6a7b8c9d32", "Pixel 8", "android", "0.1.0")
        assertEquals("""{"type":"hello","id":"0193a5c2-7b1e-7c3a-9d4f-5e6a7b8c9d32","protocol":1,"name":"Pixel 8","platform":"android","appVersion":"0.1.0","capabilities":["link","notif"]}""", hello)
        // An app with no extra features sends no `capabilities` at all (a 0.1.0 hello).
        assertEquals("""{"type":"hello","id":"h","protocol":1,"name":"Pixel 8","platform":"android","appVersion":"0.1.0"}""", Frames.hello("h", "Pixel 8", "android", "0.1.0", capabilities = emptyList()))
        assertEquals("""{"type":"ping","id":"a","sentAt":1760443200000}""", Frames.ping("a", 1760443200000))
        assertEquals("""{"type":"unpair","id":"u"}""", Frames.unpair("u"))
        assertEquals("""{"type":"error","code":"bad_request","message":"m"}""", Frames.error("bad_request", "m", null))
        assertEquals("""{"type":"error","ref":"r","code":"bad_request","message":"m"}""", Frames.error("bad_request", "m", "r"))
        assertNull(StrictJson.parse(Frames.error("x", "y", null)).jsonObject["ref"])
    }

    @Test
    fun unpairedSurrogatesAreReplacedBeforeSending() {
        assertEquals("a�b", Frames.sanitize("a\uD800b"))
        assertEquals("a�b", Frames.sanitize("a\uDC00b"))
        assertEquals("a😀b", Frames.sanitize("a😀b"))
        assertEquals(StrictJson.parse(Frames.hello("i", Frames.sanitize("x\uD800"), "android", "1")).jsonObject.getValue("name").str(), "x�")
    }

    @Test
    fun messageIdsAreLowercaseHyphenatedUuidV7() {
        val id = MessageIds.newId(0x0193a5c27b1eL)
        assertTrue(id, Regex("[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}").matches(id))
        assertTrue(id.startsWith("0193a5c2-7b1e-7"))
        assertEquals(36, id.length)
        assertTrue((1..50).map { MessageIds.newId() }.toSet().size == 50)
    }

    private companion object {
        val BINARY = Any()
    }
}
