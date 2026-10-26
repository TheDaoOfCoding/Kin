package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.data.TextSource
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.LinkWire
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.TextRules
import app.kinrelay.android.protocol.UrlRules
import app.kinrelay.android.protocol.Utf8
import app.kinrelay.android.transfer.PeerInfo

/** Where an explicit send came from. Only a clipboard send is checked against the loop guard. */
enum class SendSource(val stored: String) {
    SHARE(TextSource.SHARE),
    SHARE_LINK(TextSource.SHARE_LINK),
    PROCESS_TEXT(TextSource.PROCESS_TEXT),
    CLIPBOARD(TextSource.CLIPBOARD),
    TYPED(TextSource.TYPED),
    DEBUG(TextSource.DEBUG),
}

sealed interface SendOutcome {
    /** The PC acked. [arrivalMillis] is the time from the tap to the ack. */
    data class Delivered(val arrivalMillis: Long) : SendOutcome

    /**
     * Saved in the queue and not delivered yet: there was no connection (and none appeared in a few seconds), or the PC didn't ack in time
     * ([afterNoReply]: it may or may not have arrived). It goes out by itself when the PC is back, and is shown in the history meanwhile.
     */
    data class Queued(val reason: String, val afterNoReply: Boolean = false) : SendOutcome

    /** The PC answered with an `error`. */
    data class Refused(val code: String) : SendOutcome

    /** A sensitive text with no open connection (or no ack): never queued, never stored. The user was told. */
    data class NotSent(val reason: String) : SendOutcome

    /** Canceled in the history before it went out. */
    data object Canceled : SendOutcome

    /** No PC is paired: nothing was queued. */
    data object NotPaired : SendOutcome

    /** Over 128 KiB, so it goes as a `.txt` file ([TextFileSender]): the upload runs on its own, with its own progress notification. */
    data object SentAsFile : SendOutcome

    /** Over 128 KiB and not sent: a sensitive text (a file can't carry the mark), or the file couldn't be queued. */
    data object TooLong : SendOutcome

    /** Blank text: nothing to send. */
    data object Empty : SendOutcome

    /** A `link` whose URL fails the wire rule (`http` or `https` only): never sent or queued, and the user was told "That isn't a web link". */
    data object NotAWebLink : SendOutcome

    /** The text is what the PC just put on the clipboard (loop guard). */
    data object Echo : SendOutcome
}

/** How the sender gets a connection and the facts it needs for messages. */
interface OutgoingLink {
    /** A connection to the PC [peerDeviceId] that has accepted its `hello`: now, or one that appears within [waitMillis]. */
    suspend fun await(waitMillis: Long, peerDeviceId: String): LinkConnection?

    /** Why [await] found nothing, in words for the user. */
    fun unreachableReason(): String

    /** The PC this phone is paired with, or null. */
    suspend fun peer(): PeerInfo?
}

/** Records the tap-to-ack time (targets.md priority 2). */
fun interface ArrivalSink {
    suspend fun record(millis: Long)
}

/** Sends a text that is too long for a message as a `.txt` file transfer. The text is in wire form. True if it was queued. */
fun interface TextFileSender {
    suspend fun send(wireText: String): Boolean

    companion object {
        /** None: a text over the threshold can't be sent. */
        val None = TextFileSender { false }
    }
}

/**
 * Sends text to the PC: **saved in the queue first** ([TextOutbox]), then the queue is flushed if there is a connection ([OutboxFlusher]),
 * and the outcome tells the user what happened. With no connection (none appears within [connectWaitMillis]) the text stays queued
 * ("saved, goes out when the PC is back") instead of being lost; with no ack in 5 s it is re-queued, not failed. It reports to the user
 * itself ([Alerts]), so the activities that call it can finish at once.
 *
 * The text goes out in its wire form ([Frames.wireText]: LF only). A text over 128 KiB (UTF-8) is not sent as a message but as a
 * `.txt` file ([TextFileSender]), which is an upload (and is queued like any upload). The 1 MiB frame limit stays as the receivers' hard
 * limit. The automatic clipboard sync (PC to phone) is not sent from here and is never queued.
 */
