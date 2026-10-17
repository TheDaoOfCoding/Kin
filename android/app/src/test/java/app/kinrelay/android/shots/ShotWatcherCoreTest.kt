package app.kinrelay.android.shots

import app.kinrelay.android.link.Clock
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The screenshot watcher's logic over fake ports and virtual time (design 5.6, 10.1): the baseline taken when the feature is turned on, the 500 ms coalescing, the 5-second hold with its Cancel,
 * what a pause and a missing connection do, the permission checked at the point of use, the rate bucket, and that the baseline only goes up.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ShotWatcherCoreTest {
    private class Ports(val scope: TestScope) : ShotPorts {
        var readable = true
        var ready = true
        var maxId = 100L
        var nowSec = 1_000L
        var uploadResult = ShotUpload.SENT
        var holdVisible = true

        /** When set, an upload suspends until it is completed (a long upload in progress). */
        var uploadGate: CompletableDeferred<Unit>? = null
        val rows = ArrayList<ShotRow>()
        val queries = ArrayList<Long>()
        val shown = ArrayList<Long>()
        val shownKinds = ArrayList<ShotKind>()
        val sending = ArrayList<Long>()
        val results = ArrayList<Pair<Long, String?>>()
        val uploads = ArrayList<Pair<Long, ShotKind>>()
        val savedBaselines = ArrayList<Long>()
        val savedTimes = ArrayList<Long>()

        override fun mayRead() = readable
        override fun maxId() = maxId

        // Like MediaStore with the ports' limit: at most one page of the rows above the id, oldest first.
        override fun queryNewer(afterId: Long): List<ShotRow> {
            queries += afterId
            return rows.filter { it.id > afterId }.sortedBy { it.id }.take(ShotWatcherCore.PAGE)
        }
        override fun nowSeconds() = nowSec
        override fun readyForUpload() = ready
        override fun showHold(id: Long, kind: ShotKind): Boolean {
            if (!holdVisible) return false
            shown += id
            shownKinds += kind
            return true
        }
        override fun showSending(id: Long) { sending += id }
        override fun showResult(id: Long, text: String?) { results += id to text }
        override suspend fun upload(row: ShotRow, kind: ShotKind): ShotUpload {
            uploads += row.id to kind
            uploadGate?.await()
            return uploadResult
        }
        override fun saveBaseline(id: Long) { savedBaselines += id }
        override fun saveBaselineTime(seconds: Long) { savedTimes += seconds }

        fun shot(id: Long, path: String = "Pictures/Screenshots/", pending: Boolean = false, mime: String = "image/png", takenMillis: Long = 0) =
            ShotRow(id, path, mime, pending, nowSec, 50_000, if (path.contains("Screenshots")) "com.android.systemui" else null, takenMillis).also { rows += it }
    }

    private class Switches(var screenshots: Boolean = true, var camera: Boolean = false, var paused: Boolean = false)

    private fun TestScope.rig(
        baseline: Long = 100,
        sw: Switches = Switches(),
        configure: (Ports) -> Unit = {},
    ): Triple<ShotWatcherCore, Ports, Switches> {
        val ports = Ports(this).also(configure)
        val clock = Clock { currentTime }
        val core = ShotWatcherCore(this, clock, ports, settings = { ShotSettings(sw.screenshots, sw.camera) }, paused = { sw.paused }, initialBaseline = baseline)
        return Triple(core, ports, sw)
    }

    @Test
    fun aNewScreenshotWaitsFiveSecondsThenGoesAndTheBaselineMovesUp() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(499)
        assertTrue("coalesced for 500 ms: nothing yet", ports.queries.isEmpty())
        advanceTimeBy(1)
        runCurrent()
        assertEquals(listOf(105L), ports.shown)
        advanceTimeBy(4_999)
        assertTrue("held for the whole 5 s", ports.uploads.isEmpty())
        advanceTimeBy(1)
        runCurrent()
        assertEquals(listOf(105L to ShotKind.SCREENSHOT), ports.uploads)
        assertEquals(105L, core.baseline)
        assertEquals(listOf(105L), ports.savedBaselines)
        assertEquals(105L to "Sent", ports.results.last())
        assertEquals(1, core.stats.sent.get())
    }

    @Test
    fun cancelDuringTheHoldSendsNothingAndTheShotNeverComesBack() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        advanceTimeBy(2_000)
        core.cancel(105)
        runCurrent()
        advanceTimeBy(10_000)
        runCurrent()
        assertTrue(ports.uploads.isEmpty())
        assertEquals("the notification is taken away", 105L to null, ports.results.last())
        assertEquals(105L, core.baseline)
        assertEquals(1, core.stats.cancelled.get())
        // Another signal: the same row is old now.
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(listOf(105L), ports.shown)
        assertTrue(ports.uploads.isEmpty())
    }

    @Test
    fun whatExistedWhenTheFeatureWasTurnedOnIsNeverSentAndTurningItOnTakesTheBaseline() = runTest {
        val (core, ports, _) = rig(baseline = 0) { it.maxId = 100 }
        ports.shot(50)
        ports.shot(100)
        core.retakeBaseline()
        assertEquals(100L, core.baseline)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertTrue("existing rows are old", ports.shown.isEmpty())
        assertEquals("the query starts above the baseline", listOf(100L), ports.queries)
    }

    @Test
    fun aPauseSkipsAndMovesTheBaselineSoItNeverComesBackAfterTheResume() = runTest {
        val (core, ports, sw) = rig()
        sw.paused = true
        ports.shot(105)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertTrue(ports.shown.isEmpty())
        assertEquals(105L, core.baseline)
        assertEquals(1, core.stats.skipped("paused"))
        sw.paused = false
        core.onChange()
        advanceTimeBy(10_600)
        runCurrent()
        assertTrue("taken while paused: never sent after the pause", ports.uploads.isEmpty())
    }

    @Test
    fun aPauseThatBeginsDuringTheHoldDropsIt() = runTest {
        val (core, ports, sw) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        sw.paused = true
        advanceTimeBy(5_000)
        runCurrent()
        assertTrue(ports.uploads.isEmpty())
        assertEquals(1, core.stats.skipped("paused"))
        assertEquals(105L, core.baseline)
    }

    @Test
    fun noReadyConnectionAfterTheHoldIsDroppedCountedAndNeverQueued() = runTest {
        val (core, ports, _) = rig { it.ready = false }
        ports.shot(105)
        core.onChange()
        advanceTimeBy(5_500)
        runCurrent()
        assertTrue("no upload was even started", ports.uploads.isEmpty())
        assertEquals(1, core.stats.skipped("offline"))
        assertEquals(105L to "Not sent: PC away", ports.results.last())
        assertEquals(105L, core.baseline)
        // The same upload answered "offline" by the PC side is counted the same way.
        ports.ready = true
        ports.uploadResult = ShotUpload.OFFLINE
        ports.shot(106)
        core.onChange()
        advanceTimeBy(5_600)
        runCurrent()
        assertEquals(2, core.stats.skipped("offline"))
        assertEquals(106L to "Not sent: PC away", ports.results.last())
    }

    @Test
    fun theSwitchTurnedOffOrThePermissionLostInTheHoldSendsNothing() = runTest {
        val (core, ports, sw) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        sw.screenshots = false
        advanceTimeBy(5_000)
        runCurrent()
        assertTrue(ports.uploads.isEmpty())
        sw.screenshots = true
        ports.shot(106)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        ports.readable = false
        advanceTimeBy(5_000)
        runCurrent()
        assertTrue("the permission is checked at the point of use, not remembered", ports.uploads.isEmpty())
        assertEquals(2, core.stats.skipped("off"))
    }

    @Test
    fun withoutThePermissionNothingIsEvenQueried() = runTest {
        val (core, ports, _) = rig { it.readable = false }
        ports.shot(105)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertTrue(ports.queries.isEmpty())
    }

    @Test
    fun aBurstIsOneQueryAndAPendingRowWaitsForTheNextSignal() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105, pending = true)
        repeat(5) { core.onChange(); advanceTimeBy(50) }
        advanceTimeBy(600)
        runCurrent()
        assertEquals("five signals, one pass", 1, ports.queries.size)
        assertTrue(ports.shown.isEmpty())
        // The row finishes: the observer fires again and now it goes.
        ports.rows.clear()
        ports.shot(105, pending = false)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(listOf(105L), ports.shown)
    }

    @Test
    fun theRateBucketHoldsFiveAndTheSixthIsDroppedAndCounted() = runTest {
        val (core, ports, _) = rig()
        repeat(6) { ports.shot(101L + it) }
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(5, ports.shown.size)
        assertEquals(1, core.stats.skipped("rate"))
    }

    @Test
    fun otherImagesAndTheCameraWithItsSwitchOffAreNeverSentAndTheBaselineOnlyGoesUp() = runTest {
        val (core, ports, _) = rig()
        ports.shot(101, path = "Download/")
        ports.shot(102, path = "DCIM/Camera/", mime = "image/jpeg")
        ports.shot(103, path = "Pictures/Kin/")
        ports.shot(104)
        core.onChange()
        advanceTimeBy(5_700)
        runCurrent()
        assertEquals(listOf(104L), ports.uploads.map { it.first })
        assertEquals(1, core.stats.skipped("other"))
        assertEquals(1, core.stats.skipped("off"))
        assertEquals(1, core.stats.skipped("own"))
        assertTrue("monotonic", ports.savedBaselines.zipWithNext().all { (a, b) -> b > a })
        assertFalse(ports.savedBaselines.isEmpty())
    }

    @Test
    fun dropHoldsTakesEveryNotificationAwayAndSendsNothing() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(1, core.holding)
        core.dropHolds()
        runCurrent()
        advanceTimeBy(10_000)
        runCurrent()
        assertTrue(ports.uploads.isEmpty())
        assertEquals(105L to null, ports.results.last())
    }

    // ------------------------------------------------------------------ review round 1 (B3 to B5, cleanups 7 and 8)

    @Test
    fun withoutAVisibleHoldNothingIsSentAndTheBaselineMovesPastTheImage() = runTest {
        val (core, ports, _) = rig { it.holdVisible = false }
        ports.shot(105)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertTrue(ports.shown.isEmpty())
        assertEquals("no hold, no job", 0, core.holding)
        advanceTimeBy(10_000)
        runCurrent()
        assertTrue("there would have been no Cancel: nothing is sent", ports.uploads.isEmpty())
        assertEquals(1, core.stats.skipped("no_hold"))
        assertEquals("it will not come back when notifications do", 105L, core.baseline)
        // Notifications are back: only an image taken from now on is held.
        ports.holdVisible = true
        ports.shot(106)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(listOf(106L), ports.shown)
    }

    @Test
    fun theHoldSaysPhotoForACameraPhotoAndScreenshotOtherwise() = runTest {
        val (core, ports, _) = rig(sw = Switches(screenshots = true, camera = true))
        ports.shot(105)
        ports.shot(106, path = "DCIM/Camera/", mime = "image/jpeg")
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(listOf(ShotKind.SCREENSHOT, ShotKind.CAMERA), ports.shownKinds)
        advanceTimeBy(5_000)
        runCurrent()
    }

    @Test
    fun aCancelTappedDuringTheUploadIsIgnoredAndDoesNotRemoveTheSentNotification() = runTest {
        val gate = CompletableDeferred<Unit>()
        val (core, ports, _) = rig { it.uploadGate = gate }
        ports.shot(105)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals("the hold became a non-cancellable Sending...", listOf(105L), ports.sending)
        assertEquals(listOf(105L), ports.uploads.map { it.first })
        assertEquals(0, core.holding)
        core.cancel(105)
        assertTrue("a Cancel with no hold leaves the notification alone", ports.results.isEmpty())
        assertEquals(0, core.stats.cancelled.get())
        gate.complete(Unit)
        runCurrent()
        assertEquals(105L to "Sent", ports.results.last())
        assertEquals(1, core.stats.sent.get())
        assertEquals(105L, core.baseline)
        // A late Cancel after the result also leaves "Sent" alone.
        val before = ports.results.toList()
        core.cancel(105)
        assertEquals(before, ports.results)
    }

    @Test
    fun aCancelDoesNotWaitForAnotherImagesUpload() = runTest {
        val gate = CompletableDeferred<Unit>()
        val (core, ports, _) = rig { it.uploadGate = gate }
        ports.shot(105)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        advanceTimeBy(1_500)
        ports.shot(106)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        // t = 2500: 105 is held until 5500, 106 until 7500.
        advanceTimeBy(3_000)
        runCurrent()
        assertEquals("105 is uploading (the upload is stuck on the gate)", listOf(105L), ports.uploads.map { it.first })
        core.cancel(106)
        assertEquals("immediate: no pass, no lock, no waiting for the upload", 106L to null, ports.results.last())
        assertEquals(1, core.stats.cancelled.get())
        gate.complete(Unit)
        runCurrent()
        advanceTimeBy(10_000)
        runCurrent()
        assertEquals("106 was cancelled and never goes", listOf(105L), ports.uploads.map { it.first })
        assertEquals(105L to "Sent", ports.results.last())
    }

    @Test
    fun aCancelJustBeforeTheFiveSecondsWinsTheRace() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(500)
        runCurrent()
        // t = 5500 is the end of the hold: its delay is due but has not run yet. The Cancel button is pressed now.
        advanceTimeBy(5_000)
        core.cancel(105)
        runCurrent()
        advanceTimeBy(10_000)
        runCurrent()
        assertTrue("the cancelled image never goes", ports.uploads.isEmpty())
        assertTrue("and was never turned into Sending...", ports.sending.isEmpty())
        assertEquals(105L to null, ports.results.last())
        assertEquals(1, core.stats.cancelled.get())
        assertEquals(0, core.stats.sent.get())
    }

    @Test
    fun aCancelOfANewerImageDoesNotCarryTheBaselineOverAnOlderOneStillHeld() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105)
        ports.shot(106)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        core.cancel(106)
        assertTrue("105 is still waiting its turn: the baseline stays below it", core.baseline < 105L)
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals(listOf(105L), ports.uploads.map { it.first })
        assertEquals("once 105 is decided the baseline reaches it", 105L, core.baseline)
        // The next pass finds 106 already handled (cancelled) and moves past it.
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(106L, core.baseline)
        assertEquals(1, core.stats.cancelled.get())
    }

    @Test
    fun moreThanFiftySkippedImagesDoNotStopAutoSendAndTheBaselineFollowsThem() = runTest {
        val (core, ports, _) = rig()
        // 70 downloads (never sent), then a screenshot: the query returns 50 at most, so the pass has to go on to the next page.
        repeat(70) { ports.shot(101L + it, path = "Download/") }
        ports.shot(171)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(70, core.stats.skipped("other"))
        assertEquals("the second page started after the first one's last row", listOf(100L, 150L), ports.queries)
        assertEquals(listOf(171L), ports.shown)
        assertEquals("the baseline stops below the image that is still held", 170L, core.baseline)
        advanceTimeBy(5_000)
        runCurrent()
        assertEquals(listOf(171L to ShotKind.SCREENSHOT), ports.uploads)
        assertEquals(171L, core.baseline)
    }

    @Test
    fun skippedImagesOfEverySortMoveTheBaselineSoLaterPassesDoNotReadThemAgain() = runTest {
        val (core, ports, _) = rig()
        repeat(3) { batch ->
            repeat(30) { ports.shot(101L + batch * 30 + it, path = "Download/") }
            core.onChange()
            advanceTimeBy(600)
            runCurrent()
        }
        assertEquals(90, core.stats.skipped("other"))
        assertEquals(190L, core.baseline)
        assertEquals("each pass starts above the last", listOf(100L, 130L, 160L), ports.queries)
        ports.shot(191)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals("after 90 skipped images auto-send still works", listOf(191L), ports.shown)
        advanceTimeBy(5_000)
        runCurrent()
    }

    @Test
    fun aPendingRowBlocksTheBaselineAndTheRowsAfterItAreNotLost() = runTest {
        val (core, ports, _) = rig()
        ports.shot(101, pending = true)
        ports.shot(102, path = "Download/")
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals("stopped at the first pending row", 100L, core.baseline)
        ports.rows.clear()
        ports.shot(101, path = "Download/")
        ports.shot(102, path = "Download/")
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(102L, core.baseline)
        assertEquals("102 was counted once, not twice", 2, core.stats.skipped("other"))
    }

    @Test
    fun aRowStuckPendingForLongIsDecidedAsStaleSoItCannotHoldTheBaselineForEver() = runTest {
        val (core, ports, _) = rig()
        ports.rows += ShotRow(101, "Pictures/Screenshots/", "image/png", true, 900, 50_000, "com.android.systemui")
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals(1, core.stats.skipped("stale"))
        assertEquals(101L, core.baseline)
    }

    @Test
    fun anOldScreenshotCopiedInIsSkippedByItsDateTakenButNewOnesAndUnknownDatesGo() = runTest {
        val (core, ports, _) = rig(baseline = 0) { it.maxId = 100 }
        core.retakeBaseline()
        assertEquals("the moment of the baseline is stored as a number", listOf(1_000L), ports.savedTimes)
        ports.shot(105, takenMillis = 500_000)
        ports.shot(106, takenMillis = 1_000_500)
        ports.shot(107, takenMillis = 0)
        core.onChange()
        advanceTimeBy(600)
        runCurrent()
        assertEquals("105 was taken before the feature was turned on", listOf(106L, 107L), ports.shown)
        assertEquals(1, core.stats.skipped("old"))
        advanceTimeBy(5_000)
        runCurrent()
    }

    @Test
    fun theCountsLineNamesNoFileAPathOrASize() = runTest {
        val (core, ports, _) = rig()
        ports.shot(105)
        core.onChange()
        advanceTimeBy(5_700)
        runCurrent()
        val line = core.stats.line()
        assertTrue(line, line.startsWith("shots: seen 1, sent 1"))
        assertFalse(line, line.contains("Screenshots") || line.contains("png") || line.contains("50000"))
    }
}
