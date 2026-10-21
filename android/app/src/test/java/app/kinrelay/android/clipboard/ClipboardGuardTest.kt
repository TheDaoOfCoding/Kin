package app.kinrelay.android.clipboard

import app.kinrelay.android.log.LogFingerprint
import app.kinrelay.android.protocol.Frames
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
    fun theFingerprintHasALengthAndAShortKeyedHashButNeverTheText() {
        val text = "my secret password 😀"
        val key = ByteArray(32) { (it * 7).toByte() }
        LogFingerprint.install(key)
        try {
            val fingerprint = ClipboardGuard.fingerprint(text)
            assertEquals("len=20 sha=${LogFingerprint.mac(key, Frames.wireText(text))}", fingerprint)
            assertFalse("not the unkeyed hash, which a dictionary of guesses could be run against", fingerprint.contains(ClipboardGuard.hash(text).take(8)))
            assertFalse(fingerprint.contains("secret"))
            assertFalse(guard.describe().contains("secret"))
            // Another install has another key: the same text has another fingerprint.
            LogFingerprint.install(ByteArray(32) { (it * 11 + 1).toByte() })
            assertNotEquals(fingerprint, ClipboardGuard.fingerprint(text))
        } finally {
            LogFingerprint.install(null)
        }
        // With no key yet only the length is logged.
        assertEquals("len=20", ClipboardGuard.fingerprint(text))
    }

    @Test
    fun theKeyedHashIsAnHmacSha256CutToFourBytes() {
        // RFC 4231 test case 2: HMAC-SHA256(key "Jefe", "what do ya want for nothing?") = 5bdcc146bf60754e6a042426089575c7...
        assertEquals("5bdcc146", LogFingerprint.mac("Jefe".toByteArray(), "what do ya want for nothing?"))
    }

    @Test
    fun theHashIsOfTheWireFormSoCrlfAndLfClipboardsMatchWhatThePcSent() {
        // The PC sent LF-only text (the wire form); a clipboard that holds the CRLF form of it is the same text.
        guard.notePeerSet("a\nb")
        assertTrue(guard.isEcho("a\r\nb"))
        assertTrue(guard.isEcho("a\nb"))
        // A received text is hashed as received, never converted a second time: "\r\n" in it stays a CRLF.
        guard.notePeerSet("a\r\r\nb".replace("\r\r\n", "\r\n"))
        assertFalse(guard.isEcho("a\nb"))
    }

    @Test
    fun describeIsNoneUntilSomethingWasSet() {
        assertEquals("none", guard.describe())
        guard.notePeerSet("x")
        assertTrue(guard.describe().startsWith("set ageMs="))
    }
}
