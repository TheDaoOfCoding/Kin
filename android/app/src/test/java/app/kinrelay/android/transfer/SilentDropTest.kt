package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.link.LinkSession
import app.kinrelay.android.testing.FakePcFiles.OfferedFile
import app.kinrelay.android.testing.TransferRig
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * v0.1.1 silent-drop fix (P1): a network that goes silent without a reset (packets just stop) leaves the WebSocket "ready" for minutes. A
 * transfer must notice (no byte for a while, or a failed attempt, then a check that the PC still answers a new connection), drop that
 * connection, wait for the new one and resume. It must never fail for good just because the network was silent for a while.
 */
class SilentDropTest {
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() = rigs.forEach { it.close() }

    /** Quick checks: 300 ms without a byte, a 500 ms check. Only 2 stalled attempts fail a transfer, so a test that wrongly counted them would fail fast. */
    private val quick = TransferRig.FAST.copy(
        stallProbeAfterMillis = 300, probeTimeoutMillis = 500, maxStalledAttempts = 2, reconnectWaitMillis = 20_000,
    )

    private fun rig(timings: TransferTimings = quick): TransferRig = TransferRig(timings = timings).also { rigs += it; it.connect() }

    private fun TransferRig.share(vararg files: Pair<String, ByteArray>): String =
        blocking { engine.enqueueUpload(files.map { (name, bytes) -> ShareSource(sources.add(name, bytes)) }) }!!

    private fun TransferRig.stored(id: String, fileId: String) = pc.files.uploads.getValue(id).files.getValue(fileId)

    private fun TransferRig.awaitDead(session: LinkSession) = await("the stalled connection to be dropped", 10_000) { session.foundDead }

    /** The network comes back: the PC answers again and a new connection is made, as the reconnect loop would. */
    private fun TransferRig.networkBack() {
        pc.files.silent = false
        pc.files.failFast = false
        probeAnswers = { true }
        connect()
    }

    // ------------------------------------------------------------------ phone -> PC

    @Test
    fun aSilentNetworkDuringAnUploadIsDroppedWaitedOutAndTheUploadResumes() {
        val r = rig()
        val old = r.current!!
        r.pc.files.silent = true
        val files = (1..3).map { "IMG_$it.jpg" to TransferRig.bytes(150_000, it) }
        val id = r.share(*files.toTypedArray())

        r.awaitDead(old)
        // Not failed: the transfer waits for the new connection.
        Thread.sleep(400)
        assertEquals(TransferStatus.ACTIVE, r.store.transfers.getValue(id).status)
        assertTrue("the silent connection was replaced, not kept", old.foundDead)

        r.networkBack()
        r.awaitStatus(id, TransferStatus.DONE)
        assertNull(r.store.transfers.getValue(id).errorCode)
        files.forEachIndexed { i, (_, bytes) -> assertArrayEquals(bytes, r.stored(id, "f${i + 1}").data) }
        assertFalse(r.wake.held)
    }

    @Test
    fun anUploadWhoseAttemptsFailOnADeadLinkIsNotFailedAndResumes() {
        // Every request fails at once (no stall for the guard to see), the PC doesn't answer a new connection either.
        val r = rig(quick.copy(stallProbeAfterMillis = 60_000))
        val old = r.current!!
        r.pc.files.failFast = true
        r.probeAnswers = { false }
        val bytes = TransferRig.bytes(200_000)
        val id = r.share("big.bin" to bytes)

        r.awaitDead(old)
        Thread.sleep(300)
        assertEquals("not failed after maxStalledAttempts (2)", TransferStatus.ACTIVE, r.store.transfers.getValue(id).status)

        r.networkBack()
        r.awaitStatus(id, TransferStatus.DONE)
        assertArrayEquals(bytes, r.stored(id, "f1").data)
    }

