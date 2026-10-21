package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferKind
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.FakePcFiles.OfferedFile
import app.kinrelay.android.testing.TransferRig
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Item 52 of the protocol changelog, the phone's side, against the fake PC: a download's `complete` and `cancel` are **kept until acked and
 * sent again after every `hello`**, under the same message ID, whether the connection dropped or the process was restarted; and an `offer`
 * whose ID the phone knows is a **repeat** when it is the same paired PC with the same files (the ack again, nothing downloaded twice, and the
 * message that is still owed is sent again), and `bad_request` for every other reuse.
 */
class OwedMessagesTest {
    private val rigs = mutableListOf<TransferRig>()
    private val extraPcs = mutableListOf<FakePc>()

    @After
    fun tearDown() {
        rigs.forEach { it.close() }
        extraPcs.forEach { it.close() }
    }

    private fun rig(): TransferRig = TransferRig().also { rigs += it; it.connect() }

    private fun file(name: String, size: Int, id: String = "f1") = OfferedFile(id, name, "application/octet-stream", TransferRig.bytes(size))

    /** The frames of [type] that the fake PC has received from the phone so far. */
    private fun TransferRig.framesOf(type: String): List<JsonObject> =
        pc.received.toList().map { kotlinx.serialization.json.Json.parseToJsonElement(it) as JsonObject }.filter { it["type"]?.jsonPrimitive?.content == type }

    /** A download that is over without the PC having acked the `complete`: the connection died waiting for the ack. */
    private fun TransferRig.doneButNotAcked(id: String, vararg files: OfferedFile) {
        pc.files.ackMessages = false
        pc.sendOffer(id, *files)
        awaitStatus(id, TransferStatus.DONE)
        await("the complete to be sent and the connection given up on (no ack)") { pc.files.messages.size == 1 && current?.isReady == false }
        assertEquals("complete", store.transfers.getValue(id).owed)
        assertFalse(store.transfers.getValue(id).completeAcked)
    }

    /** A new connection, as after a reconnect: the PC acks again. */
    private fun TransferRig.reconnect() = connect().also { pc.files.ackMessages = true }

    /** The next reply to [ref] from the phone (an `ack` or an `error`). */
    private fun FakePc.replyTo(ref: String): JsonObject {
        val deadline = System.currentTimeMillis() + 5_000
        while (System.currentTimeMillis() < deadline) {
            val frame = nextFrame(500) ?: continue
            if (frame["ref"]?.jsonPrimitive?.content == ref) return frame
        }
        error("no reply to $ref")
    }

    // ---------------------------------------------------------------- complete and cancel are kept until acked

    @Test
    fun aCompleteTheCpDidNotAckIsSentAgainAfterAReconnectUnderTheSameId() {
        val r = rig()
        r.doneButNotAcked("c1", file("a.bin", 5_000))

        val session = r.reconnect()
        r.blocking { r.engine.onLinkReady(r.pcId, session) }

        assertEquals("the second complete reached the PC", 2, r.pc.files.messages.size)
        val ids = r.framesOf("complete").map { it.getValue("id").jsonPrimitive.content }
        assertEquals("one message, sent twice", 1, ids.toSet().size)
        assertEquals(2, ids.size)
        val t = r.store.transfers.getValue("c1")
        assertTrue("acked now", t.completeAcked)
        assertNull("and no longer owed", t.owed)
        assertEquals(TransferStatus.DONE, t.status)
    }

    @Test
    fun aCompleteTheCpDidNotAckIsSentAgainAfterAProcessRestart() {
        val r = rig()
        r.doneButNotAcked("c2", file("a.bin", 5_000))
        val owedId = r.store.transfers.getValue("c2").owedId

        // A restart: a new engine over what the database kept (the rows, nothing in memory).
        val restarted = TransferEngine(r.scope, r.deps, TransferRig.FAST)
        val session = r.reconnect()
        r.blocking { restarted.onLinkReady(r.pcId, session) }

        assertEquals(2, r.pc.files.messages.size)
        assertEquals(owedId, r.framesOf("complete").last().getValue("id").jsonPrimitive.content)
        assertTrue(r.store.transfers.getValue("c2").completeAcked)
        assertNull(r.store.transfers.getValue("c2").owed)
    }

    @Test
    fun aCompleteThatWasAckedIsNotSentAgain() {
        val r = rig()
        r.pc.sendOffer("c3", file("a.bin", 1_000))
        r.awaitStatus("c3", TransferStatus.DONE)
        r.await("the complete to be acked") { r.store.transfers.getValue("c3").completeAcked }
        assertNull(r.store.transfers.getValue("c3").owed)

        r.dropConnection()
        val session = r.reconnect()
        r.blocking { r.engine.onLinkReady(r.pcId, session) }
        Thread.sleep(250)
        assertEquals("only the first complete", 1, r.pc.files.messages.size)
    }

