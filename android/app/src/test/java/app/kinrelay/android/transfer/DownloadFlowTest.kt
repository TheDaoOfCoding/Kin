package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.FakePcFiles.OfferedFile
import app.kinrelay.android.testing.TransferRig
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** PC -> phone, end to end against the fake PC: the `offer`, the `GET` (with `Range` when resuming), MediaStore-style saving, `complete`. */
class DownloadFlowTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(timings: TransferTimings = TransferRig.FAST): TransferRig = TransferRig(timings = timings).also { rigs += it; it.connect() }

    private fun file(id: String, name: String, size: Int, mime: String = "application/octet-stream", seed: Int = 1) =
        OfferedFile(id, name, mime, TransferRig.bytes(size, seed))

    @Test
    fun anOfferIsAckedThenDownloadedAndCompleteIsSent() {
        val r = rig()
        val a = file("f1", "IMG_0042.jpg", 120_000, "image/jpeg")
        val b = file("f2", "notes.txt", 5_000, "text/plain", 2)
        r.pc.sendOffer("offer-1", a, b)
        r.awaitStatus("offer-1", TransferStatus.DONE)

        // acked first
        val ack = r.pc.nextFrame()!!
        assertEquals("ack", ack.getValue("type").jsonPrimitive.content)
        assertEquals("offer-1", ack.getValue("ref").jsonPrimitive.content)
        // saved, visible, with the right bytes, in the right folders
        val saved = r.sink.visible()
        assertEquals(2, saved.size)
        assertEquals(SaveFolder.PICTURES, saved.single { it.name == "IMG_0042.jpg" }.folder)
        assertEquals(SaveFolder.DOWNLOADS, saved.single { it.name == "notes.txt" }.folder)
        assertArrayEquals(a.bytes, saved.single { it.name == "IMG_0042.jpg" }.file.readBytes())
        assertArrayEquals(b.bytes, saved.single { it.name == "notes.txt" }.file.readBytes())
        // complete went out and was acked
        r.await("complete to be acked") { r.store.transfers.getValue("offer-1").completeAcked }
        assertTrue(r.pc.files.messages.single().startsWith("complete:offer-1"))
        assertFalse(r.wake.held)
        // the notification says what arrived
        val (transfer, files) = r.notifier.received.single()
        assertEquals("2 files from Desk PC", TransferNotifications.receivedTitle(transfer.peerName, files))
    }

    @Test
    fun aSinglePhotoIsPhotoFromThePcAndAVideoGoesToMovies() {
        val r = rig()
        r.pc.sendOffer("photo", file("f1", "a.jpg", 1_000, "image/jpeg"))
        r.awaitStatus("photo", TransferStatus.DONE)
        val (t1, f1) = r.notifier.received.single()
        assertEquals("Photo from Desk PC", TransferNotifications.receivedTitle(t1.peerName, f1))

        r.pc.sendOffer("video", file("f1", "clip.mp4", 2_000, "video/mp4"))
        r.awaitStatus("video", TransferStatus.DONE)
        assertEquals(SaveFolder.MOVIES, r.sink.visible().single { it.name == "clip.mp4" }.folder)
        val (t2, f2) = r.notifier.received.last()
        assertEquals("Video from Desk PC", TransferNotifications.receivedTitle(t2.peerName, f2))
    }

    @Test
    fun theMimeTypeOfAnOfferIsNotTrustedForTheFolder() {
        val r = rig()
        // An .exe that claims to be a photo is a download, not a gallery item; an unknown extension follows the sender's type.
        r.pc.sendOffer("x", file("f1", "trick.exe", 100, "image/jpeg"), file("f2", "holiday.heic", 100, "image/heic"))
        r.awaitStatus("x", TransferStatus.DONE)
        assertEquals(SaveFolder.DOWNLOADS, r.sink.visible().single { it.name == "trick.exe" }.folder)
        assertEquals(SaveFolder.PICTURES, r.sink.visible().single { it.name == "holiday.heic" }.folder)
    }

    @Test
    fun theNamesAreCleanedAndTheNameTheSystemChoseIsKept() {
        val r = rig()
        r.pc.sendOffer("n1", file("f1", "..\\evil/CON.txt", 10), file("f2", "a.jpg", 10, "image/jpeg"))
        r.awaitStatus("n1", TransferStatus.DONE)
        assertEquals(setOf("_CON.txt", "a.jpg"), r.sink.visible().map { it.name }.toSet())
        // The same name again: our own collision handling asks for "a (1).jpg".
        r.pc.sendOffer("n2", file("f1", "a.jpg", 10, "image/jpeg"))
        r.awaitStatus("n2", TransferStatus.DONE)
        assertEquals("a (1).jpg", r.store.files.getValue("n2").single().finalName)

    }

    @Test
    fun theFinalNameThatTheSinkChoseIsTheOneStored() {
        val r = rig()
        // The sink already has "b.txt" from elsewhere, which the phone doesn't list: create() renames, and that name is kept.
        r.sink.create(SaveFolder.DOWNLOADS, "b.txt", "text/plain").also { r.sink.finish(it.uri) }
        r.pc.sendOffer("n3", file("f1", "b.txt", 10))
        r.awaitStatus("n3", TransferStatus.DONE)
        assertEquals("b (1).txt", r.store.files.getValue("n3").single().finalName)
        assertTrue(r.sink.visible().any { it.name == "b (1).txt" })
    }

    @Test
    fun aNameTakenByAFileTheAppCantSeeTriesTheNextSuffixInsteadOfFailingTheTransfer() {
        val r = rig()
        // Found by the integration run: a file of an earlier install makes MediaStore's unique index refuse the insert (SQLiteConstraintException)
        r.sink.hiddenNames += "Text.txt"
        r.pc.sendOffer("h1", file("f1", "Text.txt", 10, "text/plain"))
        r.awaitStatus("h1", TransferStatus.DONE)
        assertEquals("Text (1).txt", r.store.files.getValue("h1").single().finalName)
        assertTrue(r.sink.visible().any { it.name == "Text (1).txt" })
    }

    @Test
    fun aZeroByteFileIsFetchedWithAPlainGetAndNeverARange() {
        val r = rig()
        r.pc.sendOffer("z", file("f1", "empty.txt", 0, "text/plain"))
        r.awaitStatus("z", TransferStatus.DONE)
        val get = r.pc.files.calls.single { it.method == "GET" }
        assertNull("no Range for a 0-byte file", get.range)
        assertEquals(0, r.sink.visible().single().file.length())
    }

    @Test
    fun aDroppedConnectionResumesWithARangeRequest() {
        val r = rig()
        val a = file("f1", "big.bin", 300_000)
        r.pc.files.dropGetAfter = 90_000
        r.pc.sendOffer("resume", a)
        r.awaitStatus("resume", TransferStatus.DONE)

        val gets = r.pc.files.calls.filter { it.method == "GET" }
        assertEquals(2, gets.size)
        assertNull("the first GET asks for everything", gets[0].range)
        assertEquals("bytes=90000-", gets[1].range)
        assertArrayEquals("appended in place", a.bytes, r.sink.visible().single().file.readBytes())
        assertFalse(r.wake.held)
    }

    @Test
    fun a200ToARangeRequestRestartsTheFileAtByteZero() {
        val r = rig()
        val a = file("f1", "big.bin", 200_000)
        r.pc.files.dropGetAfter = 60_000
        r.pc.files.ignoreRange = true
        r.pc.sendOffer("restart", a)
        r.awaitStatus("restart", TransferStatus.DONE)
        val gets = r.pc.files.calls.filter { it.method == "GET" }
        assertEquals("bytes=60000-", gets[1].range)
        assertArrayEquals("the 200 started the file over: no doubled bytes", a.bytes, r.sink.visible().single().file.readBytes())
    }

    @Test
    fun aFileThatIsAlreadyCompleteSkipsTheGet() {
        val r = rig()
        val a = file("f1", "done.bin", 40_000)
        r.pc.files.offered("have", a)
        // A previous run saved every byte but died before it finished the entry.
        val entry = r.sink.create(SaveFolder.DOWNLOADS, "done.bin", "application/octet-stream")
        r.sink.openForWrite(entry.uri, 0).use { it.write(a.bytes) }
        r.blocking {
            r.store.create(
                app.kinrelay.android.data.TransferEntity("have", "down", r.pc.identity.deviceId, "Desk PC", "file", TransferStatus.ACTIVE, 1, null, 40_000, null, null, false),
                listOf(app.kinrelay.android.data.TransferFileEntity("have", "f1", 0, "done.bin", entry.finalName, "application/octet-stream", 40_000, 40_000, entry.uri, null, false)),
            )
            r.engine.onLinkReady(r.pcId)
        }
        r.awaitStatus("have", TransferStatus.DONE)
        assertTrue("no GET at all", r.pc.files.calls.none { it.method == "GET" })
        assertTrue(r.sink.visible().isNotEmpty())
    }

    @Test
    fun a404FailsTheWholeOfferWithACancelWithReasonError() {
        val r = rig()
        val a = file("f1", "a.bin", 20_000)
        val b = file("f2", "b.bin", 20_000)
        r.pc.sendOffer("gone", a, b)
        r.pc.files.offers.getValue("gone").gone = true
        val t = r.awaitStatus("gone", TransferStatus.FAILED)
        r.await("the cancel") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:gone:error", r.pc.files.messages.single())
        assertEquals("not_found", t.errorCode)
        assertTrue("nothing is left behind", r.sink.entries.isEmpty())
        assertEquals(1, r.notifier.failed.size)
        assertFalse(r.wake.held)
    }

    @Test
    fun a409SourceChangedFailsTheOfferAndKeepsTheFilesAlreadySaved() {
        val r = rig()
        val a = file("f1", "a.bin", 20_000)
        val b = file("f2", "b.bin", 20_000)
        val offered = r.pc.files.offered("changed", a, b)
        offered.files.getValue("f2").changed = true
        // The files go side by side: the changed one answers late, so `a` is saved completely before the offer fails.
        offered.files.getValue("f2").delayMillis = 400
        r.pc.send(r.pc.files.offerFrame(offered))
        val t = r.awaitStatus("changed", TransferStatus.FAILED)
        r.await("the cancel") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:changed:error", r.pc.files.messages.single())
        assertEquals("source_changed", t.errorCode)
        assertEquals("the completely saved file stays", listOf("a.bin"), r.sink.visible().map { it.name })
        assertEquals("the half-done one is deleted", 1, r.sink.entries.size)
        assertEquals(1, r.store.files.getValue("changed").count { it.finished })
    }

    @Test
    fun thePcCancelsADownloadInFlightAndThePartialFileIsDeleted() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("slow", file("f1", "big.bin", 600_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.pc.sendCancel("slow", "user")
        val t = r.awaitStatus("slow", TransferStatus.CANCELED)
        assertEquals("peer_canceled", t.errorCode)
        r.await("the partial file to go") { r.sink.entries.isEmpty() }
        r.await("the runner to stop") { !r.engine.isRunning("slow") }
        assertFalse(r.wake.held)
        assertTrue("no complete", r.pc.files.messages.none { it.startsWith("complete") })
    }

    @Test
    fun theUserCancelsADownloadAndACancelWithReasonUserGoesToThePc() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("slow2", file("f1", "big.bin", 600_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.engine.cancel("slow2")
        r.awaitStatus("slow2", TransferStatus.CANCELED)
        r.await("the cancel message") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:slow2:user", r.pc.files.messages.single())
        assertTrue(r.sink.entries.isEmpty())
    }

    @Test
    fun anOfferTheDiskCantHoldIsAnsweredWithAnErrorInsteadOfTheAck() {
        val r = rig()
        r.sink.free = 10_000
        r.pc.sendOffer("big", file("f1", "huge.bin", 50_000))
        val reply = r.pc.nextFrame()!!
        assertEquals("error", reply.getValue("type").jsonPrimitive.content)
        assertEquals("disk_full", reply.getValue("code").jsonPrimitive.content)
        assertEquals("big", reply.getValue("ref").jsonPrimitive.content)
        Thread.sleep(200)
        assertNull("nothing is downloaded", r.store.transfers["big"])
        assertTrue(r.pc.files.calls.isEmpty())
    }

    @Test
    fun aRepeatedOfferIsAckedButNotStartedTwice() {
        val r = rig()
        val a = file("f1", "a.bin", 10_000)
        val offered = r.pc.sendOffer("dup", a)
        r.awaitStatus("dup", TransferStatus.DONE)
        r.pc.send(r.pc.files.offerFrame(offered))
        Thread.sleep(300)
        assertEquals("one GET only", 1, r.pc.files.calls.count { it.method == "GET" })
        assertEquals(1, r.sink.entries.size)
    }

    @Test
    fun anEntryThatVanishedDuringTheDownloadIsRecreatedAndStartedOver() {
        val r = rig()
        val a = file("f1", "a.bin", 200_000)
        r.pc.files.dropGetAfter = 50_000
        r.pc.files.throttleGetBytesPer100Ms = null
        // The user deletes the half-written file between the two attempts.
        val offered = r.pc.files.offered("vanish", a)
        r.pc.send(r.pc.files.offerFrame(offered))
        r.await("some bytes") { r.sink.entries.isNotEmpty() }
        r.sink.entries.keys.toList().forEach { r.sink.delete(it) }
        r.awaitStatus("vanish", TransferStatus.DONE)
        assertArrayEquals(a.bytes, r.sink.visible().single().file.readBytes())
    }

    @Test
    fun anEntryThatKeepsVanishingFailsTheFileInsteadOfFillingTheGallery() {
        val r = rig()
        r.sink.vanish = true
        r.pc.sendOffer("loop", file("f1", "a.jpg", 1_000, "image/jpeg"))
        val t = r.awaitStatus("loop", TransferStatus.FAILED)
        assertEquals("internal", t.errorCode)
        assertTrue("at most 3 entries were made, then it gave up: ${r.sink.created}", r.sink.created <= 3)
        assertTrue("nothing is left behind", r.sink.entries.isEmpty())
        r.await("the cancel") { r.pc.files.messages.isNotEmpty() }
        assertEquals("cancel:loop:error", r.pc.files.messages.single())
    }

    @Test
    fun theDownloadDoesNotBlockTheWebSocket() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 10_000
        r.pc.sendOffer("long", file("f1", "big.bin", 400_000))
        r.await("download under way") { r.sink.entries.values.any { it.file.length() > 0 } }
        // A ping is answered while the file is still coming in.
        val rtt = r.blocking { r.current!!.ping() }
        assertTrue("$rtt", rtt is app.kinrelay.android.link.PingResult.Rtt)
        assertEquals(TransferStatus.ACTIVE, r.store.transfers.getValue("long").status)
        r.engine.cancel("long")
    }

    @Test
    fun aTransferThatCantStartBecauseThePcIsGoneWaitsAndResumesWhenItIsBack() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 15_000
        r.pc.sendOffer("wait", file("f1", "big.bin", 300_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.dropConnection()
        r.pc.files.throttleGetBytesPer100Ms = null
        Thread.sleep(300)
        assertEquals("waiting, not failed", TransferStatus.ACTIVE, r.store.transfers.getValue("wait").status)
        r.connect()
        r.awaitStatus("wait", TransferStatus.DONE)
        assertNotNull(r.pc.files.calls.firstOrNull { it.range != null })
        assertEquals(300_000L, r.sink.visible().single().file.length())
    }
}
