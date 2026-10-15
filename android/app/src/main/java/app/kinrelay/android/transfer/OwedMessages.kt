package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.link.AckResult
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageIds

/**
 * The `complete` and `cancel` messages a download owes the PC (protocol.md, Queue: "kept until acked"). A download that ends writes the
 * message it owes in the same database write as its status ([owedComplete], [owedCancel]), so a crash can't leave a finished download that
 * the PC never hears about. The message goes out at once if there is a connection, and **again after every `hello`**, before the queue
 * is flushed ([resendAll]), under the **same message ID** every time, until the PC acks it (or answers with an `error`, which is final).
 * The PC's memory of IDs makes a repeat harmless.
 */
class OwedMessages(private val d: TransferDeps, private val ackTimeoutMillis: Long) {
    /** What an owed message came to. */
    enum class Outcome {
        /** Acked or refused: nothing is owed any more. */
        SETTLED,

        /** Nothing to send, or nothing was sent (no connection): still owed. */
        PENDING,

        /** Sent, and the PC didn't answer: the connection is dead, and the message is still owed. */
        NO_REPLY,
    }

    /** Sends what [transferId] owes on [connection] (null: no connection, so it stays owed). */
    suspend fun send(connection: LinkConnection?, transferId: String): Outcome {
        val transfer = d.store.transfer(transferId) ?: return Outcome.PENDING
        return send(connection, transfer)
    }

    private suspend fun send(connection: LinkConnection?, transfer: TransferEntity): Outcome {
        val owed = transfer.owed ?: return Outcome.SETTLED
        val messageId = transfer.owedId ?: return Outcome.SETTLED
        if (connection == null || !connection.isReady || connection.peerDeviceId != transfer.peerDeviceId) return Outcome.PENDING
        val frame = frameFor(owed, messageId, transfer.id) ?: run {
            d.store.clearOwed(transfer.id)
            return Outcome.SETTLED
        }
        return when (val ack = connection.sendAcked(messageId, frame, ackTimeoutMillis)) {
            AckResult.Acked -> {
                d.store.clearOwed(transfer.id)
                Outcome.SETTLED
            }
            is AckResult.Refused -> {
                d.log.w("transfer", "${transfer.id}: the PC refused our $owed: ${ack.code}")
                d.store.clearOwed(transfer.id)
                Outcome.SETTLED
            }
            AckResult.NoReply -> {
                d.log.w("transfer", "${transfer.id}: $owed not acked; it is sent again after the next hello")
                Outcome.NO_REPLY
            }
            AckResult.NoConnection -> Outcome.PENDING
        }
    }

    /** After a `hello` from [connection]'s PC: everything owed to it goes out again, oldest first. Stops at the first message that gets no reply. */
    suspend fun resendAll(connection: LinkConnection) {
        for (transfer in d.store.owing(connection.peerDeviceId)) {
            if (send(connection, transfer) != Outcome.SETTLED) return
        }
    }

    private fun frameFor(owed: String, messageId: String, transferId: String): String? = when {
        owed == COMPLETE -> Frames.complete(messageId, transferId)
        owed.startsWith(CANCEL_PREFIX) -> Frames.cancel(messageId, transferId, owed.removePrefix(CANCEL_PREFIX))
        else -> null
    }

    companion object {
        const val COMPLETE = "complete"
        private const val CANCEL_PREFIX = "cancel:"

        /** The value of [TransferEntity.owed] for a `cancel` with [reason] (`user` or `error`). */
        fun cancel(reason: String) = "$CANCEL_PREFIX$reason"

        fun newMessageId(): String = MessageIds.newId()
    }
}
