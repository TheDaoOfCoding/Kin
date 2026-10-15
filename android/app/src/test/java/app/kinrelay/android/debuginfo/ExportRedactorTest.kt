package app.kinrelay.android.debuginfo

import app.kinrelay.android.log.LogKeyStore
import app.kinrelay.android.identity.KeyWrapper
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files
import java.security.SecureRandom
import java.util.zip.ZipInputStream

/** What the exported logs lose on their way out (the same rules as the desktop's export), and the key of the log fingerprints. */
class ExportRedactorTest {
    private val dir = Files.createTempDirectory("kin-export").toFile()
    private val deviceId = "TLcGVnFmd5AvIonGAsfAzkBO4LGSN9M-PQuLcPBm4SY" // 43 characters, as a real device ID

    @After
    fun tearDown() {
        dir.deleteRecursively()
    }

    private val redactor = ExportRedactor(pcNames = listOf("Desk PC"), phoneNames = listOf("Alice's Pixel", "Pixel 8"))

    @Test
    fun theLanAddressOfThePcIsRemovedWithItsPort() {
        val line = "2026-10-15T10:00:00.000Z I link: connected to 'Desk PC' via 192.168.1.42:47821; fallback 10.0.2.2"
        val out = redactor.apply(line)
        assertFalse(out, out.contains("192.168") || out.contains("47821") || out.contains("10.0.2.2"))
        assertEquals("2026-10-15T10:00:00.000Z I link: connected to '<pc>' via <ip>; fallback <ip>", out)
    }

    @Test
    fun aBracketedIpv6AddressIsRemovedToo() {
        assertEquals("via <ip> and <ip>", redactor.apply("via [fe80::1c2b:3d4e%wlan0]:47821 and [2001:db8::1]"))
    }

    @Test
    fun theNamesOfThePcAndThePhoneAreReplacedInAnyCase() {
        val out = redactor.apply("Sent to DESK PC from Alice's Pixel (a Pixel 8); desk pc replied")
        assertEquals("Sent to <pc> from <phone> (a <phone>); <pc> replied", out)
    }

    @Test
    fun aPcFromAnEarlierPairingIsFoundInTheLogLinesThatNameIt() {
        val lines = listOf(
            "I link: connected to 'Old Laptop' via 10.0.0.9:47821",
            "I link: text from 'Old Laptop' len=5 sha=1a2b3c4d isUrl=false",
            "I link: unpairing from 'Old Laptop'",
            "I pairing: paired with Brand New PC (TLcGVnFm…) via 192.168.0.5:47821",
        )
        val out = redactor.apply(lines.joinToString("\n"))
        assertFalse(out, out.contains("Old Laptop") || out.contains("Brand New PC") || out.contains("192.168") || out.contains("10.0.0.9"))
        assertTrue(out.contains("text from '<pc>' len=5 sha=1a2b3c4d"))
        assertTrue(out.contains("paired with <pc> (TLcGVnFm…) via <ip>"))
    }

    @Test
    fun aFullDeviceIdIsCutToItsFirstEightCharacters() {
        val out = redactor.apply("I identity: device $deviceId and again ($deviceId)")
        assertEquals("I identity: device TLcGVnFm… and again (TLcGVnFm…)", out)
        assertFalse(out.contains(deviceId))
        // A short ID, a UUID and a version number are not device IDs.
        val kept = "short TLCGVNFM, message 0193a5c2-7b1e-7c3a-9d4f-5e6a7b8c9d32, app 0.1.0, Android API 37"
        assertEquals(kept, redactor.apply(kept))
    }

    @Test
    fun theEchoLinesOfTheDebugHooksAreLeftOut() {
        val log = "2026-10-15T10:00:00.000Z I debug: CMD TRANSFER t1 up done 10/10 files=1 names=holiday photos.jpg err=null\n" +
            "2026-10-15T10:00:01.000Z I debug: logs exported: kin-logs-1.zip, 1000 bytes\n" +
            "2026-10-15T10:00:02.000Z I link: connected"
        val out = redactor.apply(log)
        assertFalse(out, out.contains("holiday photos"))
        assertEquals("2026-10-15T10:00:01.000Z I debug: logs exported: kin-logs-1.zip, 1000 bytes\n2026-10-15T10:00:02.000Z I link: connected", out)
    }

