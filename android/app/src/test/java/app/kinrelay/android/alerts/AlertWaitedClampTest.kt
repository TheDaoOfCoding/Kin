package app.kinrelay.android.alerts

import app.kinrelay.android.protocol.StrictJson
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** 2B.5 review: a `waitedMs` written beyond 2^53-1 is cut to 7 days (it is a very long wait), not refused. The vectors only reach exactly 2^53-1. */
class AlertWaitedClampTest {
    private fun parse(waited: String): AlertWire.Parsed =
        AlertWire.parse(StrictJson.parse("""{"type":"alert","id":"0193a5f0-7b1e-7c3a-9d4f-5e6a7b8c0800","text":"x","waitedMs":$waited}""").jsonObject)

    private fun waitedOf(waited: String): Long? = (parse(waited) as AlertWire.Parsed.Ok).alert.waitedMs

    @Test
    fun beyondTheSafeIntegerIsCutToSevenDays() {
        assertEquals(AlertWire.WAITED_MAX_MILLIS, waitedOf("9007199254740992"))
        assertEquals(AlertWire.WAITED_MAX_MILLIS, waitedOf("99999999999999999999999"))
        assertEquals("the vectors' own case is unchanged", AlertWire.WAITED_MAX_MILLIS, waitedOf("9007199254740991"))
        assertEquals(1_000L, waitedOf("1000"))
    }

    @Test
    fun otherBadShapesAreStillRefused() {
        for (bad in listOf("-1", "1.5", "1e3", "1.0", "\"5\"", "[1]", "{\"a\":1}", "true", "-99999999999999999999", "1e30")) {
            assertTrue("$bad is refused", parse(bad) is AlertWire.Parsed.Bad)
        }
    }
}
