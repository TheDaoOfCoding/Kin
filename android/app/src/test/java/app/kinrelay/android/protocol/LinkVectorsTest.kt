package app.kinrelay.android.protocol

import app.kinrelay.android.links.Icu4jUts46
import app.kinrelay.android.links.LinkDomain
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import app.kinrelay.android.testing.string
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * `protocol/links.json` (2B.1, changelog 54) through the real code: which `link.url` is valid (as sent, no trimming), how a title and a source app
 * are cleaned, whether the phone sends a `link` or a `text` to a peer by its `hello` capabilities, and the text that stands in for a link. The frames
 * of the `link` message itself are the `link-*` cases of `messages.json` ([MessageVectorsTest]).
 */
class LinkVectorsTest {
    private val vectors = Vectors.load("links.json")

    private fun JsonObject.inputOf(): String {
        val synth = this["synthesize"]?.jsonObject
        return if (synth != null) {
            val text = synth.getValue("prefix").str() + synth.getValue("fill").str().repeat(synth.getValue("fillCount").jsonPrimitive.content.toInt()) + synth.string("suffix").orEmpty()
            assertEquals("${getValue("name").str()}: synthesized size", synth.getValue("bytes").jsonPrimitive.content.toInt(), Utf8.size(text))
            text
        } else {
            getValue("input").str()
        }
    }

    @Test
    fun urlsFollowTheOpenableUrlRuleAsSentWithNoTrimming() {
        val cases = vectors.cases("urls")
        assertTrue(cases.size >= 80)
        for (case in cases) {
            val name = case.getValue("name").str()
            val url = case.inputOf()
            assertEquals("urls/$name", case.getValue("valid").jsonPrimitive.content.toBoolean(), LinkWire.isLinkUrl(url))
        }
    }

    /**
     * `display[]` (changelog 55): the address the notification shows and whether it warns, as a browser reads the host. UTS 46 runs on ICU4J here
     * (`android.icu` is a stub on the JVM); `LinkDisplayVectorsInstrumentedTest` runs the same file on a device with the real `android.icu`.
     */
    @Test
    fun displayAddressesFollowTheBrowsersReadingOfTheHost() {
        val cases = vectors.cases("display")
        assertTrue(cases.size >= 46)
        for (case in cases) {
            val name = case.getValue("name").str()
            val url = case.getValue("url").str()
            assertTrue("display/$name: the url passes the rule", LinkWire.isLinkUrl(url))
            val address = LinkDomain.of(url, Icu4jUts46)!!
            assertEquals("display/$name: display", case.getValue("display").str(), address.display)
            assertEquals("display/$name: warn", case.getValue("warn").jsonPrimitive.content.toBoolean(), address.warn)
        }
    }

    @Test
    fun titlesAreCleanedAndCutToThreeHundredCodePoints() {
        val cases = vectors.cases("titles")
        assertTrue(cases.size >= 15)
        for (case in cases) {
            val name = case.getValue("name").str()
            assertEquals("titles/$name", case.string("expected"), LinkWire.cleanText(case.getValue("input").str(), vectors.getValue("maxTitleCodePoints").jsonPrimitive.content.toInt()))
        }
    }

    @Test
    fun sourceAppsAreCleanedAndCutToSixtyFourCodePoints() {
        val cases = vectors.cases("sourceApps")
        assertTrue(cases.size >= 4)
        for (case in cases) {
            val name = case.getValue("name").str()
            assertEquals("sourceApps/$name", case.string("expected"), LinkWire.cleanSourceApp(case.getValue("input").str()))
        }
    }

    /** Runs the hellos of a case through a real receiver and says whether a link may go to the peer: the latest accepted hello counts. */
    private fun understandsLink(case: JsonObject): Boolean {
        val receiver = MessageReceiver(DedupMemory({ 0L }), ReceiverRole.PHONE)
        for ((i, hello) in case.getValue("hellos").jsonArray.withIndex()) {
            val fields = hello.jsonObject.entries.joinToString("") { (k, v) -> ""","$k":$v""" }
            val outcome = receiver.onText("""{"type":"hello","id":"h$i","protocol":1,"name":"PC","platform":"windows","appVersion":"0.2.0"$fields}""")
            // A bad `capabilities` never fails a hello.
            assertTrue("${case.getValue("name").str()}: hello $i accepted", outcome.action is Action.HandshakeOk)
        }
        return receiver.peer!!.understandsLink
    }

    @Test
    fun aLinkGoesAsALinkOnlyToAPeerWhoseHelloListsTheCapability() {
        val cases = vectors.cases("sendAs")
        assertTrue(cases.size >= 11)
        for (case in cases) {
            val expected = case.getValue("expected").str()
            assertEquals("sendAs/${case.getValue("name").str()}", expected == "link", understandsLink(case))
        }
    }

    @Test
    fun theTextThatStandsInForALinkIsTheBareUrlWithTheSameId() {
        val cases = vectors.cases("textFallback")
        assertEquals(2, cases.size)
        for (case in cases) {
            val link = case.getValue("link").jsonObject
            val expected = case.getValue("expected").jsonObject
            val frame = Frames.text("same-id", link.getValue("url").str(), isUrl = true)
            assertNotNull(frame)
            val parsed = kotlinx.serialization.json.Json.parseToJsonElement(frame!!).jsonObject
            assertEquals("textFallback/${case.getValue("name").str()}: type", expected.getValue("type").str(), parsed.getValue("type").str())
            assertEquals("text", expected.getValue("text").str(), parsed.getValue("text").str())
            assertEquals("isUrl", expected.getValue("isUrl").jsonPrimitive.content, parsed.getValue("isUrl").jsonPrimitive.content)
            assertEquals("same id", "same-id", parsed.getValue("id").str())
            assertFalse("no title", parsed.containsKey("title"))
            assertFalse("no sourceApp", parsed.containsKey("sourceApp"))
        }
    }

    @Test
    fun theHelloTheAppSendsListsTheLinkAndNotifCapabilities() {
        val hello = kotlinx.serialization.json.Json.parseToJsonElement(Frames.hello("id-1", "Pixel", "android", "0.1.1")).jsonObject
        assertEquals(listOf("link", "notif", "media"), hello.getValue("capabilities").jsonArray.map { it.jsonPrimitive.content })
        assertFalse(hello["capabilities"] is JsonNull)
    }

    @Test
    fun aLinkFrameTheAppSendsCarriesTheCleanedTitleAndOmitsWhatIsEmpty() {
        val frame = Frames.link("l-1", "https://www.youtube.com/watch?v=abc&t=83s", "  My\u202E video\n ")!!
        val parsed = kotlinx.serialization.json.Json.parseToJsonElement(frame).jsonObject
        assertEquals("link", parsed.getValue("type").str())
        assertEquals("the URL is written as it is, with its timestamp", "https://www.youtube.com/watch?v=abc&t=83s", parsed.getValue("url").str())
        assertEquals("My_ video", parsed.getValue("title").str())
        assertFalse(parsed.containsKey("sourceApp"))
        val bare = kotlinx.serialization.json.Json.parseToJsonElement(Frames.link("l-2", "https://example.com/", " \n ")!!).jsonObject
        assertFalse("an empty title is left out", bare.containsKey("title"))
    }
}
