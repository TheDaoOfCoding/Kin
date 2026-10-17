package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.clipboard.ClipboardPort
import app.kinrelay.android.links.ReceivedLink
import app.kinrelay.android.log.KinLog
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A paused phone changes nothing on screen (design 3.5, 10.1, invariant of section 8): an automatic `clipboard` is dropped, and a manual `text` or `link` is **quiet**: no notification or toast,
 * no clipboard write (the loop guard isn't armed either), the link is handed back so the caller records it in the history, and the arrival is counted. Not paused: exactly as before.
 */
class QuietArrivalTest {
    private class Recording : Alerts {
        val events = ArrayList<String>()
        override fun textFrom(pcName: String, messageId: String, text: String?, openUrl: String?, copyText: String?, sensitive: Boolean, copiedAlready: Boolean) { events += "text" }
        override fun linkFrom(pcName: String, messageId: String, link: ReceivedLink, lockScreenPreviews: Boolean) { events += "link" }
        override fun sent(pcName: String?, arrivalMillis: Long) { events += "sent" }
        override fun queued(pcName: String?, reason: String, afterNoReply: Boolean) { events += "queued" }
        override fun sentLater(pcName: String?) { events += "sentLater" }
        override fun sendFailed(title: String, reason: String) { events += "failed" }
        override fun message(text: String) { events += "toast" }
    }

    private class Rig(var paused: Boolean) {
        val written = ArrayList<String>()
        val alerts = Recording()
        var quiet = 0
        val logs = ArrayList<String>()
        val guard = ClipboardGuard({ 0L })
        val handler = IncomingHandler(
            ClipboardPort { text, _ -> written += text }, alerts, guard, object : KinLog {
                override fun log(level: Char, component: String, message: String) { logs += message }
            },
            paused = { paused }, onQuiet = { quiet++ },
        )
    }

    @Test
    fun aPausedPhoneDropsTheAutomaticClipboardWithoutWritingIt() = runBlocking {
        val r = Rig(paused = true)
        r.handler.onClipboard("secret-token-123")
        assertTrue(r.written.isEmpty())
        assertTrue(r.alerts.events.isEmpty())
        assertEquals("a dropped automatic message is not a quiet arrival (it never reaches the history)", 0, r.quiet)
        assertFalse("the log holds no part of the text", r.logs.any { it.contains("secret-token") })
    }

    @Test
    fun aPausedPhoneHandlesAManualTextQuietly() = runBlocking {
        val r = Rig(paused = true)
        r.handler.onText("Desk PC", "id-1", "hello from the pc", isUrl = false)
        r.handler.onText("Desk PC", "id-2", "a password", isUrl = false, sensitive = true)
        assertTrue("no clipboard write", r.written.isEmpty())
        assertTrue("no notification or toast", r.alerts.events.isEmpty())
        assertEquals(2, r.quiet)
        assertFalse("nothing of a sensitive text is logged", r.logs.any { it.contains("a password") })
    }

    @Test
    fun aPausedPhoneHandsBackALinkForTheHistoryAndShowsAndOpensNothing() = runBlocking {
        val r = Rig(paused = true)
        val shown = r.handler.onLink("Desk PC", "id-3", "https://example.com/a?token=abc", "Example", null)
        assertNotNull("the caller records it in the history", shown)
        assertEquals("https://example.com/a?token=abc", shown!!.url)
        assertTrue(r.alerts.events.isEmpty())
        assertEquals(1, r.quiet)
        assertFalse("no URL is logged", r.logs.any { it.contains("token=abc") || it.contains("example.com") })
        // A link the phone doesn't accept is still refused and is not a quiet arrival.
        assertEquals(null, r.handler.onLink("Desk PC", "id-4", "javascript:alert(1)", null, null))
        assertEquals(1, r.quiet)
    }

    @Test
    fun aPhoneThatIsNotPausedBehavesAsBefore() = runBlocking {
        val r = Rig(paused = false)
        r.handler.onClipboard("clip")
        r.handler.onText("Desk PC", "id-1", "hello", isUrl = false)
        r.handler.onLink("Desk PC", "id-2", "https://example.com/", "Example", null)
        assertEquals(listOf("clip", "hello"), r.written)
        assertEquals(listOf("text", "link"), r.alerts.events)
        assertEquals(0, r.quiet)
    }

    @Test
    fun theResumeTakesEffectAtOnceBecauseTheGateIsReadOnEachMessage() = runBlocking {
        val r = Rig(paused = true)
        r.handler.onText("Desk PC", "id-1", "while paused", isUrl = false)
        r.paused = false
        r.handler.onText("Desk PC", "id-2", "after", isUrl = false)
        assertEquals(listOf("after"), r.written)
        assertEquals(listOf("text"), r.alerts.events)
    }
}