    @Test
    fun tokensAndKeysAreStillRemovedFirst() {
        val out = redactor.apply("""url kinrelay://pair?v=1&a=10.0.2.2:47831&tk=AAECAwQFBgcICQoLDA0ODw {"token":"abc"}""")
        assertFalse(out.contains("AAECAwQFBgcICQoLDA0ODw") || out.contains("abc") || out.contains("10.0.2.2"))
    }

    @Test
    fun shortOrEmptyNamesAreIgnoredSoEveryLetterIsNotRedacted() {
        val r = ExportRedactor(pcNames = listOf("", " ", "x"), phoneNames = emptyList())
        assertEquals("x marks the spot", r.apply("x marks the spot"))
    }

    @Test
    fun theZipHasTheRedactedFilesOnlyAndNothingElse() {
        // What LogExport does: every file goes through the redactor on its way into the zip.
        val log = "I link: connected to 'Desk PC' via 192.168.1.42:47821 device $deviceId"
        val bytes = LogZip.toBytes(listOf("kin.log" to redactor.apply(log).toByteArray(), "state.txt" to redactor.apply("pc=Desk PC").toByteArray()))
        val entries = mutableMapOf<String, String>()
        ZipInputStream(bytes.inputStream()).use { zip ->
            while (true) {
                val e = zip.nextEntry ?: break
                entries[e.name] = zip.readBytes().toString(Charsets.UTF_8)
            }
        }
        assertEquals(setOf("kin.log", "state.txt"), entries.keys)
        for ((name, content) in entries) {
            assertFalse("$name: $content", content.contains("192.168") || content.contains("Desk PC") || content.contains(deviceId))
        }
    }

    // ---------------------------------------------------------------- the key of the log fingerprints

    /** A wrapper that XORs: enough to see that what is on disk is not the key. */
    private class XorWrapper(private val mask: Byte = 0x5A) : KeyWrapper {
        override fun encrypt(plain: ByteArray) = byteArrayOf(1) + plain.map { (it.toInt() xor mask.toInt()).toByte() }
        override fun decrypt(wrapped: ByteArray): ByteArray {
            check(wrapped.isNotEmpty() && wrapped[0] == 1.toByte()) { "not ours" }
            return wrapped.drop(1).map { (it.toInt() xor mask.toInt()).toByte() }.toByteArray()
        }
    }

    @Test
    fun theLogKeyIsRandomPerInstallStoredWrappedAndStable() {
        val file = File(dir, "log-key.bin")
        val first = LogKeyStore(file, XorWrapper(), SecureRandom()).loadOrCreate()
        assertEquals(32, first.size)
        assertFalse("the key is not on disk in clear", file.readBytes().toList().windowed(32).any { it == first.toList() })
        assertArrayEquals("the same install gets the same key every time", first, LogKeyStore(file, XorWrapper(), SecureRandom()).loadOrCreate())
        val otherInstall = LogKeyStore(File(dir, "other.bin"), XorWrapper(), SecureRandom()).loadOrCreate()
        assertFalse("another install has another key", first.contentEquals(otherInstall))
    }

    @Test
    fun aLogKeyThatCantBeReadIsReplacedNotFatal() {
        val file = File(dir, "log-key.bin")
        val first = LogKeyStore(file, XorWrapper(), SecureRandom()).loadOrCreate()
        // The wrapping key is gone (a wiped Keystore): decrypting fails, and a new key takes the place.
        val broken = object : KeyWrapper {
            override fun encrypt(plain: ByteArray) = XorWrapper().encrypt(plain)
            override fun decrypt(wrapped: ByteArray): ByteArray = throw java.security.GeneralSecurityException("key gone")
        }
        val second = LogKeyStore(file, broken, SecureRandom()).loadOrCreate()
        assertEquals(32, second.size)
        assertNotEquals(first.toList(), second.toList())
        assertArrayEquals(second, LogKeyStore(file, XorWrapper(), SecureRandom()).loadOrCreate())
    }
}
