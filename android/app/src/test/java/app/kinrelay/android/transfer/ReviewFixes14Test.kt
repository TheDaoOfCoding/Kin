package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.link.readyFor
import app.kinrelay.android.protocol.Offer
import app.kinrelay.android.protocol.OfferFile
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.FakePcFiles.OfferedFile
import app.kinrelay.android.testing.TransferRig
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.atomic.AtomicInteger

/** The fixes of the 1.3 review that came with milestone 1.4 (free space, the cache key, a late cancel, `source_lost`, the re-pairing window). */
class ReviewFixes14Test {
    private val rigs = mutableListOf<TransferRig>()
    private val extraPcs = mutableListOf<FakePc>()

    @After
    fun tearDown() {
        rigs.forEach { it.close() }
        extraPcs.forEach { it.close() }
    }

    private fun rig(newId: (() -> String)? = null): TransferRig = TransferRig(newId = newId).also { rigs += it; it.connect() }

    private fun offer(id: String, vararg sizes: Long) =
        Offer(id, sizes.mapIndexed { i, size -> OfferFile("f${i + 1}", "file$i.bin", size, "application/octet-stream") }, sizes.sum())

    private fun TransferRig.share(vararg files: Pair<String, ByteArray>): String =
        blocking { engine.enqueueUpload(files.map { (name, bytes) -> ShareSource(sources.add(name, bytes)) }) }!!

    private fun TransferRig.row(id: String, direction: String, peer: String, status: String, localId: String = id, uri: String? = null, size: Long = 1_000) {
        val transfer = TransferEntity(id, direction, peer, "Some PC", TransferKind.FILE, status, 1_000, null, size, null, null, completeAcked = false, localId = localId)
        blocking { store.create(transfer, listOf(TransferFileEntity(id, "f1", 0, "f.bin", null, "application/octet-stream", size, 0, uri, null, finished = false))) }
    }

    // ---------------------------------------------------------------- 2. free space is not counted twice during a download

    @Test
    fun bytesAlreadyWrittenAreNotCountedTwiceByTheFreeSpaceCheck() {
        val r = rig()
        r.sink.capacity = 1_000_000 // free space shrinks as the download writes, like a real disk
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("big", OfferedFile("f1", "big.bin", "application/octet-stream", TransferRig.bytes(600_000)))
        r.await("250,000 bytes to be written") { r.sink.entries.values.any { it.file.length() >= 250_000 } }
        assertEquals("the store only learns the bytes when the download stops", 0L, r.store.files.getValue("big").single().bytesDone)

        // Free is about 750,000 and the download still has about 350,000 to write: another 350,000 fits (950,000 would be wrong:
        // that is what counting the written bytes twice gave, since the store still says 0 written).
        assertNull("350,000 fits next to what is still to be written", r.engine.acceptOffer(offer("second", 350_000), r.pcId))
        assertEquals("450,000 does not", "disk_full", r.engine.acceptOffer(offer("third", 450_000), r.pcId))
        r.engine.cancel("big")
        r.awaitStatus("big", TransferStatus.CANCELED)
    }

