package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.FakePcFiles.OfferedFile
import app.kinrelay.android.testing.TransferRig
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/**
 * v0.1.1: up to 4 files of one transfer are on the wire at once, in both directions (protocol.md: a client MAY run up to 4 file requests
 * per transfer). One request per file, so a single file is not split. Resume, cancel and progress have to be right with several in flight.
 */
class ParallelFilesTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(timings: TransferTimings = TransferRig.FAST): TransferRig = TransferRig(timings = timings).also { rigs += it; it.connect() }

    private fun offered(count: Int, size: Int, name: (Int) -> String = { "IMG_$it.jpg" }): List<OfferedFile> =
        (1..count).map { OfferedFile("f$it", name(it), "image/jpeg", TransferRig.bytes(size, it)) }

    private fun TransferRig.share(count: Int, size: Int): Pair<String, List<ByteArray>> {
        val bytes = (1..count).map { TransferRig.bytes(size, it) }
        val id = blocking { engine.enqueueUpload(bytes.mapIndexed { i, b -> ShareSource(sources.add("IMG_${i + 1}.jpg", b)) }) }!!
        return id to bytes
    }

    // ------------------------------------------------------------------ PC -> phone

    @Test
    fun aDownloadRunsFourFilesAtOnceAndNeverMore() {
        val r = rig()
        r.pc.files.holdFirstRequests = 4 // the first four requests only go on once all four are on the wire together
        r.pc.files.requestDelayMillis = 25
        val files = offered(12, 40_000)
        r.pc.sendOffer("many", *files.toTypedArray())
        r.awaitStatus("many", TransferStatus.DONE)

        assertEquals("four at once, never a fifth", 4, r.pc.files.maxInFlight.get())
        assertEquals(12, r.pc.files.calls.count { it.method == "GET" })
        assertTrue("no Range: nothing was resumed", r.pc.files.calls.all { it.range == null })
        val saved = r.sink.visible().associate { it.name to it.file.readBytes() }
        files.forEach { assertArrayEquals(it.name, it.bytes, saved.getValue(it.name)) }
        assertEquals(12, saved.size)
        r.await("complete to be acked") { r.store.transfers.getValue("many").completeAcked }
        assertEquals(listOf("complete:many:null"), r.pc.files.messages.toList())
        assertFalse(r.wake.held)
        assertEquals("the transfer row counts every byte", 12 * 40_000L, r.store.transfers.getValue("many").totalBytes)
    }

    @Test
    fun maxParallelFilesOfOneGivesTheOldOneByOneBehaviour() {
        val r = rig(TransferRig.FAST.copy(maxParallelFiles = 1))
        r.pc.files.requestDelayMillis = 15
        r.pc.sendOffer("serial", *offered(6, 10_000).toTypedArray())
        r.awaitStatus("serial", TransferStatus.DONE)
        assertEquals(1, r.pc.files.maxInFlight.get())
        assertEquals(6, r.sink.visible().size)
    }

    @Test
    fun aSingleBigFileIsOneRequestNotSplit() {
        val r = rig()
        val a = offered(1, 700_000).single()
        r.pc.sendOffer("one", a)
        r.awaitStatus("one", TransferStatus.DONE)
        val gets = r.pc.files.calls.filter { it.method == "GET" }
        assertEquals("one GET for the whole file", 1, gets.size)
        assertEquals(null, gets.single().range)
        assertEquals(1, r.pc.files.maxInFlight.get())
        assertArrayEquals(a.bytes, r.sink.visible().single().file.readBytes())
    }

    @Test
    fun filesWithTheSameNameInOneOfferGetDistinctNamesWhenTheyAreMadeAtTheSameTime() {
        val r = rig()
        val files = offered(6, 5_000) { "IMG.jpg" }
        r.pc.sendOffer("same", *files.toTypedArray())
        r.awaitStatus("same", TransferStatus.DONE)
        val names = r.store.files.getValue("same").map { it.finalName!! }
        assertEquals("six different names", 6, names.toSet().size)
        assertEquals(setOf("IMG.jpg", "IMG (1).jpg", "IMG (2).jpg", "IMG (3).jpg", "IMG (4).jpg", "IMG (5).jpg"), names.toSet())
        // Each name holds the bytes of its own file.
        val saved = r.sink.visible().associate { it.name to it.file.readBytes() }
        r.store.files.getValue("same").forEach { row ->
            assertArrayEquals(row.fileId, files.single { it.fileId == row.fileId }.bytes, saved.getValue(row.finalName!!))
        }
    }

    @Test
    fun oneFileBreaksMidBatchTheOthersFinishAndOnlyThatOneResumesWithRange() {
        val r = rig()
        val files = offered(6, 120_000)
        r.pc.files.dropGetAfter = 30_000
        r.pc.files.dropGetFileId = "f3"
        r.pc.sendOffer("break", *files.toTypedArray())
        r.awaitStatus("break", TransferStatus.DONE)

        val gets = r.pc.files.calls.filter { it.method == "GET" }
        assertEquals("six files and one resume", 7, gets.size)
        val ranged = gets.filter { it.range != null }
        assertEquals(1, ranged.size)
        assertEquals("bytes=30000-", ranged.single().range)
        assertTrue(ranged.single().path.endsWith("/f3"))
        assertEquals("the broken file was fetched twice, the others once", 2, gets.count { it.path.endsWith("/f3") })
        val saved = r.sink.visible().associate { it.name to it.file.readBytes() }
        files.forEach { assertArrayEquals(it.name, it.bytes, saved.getValue(it.name)) }
        assertFalse(r.wake.held)
    }

    @Test
    fun aFileThatFailsForGoodStopsTheOnesInFlightAndNothingIsLeftBehind() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000 // none of the others can finish soon
        val offered = r.pc.files.offered("fatal", *offered(6, 200_000).toTypedArray())
        offered.files.getValue("f2").changed = true // answers 409 at once
        r.pc.send(r.pc.files.offerFrame(offered))
        val t = r.awaitStatus("fatal", TransferStatus.FAILED)
        assertEquals("source_changed", t.errorCode)
        r.await("the cancel") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:fatal:error", r.pc.files.messages.single())
        r.await("partial files to go") { r.sink.entries.isEmpty() }
        r.await("the runner to stop") { !r.engine.isRunning("fatal") }
        assertFalse(r.wake.held)
    }

    @Test
    fun theUserCancelsADownloadWithSeveralFilesInFlight() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("cancel-many", *offered(6, 400_000).toTypedArray())
        r.await("two files writing at once") { r.sink.entries.values.count { it.file.length() > 0 } >= 2 }
        r.engine.cancel("cancel-many")
        r.awaitStatus("cancel-many", TransferStatus.CANCELED)
        r.await("the cancel message") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:cancel-many:user", r.pc.files.messages.single())
        r.await("partial files to go") { r.sink.entries.isEmpty() }
        r.await("the runner to stop") { !r.engine.isRunning("cancel-many") }
        assertFalse(r.wake.held)
    }

    @Test
    fun theProgressOfSeveralFilesAddsUpAndNeverPassesTheTotal() {
        val r = rig()
        val total = 8 * 120_000L
        r.pc.files.throttleGetBytesPer100Ms = 20_000 // about 0.6 s per file, so there are several progress updates (one per 500 ms)
        r.pc.sendOffer("progress", *offered(8, 120_000).toTypedArray())
        r.awaitStatus("progress", TransferStatus.DONE, timeoutMillis = 30_000)
        val seen = r.notifier.progress.map { it.first }
        assertTrue("several progress updates: $seen", seen.size >= 2)
        assertTrue("never more than the total: $seen", seen.all { it in 0..total })
        assertTrue("it counts the bytes of the files that run side by side, not of one: $seen", seen.max() > 120_000L)
    }

    // ------------------------------------------------------------------ phone -> PC

    @Test
    fun anUploadRunsFourFilesAtOnceAndNeverMore() {
        val r = rig()
        r.pc.files.holdFirstRequests = 4
        r.pc.files.requestDelayMillis = 25
        val (id, bytes) = r.share(10, 40_000)
        r.awaitStatus(id, TransferStatus.DONE)

        assertEquals(4, r.pc.files.maxInFlight.get())
        val puts = r.pc.files.calls.filter { it.method == "PUT" }
        assertEquals("one PUT per file", 10, puts.size)
        assertEquals(1, r.pc.files.calls.count { it.method == "POST" })
        bytes.forEachIndexed { i, b -> assertArrayEquals("f${i + 1}", b, r.pc.files.uploads.getValue(id).files.getValue("f${i + 1}").data) }
        assertEquals("every opened source was closed again", 0, r.sources.openStreams.get())
        assertFalse(r.wake.held)
        assertEquals(r.wake.acquired.get(), r.wake.released.get())
    }

    @Test
    fun anUploadWithMaxParallelFilesOfOneIsOneByOne() {
        val r = rig(TransferRig.FAST.copy(maxParallelFiles = 1))
        r.pc.files.requestDelayMillis = 15
        val (id, _) = r.share(5, 10_000)
        r.awaitStatus(id, TransferStatus.DONE)
        assertEquals(1, r.pc.files.maxInFlight.get())
    }

    @Test
    fun aSingleBigFileIsOnePutNotSplit() {
        val r = rig()
        val (id, bytes) = r.share(1, 700_000)
        r.awaitStatus(id, TransferStatus.DONE)
        assertEquals(listOf("POST", "PUT"), r.pc.files.calls.map { it.method })
        assertEquals(700_000L, r.pc.files.calls.last().contentLength)
        assertArrayEquals(bytes.single(), r.pc.files.uploads.getValue(id).files.getValue("f1").data)
    }

    @Test
    fun oneUploadBreaksMidBatchTheOthersFinishAndOnlyThatOneResumesFromWhatThePcKept() {
        val r = rig()
        r.pc.files.dropPutKeeping = 30_000
        r.pc.files.dropPutFileId = "f3"
        val (id, bytes) = r.share(6, 120_000)
        r.awaitStatus(id, TransferStatus.DONE)

        val puts = r.pc.files.calls.filter { it.method == "PUT" }
        assertEquals(listOf(0L, 30_000L), puts.filter { it.path.endsWith("/f3") }.map { it.offset })
        assertEquals("the others went once", 5, puts.count { !it.path.endsWith("/f3") })
        assertEquals("a second POST asked where the PC is", 2, r.pc.files.calls.count { it.method == "POST" })
        bytes.forEachIndexed { i, b -> assertArrayEquals("f${i + 1}", b, r.pc.files.uploads.getValue(id).files.getValue("f${i + 1}").data) }
        assertEquals(0, r.sources.openStreams.get())
        assertFalse(r.wake.held)
    }

    @Test
    fun anUploadThatFailsForGoodLeavesNoSourceOpenAndNoWakeLockHeld() {
        val r = rig()
        r.pc.files.putStatus = 507
        val (id, _) = r.share(8, 20_000)
        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("disk_full", t.errorCode)
        r.await("the runner to stop") { !r.engine.isRunning(id) }
        assertEquals("sources opened ahead of their requests are closed", 0, r.sources.openStreams.get())
        assertFalse(r.wake.held)
    }

    @Test
    fun theUserCancelsAnUploadWithSeveralFilesInFlight() {
        val r = rig()
        r.sources.readDelayMillis = 25 // every 64 KB read waits: the files are still going when the cancel comes
        val (id, _) = r.share(6, 600_000)
        r.await("bytes moving") { r.sources.reads.get() >= 8 }
        r.engine.cancel(id)
        r.awaitStatus(id, TransferStatus.CANCELED)
        r.await("the DELETE") { r.pc.files.deleted.contains(id) }
        r.await("the runner to stop") { !r.engine.isRunning(id) }
        assertEquals("every opened source was closed", 0, r.sources.openStreams.get())
        assertFalse(r.wake.held)
        assertTrue("nothing was reported done", r.notifier.events.none { it == "uploaded:$id" })
    }

    @Test
    fun aShareOfManyFilesIsLookedAtSeveralAtATimeAndOnlyOnceBeforeItStarts() {
        val r = rig()
        r.sources.statDelayMillis = 40 // a provider round trip
        val count = 40
        val started = System.nanoTime()
        val (id, _) = r.share(count, 100)
        val tookMillis = (System.nanoTime() - started) / 1_000_000
        // One by one it would be 40 x 40 ms = 1.6 s.
        assertTrue("took $tookMillis ms, one by one would be ${count * 40}", tookMillis < count * 40 * 0.6)
        r.awaitStatus(id, TransferStatus.DONE)
        assertEquals("each file was looked at once: the first attempt doesn't look again", count, r.sources.statCalls.get())
    }

    // ------------------------------------------------------------------ helper

    @Test
    fun mapParallelKeepsTheOrderAndNeverRunsMoreThanTheLimit() {
        val now = AtomicInteger()
        val most = AtomicInteger()
        val result = runBlocking {
            (0 until 20).toList().mapParallel(3) { i ->
                most.accumulateAndGet(now.incrementAndGet()) { a, b -> maxOf(a, b) }
                Thread.sleep(15)
                now.decrementAndGet()
                i * 2
            }
        }
        assertEquals((0 until 20).map { it * 2 }, result)
        assertTrue("at most 3 at once, was ${most.get()}", most.get() <= 3)
        assertTrue("and really in parallel, was ${most.get()}", most.get() >= 2)
    }
}
