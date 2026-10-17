package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.log.KinLog
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A `text` from the PC that is a web address is shown like a link (the real address first, a warning mark when it is tricky), not as raw text (2B.1 review). */
class TextShownLikeALinkTest {
    private class Seen(val text: String?, val openUrl: String?, val copyText: String?)

    private fun receive(text: String, isUrl: Boolean = true): Seen {
        var seen: Seen? = null
        val handler = IncomingHandler(
            clipboard = { _, _ -> },
            alerts = object : Alerts {
                override fun textFrom(pcName: String, messageId: String, text: String?, openUrl: String?, copyText: String?, sensitive: Boolean, copiedAlready: Boolean) { seen = Seen(text, openUrl, copyText) }
                override fun linkFrom(pcName: String, messageId: String, link: app.kinrelay.android.links.ReceivedLink, lockScreenPreviews: Boolean) = Unit
                override fun sent(pcName: String?, arrivalMillis: Long) = Unit
                override fun queued(pcName: String?, reason: String, afterNoReply: Boolean) = Unit
                override fun sentLater(pcName: String?) = Unit
                override fun sendFailed(title: String, reason: String) = Unit
                override fun message(text: String) = Unit
            },
            guard = ClipboardGuard({ 0L }),
            log = KinLog.None,
        )
        runBlocking { handler.onText("Desk PC", "m1", text, isUrl) }
        return seen!!
    }

    @Test
    fun anOrdinaryAddressIsShownAsItsRealHostThenTheUrl() {
        val seen = receive("https://Example.com/a?b=1")
        assertEquals("example.com\nhttps://Example.com/a?b=1", seen.text)
        assertEquals("https://Example.com/a?b=1", seen.openUrl)
        assertEquals("Copy still copies the text as it came", "https://Example.com/a?b=1", seen.copyText)
    }

    @Test
    fun anIpAddressInAnotherSpellingIsMarked() {
        val seen = receive("http://0x7f.1/login")
        assertTrue(seen.text, seen.text!!.startsWith("⚠ 127.0.0.1\n"))
    }

    @Test
    fun aTextThatIsNotAnOpenableUrlStaysRaw() {
        val trick = "https://evil.example\\@google.com"
        val seen = receive(trick)
        assertEquals(trick, seen.text)
        assertNull(seen.openUrl)
        assertEquals("not a link when the sender says it isn't", "plain words", receive("plain words", isUrl = false).text)
        assertEquals("a sentence with a link in it", "see https://example.com now", receive("see https://example.com now").text)
    }

    @Test
    fun aLongUrlIsShortenedForTheEyesOnly() {
        val url = "https://example.com/" + "a".repeat(500)
        val seen = receive(url)
        assertTrue(seen.text!!.endsWith("…"))
        assertEquals(url, seen.openUrl)
    }
}
