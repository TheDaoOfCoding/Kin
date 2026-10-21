package app.kinrelay.android.alerts

import app.kinrelay.android.log.KinLog
import app.kinrelay.android.presence.Battery
import app.kinrelay.android.presence.PcBatteryNudge
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the phone does with an acked `alert` (design 4.4 of 2B.5, 8): a history row **always**, a notification unless the phone is paused (quiet: history and a count only), nothing on the clipboard,
 * and **no alert text, title or Focus/DND history in any log line** (T1). Also the PC's low battery nudge on the phone.
 */
class AlertInboundTest {
    private class RecordingLog : KinLog {
        val lines = ArrayList<String>()
        override fun log(level: Char, component: String, message: String) {
            lines += "$level $component $message"
        }
    }

    private class Rig(var paused: Boolean = false) {
        val calls = ArrayList<String>()
        val log = RecordingLog()
        var quiet = 0
        var recordFails = false
        var showFails = false
        var denied = false
        val inbound = AlertInbound(
            object : AlertPorts {
                override suspend fun record(id: String, pcName: String, alert: AlertMessage) {
                    if (recordFails) throw IllegalStateException("database")
                    calls += "record $id ${alert.title}|${alert.text}"
                }

                override fun show(pcName: String, alert: AlertMessage): Boolean {
                    if (showFails) throw IllegalStateException("notification")
                    calls += "show $pcName ${alert.title}|${alert.text}"
                    return !denied
                }
            },
            paused = { paused }, onQuiet = { quiet++ }, log = log,
        )
    }

    @Test
    fun anAlertIsRecordedThenShown() = runBlocking {
        val r = Rig()
        r.inbound.onAlert("a1", "Desk PC", AlertMessage("Scene 12 finished", "Render", 7_200_000))
        assertEquals(listOf("record a1 Render|Scene 12 finished", "show Desk PC Render|Scene 12 finished"), r.calls)
        assertEquals(1, r.inbound.stats.shown.get())
        assertEquals(0, r.quiet)
    }

    @Test
    fun anAlertThatCouldNotBePostedBecauseTheNotificationsAreDeniedIsNotCountedAsShown() = runBlocking {
        val r = Rig().also { it.denied = true }
        r.inbound.onAlert("a1b", "Desk PC", AlertMessage("x", null, null))
        assertEquals(listOf("record a1b null|x", "show Desk PC null|x"), r.calls)
        assertEquals(0, r.inbound.stats.shown.get())
        assertEquals(1, r.inbound.stats.received.get())
    }

    @Test
    fun whilePausedItIsAHistoryRowAndACountAndNothingElse() = runBlocking {
        val r = Rig(paused = true)
        r.inbound.onAlert("a2", "Desk PC", AlertMessage("Done", null, null))
        assertEquals("history only: ${r.calls}", listOf("record a2 null|Done"), r.calls)
        assertEquals(1, r.quiet)
        assertEquals(1, r.inbound.stats.quiet.get())
        assertEquals(0, r.inbound.stats.shown.get())
    }

    @Test
    fun aFailingHistoryRowOrNotificationDoesNotLoseTheOtherOne() = runBlocking {
        val noHistory = Rig().also { it.recordFails = true }
        noHistory.inbound.onAlert("a3", "Desk PC", AlertMessage("x", null, null))
        assertEquals("the notification still goes", listOf("show Desk PC null|x"), noHistory.calls)
        val noNotification = Rig().also { it.showFails = true }
        noNotification.inbound.onAlert("a4", "Desk PC", AlertMessage("y", null, null))
        assertEquals(listOf("record a4 null|y"), noNotification.calls)
    }

    @Test
    fun noLogLineHoldsAnAlertTextOrTitle() = runBlocking {
        val secret = "SECRET-TEXT-9f3a"
        val title = "SECRET-TITLE-77b1"
        for (paused in listOf(false, true)) {
            val r = Rig(paused)
            r.inbound.onAlert("a5", "Desk PC", AlertMessage("$secret and more", title, 4_000_000))
            r.recordFails = true
            r.inbound.onAlert("a6", "Desk PC", AlertMessage(secret, title, null))
            r.showFails = true
            r.inbound.onAlert("a7", "Desk PC", AlertMessage(secret, title, null))
            val everything = r.log.lines.joinToString("\n")
            assertFalse("paused=$paused: the text is in a log line:\n$everything", everything.contains(secret))
            assertFalse("paused=$paused: the title is in a log line:\n$everything", everything.contains(title))
            assertTrue("counts and lengths are logged: $everything", everything.contains("alerts: received"))
        }
        assertTrue("the counts line is numbers only", AlertStats().line().matches(Regex("""alerts: received \d+, shown \d+, quiet \d+""")))
    }

    // ------------------------------------------------------------------ the PC's low battery

    @Test
    fun theNudgeFiresOncePerDischargeEpisodeAtFifteenOrLessAndNotCharging() {
        val n = PcBatteryNudge()
        assertNull(n.onPcBattery("pc", Battery(40, false), showOn = true, paused = false))
        assertNull(n.onPcBattery("pc", Battery(16, false), showOn = true, paused = false))
        assertEquals(15, n.onPcBattery("pc", Battery(15, false), showOn = true, paused = false))
        assertNull("one per episode", n.onPcBattery("pc", Battery(14, false), showOn = true, paused = false))
        assertNull(n.onPcBattery("pc", Battery(19, false), showOn = true, paused = false))
        assertNull("armed again only at 20", n.onPcBattery("pc", Battery(14, false), showOn = true, paused = false))
        n.onPcBattery("pc", Battery(20, false), showOn = true, paused = false)
        assertEquals(14, n.onPcBattery("pc", Battery(14, false), showOn = true, paused = false))
        n.onPcBattery("pc", Battery(30, true), showOn = true, paused = false)
        assertEquals("charging re-armed it", 10, n.onPcBattery("pc", Battery(10, false), showOn = true, paused = false))
        assertNull("charging at 10 never nudges", PcBatteryNudge().onPcBattery("pc", Battery(10, true), showOn = true, paused = false))
        assertNull("a desktop has no battery", n.onPcBattery("pc", null, showOn = true, paused = false))
        assertEquals("Desk PC battery is 14%", PcBatteryNudge.title("Desk PC", 14))
    }

    @Test
    fun aNudgeThatIsNotWantedIsNotLostAndDoesNotDisarm() {
        val n = PcBatteryNudge()
        assertNull("the PC's status is not shown", n.onPcBattery("pc", Battery(14, false), showOn = false, paused = false))
        assertNull("this phone is paused", n.onPcBattery("pc", Battery(13, false), showOn = true, paused = true))
        assertEquals("wanted again: it comes now", 12, n.onPcBattery("pc", Battery(12, false), showOn = true, paused = false))
        assertNull(PcBatteryNudge().also { it.forget("pc") }.onPcBattery("other", Battery(90, false), showOn = true, paused = false))
    }

    @Test
    fun eachPcHasItsOwnMemoryAndUnlinkingForgetsIt() {
        val n = PcBatteryNudge()
        assertEquals(10, n.onPcBattery("a", Battery(10, false), showOn = true, paused = false))
        assertEquals(10, n.onPcBattery("b", Battery(10, false), showOn = true, paused = false))
        assertNull(n.onPcBattery("a", Battery(9, false), showOn = true, paused = false))
        n.forget("a")
        assertEquals("a forgotten PC starts armed", 9, n.onPcBattery("a", Battery(9, false), showOn = true, paused = false))
    }
}