class OutgoingSender(
    private val link: OutgoingLink,
    private val alerts: Alerts,
    private val guard: ClipboardGuard,
    private val arrivals: ArrivalSink,
    private val clock: Clock,
    private val log: KinLog,
    private val outbox: TextOutbox,
    private val flusher: OutboxFlusher,
    private val connectWaitMillis: Long = CONNECT_WAIT_MILLIS,
    private val textFiles: TextFileSender = TextFileSender.None,
    private val ackTimeoutMillis: Long = ACK_TIMEOUT_MILLIS,
) {
    /**
     * [tapAtMillis] is on the [Clock]'s timeline: when the user's tap reached the app. [sensitive] is set when the clipboard's
     * `ClipDescription` says the content is sensitive (a password manager): the PC then keeps it out of its history.
     */
    suspend fun send(text: String, tapAtMillis: Long, source: SendSource, sensitive: Boolean = false): SendOutcome {
        // A sensitive text is never fingerprinted: no length, no hash, nothing about it reaches the log (a hash of a short secret can be guessed).
        val fingerprint = if (sensitive) "" else ClipboardGuard.fingerprint(text)
        if (text.isBlank()) {
            alerts.message("Nothing to send.")
            return SendOutcome.Empty
        }
        if (source == SendSource.CLIPBOARD && guard.isEcho(text)) {
            // The skip itself is logged for a sensitive text too (it is how the e2e checks the guard); only the fingerprint is left out.
            log.i("link", "send skipped ($source): the text is what the PC just set${if (sensitive) "" else " $fingerprint"}")
            alerts.message("The PC already has this text.")
            return SendOutcome.Echo
        }
        val wire = Frames.wireText(text)
        when (TextRules.plan(Utf8.size(wire), sensitive)) {
            TextRules.Plan.TEXT -> Unit
            TextRules.Plan.FILE -> return sendAsFile(wire, source, fingerprint)
            TextRules.Plan.NOT_SENT -> {
                // A file has no `sensitive` mark, and writing a password-manager secret to the PC's disk is not what the user asked for.
                log.w("link", "send refused ($source): a sensitive text over 128 KiB is not sent")
                alerts.message("That text is sensitive and too long to send.")
                return SendOutcome.TooLong
            }
        }
        val isUrl = UrlRules.isUrl(text)
        // Over 128 KiB went to a file above, so the frame is far below 1 MiB: null can't happen, but never queue a broken frame.
        if (Frames.text("probe", text, isUrl, sensitive) == null) return SendOutcome.TooLong
        val peer = link.peer()
        if (peer == null) {
            alerts.message("Pair a PC first.")
            return SendOutcome.NotPaired
        }

        // A sensitive text is never stored (protocol.md, Queue), so it can't wait: it goes out only over an open connection.
        if (sensitive) return sendSensitive(text, isUrl, tapAtMillis, source, peer)

        // Saved first. From here on the text can't be lost by a dead connection or a killed process.
        val item = outbox.enqueue(text, isUrl, sensitive, source.stored, peer)
        return deliver(item.id, peer, tapAtMillis, source, "text", "${logged(fingerprint, sensitive)} isUrl=$isUrl sensitive=$sensitive")
    }

    /**
     * Sends a web link (2B.1: a page shared to Kin): the same queue-first path as a text, so it waits for the PC when it is away and goes out in
     * order after the `hello`, for at most 7 days. [rawUrl] is what the user shared: it is trimmed here (the wire rule takes the URL as sent) and
     * must then be one `http` or `https` URL, or nothing is queued and the user is told "That isn't a web link". [rawTitle] is the page title
     * (the share's subject), cleaned and cut; a title that is just the URL is dropped. Whether it goes as a `link` or, to a PC that is an old
     * version, as a `text` is decided when it goes on the wire ([TextOutbox.sendOne]).
     */
    suspend fun sendLink(rawUrl: String, rawTitle: String?, tapAtMillis: Long, source: SendSource): SendOutcome {
        val url = rawUrl.trim { UrlRules.isWhitespace(it) }
        if (!LinkWire.isLinkUrl(url)) {
            alerts.message("That isn't a web link.")
            return SendOutcome.NotAWebLink
        }
        val title = rawTitle?.let(LinkWire::cleanTitle)?.takeIf { it != url }
        val peer = link.peer()
        if (peer == null) {
            alerts.message("Pair a PC first.")
            return SendOutcome.NotPaired
        }
        val item = outbox.enqueueLink(url, title, source.stored, peer)
        return deliver(item.id, peer, tapAtMillis, source, "link", "titleChars=${title?.length ?: 0}")
    }

    /** What follows saving a text or link: wait for a connection, flush the queue, and tell the user how it went. */
    private suspend fun deliver(itemId: String, peer: PeerInfo, tapAtMillis: Long, source: SendSource, what: String, detail: String): SendOutcome {
        val connection = link.await(connectWaitMillis, peer.deviceId)
        if (connection != null) flusher.flush(connection)

        val result = outbox.get(itemId)
        return when (result?.status) {
            TransferStatus.DONE -> {
                val arrival = clock.elapsedMillis() - tapAtMillis
                arrivals.record(arrival)
                log.i("link", "$what sent ($source) $detail; tap to ack $arrival ms")
                alerts.sent(peer.name, arrival)
                SendOutcome.Delivered(arrival)
            }
            TransferStatus.FAILED -> {
                val code = result.errorCode ?: "failed"
                log.w("link", "send ($source): not delivered, $code $detail")
                alerts.sendFailed("${peer.name} refused it", result.message ?: "The PC answered with an error ($code).")
                SendOutcome.Refused(code)
            }
            TransferStatus.CANCELED -> SendOutcome.Canceled
            else -> {
                outbox.markReportedQueued(itemId)
                val noReply = connection != null
                val reason = if (noReply) "No reply from ${peer.name} yet." else link.unreachableReason()
                log.w("link", "send ($source): queued, ${if (noReply) "no ack" else "PC not reachable: $reason"} $detail")
                alerts.queued(peer.name, reason, afterNoReply = noReply)
                SendOutcome.Queued(reason, afterNoReply = noReply)
            }
        }
    }

    /** Sends a sensitive text now or not at all: nothing goes to the database, the history or the queue, and the log has no hash. */
    private suspend fun sendSensitive(text: String, isUrl: Boolean, tapAtMillis: Long, source: SendSource, peer: PeerInfo): SendOutcome {
        val connection = link.await(connectWaitMillis, peer.deviceId)
        val id = MessageIds.newId()
        val frame = Frames.text(id, Frames.sanitize(text), isUrl, sensitive = true) ?: return SendOutcome.TooLong
        val result = connection?.sendAcked(id, frame, ackTimeoutMillis) ?: AckResult.NoConnection
        return when (result) {
            AckResult.Acked -> {
                val arrival = clock.elapsedMillis() - tapAtMillis
                arrivals.record(arrival)
                log.i("link", "sensitive text sent ($source) (its content is not logged); tap to ack $arrival ms")
                alerts.sent(peer.name, arrival)
                SendOutcome.Delivered(arrival)
            }
            is AckResult.Refused -> {
                log.w("link", "sensitive send ($source): not delivered, ${result.code}")
                alerts.sendFailed("${peer.name} refused it", "The PC answered with an error (${result.code}).")
                SendOutcome.Refused(result.code)
            }
            AckResult.NoReply, AckResult.NoConnection -> {
                val reason = if (result == AckResult.NoReply) "${peer.name} didn't confirm it." else link.unreachableReason()
                log.w("link", "sensitive send ($source): not sent, ${if (result == AckResult.NoReply) "no ack" else "PC not reachable"}; sensitive texts are not queued")
                alerts.sendFailed("Not sent: ${peer.name} offline", "A sensitive text is never saved to be sent later. $reason Copy it again when ${peer.name} is back.")
                SendOutcome.NotSent(reason)
            }
        }
    }

    private suspend fun sendAsFile(wire: String, source: SendSource, fingerprint: String): SendOutcome {
        if (!textFiles.send(wire)) {
            log.w("link", "send refused ($source): the text is over 128 KiB and could not be queued as a file $fingerprint")
            alerts.message("That text is too long to send.")
            return SendOutcome.TooLong
        }
        log.i("link", "text over 128 KiB ($source) goes as a .txt file $fingerprint")
        return SendOutcome.SentAsFile
    }

    /** A sensitive text is never logged, not even by its length or its hash. */
    private fun logged(fingerprint: String, sensitive: Boolean) = if (sensitive) "(sensitive)" else fingerprint

    companion object {
        /** protocol.md: acks are due within 5 seconds. */
        const val ACK_TIMEOUT_MILLIS = 5_000L

        /** How long a send waits for a connection that is being (re)started. */
        const val CONNECT_WAIT_MILLIS = 4_000L
    }
}
