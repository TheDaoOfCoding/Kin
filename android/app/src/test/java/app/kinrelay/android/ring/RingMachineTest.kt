package app.kinrelay.android.ring

import app.kinrelay.android.testing.FakeTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The ring machine against fake ports that write one ordered log (design 5.4, 10.1): the alarm volume is **saved durably before it is raised** and put back on every stop and at every start after
 * a crash, the `ack` goes out before the `ring.status`, the 30 s auto-stop, the 2-minute cap, a ring that can't make a sound still acks, and the debug "silent" ring never touches the volume.
 * (The shared timelines of `protocol/presence.json` `ring[]` are in `PresenceVectorsTest`.)
 */
class RingMachineTest {
    private class Rig(val time: FakeTime = FakeTime()) {
        val log = ArrayList<String>()
        var volume = 2
        var max = 7
        var saved: Int? = null
        var fixed = false
        var silence = false
        var soundFails = false
        var allowed = true
        var loud = true
        var wake = 0
        val stopEnds = ArrayList<Long>()
        var refuseVolumeChanges = false
        var throwOnVolume = false

        val ports = object : RingPorts {
            override fun volumeFixed() = fixed
            override fun alarmVolume() = volume
            override fun alarmMax() = max
            override fun setAlarmVolume(index: Int) {
                if (throwOnVolume) throw SecurityException("blocked by Do Not Disturb")
                if (!refuseVolumeChanges) volume = index
                log += "volume=$index"
            }
            override fun startSound() { if (soundFails) throw IllegalStateException("no player"); log += "sound on" }
            override fun stopSound() { log += "sound off" }
            override fun totalSilence() = silence
            override fun showStop(pcName: String, endsInMillis: Long): Boolean { log += "stop screen for $pcName"; stopEnds += endsInMillis; return true }
            override fun hideStop() { log += "stop screen gone" }
            override fun holdAwake(on: Boolean) { wake += if (on) 1 else -1 }
            override fun savedIndex() = saved
            override fun saveIndex(index: Int) { saved = index; log += "saved=$index" }
            override fun clearSavedIndex() { saved = null; log += "saved cleared" }
            override fun announce(state: String, reason: String?, audible: Boolean?) { log += "status $state ${reason ?: audible}" }
        }
        val machine = RingMachine(ports, time, time, allowed = { allowed }, loud = { loud }, pcName = { "Desk PC" })

        fun start(): RingReply? {
            var reply: RingReply? = null
            machine.start { reply = it; log += "reply ${it}" }
            return reply
        }
    }

    @Test
    fun theVolumeIsSavedThenRaisedThenPutBackOnEveryStop() {
        val r = Rig()
        r.start()
        // Saved before raised (the order of the log is the order of the calls).
        assertEquals(listOf("saved=2", "volume=7", "sound on", "stop screen for Desk PC"), r.log.take(4))
        assertEquals(7, r.volume)
        r.time.advanceBy(30_000)
        assertEquals("put back after the timeout", 2, r.volume)
        assertNull(r.saved)
        assertTrue(r.log.indexOf("volume=2") > r.log.indexOf("sound off") - 1)
    }

    @Test
    fun theAckGoesOutBeforeTheStatusAndTheStatusSaysAudible() {
        val r = Rig()
        r.start()
        val ack = r.log.indexOf("reply ${RingReply.Ack}")
        val status = r.log.indexOf("status ringing true")
        assertTrue("ack at $ack, status at $status in ${r.log}", ack in 0 until status)
        // A stop from the PC acks first and then says it stopped something.
        r.log.clear()
        r.machine.stopFromPc { r.log += "reply" }
        assertEquals(listOf("sound off", "stop screen gone", "volume=2", "saved cleared", "reply", "status stopped pc"), r.log)
    }

    @Test
    fun aSecondStartRestartsThe30SecondsButNeverSavesTheRaisedVolume() {
        val r = Rig()
        r.start()
        r.time.advanceBy(20_000)
        r.start()
        assertEquals("the saved value is still the user's", 2, r.saved)
        r.time.advanceBy(29_999)
        assertTrue(r.machine.isRinging)
        r.time.advanceBy(1)
        assertFalse(r.machine.isRinging)
        assertEquals(2, r.volume)
    }