    @Test
    fun theWrittenBytesAreSavedWhenADownloadStopsSoTheNextCheckStaysRight() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("stop", OfferedFile("f1", "s.bin", "application/octet-stream", TransferRig.bytes(600_000)))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() >= 100_000 } }
        r.dropConnection() // the connection ends: the download waits for the PC
        r.await("the live number to be saved") { r.store.files.getValue("stop").single().bytesDone > 0 }
        assertTrue(r.store.files.getValue("stop").single().bytesDone >= 100_000)
    }

    // ---------------------------------------------------------------- 3. the cache survives a new transfer ID

    @Test
    fun theStartupSweepKeepsTheCacheOfATransferThatWasRenamed() {
        val r = rig()
        // A transfer that was made under "old" and started over as "new": its cached copies are in the folder named after its local ID.
        r.row("new", TransferDirection.UP, r.pcId, TransferStatus.QUEUED, localId = "old")
        File(r.sources.cacheDir, "old").also { it.mkdirs() }.resolve("f1").writeText("cached")
        File(r.sources.cacheDir, "new").also { it.mkdirs() }.resolve("f1").writeText("nobody owns this")

        r.blocking { r.engine.sweepOrphans() }

        assertTrue("the folder of the local ID stays", File(r.sources.cacheDir, "old").exists())
        assertFalse("a folder named after nothing that is live goes", File(r.sources.cacheDir, "new").exists())
    }

    @Test
    fun aChangedSourceStartsOverUnderANewIdAndTheCachedCopyIsStillThereAndIsFinallyDeleted() {
        val ids = AtomicInteger()
        val r = rig(newId = { "id-${ids.incrementAndGet()}" })
        r.dropConnection()
        val id = r.share("a.bin" to TransferRig.bytes(500, 9)) // queued, copied into the cache under its local ID
        r.await("parked") { r.notifier.queued.any { it.first.id == id } }
        val cacheDir = File(r.sources.cacheDir, id)
        assertTrue(cacheDir.exists())
        // The cached copy changes (its size): the next attempt starts over under a new transfer ID.
        val cachedUri = r.store.files.getValue(id).single().localUri!!
        r.sources.fileOf(cachedUri).appendBytes(byteArrayOf(1, 2, 3))

        r.connect()
        r.engine.retry(id)
        r.await("the transfer to be done under another ID") { r.store.transfers.values.any { it.id != id && it.status == TransferStatus.DONE } }

        val done = r.store.transfers.values.single { it.status == TransferStatus.DONE }
        assertEquals("the local ID did not change with the transfer ID", id, done.localId)
        assertTrue("it went out from the cached copy", r.pc.files.uploads.getValue(done.id).files.getValue("f1").data.size == 503)
        r.await("the cache to be deleted at the end") { !cacheDir.exists() }
    }

    @Test
    fun aRenameThatFailsLeavesTheTransferUnderItsOldId() {
        val r = rig()
        r.dropConnection()
        val id = r.share("a.bin" to TransferRig.bytes(500, 1))
        r.await("parked") { r.notifier.queued.any { it.first.id == id } }
        r.sources.fileOf(r.store.files.getValue(id).single().localUri!!).appendBytes(byteArrayOf(1))
        r.store.failNextRename = true // the store does the rename in one transaction: if it fails, nothing moved

        r.connect()
        r.engine.retry(id)

        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("internal", t.errorCode)
        assertEquals("still one transfer, still under its ID, with its file", 1, r.store.transfers.size)
        assertEquals(1, r.store.files.getValue(id).size)
    }

    // ---------------------------------------------------------------- 4. a late cancel never turns DONE into canceled

    @Test
    fun aLateCancelFromThePcNeverDowngradesADoneTransfer() {
        val r = rig()
        r.pc.sendOffer("done-1", OfferedFile("f1", "a.bin", "application/octet-stream", TransferRig.bytes(10_000)))
        r.awaitStatus("done-1", TransferStatus.DONE)
        val failedBefore = r.notifier.failed.size

        r.engine.onPeerCancel(r.pcId, "done-1", "user")
        Thread.sleep(300)

        assertEquals(TransferStatus.DONE, r.store.transfers.getValue("done-1").status)
        assertEquals("no 'canceled' notification either", failedBefore, r.notifier.failed.size)
        assertEquals("the saved file stays", 1, r.sink.visible().size)
    }

    @Test
    fun theStoreNeverRewritesATransferThatIsOver() {
        val r = rig()
        r.row("over", TransferDirection.DOWN, r.pcId, TransferStatus.DONE)
        assertFalse(r.blocking { r.store.setStatus("over", TransferStatus.CANCELED, "peer_canceled", "x", 5) })
        assertEquals(TransferStatus.DONE, r.store.transfers.getValue("over").status)
        r.row("failed", TransferDirection.UP, r.pcId, TransferStatus.FAILED)
        assertFalse("a failed transfer isn't rewritten by a status change either (only Retry requeues it)", r.blocking { r.store.setStatus("failed", TransferStatus.ACTIVE) })
        assertTrue(r.blocking { r.store.requeue("failed") })
        assertEquals(TransferStatus.QUEUED, r.store.transfers.getValue("failed").status)
    }

    // ---------------------------------------------------------------- 5. a source that vanishes while starting over is `source_lost`

    @Test
    fun aSourceThatDisappearsWhileTheTransferStartsOverIsSourceLostNotACrash() {
        val r = rig()
        r.dropConnection()
        val id = r.share("a.bin" to TransferRig.bytes(500, 2))
        r.await("parked") { r.notifier.queued.any { it.first.id == id } }
        val info = r.sources.stat(r.store.files.getValue(id).single().localUri!!)!!
        // The check sees a changed size (so the transfer must start over), and when it reads the file again to start over it is gone.
        r.sources.statScript += TransferRigStat(info.copy(size = info.size + 1))
        r.sources.statScript += TransferRigStat(null)
        r.connect()
        r.engine.retry(id)

        val t = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("source_lost", t.errorCode)
        assertEquals("not a crash", false, t.errorCode == "internal")
        assertEquals("renamed nothing: it is still the same transfer", 1, r.store.transfers.size)
        assertTrue(r.notifier.failed.single().second.contains("Share it again"))
    }

    // ---------------------------------------------------------------- 6. the re-pairing window

    private fun secondPc(r: TransferRig): FakePc = FakePc(name = "New PC").apply { pairedPhones += r.phone.deviceId }.start().also { extraPcs += it }

    @Test
    fun aTransferIsNeverSentOnTheConnectionOfAnotherPc() {
        val r = rig() // connected to the first PC
        val second = secondPc(r)
        // The user paired the second PC; the old connection is still the current one for a moment.
        r.pairedPeer = PeerInfo(second.identity.deviceId, "New PC")
        val id = r.share("a.bin" to TransferRig.bytes(300, 5))

        val t = r.awaitStatus(id, TransferStatus.QUEUED)
        assertEquals("the transfer is for the new PC", second.identity.deviceId, t.peerDeviceId)
        assertTrue("nothing went to the old PC", r.pc.files.calls.isEmpty())

        // The connection to the new PC comes up: now it goes, to that PC.
        r.connect(to = second)
        r.blocking { r.engine.retryQueuedUploads() }
        r.awaitStatus(id, TransferStatus.DONE)
        assertEquals(1, second.files.uploads.size)
        assertTrue("and still nothing at the old PC", r.pc.files.calls.isEmpty())
    }

    @Test
    fun anOfferFromAPcThatIsNotThePairedOneIsRefusedAndNotRecordedAgainstThePairedPc() {
        val r = rig()
        val second = secondPc(r)
        r.pairedPeer = PeerInfo(second.identity.deviceId, "New PC") // re-paired; the old session still delivers messages

        assertEquals("bad_request", r.engine.acceptOffer(offer("stale", 100), r.pcId))
        r.blocking { r.engine.onOffer(r.pcId, "Desk PC", offer("stale", 100)) }
        assertNull("nothing was recorded for the new PC", r.store.transfers["stale"])
        assertNull("the paired PC's own offer is accepted", r.engine.acceptOffer(offer("fresh", 100), second.identity.deviceId))
    }

    @Test
    fun aCancelFromAPcThatDoesNotOwnTheTransferIsIgnored() {
        val r = rig()
        r.row("mine", TransferDirection.DOWN, "the-new-pc", TransferStatus.QUEUED)
        r.engine.onPeerCancel(r.pcId, "mine", "user") // the old PC's connection
        Thread.sleep(250)
        assertEquals(TransferStatus.QUEUED, r.store.transfers.getValue("mine").status)
        r.engine.onPeerCancel("the-new-pc", "mine", "user")
        r.awaitStatus("mine", TransferStatus.CANCELED)
    }

    @Test
    fun theUserCancelOfADownloadIsOnlyEverSaidToThePcItBelongsTo() {
        val r = rig() // connected to the old PC
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("dl", OfferedFile("f1", "d.bin", "application/octet-stream", TransferRig.bytes(300_000)))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        // The transfer belongs to "the old PC"; the paired one is another, and only the old PC's connection is up.
        r.blocking { r.store.create(
            TransferEntity("dl2", TransferDirection.DOWN, "somebody-else", "Else", TransferKind.FILE, TransferStatus.QUEUED, 5_000, null, 10, null, null, false),
            listOf(TransferFileEntity("dl2", "f1", 0, "x.bin", null, "application/octet-stream", 10, 0, null, null, false)),
        ) }
        r.engine.cancel("dl2")
        r.awaitStatus("dl2", TransferStatus.CANCELED)
        assertTrue("no cancel message went to the connected PC", r.pc.files.messages.none { it.contains("dl2") })
        r.engine.cancel("dl")
        r.awaitStatus("dl", TransferStatus.CANCELED)
    }

    @Test
    fun aConnectionIsOnlyReadyForThePcItIsPinnedTo() {
        val r = rig()
        val session = r.current!!
        assertSame(session, session.readyFor(r.pcId))
        assertNull(session.readyFor("another-pc"))
        val none: LinkConnection? = null
        assertNull(none.readyFor(r.pcId))
        assertNotNull(session.peerDeviceId)
        assertEquals(r.pcId, session.peerDeviceId)
    }
}

private typealias TransferRigStat = app.kinrelay.android.testing.TempSources.StatStep