    @Test
    fun aCompleteTheCpRefusedIsNotSentAgain() {
        val r = rig()
        r.pc.onFrame = { socket, frame ->
            if (frame["type"]?.jsonPrimitive?.content == "complete") {
                socket.send("""{"type":"error","ref":"${frame.getValue("id").jsonPrimitive.content}","code":"internal","message":"no"}""")
            }
        }
        r.pc.files.ackMessages = false
        r.pc.sendOffer("c4", file("a.bin", 1_000))
        r.awaitStatus("c4", TransferStatus.DONE)
        r.await("the refusal to settle it") { r.store.transfers.getValue("c4").owed == null }
        r.dropConnection()
        val session = r.reconnect()
        r.blocking { r.engine.onLinkReady(r.pcId, session) }
        Thread.sleep(250)
        assertEquals("an error is final: no second complete", 1, r.pc.files.messages.size)
    }

    @Test
    fun theCancelOfADownloadThatFailedIsSentAgainAfterAReconnect() {
        val r = rig()
        r.pc.files.ackMessages = false
        // The offer expired on the PC: every GET is 404, so the download fails and the phone owes the PC a cancel (reason error).
        val offered = r.pc.files.offered("f1", file("a.bin", 5_000)).also { it.gone = true }
        r.pc.send(r.pc.files.offerFrame(offered))
        r.awaitStatus("f1", TransferStatus.FAILED)
        r.await("the cancel to be sent and given up on") { r.pc.files.messages.size == 1 && r.current?.isReady == false }
        assertEquals("cancel:error", r.store.transfers.getValue("f1").owed)

        val session = r.reconnect()
        r.blocking { r.engine.onLinkReady(r.pcId, session) }

        assertEquals(listOf("cancel:f1:error", "cancel:f1:error"), r.pc.files.messages)
        assertEquals("the same message", 1, r.framesOf("cancel").map { it.getValue("id").jsonPrimitive.content }.toSet().size)
        assertNull(r.store.transfers.getValue("f1").owed)
        assertEquals(TransferStatus.FAILED, r.store.transfers.getValue("f1").status)
    }

    @Test
    fun theCancelOfTheUserIsKeptWhileThePhoneIsOfflineAndSentAfterTheHello() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("u1", file("big.bin", 400_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.dropConnection()
        r.engine.cancel("u1")
        r.awaitStatus("u1", TransferStatus.CANCELED)
        assertEquals("cancel:user", r.store.transfers.getValue("u1").owed)
        assertTrue("no connection: nothing was sent", r.pc.files.messages.isEmpty())

        val session = r.reconnect()
        r.blocking { r.engine.onLinkReady(r.pcId, session) }

        assertEquals(listOf("cancel:u1:user"), r.pc.files.messages)
        assertNull(r.store.transfers.getValue("u1").owed)
    }

    @Test
    fun anOwedMessageIsWrittenInTheSameStepAsTheStatus() {
        // Done and owing are one write: a crash between two writes could leave a finished download that the PC never hears about.
        val r = rig()
        r.pc.files.ackMessages = false
        r.pc.sendOffer("w1", file("a.bin", 100))
        val t = r.awaitStatus("w1", TransferStatus.DONE)
        assertEquals("complete", t.owed)
        assertNotNull(t.owedId)
    }

    // ---------------------------------------------------------------- the same PC repeating an offer

    @Test
    fun aRepeatedOfferOfADoneDownloadIsAckedAndAnUnackedCompleteIsSentAgain() {
        val r = rig()
        r.doneButNotAcked("d1", file("a.bin", 5_000))
        val gets = r.pc.files.calls.count { it.method == "GET" }

        r.reconnect()
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offers.getValue("d1")))

