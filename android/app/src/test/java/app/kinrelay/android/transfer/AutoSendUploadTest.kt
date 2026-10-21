package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.TransferRig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * An image the phone sends by itself (design 3.7, 5.6) against the fake PC over real TLS: `origin` goes in the first `POST` (and stays on the transfer for a resume), a user's share has none, and an
 * auto-sent image is **never queued**: with no answer it is cancelled, a leftover queued one is ended as "not sent" and is never started by the queue flush.
 */
class AutoSendUploadTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(connect: Boolean = true): TransferRig = TransferRig(timings = TransferRig.FAST).also { rigs += it; if (connect) it.connect() }

    @Test
    fun anAutoSentScreenshotCarriesItsOriginInThePost() {
        val r = rig()
        val ok = r.blocking { r.engine.sendNow(listOf(ShareSource(r.sources.add("Screenshot_1.png", TransferRig.bytes(20_000)), mimeHint = "image/png")), "screenshot", 3_000) }
        assertTrue(ok)
        val id = r.store.transfers.keys.single()
        r.awaitStatus(id, TransferStatus.DONE)
        val post = r.pc.requests.first { it.method == "POST" }
        assertTrue(post.body, post.body.contains("\"origin\":\"screenshot\""))
        assertEquals("screenshot", r.store.transfers.getValue(id).origin)
    }

    @Test
    fun aUsersShareHasNoOrigin() {
        val r = rig()
        val id = r.blocking { r.engine.enqueueUpload(listOf(ShareSource(r.sources.add("a.png", TransferRig.bytes(1_000))))) }!!
        r.awaitStatus(id, TransferStatus.DONE)
        assertFalse(r.pc.requests.first { it.method == "POST" }.body.contains("origin"))
        assertNull(r.store.transfers.getValue(id).origin)
    }

    @Test
    fun withNoConnectionAnAutoSentImageIsCancelledAndNeverQueued() {
        val r = rig(connect = false)
        val ok = r.blocking { r.engine.sendNow(listOf(ShareSource(r.sources.add("Screenshot_2.png", TransferRig.bytes(20_000)))), "screenshot", 700) }
        assertFalse(ok)
        val transfer = r.store.transfers.values.single()
        r.awaitStatus(transfer.id, TransferStatus.CANCELED, TransferStatus.FAILED)
        assertTrue("never a queued upload", r.blocking { r.engine.queuedUploads() }.isEmpty())
        assertTrue("the PC saw nothing", r.pc.requests.none { it.method == "POST" || it.method == "PUT" })
    }

    @Test
    fun aLeftoverQueuedAutoSentImageIsEndedAndTheQueueFlushNeverStartsIt() {
        val r = rig(connect = false)
        // The process died after the transfer was made and before anything was sent: it is `queued` in the store.
        val id = r.blocking { r.engine.enqueueUpload(listOf(ShareSource(r.sources.add("Screenshot_3.png", TransferRig.bytes(5_000)))), "screenshot") }!!
        r.await("the transfer to be parked") { r.notifier.queued.isNotEmpty() }
        assertEquals(TransferStatus.QUEUED, r.store.transfers.getValue(id).status)
        assertTrue("the flush's list leaves it out", r.blocking { r.engine.queuedUploads() }.none { it.id == id })
        r.blocking { r.engine.endAutoSentUnfinished() }
        val ended = r.store.transfers.getValue(id)
        assertEquals(TransferStatus.FAILED, ended.status)
        assertEquals("offline", ended.errorCode)
    }

    @Test
    fun anActiveAutoSentImageOfAnEarlierConnectionIsEndedAndNeverResumedButAUsersUploadIs() {
        val r = rig(connect = false)
        val auto = r.blocking { r.engine.enqueueUpload(listOf(ShareSource(r.sources.add("Screenshot_4.png", TransferRig.bytes(5_000)))), "screenshot") }!!
        val user = r.blocking { r.engine.enqueueUpload(listOf(ShareSource(r.sources.add("c.png", TransferRig.bytes(5_000))))) }!!
        r.await("both transfers to be parked") { r.notifier.queued.size >= 2 }
        // Both were mid-upload when the connection (or the process) was lost: `active`, with no job running.
        listOf(auto, user).forEach { id -> r.store.transfers.computeIfPresent(id) { _, t -> t.copy(status = TransferStatus.ACTIVE) } }
        r.blocking { r.engine.endAutoSentUnfinished() }
        assertEquals("never resumed later", TransferStatus.FAILED, r.store.transfers.getValue(auto).status)
        assertEquals("offline", r.store.transfers.getValue(auto).errorCode)
        assertEquals("a user's upload resumes as before", TransferStatus.ACTIVE, r.store.transfers.getValue(user).status)
    }

    @Test
    fun aDroppedConnectionMidUploadEndsAnAutoSentImageAsNotSentAndItIsNeverResumed() {
        // 2B.4 review: the runner used to wait up to 15 minutes for a reconnect and resume, without looking at Pause again.
        val r = rig()
        r.pc.files.dropPutKeeping = 100_000
        r.sources.readDelayMillis = 30 // the upload is still going when the connection goes
        assertTrue(r.blocking { r.engine.sendNow(listOf(ShareSource(r.sources.add("Screenshot_5.png", TransferRig.bytes(400_000)), mimeHint = "image/png")), "screenshot", 3_000) })
        val id = r.store.transfers.keys.single()
        r.await("the PUT to be under way") { r.sources.reads.get() > 0 }
        r.dropConnection()
        r.sources.readDelayMillis = 0
        val ended = r.awaitStatus(id, TransferStatus.FAILED)
        assertEquals("offline", ended.errorCode)
        // The PC is back: nothing is resumed.
        r.connect()
        Thread.sleep(600)
        assertEquals("still not sent", TransferStatus.FAILED, r.store.transfers.getValue(id).status)
        assertEquals("one POST only: it never asked again", 1, r.pc.files.calls.count { it.method == "POST" })
    }

    @Test
    fun aBrokenPutOnAnAutoSentImageIsNotRetriedEither() {
        val r = rig()
        r.pc.files.dropPutKeeping = 150_000 // the PC drops the request half way, the connection itself stays
        assertTrue(r.blocking { r.engine.sendNow(listOf(ShareSource(r.sources.add("Screenshot_6.png", TransferRig.bytes(400_000)), mimeHint = "image/png")), "screenshot", 3_000) })
        val id = r.store.transfers.keys.single()
        val ended = r.awaitStatus(id, TransferStatus.FAILED, TransferStatus.DONE)
        assertEquals(TransferStatus.FAILED, ended.status)
        assertEquals("offline", ended.errorCode)
        assertEquals(1, r.pc.files.calls.count { it.method == "POST" })
        assertEquals(1, r.pc.files.calls.count { it.method == "PUT" })
    }

    @Test
    fun aConnectionThatComesUpWhileTheUploadIsBeingCreatedDoesNotEndIt() {
        // 2B.4 review: between `store.create` and `jobs[id] = job` the unfinished-and-not-running check could end a brand-new auto-sent upload as "not sent".
        val r = rig()
        val sources = r.sources
        val engine = r.engine
        val taken = r.blocking {
            coroutineScope {
                val created = async(Dispatchers.IO) { engine.sendNow(listOf(ShareSource(sources.add("Screenshot_7.png", TransferRig.bytes(300_000)), mimeHint = "image/png")), "screenshot", 3_000) }
                // The link comes up (the engine is asked to end what is unfinished) over and over while the upload is made and started.
                repeat(200) { engine.endAutoSentUnfinished() }
                created.await()
            }
        }
        assertTrue("the upload was taken by the PC", taken)
        val id = r.store.transfers.keys.single()
        r.awaitStatus(id, TransferStatus.DONE)
    }

    @Test
    fun anAutoSentImageIsStoredUnderAGenericNameNotItsRealOne() {
        val r = rig()
        val source = ShareSource(r.sources.add("Screenshot_20261017-143205_Bank.png", TransferRig.bytes(8_000)), mimeHint = "image/png", genericStem = "Screenshot 2026-10-17 14.32.05")
        assertTrue(r.blocking { r.engine.sendNow(listOf(source), "screenshot", 3_000) })
        val id = r.store.transfers.keys.single()
        r.awaitStatus(id, TransferStatus.DONE)
        val stored = r.store.files.getValue(id).single().name
        assertEquals("Screenshot 2026-10-17 14.32.05.png", stored)
        assertFalse(stored.contains("Bank"))
        assertTrue("the PC is told the generic name too", r.pc.requests.first { it.method == "POST" }.body.let { it.contains("Screenshot 2026-10-17 14.32.05.png") && !it.contains("Bank") })
    }

    @Test
    fun aUserUploadThatIsQueuedIsStillInTheFlush() {
        val r = rig(connect = false)
        val id = r.blocking { r.engine.enqueueUpload(listOf(ShareSource(r.sources.add("b.png", TransferRig.bytes(5_000))))) }!!
        r.await("the transfer to be parked") { r.notifier.queued.isNotEmpty() }
        assertEquals(listOf(id), r.blocking { r.engine.queuedUploads() }.map { it.id })
    }
}
