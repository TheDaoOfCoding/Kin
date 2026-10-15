package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.TextRules
import app.kinrelay.android.protocol.UrlRules
import app.kinrelay.android.protocol.Utf8

/** Where an explicit send came from. Only a clipboard send is checked against the loop guard. */
enum class SendSource { SHARE, PROCESS_TEXT, CLIPBOARD, DEBUG }

sealed interface SendOutcome {
    /** The PC acked. [arrivalMillis] is the time from the tap to the ack. */
    data class Delivered(val arrivalMillis: Long) : SendOutcome

    /** No connection (and none appeared in a few seconds). Nothing was sent. */
    data class NotReachable(val reason: String) : SendOutcome

    /** Sent, but no ack in time. It may or may not have arrived. */
    data object NoReply : SendOutcome

    /** The PC answered with an `error`. */
    data class Refused(val code: String) : SendOutcome

    /** Over 128 KiB, so it goes as a `.txt` file ([TextFileSender]): the upload runs on its own, with its own progress notification. */
    data object SentAsFile : SendOutcome

    /** Over 128 KiB and not sent: a sensitive text (a file can't carry the mark), or the file couldn't be queued. */
    data object TooLong : SendOutcome

    /** Blank text: nothing to send. */
    data object Empty : SendOutcome

    /** The text is what the PC just put on the clipboard (loop guard). */
    data object Echo : SendOutcome
}

/** How the sender gets a connection and the facts it needs for messages. */
interface OutgoingLink {
    /** A connection that has accepted the PC's `hello`: now, or one that appears within [waitMillis]. */
    suspend fun await(waitMillis: Long): LinkConnection?

    /** Why [await] found nothing, in words for the user. */
    fun unreachableReason(): String

    suspend fun peerName(): String?
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
 * Sends text to the PC as a `text` message and waits for the ack (5 s, protocol.md). With no connection it
 * says "PC not reachable" and sends nothing: the persistent queue is 1.4. It reports to the user itself
 * ([Alerts]), so the activities that call it can finish at once.
 *
 * The text goes out in its wire form ([Frames.wireText]: LF only). A text over 128 KiB (UTF-8) is not sent as a message but as a
 * `.txt` file ([TextFileSender]), which is an upload like any other. The 1 MiB frame limit stays as the receivers' hard limit.
 */
class OutgoingSender(
    private val link: OutgoingLink,
    private val alerts: Alerts,
    private val guard: ClipboardGuard,
    private val arrivals: ArrivalSink,
    private val clock: Clock,
    private val log: KinLog,
    private val ackTimeoutMillis: Long = ACK_TIMEOUT_MILLIS,
    private val connectWaitMillis: Long = CONNECT_WAIT_MILLIS,
    private val textFiles: TextFileSender = TextFileSender.None,
) {
    /**
     * [tapAtMillis] is on the [Clock]'s timeline: when the user's tap reached the app. [sensitive] is set when the clipboard's
     * `ClipDescription` says the content is sensitive (a password manager): the PC then keeps it out of its history.
     */
    suspend fun send(text: String, tapAtMillis: Long, source: SendSource, sensitive: Boolean = false): SendOutcome {
        val fingerprint = ClipboardGuard.fingerprint(text)
        if (text.isBlank()) {
            alerts.message("Nothing to send.")
            return SendOutcome.Empty
        }
        if (source == SendSource.CLIPBOARD && guard.isEcho(text)) {
            log.i("link", "send skipped ($source): the text is what the PC just set $fingerprint")
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
        val id = MessageIds.newId()
        val isUrl = UrlRules.isUrl(text)
        // Over 128 KiB went to a file above, so the frame is far below 1 MiB: null can't happen, but never send a broken frame.
        val frame = Frames.text(id, text, isUrl, sensitive) ?: return SendOutcome.TooLong
        val pcName = link.peerName()
        val connection = link.await(connectWaitMillis)
        if (connection == null) {
            val reason = link.unreachableReason()
            log.w("link", "send failed ($source): PC not reachable: $reason ${logged(fingerprint, sensitive)}")
            alerts.sendFailed("PC not reachable", "Couldn't send to ${pcName ?: "your PC"}. $reason")
            return SendOutcome.NotReachable(reason)
        }
        return when (val result = connection.sendAcked(id, frame, ackTimeoutMillis)) {
            AckResult.Acked -> {
                val arrival = clock.elapsedMillis() - tapAtMillis
                arrivals.record(arrival)
                log.i("link", "text sent ($source) ${logged(fingerprint, sensitive)} isUrl=$isUrl sensitive=$sensitive; tap to ack $arrival ms")
                alerts.sent(pcName, arrival)
                SendOutcome.Delivered(arrival)
            }
            AckResult.NoConnection -> {
                log.w("link", "send ($source): the connection dropped before the send ${logged(fingerprint, sensitive)}")
                alerts.sendFailed("PC not reachable", "Couldn't send to ${pcName ?: "your PC"}. The connection dropped.")
                SendOutcome.NotReachable("The connection dropped.")
            }
            AckResult.NoReply -> {
                log.w("link", "send ($source): no ack in $ackTimeoutMillis ms ${logged(fingerprint, sensitive)}")
                alerts.sendFailed("No reply from ${pcName ?: "your PC"}", "It may or may not have arrived. Try again if it didn't.")
                SendOutcome.NoReply
            }
            is AckResult.Refused -> {
                log.w("link", "send ($source): the PC answered ${result.code}: ${result.message}")
                alerts.sendFailed("${pcName ?: "Your PC"} refused it", "The PC answered with an error (${result.code}).")
                SendOutcome.Refused(result.code)
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

    /** A sensitive text is never logged, not even by its hash. */
    private fun logged(fingerprint: String, sensitive: Boolean) = if (sensitive) "(sensitive)" else fingerprint

    companion object {
        /** protocol.md: acks are due within 5 seconds. */
        const val ACK_TIMEOUT_MILLIS = 5_000L

        /** How long a send waits for a connection that is being (re)started. */
        const val CONNECT_WAIT_MILLIS = 4_000L
    }
}
