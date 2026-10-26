package app.kinrelay.android.ui.send

import app.kinrelay.android.clipboard.ClipboardContent
import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.data.TextSource
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.link.SendSource
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Send button's decisions that need no Android: text or link, nothing to send, what a paste does, the stored source and the words. */
class SendPlanTest {
    @Test
    fun aBoxWithExactlyOneWebLinkGoesAsALinkTrimmed() {
        assertEquals(SendPlan.Typed.Link("https://example.com/a?b=1"), SendPlan.typed("  https://example.com/a?b=1 \n"))
        assertEquals(SendPlan.Typed.Link("http://example.com"), SendPlan.typed("http://example.com"))
    }

    @Test
    fun aSentenceWithALinkInItStaysAText() {
        val sentence = "look at https://example.com now"
        assertEquals(SendPlan.Typed.Plain(sentence), SendPlan.typed(sentence))
        val two = "https://a.example https://b.example"
        assertEquals(SendPlan.Typed.Plain(two), SendPlan.typed(two))
    }

    @Test
    fun aNonWebAddressStaysAText() {
        for (text in listOf("javascript:alert(1)", "file:///etc/passwd", "ftp://example.com", "example.com", "https://user@evil.example/")) {
            assertEquals(SendPlan.Typed.Plain(text), SendPlan.typed(text))
        }
    }

    @Test
    fun emptyAndWhitespaceOnlyBoxesHaveNothingToSend() {
        for (text in listOf("", " ", "\n\t  \r\n")) assertEquals(SendPlan.Typed.Empty, SendPlan.typed(text))
    }

    @Test
    fun aTypedTextIsKeptAsTyped() {
        assertEquals(SendPlan.Typed.Plain("  two  spaces \n"), SendPlan.typed("  two  spaces \n"))
    }

    @Test
    fun anEmptyOrBlankClipboardSendsNothing() {
        assertEquals(SendPlan.Paste.Empty, SendPlan.paste(null))
        assertEquals(SendPlan.Paste.Empty, SendPlan.paste(ClipboardContent("  \n", sensitive = false)))
    }

    @Test
    fun aPasteGoesAsAClipboardSendSoTheLoopGuardAppliesAndKeepsTheSensitiveMark() {
        val plain = SendPlan.paste(ClipboardContent("hello", sensitive = false)) as SendPlan.Paste.Send
        assertEquals("hello", plain.text)
        assertFalse(plain.sensitive)
        assertEquals(SendSource.CLIPBOARD, plain.source)
        val secret = SendPlan.paste(ClipboardContent("hunter2", sensitive = true)) as SendPlan.Paste.Send
        assertTrue(secret.sensitive)
    }

    @Test
    fun theLoopGuardSkipsWhatThePcJustPutOnTheClipboardAndNothingElse() {
        var now = 10_000L
        val guard = ClipboardGuard({ now })
        guard.notePeerSet("from the PC")
        // The sender checks this for a CLIPBOARD send, which is the source a paste uses.
        assertTrue(guard.isEcho((SendPlan.paste(ClipboardContent("from the PC", false)) as SendPlan.Paste.Send).text))
        assertFalse(guard.isEcho("something the user copied"))
        now += ClipboardGuard.WINDOW_MILLIS + 1
        assertFalse(guard.isEcho("from the PC"))
    }

    @Test
    fun typedTextIsStoredAsTypedAndRoundTripsThroughItsSource() {
        assertEquals("typed", TextSource.TYPED)
        assertEquals(TextSource.TYPED, SendSource.TYPED.stored)
        assertEquals(SendSource.TYPED, SendSource.entries.single { it.stored == "typed" })
        // Every source has its own stored string: history and the debug dump tell them apart by it.
        assertEquals(SendSource.entries.size, SendSource.entries.map { it.stored }.toSet().size)
    }

    @Test
    fun theWordsSayWhetherItWaits() {
        val connected = LinkState.Connected("PC", app.kinrelay.android.pairing.PairingAddress("192.168.1.2", 1))
        assertEquals("Text, links, files or photos", SendPlan.buttonSubtitle(connected))
        assertEquals("It will wait until your PC is back", SendPlan.buttonSubtitle(LinkState.Connecting()))
        assertEquals("It will wait until your PC is back", SendPlan.buttonSubtitle(LinkState.NotOnWifi))
        assertEquals("It will wait, see the card above", SendPlan.buttonSubtitle(LinkState.PairAgain))
        assertEquals("Not connected, check the PC card on Home", SendPlan.sheetStatus(LinkState.UpdateApp))
        assertEquals("Connected, it arrives in a moment", SendPlan.sheetStatus(connected))
        assertEquals("Not connected yet, it will wait", SendPlan.sheetStatus(LinkState.Connecting()))
        assertEquals("Not connected, check the PC card on Home", SendPlan.sheetStatus(LinkState.Stopped))
    }
}