    @Test
    fun aDownloadStillFailsWhenThePcAnswersTheCheckButEveryAttemptFails() {
        // The control: a PC that answers a new connection but ends every file request at once (no byte moves) is a real failure, as before.
        val r = rig(quick.copy(stallProbeAfterMillis = 60_000))
        r.pc.files.failFast = true
        r.pc.sendOffer("bad", OfferedFile("f1", "x.bin", "application/octet-stream", TransferRig.bytes(50_000)))

        val failed = r.awaitStatus("bad", TransferStatus.FAILED)
        assertEquals("connection_lost", failed.errorCode)
        assertFalse("the connection was fine, so it was kept", r.current!!.foundDead)
    }

    @Test
    fun aPcThatAnswersTheCheckIsNotTornDownBySlowRequests() {
        val r = rig()
        val old = r.current!!
        r.pc.files.silent = true
        r.probeAnswers = { true } // slow, not gone
        val bytes = TransferRig.bytes(150_000)
        val id = r.share("slow.bin" to bytes)

        Thread.sleep(1_200) // several stall windows
        assertFalse(old.foundDead)
        assertEquals(TransferStatus.ACTIVE, r.store.transfers.getValue(id).status)
        r.pc.files.silent = false
        r.awaitStatus(id, TransferStatus.DONE)
        assertSame("the same connection all along", old, r.current)
        assertArrayEquals(bytes, r.stored(id, "f1").data)
    }

    @Test
    fun aTransferThatKeepsMovingBytesIsNeverChecked() {
        var checks = 0
        val r = rig()
        r.probeAnswers = { checks++; true }
        r.sources.readDelayMillis = 5 // slow, but always moving
        val id = r.share("steady.bin" to TransferRig.bytes(300_000))
        r.awaitStatus(id, TransferStatus.DONE)
        assertEquals(0, checks)
    }

    // ------------------------------------------------------------------ PC -> phone

    @Test
    fun aSilentNetworkDuringADownloadIsDroppedWaitedOutAndTheDownloadResumes() {
        val r = rig()
        val old = r.current!!
        r.pc.files.silent = true
        val files = (1..3).map { OfferedFile("f$it", "IMG_$it.jpg", "image/jpeg", TransferRig.bytes(150_000, it)) }
        r.pc.sendOffer("dl", *files.toTypedArray())

        r.awaitDead(old)
        Thread.sleep(400)
        assertEquals(TransferStatus.ACTIVE, r.store.transfers.getValue("dl").status)

        r.networkBack()
        r.awaitStatus("dl", TransferStatus.DONE)
        assertNull(r.store.transfers.getValue("dl").errorCode)
        val saved = r.sink.visible().associate { it.name to it.file.readBytes() }
        files.forEach { assertArrayEquals(it.name, it.bytes, saved.getValue(it.name)) }
        assertEquals(3, saved.size)
    }

    @Test
    fun aDownloadWhoseAttemptsFailOnADeadLinkIsNotFailedAndResumes() {
        val r = rig(quick.copy(stallProbeAfterMillis = 60_000))
        val old = r.current!!
        r.pc.files.failFast = true
        r.probeAnswers = { false }
        val a = OfferedFile("f1", "big.bin", "application/octet-stream", TransferRig.bytes(200_000))
        r.pc.sendOffer("dl2", a)

        r.awaitDead(old)
        Thread.sleep(300)
        assertEquals(TransferStatus.ACTIVE, r.store.transfers.getValue("dl2").status)

        r.networkBack()
        r.awaitStatus("dl2", TransferStatus.DONE)
        assertArrayEquals(a.bytes, r.sink.visible().single().file.readBytes())
    }

    // ------------------------------------------------------------------ the check itself

    @Test
    fun checkAliveKeepsAnAnsweringConnectionAndAbandonsASilentOne() {
        val r = rig()
        val session = r.current!!
        assertTrue(r.blocking { session.checkAlive(500) })
        assertFalse(session.foundDead)
        assertTrue(session.isReady)

        r.probeAnswers = { false }
        assertFalse(r.blocking { session.checkAlive(500) })
        assertTrue(session.foundDead)
        assertFalse("an abandoned connection is never handed out again", session.isReady)
        r.blocking { session.awaitEnd() } // it ended
    }
}
