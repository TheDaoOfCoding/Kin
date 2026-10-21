package app.kinrelay.android.protocol

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** `protocol/urls.json` (the openable-URL rule) and `protocol/text-rules.json` (line ends, the 128 KiB threshold, `sensitive`). */
class TextAndUrlVectorsTest {
    private val urls = Vectors.load("urls.json")
    private val rules = Vectors.load("text-rules.json")

    @Test
    fun everyUrlCase() {
        val cases = urls.cases("cases")
        assertTrue("there are ${cases.size} cases", cases.size >= 138)
        assertEquals(8192, urls.getValue("maxBytes").jsonPrimitive.int)
        for (case in cases) {
            val name = case.getValue("name").str()
            val text = case["input"]?.str() ?: synthesize(case.getValue("synthesize").jsonObject)
            assertEquals("urls/$name", case.getValue("isUrl").jsonPrimitive.boolean, UrlRules.isUrl(text))
        }
    }

    private fun synthesize(s: JsonObject): String {
        val text = s.getValue("prefix").str() + s.getValue("fill").str().repeat(s.getValue("fillCount").jsonPrimitive.int) + s.getValue("suffix").str()
        assertEquals(s.getValue("trimmedBytes").jsonPrimitive.int, Utf8.size(text.trim { UrlRules.isWhitespace(it) }))
        return text
    }

    @Test
    fun theOpenActionOpensTheTrimmedText() {
        assertEquals("https://example.com/x", UrlRules.httpUrl(" https://example.com/x　"))
        assertEquals(null, UrlRules.httpUrl("https://example.com/ x"))
    }

    @Test
    fun lineEndsOnTheWireAndTheirHash() {
        var run = 0
        for (case in rules.cases("lineEndings")) {
            val name = case.getValue("name").str()
            val clipboard = case["clipboardText"]?.str() ?: continue
            val wire = Frames.wireText(clipboard)
            assertEquals("lineEndings/$name: wire", case.getValue("wire").str(), wire)
            // The loop guard hashes the wire form, whichever form it is given.
            assertEquals("lineEndings/$name: hash", case.getValue("wireSha256Hex").str(), ClipboardGuard.hash(clipboard))
            assertEquals("lineEndings/$name: hash of the wire form", ClipboardGuard.hashOfWire(wire), ClipboardGuard.hash(clipboard))
            run++
        }
        assertTrue(run >= 8)
    }

    /** The phone only sends explicit texts (kind `text`); the automatic `clipboard` is the PC's. */
    @Test
    fun theSenderSizeThreshold() {
        assertEquals(131_072, rules.getValue("senderThresholdBytes").jsonPrimitive.int)
        var run = 0
        for (case in rules.cases("senderSize")) {
            if (case.getValue("kind").str() != "text") continue
            val name = case.getValue("name").str()
            val sensitive = case["sensitive"]?.jsonPrimitive?.boolean ?: false
            val wireBytes = case["synthesize"]?.let {
                val s = it.jsonObject
                val raw = s.getValue("fill").str().repeat(s.getValue("fillCount").jsonPrimitive.int) + (s["suffix"]?.str() ?: "")
                assertEquals("$name: raw bytes", s.getValue("rawBytes").jsonPrimitive.int, Utf8.size(raw))
                Utf8.size(Frames.wireText(raw))
            } ?: case.getValue("wireBytes").jsonPrimitive.int
            assertEquals("senderSize/$name: wire bytes", case.getValue("wireBytes").jsonPrimitive.int, wireBytes)
            val expected = when (val e = case.getValue("expected").str()) {
                "text" -> TextRules.Plan.TEXT
                "file" -> TextRules.Plan.FILE
                "not_sent" -> TextRules.Plan.NOT_SENT
                else -> error("unexpected $e for a text")
            }
            assertEquals("senderSize/$name", expected, TextRules.plan(wireBytes, sensitive))
            run++
        }
        assertTrue(run >= 8)
    }

    /** What the phone does with a sensitive `text` from the PC: the vectors' `receiver[]` cases, with a fake clipboard and notifier. */
    @Test
    fun theReceiverCases() {
        for (case in rules.cases("receiver")) {
            val name = case.getValue("name").str()
            val frame = case.getValue("frame").jsonObject
            val text = frame.getValue("text").str()
            val sensitive = frame["sensitive"]?.jsonPrimitive?.boolean ?: false
            val expect = case.getValue("expect").jsonObject
            val clipboard = mutableListOf<Pair<String, Boolean>>()
            val shown = mutableListOf<Triple<String?, String?, String>>()
            val logged = mutableListOf<String>()
            val handler = app.kinrelay.android.link.IncomingHandler(
                clipboard = { t, s -> clipboard += t to s },
                alerts = object : app.kinrelay.android.link.Alerts {
                    override fun textFrom(pcName: String, messageId: String, text: String?, openUrl: String?, copyText: String?, sensitive: Boolean, copiedAlready: Boolean) { shown += Triple(text, openUrl, pcName) }
                    override fun linkFrom(pcName: String, messageId: String, link: app.kinrelay.android.links.ReceivedLink, lockScreenPreviews: Boolean) = Unit
                    override fun sent(pcName: String?, arrivalMillis: Long) = Unit
                    override fun queued(pcName: String?, reason: String, afterNoReply: Boolean) = Unit
                    override fun sentLater(pcName: String?) = Unit
                    override fun sendFailed(title: String, reason: String) = Unit
                    override fun message(text: String) = Unit
                },
                guard = ClipboardGuard({ 0L }),
                log = object : app.kinrelay.android.log.KinLog {
                    override fun log(level: Char, component: String, message: String) { logged += message }
                },
            )
            kotlinx.coroutines.runBlocking {
                handler.onText("Desk PC", "m1", text, frame.getValue("isUrl").jsonPrimitive.boolean, sensitive)
            }
            assertEquals("receiver/$name: clipboard set", listOf(text to expect.getValue("clipboardMarkedSensitive").jsonPrimitive.boolean), clipboard)
            val (preview, openUrl, _) = shown.single()
            assertEquals("receiver/$name: preview", expect.getValue("preview").jsonPrimitive.boolean, preview != null)
            assertEquals("receiver/$name: open action", expect.getValue("openAction").jsonPrimitive.boolean, openUrl != null)
            val logsLengthOrHash = logged.any { it.contains("len=") || it.contains("sha=") }
            assertEquals("receiver/$name: logs length or hash", expect.getValue("logsTextLengthOrHash").jsonPrimitive.boolean, logsLengthOrHash)
            if (sensitive) assertTrue("receiver/$name: the text is nowhere in the log", logged.none { it.contains(text) })
        }
    }

    @Test
    fun theSensitiveFlagOfTheClipDescription() {
        // sensitiveSender[], phone side: the flag on the clip is the flag on the message, on API 33 and later.
        for (case in rules.cases("sensitiveSender")) {
            if (case.getValue("side").str() != "phone") continue
            val extra = case.getValue("extraIsSensitive").jsonPrimitive.boolean
            val expected = case.getValue("expect").jsonObject.getValue("sensitive").jsonPrimitive.boolean
            assertEquals(case.getValue("name").str(), expected, app.kinrelay.android.clipboard.ClipboardContent.sensitiveFlag(sdkInt = 33, extra = extra))
            assertEquals("older Androids never set it", false, app.kinrelay.android.clipboard.ClipboardContent.sensitiveFlag(sdkInt = 32, extra = extra))
        }
    }
}
