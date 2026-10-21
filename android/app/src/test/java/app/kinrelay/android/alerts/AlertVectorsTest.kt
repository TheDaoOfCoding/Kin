package app.kinrelay.android.alerts

import app.kinrelay.android.link.Clock
import app.kinrelay.android.protocol.Action
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.MessageReceiver
import app.kinrelay.android.protocol.ReceiverRole
import app.kinrelay.android.protocol.StrictJson
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Runs the alert part of `protocol/alerts.json` (changelog 59) on the phone's own code: `limits`, `alert[]` (every frame through the real [MessageReceiver], cleaning and cutting included) and
 * `alertRate[]` (the real [AlertBudget] on a virtual clock). The `alert-*` cases of `messages.json` are run by `MessageVectorsTest`; `alertSend[]`, `focusState[]`, `holdToasts[]`, `notifyCli[]`
 * and `downloadAlert[]` are the PC's and the extension's.
 */
class AlertVectorsTest {
    private val v = Vectors.load("alerts.json")

    @Test
    fun theLimitsOfTheSpecAreTheConstantsOfTheCode() {
        val l = v.getValue("limits").jsonObject
        assertEquals(l.getValue("frame").jsonPrimitive.long, AlertWire.FRAME_BYTES.toLong())
        assertEquals(l.getValue("textMax").jsonPrimitive.long, AlertWire.TEXT_MAX_CODE_POINTS.toLong())
        assertEquals(l.getValue("titleMax").jsonPrimitive.long, AlertWire.TITLE_MAX_CODE_POINTS.toLong())
        assertEquals(l.getValue("waitedMaxMs").jsonPrimitive.long, AlertWire.WAITED_MAX_MILLIS)
        assertEquals(l.getValue("phoneCapacity").jsonPrimitive.long, AlertBudget.CAPACITY.toLong())
        assertEquals(l.getValue("phoneRefillMs").jsonPrimitive.long, AlertBudget.REFILL_MILLIS)
    }

    private fun pieces(parts: JsonObject): Map<String, String> =
        parts.mapValues { (_, list) -> list.jsonArray.joinToString("") { it.jsonArray[0].jsonPrimitive.content.repeat(it.jsonArray[1].jsonPrimitive.content.toInt()) } }

