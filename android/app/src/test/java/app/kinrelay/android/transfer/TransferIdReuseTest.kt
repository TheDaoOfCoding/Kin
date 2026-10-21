package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.FakePcFiles
import app.kinrelay.android.testing.TransferRig
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/**
 * A transfer ID is never reused (protocol.md, Files: phone -> PC; the 1.4 rulings). A `404` to a `POST` means the PC ended that transfer
 * for good, or the ID belongs to another device, or it was used before and is over.
 * - An **automatic** resume that gets it marks the transfer failed ("the PC no longer has it") and **never sends again**.
 * - Only an explicit **user Retry** may send again: it first `POST`s the old ID (so a transfer the PC finished within 24 h is recognised),
 *   and on `404` sends under a **new ID only the files that never got a `200`**.
 */
class TransferIdReuseTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(newId: (() -> String)? = null, connected: Boolean = true): TransferRig =
        TransferRig(newId = newId).also { rigs += it; if (connected) it.connect() }

    private fun TransferRig.share(vararg files: Pair<String, ByteArray>): String =
        blocking { engine.enqueueUpload(files.map { (name, bytes) -> ShareSource(sources.add(name, bytes)) }) }!!

    /** A failed upload of [files] ([name, bytes, finished]) as a user would find it in the history, with its sources readable. */
    private fun TransferRig.failedUpload(id: String, vararg files: Triple<String, ByteArray, Boolean>): List<TransferFileEntity> {
        val rows = files.mapIndexed { i, (name, bytes, finished) ->
            TransferFileEntity(id, "f${i + 1}", i, name, null, "application/octet-stream", bytes.size.toLong(), if (finished) bytes.size.toLong() else 0, sources.add(name, bytes), null, finished)
        }
        val transfer = TransferEntity(
            id, TransferDirection.UP, pcId, "Desk PC", TransferKind.FILE, TransferStatus.FAILED, 1_000, 2_000, rows.sumOf { it.size },
            "connection_lost", "It failed.", completeAcked = false,
        )
        blocking { store.create(transfer, rows) }
        return rows
    }

    private fun TransferRig.pcHas(id: String, vararg files: Triple<String, ByteArray, Boolean>) {
        val uploaded = LinkedHashMap<String, FakePcFiles.UploadedFile>()
        files.forEachIndexed { i, (name, bytes, complete) ->
            uploaded["f${i + 1}"] = FakePcFiles.UploadedFile("f${i + 1}", name, bytes.size.toLong(), "application/octet-stream").also { if (complete) it.data = bytes }
        }
        pc.files.uploads[id] = FakePcFiles.Upload(id, uploaded)
    }

    // ------------------------------------------------------------ an automatic resume never sends again

    @Test
    fun anAutomaticResumeThatGetsA404FailsTheTransferAndSendsNothing() {
        val r = rig(newId = { "fixed-1" }, connected = false)
        val id = r.share("a.bin" to TransferRig.bytes(500)) // no connection: queued, a copy in the cache
        assertEquals("fixed-1", id)
        r.await("the upload to be parked") { r.notifier.queued.any { it.first.id == id } }
        // While the phone was away the PC canceled it (a cancel on the PC): the ID is used up.
        r.pc.files.usedIds += id

        r.connect()
        r.blocking { r.engine.retryQueuedUploads() } // what the queue does after the hello: an automatic start

        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("not_found", t.errorCode)
        assertTrue(t.message.orEmpty(), t.message.orEmpty().contains("no longer has"))
        Thread.sleep(250)
        assertEquals("one transfer row: nothing was made under a new ID", 1, r.store.transfers.size)
        assertEquals("a single POST, answered 404, and no PUT", listOf("POST"), r.pc.files.calls.map { it.method })
        assertTrue(r.pc.files.uploads.isEmpty())
        assertEquals(1, r.notifier.failed.size)
        assertFalse(r.wake.held)
    }

    @Test
    fun theRecoveryOfAnActiveUploadAtReconnectIsAutomaticToo() {
        val r = rig()
        r.sources.readDelayMillis = 30
        val id = r.share("big.bin" to TransferRig.bytes(600_000))
        r.await("the PUT to be under way") { r.sources.reads.get() > 2 }
        r.pc.files.usedIds += id // the PC forgot it (or canceled it on its side) meanwhile
        r.dropConnection()
        r.connect()

        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("not_found", t.errorCode)
        assertEquals(1, r.store.transfers.size)
        assertTrue("no new transfer was made", r.pc.files.uploads.values.none { it.id != id })
    }

    @Test
    fun aSecond404IsFinalSoThePhoneNeverLoops() {
        val r = rig()
        r.pc.files.postStatus = 404
        r.share("c.bin" to TransferRig.bytes(1_000))
        r.await("the transfer to fail") { r.store.transfers.values.singleOrNull()?.status == TransferStatus.FAILED }
        assertEquals("not_found", r.store.transfers.values.single().errorCode)
        Thread.sleep(200)
        assertEquals("one POST and no start over", listOf("POST"), r.pc.files.calls.map { it.method })
    }

    // ------------------------------------------------------------ a user Retry

    @Test
    fun aUserRetryPostsTheOldIdFirstSoATransferThePcFinishedIsRecognised() {
        val r = rig()
        val bytes = TransferRig.bytes(20_000, 4)
        val file = Triple("a.bin", bytes, false)
        r.failedUpload("old-1", file)
        r.pcHas("old-1", Triple("a.bin", bytes, true)) // the PC has it all (it finished, and the phone missed the 200)

        r.engine.retry("old-1")
        r.awaitStatus("old-1", TransferStatus.DONE)

        assertEquals("the same transfer, the same ID, no new one", 1, r.store.transfers.size)
        val calls = r.pc.files.calls
        assertEquals("POST", calls.first().method)
        assertEquals("/v1/transfers", calls.first().path)
        assertEquals("an empty PUT at the file's size closes it", "/v1/transfers/old-1/files/f1", calls[1].path)
        assertEquals(20_000L, calls[1].offset)
        assertEquals(2, calls.size)
        assertNull(r.store.transfers.getValue("old-1").retriedAs)
    }

    @Test
    fun aUserRetryThatGets404SendsOnlyTheFilesWithoutA200UnderANewId() {
        val ids = AtomicInteger()
        val r = rig(newId = { "new-${ids.incrementAndGet()}" })
        val a = Triple("a.bin", TransferRig.bytes(4_000, 1), true) // got its 200 before the failure
        val b = Triple("b.bin", TransferRig.bytes(5_000, 2), false)
        val c = Triple("c.bin", TransferRig.bytes(6_000, 3), false)
        r.failedUpload("old-2", a, b, c)
        r.pc.files.usedIds += "old-2" // the PC canceled it: 404 from now on

        r.engine.retry("old-2")
        r.awaitStatus("new-1", TransferStatus.DONE)

        // The PC got a POST with the old ID (404), then a POST with a new ID that lists only b and c, and only those were sent.
        assertEquals(listOf("POST", "POST", "PUT", "PUT"), r.pc.files.calls.map { it.method })
        val upload = r.pc.files.uploads.getValue("new-1")
        assertEquals(setOf("f2", "f3"), upload.files.keys)
        assertArrayEquals(b.second, upload.files.getValue("f2").data)
        assertArrayEquals(c.second, upload.files.getValue("f3").data)
        assertNull("the old ID never became a transfer on the PC", r.pc.files.uploads["old-2"])
        // The old row stays as it ended, and points at its successor; Retry on it does nothing more.
        val old = r.store.transfers.getValue("old-2")
        assertEquals(TransferStatus.FAILED, old.status)
        assertEquals("new-1", old.retriedAs)
        assertEquals("the new transfer has only the two files", 2, r.store.files.getValue("new-1").size)
        assertEquals("the old row keeps its three files", 3, r.store.files.getValue("old-2").size)
        val callsBefore = r.pc.files.calls.size
        r.engine.retry("old-2")
        Thread.sleep(250)
        assertEquals("no second retry of an already retried transfer", callsBefore, r.pc.files.calls.size)
        assertEquals(1, r.store.transfers.values.count { it.id.startsWith("new-") })
    }

    @Test
    fun aUserRetryThatMeets404TwiceStopsThere() {
        val ids = AtomicInteger()
        val r = rig(newId = { "new-${ids.incrementAndGet()}" })
        r.failedUpload("old-3", Triple("a.bin", TransferRig.bytes(1_000), false))
        r.pc.files.usedIds += listOf("old-3", "new-1")

        r.engine.retry("old-3")

        val t = r.awaitStatus("new-1", TransferStatus.FAILED)
        assertEquals("not_found", t.errorCode)
        Thread.sleep(200)
        assertEquals("the old ID, the new one, and no third", listOf("POST", "POST"), r.pc.files.calls.map { it.method })
        assertEquals(2, r.store.transfers.size)
    }

    @Test
    fun theNewTransferOfARetryKeepsTheCachedCopiesOfTheFiles() {
        val ids = AtomicInteger()
        val r = rig(newId = { "new-${ids.incrementAndGet()}" })
        r.failedUpload("old-4", Triple("a.bin", TransferRig.bytes(2_000), false))
        r.pc.files.usedIds += "old-4"
        java.io.File(r.sources.cacheDir, "old-4").also { it.mkdirs() }.resolve("f1").writeText("cached")

        r.engine.retry("old-4")
        r.awaitStatus("new-1", TransferStatus.DONE)

        // The cache is keyed by the stable local ID ("old-4" for both rows), and it is emptied when the new transfer is over.
        assertEquals("old-4", r.store.transfers.getValue("new-1").localId)
        r.await("the cache to be emptied") { !java.io.File(r.sources.cacheDir, "old-4").exists() }
    }

    // ------------------------------------------------------------ an offer that reuses an ID (ruling 2)

    private fun offerFile(name: String, size: Int) = FakePcFiles.OfferedFile("f1", name, "application/octet-stream", TransferRig.bytes(size))

    @Test
    fun anOfferWithTheIdOfAFinishedDownloadIsRefusedWithBadRequestAndTheOfferId() {
        val r = rig()
        r.pc.sendOffer("once", offerFile("a.bin", 5_000))
        r.awaitStatus("once", TransferStatus.DONE)
        // The de-duplication memory of a fresh session doesn't know the ID (24 hours passed), but the transfer store does.
        r.dropConnection()
        r.connect(dedup = app.kinrelay.android.protocol.DedupMemory(System::currentTimeMillis))
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offered("once", offerFile("b.bin", 100))))

        val reply = r.pc.replyTo("once")
        assertEquals("error", reply.getValue("type").jsonPrimitive.content)
        assertEquals("bad_request", reply.getValue("code").jsonPrimitive.content)
        assertEquals("once", reply.getValue("ref").jsonPrimitive.content)
        assertEquals("nothing new was saved", 1, r.sink.entries.size)
    }

    @Test
    fun theSamePcOfferingAnUnfinishedDownloadAgainIsAckedAndResumes() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 40_000
        r.pc.sendOffer("again", offerFile("big.bin", 400_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.dropConnection()
        r.pc.files.throttleGetBytesPer100Ms = null
        r.connect(dedup = app.kinrelay.android.protocol.DedupMemory(System::currentTimeMillis)) // a phone that remembers nothing of the offer's ID
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offers.getValue("again")))

        val reply = r.pc.replyTo("again")
        assertEquals("an ack, not an error", "ack", reply.getValue("type").jsonPrimitive.content)
        assertEquals("again", reply.getValue("ref").jsonPrimitive.content)
        r.awaitStatus("again", TransferStatus.DONE)
        assertEquals("still one entry: it was resumed, not started twice", 1, r.sink.entries.size)
        assertNotNull(r.pc.files.calls.firstOrNull { it.range != null })
    }
}

/** The next frame from the phone that answers [ref] (an `ack` or an `error`), skipping other messages such as `hello` and `complete`. */
private fun FakePc.replyTo(ref: String): kotlinx.serialization.json.JsonObject {
    val deadline = System.currentTimeMillis() + 5_000
    while (System.currentTimeMillis() < deadline) {
        val frame = nextFrame(500) ?: continue
        if (frame["ref"]?.jsonPrimitive?.content == ref) return frame
    }
    error("no reply to $ref")
}
