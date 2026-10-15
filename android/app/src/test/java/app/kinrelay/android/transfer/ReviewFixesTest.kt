package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.protocol.Offer
import app.kinrelay.android.protocol.OfferFile
import app.kinrelay.android.testing.FakePcFiles.OfferedFile
import app.kinrelay.android.testing.TransferRig
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** The fixes the 1.3 review asked for on the Android side (reused IDs, name limit, unpair, crashes, free space, notifications). */
class ReviewFixesTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(timings: TransferTimings = TransferRig.FAST): TransferRig = TransferRig(timings = timings).also { rigs += it; it.connect() }

    private fun file(id: String, name: String, size: Int, mime: String = "application/octet-stream", seed: Int = 1) =
        OfferedFile(id, name, mime, TransferRig.bytes(size, seed))

    private fun offer(id: String, vararg sizes: Long) =
        Offer(id, sizes.mapIndexed { i, size -> OfferFile("f${i + 1}", "file$i.bin", size, "application/octet-stream") }, sizes.sum())

    private fun TransferRig.accept(offer: Offer): String? = engine.acceptOffer(offer, pcId)

    private fun TransferRig.share(vararg files: Pair<String, ByteArray>): String =
        blocking { engine.enqueueUpload(files.map { (name, bytes) -> ShareSource(sources.add(name, bytes)) }) }!!


    private fun transferRow(id: String, direction: String, peer: String, status: String, size: Long = 1_000, bytesDone: Long = 0, uri: String? = null) =
        TransferEntity(id, direction, peer, "Some PC", TransferKind.FILE, status, 1_000, null, size, null, null, completeAcked = false) to
            listOf(TransferFileEntity(id, "f1", 0, "f.bin", null, "application/octet-stream", size, bytesDone, uri, null, finished = false))

    private fun TransferRig.put(id: String, direction: String, peer: String, status: String, size: Long = 1_000, bytesDone: Long = 0, uri: String? = null) {
        val (transfer, files) = transferRow(id, direction, peer, status, size, bytesDone, uri)
        blocking { store.create(transfer, files) }
    }

    // ---------------------------------------------------------------- 1. an offer can't reuse an upload's ID

    @Test
    fun anOfferWithTheIdOfARunningUploadIsRefusedAndTheUploadIsUntouched() {
        val r = rig()
        r.sources.readDelayMillis = 40
        val id = r.share("big.bin" to TransferRig.bytes(1_000_000))
        r.await("the PUT to be under way") { r.sources.reads.get() > 2 }
        val rowsBefore = r.store.files.getValue(id).toList()

        // A paired PC offers a "download" with the ID of the upload: it would be run on the upload's rows.
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offered(id, file("f1", "evil.bin", 100))))
        val reply = r.pc.nextFrame()!!
        assertEquals("error", reply.getValue("type").jsonPrimitive.content)
        assertEquals("bad_request", reply.getValue("code").jsonPrimitive.content)
        assertEquals(id, reply.getValue("ref").jsonPrimitive.content)

        Thread.sleep(300)
        val t = r.store.transfers.getValue(id)
        assertEquals(TransferDirection.UP, t.direction)
        assertEquals("the upload goes on, it is not marked done", TransferStatus.ACTIVE, t.status)
        assertEquals(rowsBefore.map { it.localUri }, r.store.files.getValue(id).map { it.localUri })
        assertTrue("no download ran", r.pc.files.calls.none { it.method == "GET" })
        r.engine.cancel(id)
        r.awaitStatus(id, TransferStatus.CANCELED)
    }

    @Test
    fun onOfferItselfIgnoresTheIdOfAnUploadAndLeavesItsRowsAlone() {
        val r = rig()
        val id = r.share("a.bin" to TransferRig.bytes(50_000))
        r.awaitStatus(id, TransferStatus.DONE)
        val filesBefore = r.store.files.getValue(id).toList()
        r.pc.files.offered(id, file("f1", "x.bin", 10))

        r.blocking { r.engine.onOffer(r.pcId, "Desk PC", offer(id, 10)) }

        Thread.sleep(200)
        assertEquals(TransferStatus.DONE, r.store.transfers.getValue(id).status)
        assertEquals(TransferDirection.UP, r.store.transfers.getValue(id).direction)
        assertEquals(filesBefore, r.store.files.getValue(id))
        assertTrue("nothing was downloaded", r.pc.files.calls.none { it.method == "GET" })
        assertTrue(r.sink.entries.isEmpty())
    }

    @Test
    fun aDownloadOfAnotherPcIsNotTakenOverByAnOfferWithItsId() {
        val r = rig()
        r.put("theirs", TransferDirection.DOWN, peer = "another-pc", status = TransferStatus.QUEUED)
        assertEquals("bad_request", r.accept(offer("theirs", 1_000)))
        r.blocking { r.engine.onOffer(r.pcId, "Desk PC", offer("theirs", 1_000)) }
        assertEquals("another-pc", r.store.transfers.getValue("theirs").peerDeviceId)
        assertFalse(r.engine.isRunning("theirs"))
    }

    @Test
    fun aFinishedDownloadIsNotStartedAgainByAnOfferWithItsId() {
        val r = rig()
        val offered = r.pc.sendOffer("once", file("f1", "a.bin", 10_000))
        r.awaitStatus("once", TransferStatus.DONE)
        val gets = r.pc.files.calls.count { it.method == "GET" }
        val offer = offer("once", 10_000)

        // After the 24 hours of the de-duplication memory the receiver no longer answers for it, so the gate and onOffer meet the ID.
        assertEquals("bad_request", r.accept(offer))
        r.blocking { r.engine.onOffer(r.pcId, "Desk PC", offer) }
        Thread.sleep(200)
        assertEquals(TransferStatus.DONE, r.store.transfers.getValue("once").status)
        assertEquals("no second GET", gets, r.pc.files.calls.count { it.method == "GET" })
        assertEquals(1, r.sink.entries.size)
        assertNotNull(offered)
    }

    @Test
    fun theSamePcOfferingAnUnfinishedDownloadAgainResumesItAndDoesNotStartTwo() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("again", file("f1", "big.bin", 600_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        val offer = offer("again", 600_000)

        assertEquals("a repeat by the same PC, not a reuse", app.kinrelay.android.protocol.OfferHistory.REPEAT, r.engine.offerHistory(offer, r.pcId))
        r.blocking { r.engine.onOfferRepeated(r.pcId, offer) }
        assertEquals("still one entry, one job", 1, r.sink.entries.size)
        assertTrue(r.engine.isRunning("again"))
        r.engine.cancel("again")
        r.awaitStatus("again", TransferStatus.CANCELED)
    }

    // ---------------------------------------------------------------- 2. the name loop has a limit

    @Test
    fun aNameThatKeepsBeingRefusedFailsTheFileWithACancelErrorAndFreesTheLock() {
        val r = rig()
        r.sink.refuseAllNames = true
        r.pc.sendOffer("stuck", file("f1", "Text.txt", 10, "text/plain"))
        val t = r.awaitStatus("stuck", TransferStatus.FAILED, timeoutMillis = 20_000)
        assertEquals("internal", t.errorCode)
        assertEquals("the name and (1) to (99), then it stopped", 100, r.sink.refusals.get())
        r.await("the cancel") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:stuck:error", r.pc.files.messages.single())
        assertTrue(r.sink.entries.isEmpty())
        r.await("the runner to end") { !r.engine.isRunning("stuck") }

        // The download lock was let go: the next offer is taken at once.
        r.sink.refuseAllNames = false
        r.pc.sendOffer("next", file("f1", "ok.txt", 10, "text/plain"))
        r.awaitStatus("next", TransferStatus.DONE)
    }

    // ---------------------------------------------------------------- 3. transfers belong to their PC

    @Test
    fun unpairingCancelsAnActiveDownloadDeletesItsPartialAndSendsNothing() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("slow", file("f1", "big.bin", 600_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }

        r.blocking { r.engine.onUnpaired(r.pcId) }

        val t = r.store.transfers.getValue("slow")
        assertEquals(TransferStatus.CANCELED, t.status)
        assertEquals("unpaired", t.errorCode)
        assertTrue("the partial file is gone", r.sink.entries.isEmpty())
        assertFalse(r.engine.isRunning("slow"))
        assertFalse(r.wake.held)
        assertTrue("the old PC is told nothing", r.pc.files.messages.isEmpty())
        assertTrue(r.notifier.events.contains("clear:slow"))
    }

    @Test
    fun completedFilesOfAnUnpairedPcStayAndOnlyTheUnfinishedOnesGo() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("two", file("f1", "small.bin", 5_000), file("f2", "big.bin", 600_000, seed = 2))
        r.await("the first file to be saved") { r.sink.visible().isNotEmpty() && r.sink.entries.size == 2 }

        r.blocking { r.engine.onUnpaired(r.pcId) }

        assertEquals(listOf("small.bin"), r.sink.entries.values.map { it.name })
        assertTrue(r.sink.entries.values.single().file.length() == 5_000L)
    }

    @Test
    fun unpairingCancelsAnActiveUploadAndNoMoreBytesGoOut() {
        val r = rig()
        r.sources.readDelayMillis = 40
        val id = r.share("big.bin" to TransferRig.bytes(1_000_000))
        r.await("the PUT to be under way") { r.sources.reads.get() > 2 }

        r.blocking { r.engine.onUnpaired(r.pcId) }

        val t = r.store.transfers.getValue(id)
        assertEquals(TransferStatus.CANCELED, t.status)
        assertEquals("unpaired", t.errorCode)
        assertFalse(r.engine.isRunning(id))
        assertFalse(r.wake.held)
        val callsAfter = r.pc.files.calls.size
        Thread.sleep(300)
        assertEquals("nothing more was sent", callsAfter, r.pc.files.calls.size)
        assertTrue("and no DELETE: the PC it was for is no longer paired", r.pc.files.deleted.isEmpty())
    }

    @Test
    fun unpairingOnePcLeavesTheTransfersOfAnotherAlone() {
        val r = rig()
        r.put("other", TransferDirection.DOWN, peer = "the-other-pc", status = TransferStatus.QUEUED)
        r.put("mine", TransferDirection.DOWN, peer = r.pcId, status = TransferStatus.QUEUED)
        r.blocking { r.engine.onUnpaired("the-other-pc") }
        assertEquals(TransferStatus.CANCELED, r.store.transfers.getValue("other").status)
        assertEquals(TransferStatus.QUEUED, r.store.transfers.getValue("mine").status)
    }

    @Test
    fun aQueuedUploadNeverGoesToAPcThatWasPairedLater() {
        val r = rig()
        r.dropConnection()
        val id = r.share("a.bin" to TransferRig.bytes(300)) // no connection within the wait: queued, copied into the cache
        r.await("the upload to be parked") { r.notifier.queued.any { it.first.id == id } }
        assertTrue(r.sources.cached.isNotEmpty())

        // Another PC is paired; the old one was never reported as unpaired (the backstop must still hold).
        r.pairedPeer = PeerInfo("a-different-pc", "New PC")
        r.connect()
        r.engine.retry(id)

        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("peer_changed", t.errorCode)
        assertTrue("nothing reached the PC that answered", r.pc.files.calls.isEmpty())
        assertTrue("the cached copy is gone", r.sources.cached.isEmpty())
        assertTrue(r.notifier.failed.any { it.first.id == id })
    }

    @Test
    fun aDownloadThatMeetsADifferentPairedPcFailsAndAsksNobody() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("swap", file("f1", "big.bin", 600_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }

        r.pairedPeer = PeerInfo("a-different-pc", "New PC") // paired behind the transfer's back
        r.dropConnection()
        r.connect()

        val t = r.awaitStatus("swap", TransferStatus.FAILED)
        assertEquals("peer_changed", t.errorCode)
        assertTrue("the partial file is gone", r.sink.entries.isEmpty())
        assertTrue("no cancel went to the new PC", r.pc.files.messages.isEmpty())
    }

    @Test
    fun whenTheLinkComesUpTransfersOfOtherPcsAreCancelled() {
        val r = rig()
        r.put("old", TransferDirection.DOWN, peer = "gone-pc", status = TransferStatus.QUEUED)
        r.put("up", TransferDirection.UP, peer = "gone-pc", status = TransferStatus.ACTIVE)
        r.blocking { r.engine.onLinkReady(r.pcId) }
        assertEquals(TransferStatus.CANCELED, r.store.transfers.getValue("old").status)
        assertEquals(TransferStatus.CANCELED, r.store.transfers.getValue("up").status)
        assertEquals("unpaired", r.store.transfers.getValue("up").errorCode)
    }

    // ---------------------------------------------------------------- 4. crashes leave nothing behind

    @Test
    fun aCrashInADownloadDeletesThePartialSendsCancelErrorAndFailsTheTransfer() {
        val r = rig()
        r.sink.crashOnWrite = true // a bug, not an IOException: the entry already exists when it hits
        r.pc.sendOffer("boom", file("f1", "a.bin", 50_000))
        val t = r.awaitStatus("boom", TransferStatus.FAILED)
        assertEquals("internal", t.errorCode)
        r.await("the cancel message") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:boom:error", r.pc.files.messages.single())
        assertTrue("no pending entry is left", r.sink.entries.isEmpty())
        assertNull(r.store.files.getValue("boom").single().localUri)
        assertTrue(r.notifier.failed.any { it.first.id == "boom" })
        r.await("the runner to end") { !r.engine.isRunning("boom") }
        assertFalse(r.wake.held)
    }

    @Test
    fun theStartupSweepDeletesPendingEntriesAndCachesNoTransferOwns() {
        val r = rig()
        val owned = r.sink.create(SaveFolder.DOWNLOADS, "owned.bin", "application/octet-stream")
        val orphan = r.sink.create(SaveFolder.PICTURES, "orphan.jpg", "image/jpeg")
        val saved = r.sink.create(SaveFolder.DOWNLOADS, "saved.bin", "application/octet-stream").also { r.sink.finish(it.uri) }
        r.put("live", TransferDirection.DOWN, peer = r.pcId, status = TransferStatus.QUEUED, uri = owned.uri)
        // Cached copies: one of a failed upload (it can be retried), one of nothing.
        r.put("failed-up", TransferDirection.UP, peer = r.pcId, status = TransferStatus.FAILED)
        File(r.sources.cacheDir, "failed-up").also { it.mkdirs() }.resolve("f1").writeText("x")
        File(r.sources.cacheDir, "nobody").also { it.mkdirs() }.resolve("f1").writeText("x")

        r.blocking { r.engine.sweepOrphans() }

        assertNull("the orphan is gone", r.sink.entries[orphan.uri])
        assertNotNull("a download's own partial stays", r.sink.entries[owned.uri])
        assertNotNull("a finished file is never touched", r.sink.entries[saved.uri])
        assertTrue(File(r.sources.cacheDir, "failed-up").exists())
        assertFalse(File(r.sources.cacheDir, "nobody").exists())
    }

    @Test
    fun theSweepLeavesAFreshCacheAloneBecauseAShareIsBeingSetUp() {
        val r = rig()
        r.sources.cacheAgeMillis = 1_000 // younger than ten minutes
        File(r.sources.cacheDir, "being-shared").also { it.mkdirs() }.resolve("f1").writeText("x")
        r.blocking { r.engine.sweepOrphans() }
        assertTrue(File(r.sources.cacheDir, "being-shared").exists())
    }

    // ---------------------------------------------------------------- 6. free space and offer limits

    @Test
    fun anOfferOfMoreThan800FilesIsRefusedWithTooLarge() {
        val r = rig()
        assertNull(r.accept(offer("o800", *LongArray(800) { 1 })))
        assertEquals("too_large", r.accept(offer("o801", *LongArray(801) { 1 })))

        // and on the wire: the error replaces the ack
        r.pc.sendOffer("wire801", *(1..801).map { file("f$it", "n$it.txt", 1) }.toTypedArray())
        val reply = r.pc.nextFrame()!!
        assertEquals("error", reply.getValue("type").jsonPrimitive.content)
        assertEquals("too_large", reply.getValue("code").jsonPrimitive.content)
        assertEquals("wire801", reply.getValue("ref").jsonPrimitive.content)
        assertNull(r.store.transfers["wire801"])
    }

    @Test
    fun sizesThatOverflowTheSumAreRefusedNotWrappedAround() {
        val r = rig()
        r.sink.free = Long.MAX_VALUE
        assertEquals("too_large", r.accept(offer("ov1", Long.MAX_VALUE, 1)))
        assertEquals("too_large", r.accept(offer("ov2", Long.MAX_VALUE / 2 + 1, Long.MAX_VALUE / 2 + 1)))
        assertEquals("too_large", r.accept(offer("ov3", Long.MAX_VALUE, Long.MAX_VALUE, 5)))
        assertNull("the largest sum that fits is fine", r.accept(offer("ok", Long.MAX_VALUE - 1, 1)))
    }

    @Test
    fun aNegativeSizeIsBadRequest() {
        val r = rig()
        assertEquals("bad_request", r.accept(offer("neg", 10, -5)))
        assertEquals("bad_request", r.accept(offer("neg2", Long.MAX_VALUE, Long.MAX_VALUE, -Long.MAX_VALUE)))
    }

    @Test
    fun bytesThatActiveDownloadsStillHaveToWriteAreNotFreeSpace() {
        val r = rig()
        r.sink.free = 100_000
        // An unfinished download that has 80,000 bytes to go (100,000 - 20,000).
        r.put("busy", TransferDirection.DOWN, peer = r.pcId, status = TransferStatus.ACTIVE, size = 100_000, bytesDone = 20_000)
        assertEquals("30,000 + 80,000 > 100,000", "disk_full", r.accept(offer("a", 30_000)))
        assertNull("15,000 + 80,000 fits", r.accept(offer("b", 15_000)))
    }

    @Test
    fun twoOffersInARowCantBothTakeTheLastSpace() {
        val r = rig()
        r.sink.free = 100_000
        assertNull(r.accept(offer("first", 60_000)))
        // The first one is acked but its rows are not stored yet: its bytes are still claimed.
        assertEquals("disk_full", r.accept(offer("second", 60_000)))
        assertNull(r.accept(offer("small", 30_000)))
    }

    @Test
    fun aFinishedDownloadsBytesAreFreeAgain() {
        val r = rig()
        r.sink.free = 100_000
        r.put("done", TransferDirection.DOWN, peer = r.pcId, status = TransferStatus.DONE, size = 90_000)
        assertNull(r.accept(offer("x", 95_000)))
    }

    // ---------------------------------------------------------------- 7. progress is throttled and never forced

    @Test
    fun aBatchOfSmallFilesDoesNotFloodTheNotifierWithProgress() {
        val r = rig()
        val start = System.currentTimeMillis()
        r.pc.sendOffer("many", *(1..40).map { file("f$it", "n$it.txt", 2_000, seed = it) }.toTypedArray())
        r.awaitStatus("many", TransferStatus.DONE)
        val elapsed = System.currentTimeMillis() - start
        assertTrue("${r.notifier.progress.size} updates in $elapsed ms", r.notifier.progress.size <= elapsed / 500 + 2)
        assertEquals("the result is announced once", 1, r.notifier.received.size)
    }

    @Test
    fun aBatchUploadDoesNotFloodTheNotifierEither() {
        val r = rig()
        val start = System.currentTimeMillis()
        val id = r.share(*(1..40).map { "n$it.txt" to TransferRig.bytes(2_000, it) }.toTypedArray())
        r.awaitStatus(id, TransferStatus.DONE)
        val elapsed = System.currentTimeMillis() - start
        assertTrue("${r.notifier.progress.size} updates in $elapsed ms", r.notifier.progress.size <= elapsed / 500 + 2)
    }
}
