package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.TransferRig
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A transfer ID is never reused (protocol.md, Files: phone -> PC, changelog 40). A PC answers `404` to a `POST` with the ID of a transfer
 * that was canceled or failed (or that it forgot after 24 hours) and creates nothing, so the phone starts over once under a new ID.
 */
class TransferIdReuseTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    private fun rig(): TransferRig = TransferRig().also { rigs += it; it.connect() }

    private fun TransferRig.share(vararg files: Pair<String, ByteArray>): String =
        blocking { engine.enqueueUpload(files.map { (name, bytes) -> ShareSource(sources.add(name, bytes)) }) }!!

    @Test
    fun aRetryOfAFailedUploadIsMadeUnderANewTransferId() {
        val r = rig()
        val bytes = TransferRig.bytes(50_000)
        r.pc.files.postStatus = 507
        val id = r.share("a.bin" to bytes)
        r.awaitStatus(id, TransferStatus.FAILED)
        // The PC failed the transfer, so its ID is used up from now on: a POST with it is 404, whatever the disk says now.
        r.pc.files.usedIds += id
        r.pc.files.postStatus = null

        r.engine.retry(id)
        r.await("the retry to be done under a new ID") { r.store.transfers.values.any { it.id != id && it.status == TransferStatus.DONE } }

        val newId = r.store.transfers.values.single { it.status == TransferStatus.DONE }.id
        assertNull("the old row moved to the new ID", r.store.transfers[id])
        assertArrayEquals(bytes, r.pc.files.uploads.getValue(newId).files.getValue("f1").data)
        assertNull("the used-up ID was never created", r.pc.files.uploads[id])
        val posts = r.pc.files.calls.filter { it.method == "POST" }
        assertEquals("the failed POST, the one the PC refused with 404, and the new one", 3, posts.size)
        assertEquals("/v1/transfers/$newId/files/f1", r.pc.files.calls.last { it.method == "PUT" }.path)
        assertEquals("from the start of the file", 0L, r.pc.files.calls.last { it.method == "PUT" }.offset)
        assertEquals("only the first failure was shown, not the 404", 1, r.notifier.failed.size)
        assertFalse(r.wake.held)
    }

    @Test
    fun aCanceledUploadThatIsRetriedStartsOverUnderANewId() {
        val r = rig()
        val bytes = TransferRig.bytes(30_000, 3)
        r.pc.files.postStatus = 500
        val id = r.share("b.bin" to bytes)
        r.awaitStatus(id, TransferStatus.FAILED)
        r.pc.files.usedIds += id // a transfer the PC canceled by itself: `404` at once from then on
        r.pc.files.postStatus = null
        r.engine.retry(id)
        r.await("done under a new ID") { r.store.transfers.values.any { it.id != id && it.status == TransferStatus.DONE } }
        assertEquals(1, r.store.transfers.size)
    }

    @Test
    fun aSecond404IsFinalSoThePhoneNeverLoops() {
        val r = rig()
        r.pc.files.postStatus = 404
        r.share("c.bin" to TransferRig.bytes(1_000))
        r.await("the transfer to fail") { r.store.transfers.values.singleOrNull()?.status == TransferStatus.FAILED }
        val t = r.store.transfers.values.single()
        assertEquals("not_found", t.errorCode)
        Thread.sleep(200)
        assertEquals("one POST, one start over, and no third", listOf("POST", "POST"), r.pc.files.calls.map { it.method })
    }
}