    private fun receiver(spent: Boolean = false): MessageReceiver {
        val r = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PHONE, alertAdmit = { !spent })
        r.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Desk PC","platform":"windows","appVersion":"0.2.0","capabilities":["presence"]}""")
        return r
    }

    @Test
    fun everyAlertFrameIsAcceptedOrBadRequestAsTheVectorsSayWithTheContentCleaned() {
        var run = 0
        for (case in v.cases("alert")) {
            val name = case.getValue("name").str()
            val text = case["raw"]?.str() ?: run {
                val frame = case.getValue("frame").jsonObject.toMutableMap()
                case["frameParts"]?.let { parts -> pieces(parts.jsonObject).forEach { (field, value) -> frame[field] = JsonPrimitive(value) } }
                JsonObject(frame).toString()
            }
            val outcome = receiver().onText(text)
            when (case.getValue("expect").str()) {
                "accepted" -> {
                    val action = outcome.action as? Action.DeliverAlert ?: error("$name: expected a deliver_alert action, got ${outcome.action} (${outcome.replies})")
                    val expected = HashMap<String, String?>()
                    case["content"]?.jsonObject?.forEach { (k, value) -> expected[k] = (value as? JsonPrimitive)?.content }
                    case["contentParts"]?.let { parts -> expected.putAll(pieces(parts.jsonObject)) }
                    assertEquals("$name: text", expected["text"], action.alert.text)
                    assertEquals("$name: title", expected["title"], action.alert.title)
                    assertEquals("$name: waitedMs", expected["waitedMs"]?.toLong(), action.alert.waitedMs)
                }
                "bad_request" -> {
                    assertEquals("$name: one reply, got ${outcome.replies}", 1, outcome.replies.size)
                    assertEquals("$name: code", "bad_request", StrictJson.parse(outcome.replies[0]).jsonObject.getValue("code").str())
                }
                else -> error("$name: unknown expectation")
            }
            run++
        }
        assertTrue("ran $run", run >= 40)
    }

    // ------------------------------------------------------------------ alertRate[]

    @Test
    fun theBucketIsTenPerSixtySecondsWithOneTokenBackEverySixSeconds() {
        var run = 0
        for (case in v.cases("alertRate")) {
            val name = case.getValue("name").str()
            var now = 0L
            val budget = AlertBudget(Clock { now })
            val got = case.getValue("events").jsonArray.map { e ->
                now = e.jsonObject.getValue("t").jsonPrimitive.long
                if (budget.tryTake()) "accepted" else "rate_limited"
            }
            assertEquals("$name: ${case["note"]?.str()}", case.getValue("expect").jsonArray.map { it.str() }, got)
            run++
        }
        assertTrue("ran $run", run >= 7)
    }

    // ------------------------------------------------------------------ our own cases

    @Test
    fun aRepeatedIdIsAckedAgainAndCostsNoTokenAndARefusedOneIsNotRemembered() {
        var tokens = 1
        val r = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PHONE, alertAdmit = { if (tokens > 0) { tokens--; true } else false })
        r.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Desk PC","platform":"windows","appVersion":"0.2.0"}""")
        val first = r.onText("""{"type":"alert","id":"a1","text":"one"}""")
        assertTrue(first.action is Action.DeliverAlert)
        // The bucket is empty now: the repeat still gets the first answer, because it is looked up before the bucket is asked.
        val again = r.onText("""{"type":"alert","id":"a1","text":"one"}""")
        assertEquals(Action.IgnoreDuplicate, again.action)
        assertEquals("""{"type":"ack","ref":"a1"}""", again.replies.single())
        // A new one is refused with `rate_limited` and its `ref`, and the id is free for later.
        val refused = r.onText("""{"type":"alert","id":"a2","text":"two"}""")
        assertEquals(Action.Rejected, refused.action)
        val error = StrictJson.parse(refused.replies.single()).jsonObject
        assertEquals("rate_limited", error.getValue("code").str())
        assertEquals("a2", error.getValue("ref").str())
        tokens = 1
        assertTrue(r.onText("""{"type":"alert","id":"a2","text":"two"}""").action is Action.DeliverAlert)
    }

    @Test
    fun anInvalidAlertCostsNoToken() {
        var asked = 0
        val r = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PHONE, alertAdmit = { asked++; true })
        r.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Desk PC","platform":"windows","appVersion":"0.2.0"}""")
        r.onText("""{"type":"alert","id":"bad1"}""")
        r.onText("""{"type":"alert","id":"bad2","text":"   "}""")
        assertEquals(0, asked)
    }

    @Test
    fun theWaitedTextAppearsOnlyAfterTwoMinutesAndInTheUnitsTheNotificationUses() {
        assertNull(AlertArrivals.waitedText(null))
        assertNull(AlertArrivals.waitedText(0))
        assertNull(AlertArrivals.waitedText(120_000))
        assertEquals("2 min ago", AlertArrivals.waitedText(120_001))
        assertEquals("59 min ago", AlertArrivals.waitedText(59 * 60_000L))
        assertEquals("3 h ago", AlertArrivals.waitedText(3 * 3_600_000L))
        assertEquals("47 h ago", AlertArrivals.waitedText(47 * 3_600_000L))
        assertEquals("2 d ago", AlertArrivals.waitedText(2 * 86_400_000L + 3_600_000L))
        assertEquals("Desk PC · 3 h ago", AlertArrivals.subText("Desk PC", 3 * 3_600_000L))
        assertEquals("Desk PC", AlertArrivals.subText("Desk PC", 5_000))
    }

    @Test
    fun severalAlertsAreOneNotificationNotOneEach() {
        val arrivals = AlertArrivals()
        repeat(60) { arrivals.add("Desk PC", AlertArrivals.Entry(null, "alert $it", null)) }
        val state = arrivals.state()
        assertEquals(60, state.count)
        assertEquals("only the newest few lines are kept", 5, state.entries.size)
        assertEquals("60 alerts from Desk PC", AlertArrivals.title(state))
        assertEquals("the lock screen shows who it is from, never what it says", "60 alerts from Desk PC", AlertArrivals.publicTitle(state))
        arrivals.reset()
        assertTrue(arrivals.isEmpty())
        val one = arrivals.add("Desk PC", AlertArrivals.Entry("Render", "Scene 12 finished", 3_600_000L))
        assertEquals("one alert shows its own title", "Render", AlertArrivals.title(one))
        assertEquals("Alert from Desk PC", AlertArrivals.publicTitle(one))
        assertEquals("Render: Scene 12 finished · 1 h ago", AlertArrivals.line(one.entries.last()))
        assertEquals("a line without a title is the text", "done", AlertArrivals.line(AlertArrivals.Entry(null, "done", null)))
    }
}
