package app.kinrelay.android.debuginfo

import java.io.ByteArrayInputStream
import java.util.zip.ZipInputStream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DebugInfoTest {
    private val lines = listOf(
        "2026-10-15T10:00:00.000Z I app: process started, version 0.1.0, sdk 37",
        "2026-10-15T10:00:01.000Z I service: service started",
        "2026-10-15T10:00:02.500Z I link: websocket open to 10.0.2.2:47831",
        "2026-10-15T10:00:02.600Z I link: connected to 'Desk PC' via 10.0.2.2:47831",
        "2026-10-15T10:05:00.000Z W link: disconnected: closed by the peer: 1001 'going away' (code 1001) after 297400 ms",
        "2026-10-15T10:05:05.000Z I link: Wi-Fi lost: 123",
        "2026-10-15T10:06:00.000Z W service: kill detected: the previous run ended without a clean stop",
        "2026-10-15T10:07:00.000Z I service: service stopped",
        "not a log line",
    )

    @Test
    fun theConnectionHistoryIsReadFromTheLogNewestFirst() {
        val events = ConnectionHistory.parse(lines)
        assertEquals(
            listOf(
                ConnectionEvent.Kind.SERVICE_STOPPED, ConnectionEvent.Kind.KILL_DETECTED, ConnectionEvent.Kind.DISCONNECTED,
                ConnectionEvent.Kind.CONNECTED, ConnectionEvent.Kind.SERVICE_STARTED,
            ),
            events.map { it.kind },
        )
        assertTrue(events.first { it.kind == ConnectionEvent.Kind.DISCONNECTED }.detail.contains("going away"))
        assertEquals("2026-10-15T10:00:02.600Z", events.first { it.kind == ConnectionEvent.Kind.CONNECTED }.time)
    }

    @Test
    fun theConnectionHistoryIsCapped() {
        val many = (1..100).map { "2026-10-15T10:00:%02d.000Z I link: connected to 'Desk PC' via x".format(it % 60) }
        assertEquals(40, ConnectionHistory.parse(many).size)
        assertEquals(5, ConnectionHistory.parse(many, limit = 5).size)
    }

    @Test
    fun latencyIsLastAndMedian() {
        assertNull(LatencySummary.of(emptyList()).median)
        assertEquals(0, LatencySummary.of(emptyList()).count)
        val odd = LatencySummary.of(listOf(10.0, 50.0, 30.0))
        assertEquals(30.0, odd.median!!, 0.0)
        assertEquals(30.0, odd.last!!, 0.0)
        val even = LatencySummary.of(listOf(10.0, 20.0, 30.0, 40.0))
        assertEquals(25.0, even.median!!, 0.0)
        assertEquals(4, even.count)
    }

    private fun input() = StateDump.Input(
        appVersion = "0.1.0", sdk = 37, manufacturer = "Google", model = "sdk_gphone64_x86_64", linkState = "Connected", paired = true,
        phoneShortId = "AAAA-BBBB", pcShortId = "CCCC-DDDD", connects = 3, disconnects = 2, kills = 1, successRate = "99.0% (99 of 100)",
        rtt = LatencySummary(12.0, 15.0, 4), arrival = LatencySummary(40.0, 38.0, 4), queued = 2,
        recentErrors = listOf("connection_lost" to "2026-10-15T10:05:00Z"), notificationsAllowed = true, localNetworkAllowed = true,
        batteryUnrestricted = false, oemGuide = "xiaomi",
    )

    @Test
    fun theStateDumpHasTheNumbersAndNoSecretsOrTexts() {
        val dump = StateDump.build(input(), "2026-10-15T12:00:00Z")
        assertTrue(dump.contains("first-try success: 99.0% (99 of 100)"))
        assertTrue(dump.contains("connects=3 disconnects=2 service kills=1"))
        assertTrue(dump.contains("connection_lost"))
        assertTrue(dump.contains("queued items=2"))
        assertTrue(dump.contains("oem=xiaomi"))
        // The input type has no field for a key, a token, a text or a file name; and a token that did get into a string would be masked.
        val leaky = StateDump.build(input().copy(linkState = "link kinrelay://pair?v=1&tk=SECRETTOKEN&id=x"), "now")
        assertFalse(leaky.contains("SECRETTOKEN"))
        assertTrue(leaky.contains("[redacted]"))
    }

    @Test
    fun theZipHasTheLogsAndTheDumpAndNothingElse() {
        val bytes = LogZip.toBytes(
            listOf("state.txt" to "dump".toByteArray(), "kin.log.1" to "old".toByteArray(), "kin.log" to "new".toByteArray()),
        )
        val seen = LinkedHashMap<String, String>()
        ZipInputStream(ByteArrayInputStream(bytes)).use { zip ->
            while (true) {
                val entry = zip.nextEntry ?: break
                seen[entry.name] = zip.readBytes().toString(Charsets.UTF_8)
            }
        }
        assertEquals(listOf("state.txt", "kin.log.1", "kin.log"), seen.keys.toList())
        assertEquals("new", seen["kin.log"])
    }
}
