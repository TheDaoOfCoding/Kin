package app.kinrelay.android.data

import app.kinrelay.android.data.KinSettings.PreviousRun
import org.junit.Assert.assertEquals
import org.junit.Test

class PreviousRunTest {
    @Test
    fun aCleanStopIsClean() = assertEquals(PreviousRun.CLEAN, PreviousRun.classify(false, 7, 7))

    @Test
    fun runningWithoutACleanStopOnTheSameBootIsAKill() = assertEquals(PreviousRun.KILLED, PreviousRun.classify(true, 7, 7))

    @Test
    fun runningWithoutACleanStopAfterARebootIsNotAKill() = assertEquals(PreviousRun.REBOOT, PreviousRun.classify(true, 7, 8))

    @Test
    fun anUnknownEarlierBootCountCountsAsAKill() = assertEquals(PreviousRun.KILLED, PreviousRun.classify(true, null, 8))
}
