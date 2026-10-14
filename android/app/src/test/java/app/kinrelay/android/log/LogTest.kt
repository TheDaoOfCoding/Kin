package app.kinrelay.android.log

import app.kinrelay.android.link.Backoff
import java.io.File
import java.nio.file.Files
import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LogTest {
    private fun dir(): File = Files.createTempDirectory("kin-log").toFile().also { it.deleteOnExit() }

    @Test
    fun redactsPairingTokensAndPrivateKeys() {
        assertEquals("kinrelay://pair?v=1&tk=[redacted]&n=PC", Redactor.redact("kinrelay://pair?v=1&tk=AAECAwQFBgcICQoLDA0ODw&n=PC"))
        assertEquals("""body {"token":"[redacted]","name":"x"}""", Redactor.redact("""body {"token":"AAECAwQFBgcICQoLDA0ODw","name":"x"}"""))
        assertEquals("token=[redacted]", Redactor.redact("token=abc123"))
        val pem = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMG\n-----END PRIVATE KEY-----"
        assertEquals("key: [redacted private key]", Redactor.redact("key: $pem"))
        assertEquals("device ID yetOwJHO-3liWauefLv2Zs8_NNIkRYvy7_i5RBG-fP8 stays", Redactor.redact("device ID yetOwJHO-3liWauefLv2Zs8_NNIkRYvy7_i5RBG-fP8 stays"))
    }

    @Test
    fun theFileLogRedactsWhatItWrites() {
        val d = dir()
        val log = FileLog(d, now = { Instant.parse("2026-10-14T12:00:00Z") })
        log.i("pairing", "link kinrelay://pair?tk=AAECAwQFBgcICQoLDA0ODw")
        val text = File(d, "kin.log").readText()
        assertEquals("2026-10-14T12:00:00Z I pairing: link kinrelay://pair?tk=[redacted]\n", text)
        assertFalse(text.contains("AAECAwQ"))
    }

    @Test
    fun rotatesWhenFullAndKeepsOnePreviousFile() {
        val d = dir()
        val log = FileLog(d, maxBytes = 200)
        repeat(30) { log.i("t", "line number $it with some padding") }
        assertTrue(File(d, "kin.log").length() < 400)
        assertTrue(File(d, "kin.log.1").exists())
        assertFalse(File(d, "kin.log.2").exists())
        assertTrue(log.recent.value.last().endsWith("line number 29 with some padding"))
    }

    @Test
    fun recentLinesSurviveARestart() {
        val d = dir()
        FileLog(d).apply { i("a", "one"); w("a", "two") }
        val again = FileLog(d)
        assertEquals(2, again.recent.value.size)
        assertTrue(again.recent.value[1].contains(" W a: two"))
    }

    @Test
    fun backoffStepsAreFixed() {
        val backoff = Backoff()
        assertEquals(listOf(5_000L, 30_000, 120_000, 600_000, 600_000, 600_000), List(6) { backoff.next() })
        backoff.reset()
        assertEquals(5_000L, backoff.next())
    }
}
