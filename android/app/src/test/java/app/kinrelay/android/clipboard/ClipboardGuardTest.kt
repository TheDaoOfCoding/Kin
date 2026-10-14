package app.kinrelay.android.clipboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipboardGuardTest {
    private var now = 10_000L
    private val guard = ClipboardGuard({ now })

    @Test
    fun nothingIsAnEchoBeforeThePeerSetAnything() {
        assertFalse(guard.isEcho("anything"))
    }

    @Test
    fun theTextThePeerJustSetIsAnEcho() {
        guard.notePeerSet("from the PC")
        assertTrue(guard.isEcho("from the PC"))
        assertFalse(guard.isEcho("from the PC "))
        assertFalse(guard.isEcho("From the PC"))
        assertFalse(guard.isEcho(""))
    }

    @Test
    fun onlyTheNewestPeerTextCounts() {
        guard.notePeerSet("first")
        now += 1_000
        guard.notePeerSet("second")
        assertFalse(guard.isEcho("first"))
        assertTrue(guard.isEcho("second"))
    }

    @Test
    fun itForgetsAfterTheWindow() {
        guard.notePeerSet("x")
        now += ClipboardGuard.WINDOW_MILLIS
        assertTrue("still inside at the boundary", guard.isEcho("x"))
        now += 1
        assertFalse(guard.isEcho("x"))
    }

    @Test
    fun aClockThatMovedBackIsNotAnEcho() {
        guard.notePeerSet("x")
        now -= 5_000
        assertFalse(guard.isEcho("x"))
    }

    @Test
    fun theHashIsTheSha256OfTheUtf8TextAsTheWireCarriesIt() {
        // sha256("abc")
        assertEquals("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", ClipboardGuard.hash("abc"))
        assertEquals(64, ClipboardGuard.hash("é").length)
        assertNotEquals(ClipboardGuard.hash("é"), ClipboardGuard.hash("e"))
        // An unpaired surrogate goes out as U+FFFD, so both forms are the same text on the wire.
        assertEquals(ClipboardGuard.hash("a�b"), ClipboardGuard.hash("a\uD800b"))
    }

    @Test
    fun theFingerprintHasALengthAndAShortHashButNeverTheText() {
        val text = "my secret password 😀"
        val fingerprint = ClipboardGuard.fingerprint(text)
        assertEquals("len=20 sha=${ClipboardGuard.hash(text).take(8)}", fingerprint)
        assertFalse(fingerprint.contains("secret"))
        assertFalse(guard.describe().contains("secret"))
    }

    @Test
    fun describeIsNoneUntilSomethingWasSet() {
        assertEquals("none", guard.describe())
        guard.notePeerSet("x")
        assertTrue(guard.describe().startsWith("sha="))
    }
}