        assertEquals("ack", r.pc.replyTo("d1").getValue("type").jsonPrimitive.content)
        r.await("the complete to be sent again") { r.pc.files.messages.size == 2 }
        r.await("and acked") { r.store.transfers.getValue("d1").completeAcked }
        assertEquals("nothing was downloaded a second time", gets, r.pc.files.calls.count { it.method == "GET" })
        assertEquals(1, r.sink.entries.size)
        assertEquals(TransferStatus.DONE, r.store.transfers.getValue("d1").status)
    }

    @Test
    fun aRepeatedOfferOfADoneDownloadWhoseCompleteWasAckedGetsOnlyTheAck() {
        val r = rig()
        r.pc.sendOffer("d2", file("a.bin", 2_000))
        r.awaitStatus("d2", TransferStatus.DONE)
        r.await("the complete to be acked") { r.store.transfers.getValue("d2").completeAcked }
        r.dropConnection()

        r.reconnect()
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offers.getValue("d2")))

        assertEquals("ack", r.pc.replyTo("d2").getValue("type").jsonPrimitive.content)
        Thread.sleep(250)
        assertEquals("no second complete", 1, r.pc.files.messages.size)
    }

    @Test
    fun aRepeatedOfferOfACanceledDownloadIsAckedAndItsCancelIsSentAgain() {
        val r = rig()
        r.pc.files.ackMessages = false
        r.pc.files.throttleGetBytesPer100Ms = 20_000
        r.pc.sendOffer("d3", file("big.bin", 400_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.engine.cancel("d3")
        r.awaitStatus("d3", TransferStatus.CANCELED)
        r.await("the cancel to be sent and given up on") { r.pc.files.messages.size == 1 && r.current?.isReady == false }

        r.reconnect()
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offers.getValue("d3")))

        assertEquals("ack", r.pc.replyTo("d3").getValue("type").jsonPrimitive.content)
        r.await("the cancel to be sent again") { r.pc.files.messages.size == 2 }
        assertEquals(listOf("cancel:d3:user", "cancel:d3:user"), r.pc.files.messages)
        r.await("and acked") { r.store.transfers.getValue("d3").owed == null }
        assertEquals("still canceled, never restarted", TransferStatus.CANCELED, r.store.transfers.getValue("d3").status)
    }

    @Test
    fun aRepeatedOfferOfAFailedDownloadIsAckedAndItsCancelIsSentAgain() {
        val r = rig()
        r.pc.files.ackMessages = false
        val offered = r.pc.files.offered("f1", file("a.bin", 5_000)).also { it.gone = true }
        r.pc.send(r.pc.files.offerFrame(offered))
        r.awaitStatus("f1", TransferStatus.FAILED)
        r.await("the cancel to be sent and given up on") { r.pc.files.messages.size == 1 && r.current?.isReady == false }

        r.reconnect()
        r.pc.send(r.pc.files.offerFrame(offered))

        assertEquals("ack", r.pc.replyTo("f1").getValue("type").jsonPrimitive.content)
        r.await("the cancel to be sent again") { r.pc.files.messages.size == 2 }
        assertEquals(TransferStatus.FAILED, r.store.transfers.getValue("f1").status)
    }

    @Test
    fun aRepeatedOfferOfAnUnfinishedDownloadCarriesOn() {
        val r = rig()
        r.pc.files.throttleGetBytesPer100Ms = 40_000
        r.pc.sendOffer("d4", file("big.bin", 400_000))
        r.await("some bytes") { r.sink.entries.values.any { it.file.length() > 0 } }
        r.dropConnection()
        r.pc.files.throttleGetBytesPer100Ms = null
        r.reconnect()
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offers.getValue("d4")))

        assertEquals("ack", r.pc.replyTo("d4").getValue("type").jsonPrimitive.content)
        r.awaitStatus("d4", TransferStatus.DONE)
        assertEquals("one entry: resumed, not started twice", 1, r.sink.entries.size)
    }

    // ---------------------------------------------------------------- any other reuse is bad_request

    @Test
    fun aRepeatWithADifferentFileListIsBadRequest() {
        val r = rig()
        r.pc.sendOffer("m1", file("a.bin", 10, "f1"), file("b.bin", 20, "f2"))
        r.awaitStatus("m1", TransferStatus.DONE)
        r.await("the complete to be acked") { r.store.transfers.getValue("m1").completeAcked }
        r.dropConnection()
        r.reconnect()

        // One file fewer, then another size: both are a different offer under a used ID.
        for (files in listOf(listOf(file("a.bin", 10, "f1")), listOf(file("a.bin", 10, "f1"), file("b.bin", 21, "f2")))) {
            r.pc.send(r.pc.files.offerFrame(r.pc.files.offered("m1", *files.toTypedArray())))
            val reply = r.pc.replyTo("m1")
            assertEquals("error", reply.getValue("type").jsonPrimitive.content)
            assertEquals("bad_request", reply.getValue("code").jsonPrimitive.content)
            assertEquals("m1", reply.getValue("ref").jsonPrimitive.content)
        }
        // The same fileIds and sizes with other names and types, in another order, are the same offer.
        r.pc.send(r.pc.files.offerFrame(r.pc.files.offered("m1", file("other.bin", 20, "f2"), file("x", 10, "f1"))))
        assertEquals("ack", r.pc.replyTo("m1").getValue("type").jsonPrimitive.content)
    }

    @Test
    fun anOfferWithTheIdOfAnUploadOfOursIsBadRequest() {
        val r = rig()
        val up = TransferEntity("up-1", TransferDirection.UP, r.pcId, "Desk PC", TransferKind.FILE, TransferStatus.DONE, 1_000, 2_000, 10, null, null, completeAcked = false)
        r.blocking { r.store.create(up, listOf(TransferFileEntity("up-1", "f1", 0, "a.bin", null, "application/octet-stream", 10, 10, null, null, finished = true))) }

        r.pc.send(r.pc.files.offerFrame(r.pc.files.offered("up-1", file("a.bin", 10))))

        val reply = r.pc.replyTo("up-1")
        assertEquals("bad_request", reply.getValue("code").jsonPrimitive.content)
        assertEquals("up-1", reply.getValue("ref").jsonPrimitive.content)
        assertEquals("nothing was started", 0, r.sink.entries.size)
    }

    @Test
    fun anOfferWithTheIdOfATextIsBadRequest() {
        val r = rig()
        r.pc.sendText("same-id", "hello", isUrl = false)
        assertEquals("ack", r.pc.replyTo("same-id").getValue("type").jsonPrimitive.content)

        r.pc.send(r.pc.files.offerFrame(r.pc.files.offered("same-id", file("a.bin", 10))))

        val reply = r.pc.replyTo("same-id")
        assertEquals("bad_request", reply.getValue("code").jsonPrimitive.content)
        assertEquals("same-id", reply.getValue("ref").jsonPrimitive.content)
        assertNull(r.store.transfers["same-id"])
    }

    @Test
    fun anOfferWithTheIdOfAnOfferFromAnotherDeviceIsBadRequest() {
        val r = rig()
        r.pc.sendOffer("o1", file("a.bin", 100))
        r.awaitStatus("o1", TransferStatus.DONE)

        // Another PC is paired now (a re-pair); it offers the ID the first PC used.
        val second = FakePc(name = "Other PC").start().also { extraPcs += it }
        r.pairedPeer = app.kinrelay.android.transfer.PeerInfo(second.identity.deviceId, "Other PC")
        r.connect(to = second)
        second.send(second.files.offerFrame(second.files.offered("o1", file("a.bin", 100))))

        val reply = second.replyTo("o1")
        assertEquals("bad_request", reply.getValue("code").jsonPrimitive.content)
        assertEquals("o1", reply.getValue("ref").jsonPrimitive.content)
    }

    // ---------------------------------------------------------------- history wording (item 52, rule 4)

    private fun TransferRig.failedNotFound(id: String, direction: String = TransferDirection.UP, code: String = "not_found") {
        val t = TransferEntity(id, direction, pcId, "Desk PC", TransferKind.FILE, TransferStatus.FAILED, 1_000, 2_000, 10, code, "Desk PC no longer has this transfer.", completeAcked = false)
        blocking { store.create(t, listOf(TransferFileEntity(id, "f1", 0, "a.bin", null, "application/octet-stream", 10, 0, null, null, finished = false))) }
    }

    @Test
    fun theCancelOfTheOwningPcTurnsAFailedNotFoundTransferIntoCanceledOnThePc() {
        val r = rig()
        r.failedNotFound("nf-1")

        r.pc.sendCancel("nf-1", "user")

        val t = r.awaitStatus("nf-1", TransferStatus.CANCELED)
        assertEquals("peer_canceled", t.errorCode)
        assertEquals("Canceled on Desk PC.", t.message)
    }

    @Test
    fun aFailedNotFoundTransferWithNoCancelKeepsTheNoLongerHasWording() {
        val r = rig()
        r.failedNotFound("nf-2")
        Thread.sleep(150)
        val t = r.store.transfers.getValue("nf-2")
        assertEquals(TransferStatus.FAILED, t.status)
        assertEquals("Desk PC no longer has this transfer.", t.message)
    }

    @Test
    fun aCancelNeverChangesADoneTransferOrAnotherFailure() {
        val r = rig()
        r.pc.sendOffer("keep", file("a.bin", 100))
        r.awaitStatus("keep", TransferStatus.DONE)
        r.failedNotFound("other-failure", code = "connection_lost")

        r.pc.sendCancel("keep", "user")
        r.pc.sendCancel("other-failure", "user")
        Thread.sleep(300)

        assertEquals(TransferStatus.DONE, r.store.transfers.getValue("keep").status)
        assertEquals(1, r.sink.visible().size)
        assertEquals(TransferStatus.FAILED, r.store.transfers.getValue("other-failure").status)
        assertEquals("connection_lost", r.store.transfers.getValue("other-failure").errorCode)
    }

    @Test
    fun aCancelFromAPcThatDoesNotOwnTheFailedTransferChangesNothing() {
        val r = rig()
        val t = TransferEntity("nf-3", TransferDirection.UP, "some-other-pc-device-id", "Some PC", TransferKind.FILE, TransferStatus.FAILED, 1_000, 2_000, 10, "not_found", "x", completeAcked = false)
        r.blocking { r.store.create(t, emptyList()) }

        r.pc.sendCancel("nf-3", "user")
        Thread.sleep(300)

        assertEquals(TransferStatus.FAILED, r.store.transfers.getValue("nf-3").status)
    }
}