    @Test
    fun aRestartPostsTheStopNotificationAgainWithTheNewEndUpToTheCap() {
        val r = Rig()
        // A start every 25 s (never more than 3 in a minute): at 0, 25, 50, 75 and 100 s.
        r.start()
        repeat(4) {
            r.time.advanceBy(25_000)
            r.start()
        }
        assertEquals("every restart posts the Stop notification again; the last one is cut by the 2-minute cap (20 s left)", listOf(30_000L, 30_000L, 30_000L, 30_000L, 20_000L), r.stopEnds)
        assertTrue(r.machine.isRinging)
        r.time.advanceBy(20_000)
        assertFalse(r.machine.isRinging)
        assertEquals("limit", r.machine.lastStopReason)
    }

    @Test
    fun aStopThatFindsNothingRingingSaysNothingAndIsStillAcked() {
        val r = Rig()
        var replies = 0
        r.machine.stopFromPc { replies++ }
        assertEquals(1, replies)
        assertTrue("no status for a stop that stopped nothing: ${r.log}", r.log.none { it.startsWith("status") })
        r.machine.stopByUser()
        // Nothing rang, so nothing is announced: but the Stop notification (which may outlive a process that died mid-ring) is taken down all the same.
        assertEquals(listOf("stop screen gone"), r.log)
    }

    @Test
    fun theStopNotificationOfADeadProcessGoesAtTheNextStartAndOnItsOwnStopButton() {
        // 2B.4 review: after process death the notification stays and its Stop did nothing.
        val after = Rig(FakeTime()).also { it.saved = 2; it.volume = 7 }
        after.machine.restoreAtStart()
        assertTrue("restoreAtStart takes the Stop notification down: ${after.log}", after.log.contains("stop screen gone"))
        val idle = Rig()
        idle.machine.restoreAtStart()
        assertEquals("even with nothing saved", listOf("stop screen gone"), idle.log)
    }

    @Test
    fun underADoNotDisturbThatSilencesTheAlarmNothingIsSavedOrRaised() {
        val r = Rig().also { it.silence = true }
        r.start()
        assertEquals(2, r.volume)
        assertNull("nothing saved under total silence", r.saved)
        assertTrue(r.log.none { it.startsWith("volume") || it.startsWith("saved") })
        r.machine.stopByUser()
        assertTrue("and nothing to put back: ${r.log}", r.log.none { it.startsWith("volume") })
    }

    @Test
    fun theSavedVolumeIsClearedOnlyWhenTheVolumeReallyChangedBack() {
        // A restore that does not take (a Do Not Disturb that blocks the change): the saved value stays and the next start tries again.
        val r = Rig()
        r.start()
        assertEquals(7, r.volume)
        r.refuseVolumeChanges = true
        r.machine.stopByUser()
        assertEquals("the volume could not be changed back", 7, r.volume)
        assertEquals("the saved value is kept", 2, r.saved)
        r.refuseVolumeChanges = false
        assertTrue(r.machine.restoreAtStart())
        assertEquals(2, r.volume)
        assertNull(r.saved)
        // A refusal by exception keeps it too.
        val thrown = Rig()
        thrown.start()
        thrown.throwOnVolume = true
        thrown.machine.stopByUser()
        assertEquals(2, thrown.saved)
    }

    @Test
    fun aVolumeTheUserSetByHandAfterTheRaiseIsNotRestoredOver() {
        // 2B.5 review: Kin raised the alarm volume to 7; the user then set 4 themselves. The saved 2 must not replace it.
        val r = Rig()
        r.start()
        assertEquals(7, r.volume)
        r.volume = 4
        r.machine.stopByUser()
        assertEquals("the user's own volume stays", 4, r.volume)
        assertNull("and the saved value is dropped", r.saved)
        assertTrue("no write of the saved value: ${r.log}", r.log.none { it == "volume=2" })
        // The same at the next start (a crash in between): the volume is not Kin's raised one, so nothing is put back.
        val after = Rig(FakeTime()).also { it.saved = 2; it.volume = 4 }
        after.machine.restoreAtStart()
        assertEquals(4, after.volume)
        assertNull(after.saved)
    }

