package app.kinrelay.android.transfer

import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.FakePcFiles
import app.kinrelay.android.testing.TransferRig
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.atomic.AtomicInteger

/** The upload-queue fixes of the 1.4 review: a Retry made offline stays the user's, started uploads don't expire at 7 days, the cache sweep keeps what is live. */
class QueueReviewFixesTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(newId: (() -> String)? = null, connected: Boolean = true): TransferRig =
        TransferRig(newId = newId).also { rigs += it; if (connected) it.connect() }

    private val day = 24L * 60 * 60 * 1000

    private fun TransferRig.upload(
        id: String, status: String, bytes: ByteArray = TransferRig.bytes(2_000), finished: Boolean = false, createdAt: Long = System.currentTimeMillis(),
        started: Boolean = false, errorCode: String? = null, retriedAs: String? = null,
    ) {
        val file = TransferFileEntity(id, "f1", 0, "a.bin", null, "application/octet-stream", bytes.size.toLong(), if (finished) bytes.size.toLong() else 0, sources.add("$id.bin", bytes), null, finished)
        val t = TransferEntity(
            id, TransferDirection.UP, pcId, "Desk PC", TransferKind.FILE, status, createdAt, null, bytes.size.toLong(), errorCode, errorCode?.let { "It failed." },
            completeAcked = false, started = started, retriedAs = retriedAs,
        )
        blocking { store.create(t, listOf(file)) }
    }

    // ---------------------------------------------------------------- 1. a Retry made while offline stays the user's

    @Test
    fun aUserRetryMadeWhileOfflineIsStillTheUsersWhenTheQueueStartsItAfterTheReconnect() {
        val ids = AtomicInteger()
        val r = rig(newId = { "new-${ids.incrementAndGet()}" }, connected = false)
        val bytes = TransferRig.bytes(3_000, 5)
        r.upload("old-1", TransferStatus.FAILED, bytes, errorCode = "connection_lost")
        r.pc.files.usedIds += "old-1" // the PC canceled it meanwhile: the old ID answers 404

        r.engine.retry("old-1") // the user taps Retry; the PC can't be reached
        r.await("the retry to be parked") { r.store.transfers.getValue("old-1").status == TransferStatus.QUEUED && r.notifier.queued.isNotEmpty() }
        assertTrue("the user's Retry is stored on the transfer", r.store.transfers.getValue("old-1").userRetry)

        r.connect()
        r.engine.startQueuedUpload("old-1") // what the queue flush does after the hello (the flusher passes no flag)

        r.awaitStatus("new-1", TransferStatus.DONE)
        assertEquals("the files went under a new ID, as for any user Retry", "old-1", r.store.transfers.getValue("new-1").localId)
        assertArrayEquals(bytes, r.pc.files.uploads.getValue("new-1").files.getValue("f1").data)
        val old = r.store.transfers.getValue("old-1")
        assertEquals(TransferStatus.FAILED, old.status)
        assertEquals("new-1", old.retriedAs)
        assertFalse("the flag is spent", old.userRetry)
        assertFalse(r.store.transfers.getValue("new-1").userRetry)
    }

    @Test
    fun aQueuedUploadThatTheUserDidNotRetryStillFailsOnA404() {
        val r = rig(newId = { "fixed" }, connected = false)
        r.upload("auto-1", TransferStatus.QUEUED)
        r.pc.files.usedIds += "auto-1"
        r.connect()

        r.engine.startQueuedUpload("auto-1")

        val t = r.awaitStatus("auto-1", TransferStatus.FAILED)
        assertEquals("not_found", t.errorCode)
        Thread.sleep(200)
        assertEquals("one row, nothing re-sent under a new ID", 1, r.store.transfers.size)
        assertEquals(listOf("POST"), r.pc.files.calls.map { it.method })
    }

    @Test
    fun theStoredUserRetryIsClearedOnceTheRunHasReachedItsPost() {
        val r = rig()
        val bytes = TransferRig.bytes(20_000, 4)
        r.upload("old-2", TransferStatus.FAILED, bytes, errorCode = "connection_lost")
        r.pc.files.uploads["old-2"] = FakePcFiles.Upload("old-2", linkedMapOf("f1" to FakePcFiles.UploadedFile("f1", "a.bin", bytes.size.toLong(), "application/octet-stream").also { it.data = bytes }))

        r.engine.retry("old-2")
        r.awaitStatus("old-2", TransferStatus.DONE)

        assertFalse(r.store.transfers.getValue("old-2").userRetry)
        assertTrue("the PC answered the POST: the upload has started", r.store.transfers.getValue("old-2").started)
    }

    @Test
    fun aRetryOfAFailedUploadKeepsItsCreationTime() {
        val r = rig(connected = false)
        val created = System.currentTimeMillis() - 3 * day
        r.upload("old-3", TransferStatus.FAILED, createdAt = created, errorCode = "connection_lost")

        r.engine.retry("old-3")
        r.await("the retry to be parked") { r.store.transfers.getValue("old-3").status == TransferStatus.QUEUED }

        assertEquals("the 7 days of the queue run from the creation, not from the Retry", created, r.store.transfers.getValue("old-3").createdAtMillis)
    }

    // ---------------------------------------------------------------- 2. expiry only for what never started

    @Test
    fun anUploadThatNeverStartedExpiresAtSevenDaysAndOneThatStartedDoesNot() {
        val r = rig(connected = false)
        val old = System.currentTimeMillis() - 8 * day
        r.upload("never", TransferStatus.QUEUED, createdAt = old)
        r.upload("started", TransferStatus.QUEUED, createdAt = old, started = true)
        r.upload("young", TransferStatus.QUEUED, createdAt = System.currentTimeMillis() - 6 * day)
        for (id in listOf("never", "started", "young")) File(r.sources.cacheDir, id).mkdirs()

        val expired = r.blocking { r.engine.expireQueued(QueuePolicy.EXPIRY_MILLIS) }

        assertEquals(1, expired)
        val never = r.store.transfers.getValue("never")
        assertEquals(TransferStatus.FAILED, never.status)
        assertEquals("expired", never.errorCode)
        assertFalse("its cached copies went with it", File(r.sources.cacheDir, "never").exists())
        assertEquals("an upload the PC has answered follows the PC's .part rule, not the 7 days", TransferStatus.QUEUED, r.store.transfers.getValue("started").status)
        assertTrue(File(r.sources.cacheDir, "started").exists())
        assertEquals(TransferStatus.QUEUED, r.store.transfers.getValue("young").status)
    }

    @Test
    fun theStartupSweepKeepsTheCacheOfEveryQueuedActiveOrFailedUploadHoweverOldItIs() {
        val r = rig(connected = false)
        r.upload("q", TransferStatus.QUEUED, started = true)
        r.upload("a", TransferStatus.ACTIVE, started = true)
        r.upload("f", TransferStatus.FAILED, errorCode = "connection_lost")
        r.upload("d", TransferStatus.DONE)
        r.upload("r", TransferStatus.FAILED, errorCode = "not_found", retriedAs = "q")
        for (id in listOf("q", "a", "f", "d", "r", "orphan")) File(r.sources.cacheDir, id).mkdirs()
        r.sources.cacheAgeMillis = 30 * day // every copy is a month old

        r.blocking { r.engine.sweepOrphans() }

        assertEquals("kept: queued, active and failed (it can be retried)", setOf("q", "a", "f"), r.sources.cacheDir.list()!!.toSet())
    }

    @Test
    fun aFailedUploadWhoseFileRowsWereDroppedNoLongerHoldsACacheFolder() {
        val r = rig(connected = false)
        r.upload("f", TransferStatus.FAILED, errorCode = "connection_lost")
        File(r.sources.cacheDir, "f").mkdirs()
        r.store.files.remove("f") // the history's 500 pushed it out: the storage query dropped its file rows

        r.blocking { r.engine.sweepOrphans() }

        assertNull("nothing can be retried without its files, so the copies go", r.sources.cacheDir.list()?.firstOrNull { it == "f" })
    }
}
