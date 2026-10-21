package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.TransferRig
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Phone -> PC, end to end against the fake PC (real TLS 1.3, real WebSocket and HTTPS, MockWebServer): the upload flow of protocol.md,
 * "Files: phone -> PC", with the Android parts (shared files, wake lock, notifications, store) replaced by fakes.
 */
class UploadFlowTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(timings: TransferTimings = TransferRig.FAST): TransferRig = TransferRig(timings = timings).also { rigs += it; it.connect() }

    private fun TransferRig.share(vararg files: Pair<String, ByteArray>): String =
        blocking { engine.enqueueUpload(files.map { (name, bytes) -> ShareSource(sources.add(name, bytes)) }) }!!

    private fun TransferRig.stored(id: String, fileId: String) = pc.files.uploads.getValue(id).files.getValue(fileId)

    @Test
    fun aSingleFileIsPostedPutAndDone() {
        val r = rig()
        val bytes = TransferRig.bytes(300_000)
        val id = r.share("IMG_0042.jpg" to bytes)
        r.awaitStatus(id, TransferStatus.DONE)

        assertArrayEquals(bytes, r.stored(id, "f1").data)
        assertEquals("IMG_0042.jpg", r.stored(id, "f1").name)
        assertEquals(listOf("POST", "PUT"), r.pc.files.calls.map { it.method })
        val put = r.pc.files.calls.last()
        assertEquals(0L, put.offset)
        assertEquals(300_000L, put.contentLength)
        // The notification is posted just after the row says DONE, so on a slow machine it can arrive a moment later.
        r.await("uploaded notification") { r.notifier.events.contains("uploaded:$id") }
        assertEquals("the transfer row says so", 300_000L, r.store.transfers.getValue(id).totalBytes)
        assertTrue(r.store.files.getValue(id).single().finished)
    }

    @Test
    fun theWakeLockIsHeldOnlyWhileBytesMove() {
        val r = rig()
        assertFalse("not held while idle", r.wake.held)
        val id = r.share("a.bin" to TransferRig.bytes(200_000))
        r.awaitStatus(id, TransferStatus.DONE)
        assertTrue("held for the PUT", r.wake.acquired.get() >= 1)
        assertEquals("every acquire has its release", r.wake.acquired.get(), r.wake.released.get())
        assertFalse(r.wake.held)
    }

    @Test
    fun severalFilesAreAllSentAndAZeroByteFileGetsItsOwnPut() {
        val r = rig()
        val a = TransferRig.bytes(70_000, 1)
        val c = TransferRig.bytes(5, 3)
        val id = r.share("a.jpg" to a, "empty.txt" to ByteArray(0), "c.bin" to c)
        r.awaitStatus(id, TransferStatus.DONE)

        assertEquals(listOf("f1", "f2", "f3"), r.pc.files.uploads.getValue(id).files.keys.toList())
        assertArrayEquals(a, r.stored(id, "f1").data)
        assertEquals(0, r.stored(id, "f2").data.size)
        assertArrayEquals(c, r.stored(id, "f3").data)
        val puts = r.pc.files.calls.filter { it.method == "PUT" }
        // Files go side by side, so the order in which the PC sees the requests is not fixed; what it sees is one PUT per file.
        assertEquals(setOf("/v1/transfers/$id/files/f1", "/v1/transfers/$id/files/f2", "/v1/transfers/$id/files/f3"), puts.map { it.path }.toSet())
        assertEquals(3, puts.size)
        val empty = puts.single { it.path.endsWith("/f2") }
        assertEquals("the 0-byte file is a PUT with Content-Length 0 at offset 0", Triple(0L, 0L, 0L), Triple(empty.offset, empty.contentLength, empty.bodyBytes))
        assertEquals(70_005L, r.store.transfers.getValue(id).totalBytes)
    }

    @Test
    fun aZeroByteFileAlone() {
        val r = rig()
        val id = r.share("empty.txt" to ByteArray(0))
        r.awaitStatus(id, TransferStatus.DONE)
        assertTrue(r.stored(id, "f1").complete)
    }

    @Test
    fun theRequestsUseTheProtocolShapes() {
        val r = rig()
        val id = r.share("My photo.jpg" to TransferRig.bytes(10))
        r.awaitStatus(id, TransferStatus.DONE)
        val post = r.pc.requests.first { it.method == "POST" }
        assertEquals("application/json; charset=utf-8", post.contentType)
        assertTrue(post.body, post.body.startsWith("""{"id":"$id","files":[{"fileId":"f1","name":"My photo.jpg","size":10,"mime":"application/octet-stream"}]}"""))
        assertEquals("the transfer ID is a path-safe ID", true, Regex("[A-Za-z0-9_-]{1,64}").matches(id))
    }

    @Test
    fun aDroppedConnectionResumesFromWhatThePcKept() {
        val r = rig()
        val bytes = TransferRig.bytes(400_000)
        r.pc.files.dropPutKeeping = 150_000
        val id = r.share("big.bin" to bytes)
        r.awaitStatus(id, TransferStatus.DONE)

        assertArrayEquals(bytes, r.stored(id, "f1").data)
        val calls = r.pc.files.calls
        assertEquals(listOf("POST", "PUT", "POST", "PUT"), calls.map { it.method })
        assertEquals(0L, calls[1].offset)
        assertEquals("the second PUT continues from what the PC kept", 150_000L, calls[3].offset)
        assertEquals(250_000L, calls[3].contentLength)
        assertFalse(r.wake.held)
    }

    @Test
    fun turningWifiOffAndBackOnResumesTheTransfer() {
        val r = rig()
        val bytes = TransferRig.bytes(400_000)
        r.pc.files.dropPutKeeping = 100_000
        r.sources.readDelayMillis = 30 // the transfer is still going when the connection goes
        val id = r.share("big.bin" to bytes)
        r.await("the PUT to be under way") { r.sources.reads.get() > 0 }
        r.dropConnection()
        // The phone waits for the PC (no failure) ...
        Thread.sleep(300)
        assertEquals(TransferStatus.ACTIVE, r.store.transfers.getValue(id).status)
        r.sources.readDelayMillis = 0
        // ... and carries on when the connection is back.
        r.connect()
        r.awaitStatus(id, TransferStatus.DONE)
        assertArrayEquals(bytes, r.stored(id, "f1").data)
        assertTrue("more than one POST: it resumed", r.pc.files.calls.count { it.method == "POST" } >= 2)
        assertFalse(r.wake.held)
    }

    @Test
    fun a409OffsetMismatchContinuesFromTheOffsetThePcReturns() {
        val r = rig()
        val bytes = TransferRig.bytes(300_000)
        r.pc.files.dropPutKeeping = 120_000
        r.pc.files.staleOffsetOnce = 0 // after the drop the POST reports 0 (stale), the PUT from 0 gets 409
        val id = r.share("big.bin" to bytes)
        r.awaitStatus(id, TransferStatus.DONE)

        assertArrayEquals(bytes, r.stored(id, "f1").data)
        val puts = r.pc.files.calls.filter { it.method == "PUT" }
        assertEquals(listOf(0L, 0L, 120_000L), puts.map { it.offset })
    }

    @Test
    fun aLostFinalResponseFinishesWithAnEmptyPutAtTheSize() {
        val r = rig()
        val bytes = TransferRig.bytes(50_000)
        r.pc.files.loseFinalResponseOnce = true
        val id = r.share("a.bin" to bytes)
        r.awaitStatus(id, TransferStatus.DONE)

        val puts = r.pc.files.calls.filter { it.method == "PUT" }
        assertEquals(listOf(0L, 50_000L), puts.map { it.offset })
        assertEquals("the second PUT has no body", 0L, puts[1].bodyBytes)
        assertArrayEquals(bytes, r.stored(id, "f1").data)
    }

    @Test
    fun aFailureOnTheTransferIsFinalAndNotRetried() {
        for ((status, word) in listOf(507 to "free space", 413 to "large", 500 to "save")) {
            val r = rig()
            r.pc.files.postStatus = status
            val id = r.share("a.bin" to TransferRig.bytes(1_000))
            val t = r.awaitStatus(id, TransferStatus.FAILED)
            assertEquals("$status: one POST, nothing else", listOf("POST"), r.pc.files.calls.map { it.method })
            assertTrue("$status: ${t.message}", t.message!!.contains(word))
            assertEquals(1, r.notifier.failed.size)
            Thread.sleep(150)
            assertEquals("$status: still one call (no automatic retry)", 1, r.pc.files.calls.size)
        }
    }

    @Test
    fun aFailureOnAPutIsFinalToo() {
        for (status in listOf(507, 413, 500)) {
            // One file at a time: the second file is never started.
            val r = rig(TransferRig.FAST.copy(maxParallelFiles = 1))
            r.pc.files.putStatus = status
            val id = r.share("a.bin" to TransferRig.bytes(1_000), "b.bin" to TransferRig.bytes(1_000))
            val t = r.awaitStatus(id, TransferStatus.FAILED)
            assertEquals("$status: POST and one PUT, the second file is not started", listOf("POST", "PUT"), r.pc.files.calls.map { it.method })
            assertNotNull(t.message)
            assertFalse(r.wake.held)
        }
    }

    @Test
    fun aFailureOnAPutIsFinalWithSeveralFilesInFlightToo() {
        for (status in listOf(507, 413, 500)) {
            val r = rig()
            r.pc.files.putStatus = status
            val id = r.share("a.bin" to TransferRig.bytes(1_000), "b.bin" to TransferRig.bytes(1_000), "c.bin" to TransferRig.bytes(1_000))
            val t = r.awaitStatus(id, TransferStatus.FAILED)
            Thread.sleep(200)
            val calls = r.pc.files.calls.map { it.method }
            assertEquals("$status: one POST, and the PUTs of the files that were already started, no second POST (no retry)", 1, calls.count { it == "POST" })
            assertTrue("$status: at most one PUT per file: $calls", calls.count { it == "PUT" } in 1..3)
            assertNotNull(t.message)
            r.await("the runner to stop") { !r.engine.isRunning(id) }
            assertFalse(r.wake.held)
            assertEquals("every opened source was closed", 0, r.sources.openStreams.get())
        }
    }

    @Test
    fun theUserCancelsInFlightAndTheTransferIsDeletedOnThePc() {
        val r = rig()
        r.sources.readDelayMillis = 40
        val id = r.share("big.bin" to TransferRig.bytes(1_000_000))
        r.await("the PUT to be under way") { r.sources.reads.get() > 2 }
        r.engine.cancel(id)
        val t = r.awaitStatus(id, TransferStatus.CANCELED)
        r.await("the DELETE") { r.pc.files.deleted.isNotEmpty() }
        assertEquals(listOf(id), r.pc.files.deleted.toList())
        assertNull("the PC dropped the partial files", r.pc.files.uploads[id])
        assertEquals("user", t.errorCode)
        assertTrue(r.notifier.events.contains("clear:$id"))
        assertFalse("nothing is left running", r.engine.isRunning(id))
        assertFalse(r.wake.held)
    }

    @Test
    fun aDeleteThatGets404CountsAsDone() {
        val r = rig()
        r.pc.files.postStatus = null
        r.sources.readDelayMillis = 40
        val id = r.share("big.bin" to TransferRig.bytes(600_000))
        r.await("the PUT to be under way") { r.sources.reads.get() > 2 }
        // The PC has forgotten the transfer (the first DELETE's 204 was lost, say): the DELETE gets 404 and that is fine.
        r.pc.files.uploads.remove(id)
        r.engine.cancel(id)
        r.awaitStatus(id, TransferStatus.CANCELED)
        r.await("the DELETE") { r.pc.files.deleted.isNotEmpty() }
        assertEquals("one DELETE: the 404 ended it", listOf(id), r.pc.files.deleted.toList())
        assertTrue(r.notifier.failed.isEmpty())
    }

    @Test
    fun thePcCancelsInFlightAndTheUploadStops() {
        val r = rig()
        r.sources.readDelayMillis = 40
        val id = r.share("big.bin" to TransferRig.bytes(1_000_000))
        r.await("the PUT to be under way") { r.sources.reads.get() > 2 }
        r.pc.sendCancel(id, "user")
        val t = r.awaitStatus(id, TransferStatus.CANCELED)
        assertEquals("peer_canceled", t.errorCode)
        assertEquals("Canceled on Desk PC.", r.notifier.failed.single().second)
        r.await("the runner to stop") { !r.engine.isRunning(id) }
        assertFalse(r.wake.held)
    }

    @Test
    fun aChangedSourceStartsOverUnderANewTransferIdAfterADelete() {
        val r = rig()
        val first = TransferRig.bytes(300_000, 1)
        val uri = r.sources.add("a.bin", first)
        val changed = TransferRig.bytes(320_000, 9)
        r.pc.files.dropPutKeeping = 100_000
        r.pc.files.afterDrop = { r.sources.fileOf(uri).writeBytes(changed) }
        val id = r.blocking { r.engine.enqueueUpload(listOf(ShareSource(uri))) }!!
        r.await("a second transfer to finish") { r.store.transfers.values.any { it.status == TransferStatus.DONE } }

        val done = r.store.transfers.values.single { it.status == TransferStatus.DONE }
        assertTrue("a new transfer ID", done.id != id)
        assertEquals("the old transfer was deleted on the PC", listOf(id), r.pc.files.deleted.toList())
        assertNull(r.pc.files.uploads[id])
        assertArrayEquals(changed, r.pc.files.uploads.getValue(done.id).files.getValue("f1").data)
        assertEquals(320_000L, done.totalBytes)
        assertEquals(listOf("POST", "PUT", "DELETE", "POST", "PUT"), r.pc.files.calls.map { it.method })
        assertEquals("the new transfer starts at 0", 0L, r.pc.files.calls.last().offset)
        assertNull("the old ID is gone from the store", r.store.transfers[id])
    }

    @Test
    fun aSourceThatCanNoLongerBeReadFailsWithShareItAgain() {
        val r = rig()
        r.pc.files.dropPutKeeping = 10_000
        r.pc.files.afterDrop = { r.sources.lost = true }
        val id = r.share("a.bin" to TransferRig.bytes(100_000))
        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("source_lost", t.errorCode)
        assertTrue(t.message!!, t.message!!.contains("Share it again"))
    }

    // ---- the PC isn't reachable

    @Test
    fun withNoConnectionTheTransferIsKeptReadyAndSmallFilesAreCopiedToTheCache() {
        val r = TransferRig().also { rigs += it } // never connected
        val small = TransferRig.bytes(400)
        val big = TransferRig.bytes(5_000) // over the 1,000-byte copy limit of the test timings
        val id = r.share("small.bin" to small, "big.bin" to big)
        r.await("the transfer to be parked") { r.notifier.queued.isNotEmpty() }
        val t = r.awaitStatus(id, TransferStatus.QUEUED)

        assertTrue("PC not reachable is shown", r.notifier.queued.single().second.contains("Wi-Fi"))
        assertEquals("unreachable", t.errorCode)
        val files = r.store.files.getValue(id).sortedBy { it.ordinal }
        assertTrue("the small file now reads from the cache", files[0].localUri!!.startsWith("content://test-cache/"))
        assertEquals("the big one still reads from the share", "content://test/big.bin", files[1].localUri)
        assertEquals(1, r.sources.cached.size)
        assertTrue("nothing went to the PC", r.pc.files.calls.isEmpty())
    }

    @Test
    fun aQueuedTransferGoesOutOnRetryAndItsCacheIsCleaned() {
        val r = TransferRig().also { rigs += it }
        val small = TransferRig.bytes(400)
        val id = r.share("small.bin" to small)
        r.await("the transfer to be parked") { r.notifier.queued.isNotEmpty() }
        assertEquals(1, r.sources.cached.size)

        r.connect()
        r.engine.retry(id)
        r.awaitStatus(id, TransferStatus.DONE)
        assertArrayEquals(small, r.stored(id, "f1").data)
        // The status is `done` a moment before the runner deletes the cache: wait for it instead of racing it.
        r.await("the cached copies to be deleted") { r.sources.cached.isEmpty() }
    }

    @Test
    fun aQueuedTransferDoesNotStartByItselfWhenThePcComesBack() {
        val r = TransferRig().also { rigs += it }
        val id = r.share("a.bin" to TransferRig.bytes(100))
        r.await("the transfer to be parked") { r.notifier.queued.isNotEmpty() }
        r.connect()
        r.blocking { r.engine.onLinkReady(r.pcId) }
        Thread.sleep(300)
        assertEquals("the persistent queue is 1.4: a reconnect doesn't send it", TransferStatus.QUEUED, r.store.transfers.getValue(id).status)
        r.blocking { r.engine.retryQueuedUploads() }
        r.awaitStatus(id, TransferStatus.DONE)
    }

    @Test
    fun anActiveUploadLeftByAnEarlierRunResumesWhenTheLinkIsReady() {
        val r = rig()
        val bytes = TransferRig.bytes(250_000)
        r.pc.files.dropPutKeeping = 90_000
        r.sources.readDelayMillis = 0
        // Simulate a killed process: the row says active, no job is running.
        val id = r.share("a.bin" to bytes)
        r.awaitStatus(id, TransferStatus.DONE)
        r.store.transfers.computeIfPresent(id) { _, t -> t.copy(status = TransferStatus.ACTIVE) }
        r.store.files.getValue(id).replaceAll { it.copy(finished = false) }
        r.pc.files.uploads.getValue(id).files.getValue("f1").data = bytes.copyOf(90_000)
        r.blocking { r.engine.onLinkReady(r.pcId) }
        r.awaitStatus(id, TransferStatus.DONE)
        assertArrayEquals(bytes, r.stored(id, "f1").data)
    }

    @Test
    fun aLongTextBecomesATxtFile() {
        val r = rig()
        val text = "line\n".repeat(30_000)
        val id = r.blocking { r.engine.enqueueText(text, "Kin text 2026-10-15 10.00.00.txt") }!!
        r.awaitStatus(id, TransferStatus.DONE)
        val file = r.stored(id, "f1")
        assertEquals("Kin text 2026-10-15 10.00.00.txt", file.name)
        assertEquals("text/plain", file.mime)
        assertEquals(text, String(file.data, Charsets.UTF_8))
    }

    @Test
    fun tooManyFilesAreRefusedWithAMessage() {
        val r = rig()
        val sources = (1..1001).map { ShareSource("content://test/$it") }
        assertNull(r.blocking { r.engine.enqueueUpload(sources) })
        assertEquals(1, r.notifier.messages.size)
        assertTrue(r.pc.files.calls.isEmpty())
    }

    @Test
    fun filesThatCantBeReadAreSkippedAndIfNoneIsLeftNothingIsCreated() {
        val r = rig()
        val good = r.sources.add("ok.bin", TransferRig.bytes(10))
        val id = r.blocking { r.engine.enqueueUpload(listOf(ShareSource("content://test/missing"), ShareSource(good))) }!!
        r.awaitStatus(id, TransferStatus.DONE)
        assertEquals(1, r.pc.files.uploads.getValue(id).files.size)
        assertNull(r.blocking { r.engine.enqueueUpload(listOf(ShareSource("content://test/missing"))) })
    }
}