    @Test
    fun aReplacedTimerCannotStopTheRing() {
        val r = Rig()
        r.start()
        // A restart replaces the 30 s timer: the old one (already waiting for the lock) must find a newer generation and do nothing.
        r.time.advanceBy(20_000)
        r.start()
        r.time.advanceBy(10_000)
        assertTrue("the first timer (due at 30 s) must not stop the ring that was extended", r.machine.isRinging)
        r.time.advanceBy(20_000)
        assertFalse(r.machine.isRinging)
        assertEquals("timeout", r.machine.lastStopReason)
    }

    @Test
    fun aCrashMidRingIsPutRightAtTheNextStart() {
        val r = Rig()
        r.start()
        // The process dies: a new machine over the same durable ports starts with a saved index and nothing ringing.
        val after = Rig(FakeTime()).also { it.saved = 2; it.volume = 7 }
        assertTrue(after.machine.restoreAtStart())
        assertEquals(2, after.volume)
        assertNull(after.saved)
        assertFalse("nothing saved: nothing to do", after.machine.restoreAtStart())
    }

    @Test
    fun useMyAlarmVolumeTouchesNothing() {
        val r = Rig().also { it.loud = false }
        r.start()
        assertEquals(2, r.volume)
        assertTrue(r.log.none { it.startsWith("volume") || it.startsWith("saved") })
        r.machine.stopByUser()
        assertEquals(2, r.volume)
    }

    @Test
    fun aVolumeFixedDeviceKeepsItsVolumeAndAnAlreadyMaximumAlarmSavesNothing() {
        val fixed = Rig().also { it.fixed = true }
        fixed.start()
        assertTrue(fixed.log.none { it.startsWith("volume") || it.startsWith("saved") })
        val full = Rig().also { it.volume = 7 }
        full.start()
        assertNull("nothing to restore when it was already at maximum", full.saved)
    }

    @Test
    fun noSoundStillAcksAndSaysItIsNotAudible() {
        val r = Rig().also { it.soundFails = true }
        assertEquals(RingReply.Ack, r.start())
        assertTrue(r.log.contains("status ringing false"))
        val silent = Rig().also { it.silence = true }
        silent.start()
        assertTrue(silent.log.contains("status ringing false"))
    }

    @Test
    fun theSwitchOffAnswersDisabledAndStopsARingInProgress() {
        val r = Rig().also { it.allowed = false }
        assertEquals("disabled", (r.start() as RingReply.Error).code)
        assertTrue(r.log.none { it.startsWith("sound") })
        r.allowed = true
        r.start()
        r.allowed = false
        r.machine.allowedChanged()
        assertFalse(r.machine.isRinging)
        assertEquals("user", r.machine.lastStopReason)
    }

    @Test
    fun unlinkingStopsAndPutsTheVolumeBackWithNobodyToTell() {
        val r = Rig()
        r.start()
        r.log.clear()
        r.machine.onUnlink()
        assertFalse(r.machine.isRinging)
        assertEquals(2, r.volume)
        assertTrue("nothing is announced to a peer that is gone: ${r.log}", r.log.none { it.startsWith("status") })
    }

    @Test
    fun theWakeLockIsHeldOnlyWhileRingingAndThereIsNoPeriodicTimer() {
        val r = Rig()
        assertEquals(0, r.time.pending)
        r.start()
        assertEquals(1, r.wake)
        assertEquals("one timer: the 30 s", 1, r.time.pending)
        r.machine.stopByUser()
        assertEquals(0, r.wake)
        assertEquals(0, r.time.pending)
    }

    @Test
    fun theCountersHoldCountsOnly() {
        val r = Rig()
        r.start()
        r.time.advanceBy(30_000)
        val line = r.machine.stats.line()
        assertTrue(line, line.startsWith("ring: started 1"))
        assertTrue(line, line.contains("timeout: 1"))
    }
}
