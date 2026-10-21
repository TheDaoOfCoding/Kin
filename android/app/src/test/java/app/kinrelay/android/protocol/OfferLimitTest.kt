package app.kinrelay.android.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** At most 800 files in an `offer` (protocol.md, Files: PC -> phone, changelog 42): `error` `too_large` instead of the ack, and the ID is not remembered. */
class OfferLimitTest {
    private val receiver = MessageReceiver(DedupMemory({ 0L })).also {
        it.onText("""{"type":"hello","id":"prime","protocol":1,"name":"Desk PC","platform":"windows","appVersion":"0.1.0"}""")
    }

    private fun offer(id: String, count: Int): String {
        val files = (1..count).joinToString(",") { """{"fileId":"f$it","name":"a","size":1,"mime":"text/plain"}""" }
        return """{"type":"offer","id":"$id","files":[$files],"totalSize":$count}"""
    }

    @Test
    fun anOfferOfMoreThanTheLimitIsAnErrorAndNotAnAck() {
        val outcome = receiver.onText(offer("o1", MAX_OFFER_FILES + 1))
        assertEquals(1, outcome.replies.size)
        val reply = StrictJson.parse(outcome.replies.single()).let { it as kotlinx.serialization.json.JsonObject }
        assertEquals("error", Fields.string(reply["type"]))
        assertEquals("too_large", Fields.string(reply["code"]))
        assertEquals("o1", Fields.string(reply["ref"]))
        assertTrue(outcome.action is Action.OfferRefused)
        assertEquals(800, MAX_OFFER_FILES)
    }

    @Test
    fun theLimitItselfIsAccepted() {
        val outcome = receiver.onText(offer("o800", MAX_OFFER_FILES))
        assertTrue(outcome.action is Action.StartDownload)
    }

    @Test
    fun theIdOfARefusedOfferIsNotRememberedSoASmallerOfferMayUseIt() {
        assertTrue(receiver.onText(offer("again", 801)).action is Action.OfferRefused)
        // The same ID, now with a size that fits: an ack and a download, not the first error again.
        val second = receiver.onText(offer("again", 3))
        assertTrue("${second.action}", second.action is Action.StartDownload)
        assertTrue(second.replies.single().contains("\"ack\""))
    }

    @Test
    fun aTooBigOfferSentTwiceIsAnsweredWithTheErrorEveryTime() {
        repeat(2) { assertTrue(receiver.onText(offer("twice", 900)).action is Action.OfferRefused) }
    }
}
