package app.kinrelay.android.link

import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.UrlRules

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

    /** Over 1 MiB: the spec sends that as a `.txt` file, which comes with files in 1.3. */
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

/**
 * Sends text to the PC as a `text` message and waits for the ack (5 s, protocol.md). With no connection it
 * says "PC not reachable" and sends nothing: the persistent queue is 1.4. It reports to the user itself
 * ([Alerts]), so the activities that call it can finish at once.
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
) {
    /** [tapAtMillis] is on the [Clock]'s timeline: when the user's tap reached the app. */
    suspend fun send(text: String, tapAtMillis: Long, source: SendSource): SendOutcome {
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
        val id = MessageIds.newId()
        val isUrl = UrlRules.isUrl(text)
        val frame = Frames.text(id, text, isUrl)
        if (frame == null) {
            log.w("link", "send refused ($source): the text is over 1 MiB $fingerprint; a .txt file comes with files in 1.3")
            alerts.message("That text is too long to send for now.")
            return SendOutcome.TooLong
        }
        val pcName = link.peerName()
        val connection = link.await(connectWaitMillis)
        if (connection == null) {
            val reason = link.unreachableReason()
            log.w("link", "send failed ($source): PC not reachable: $reason $fingerprint")
            alerts.sendFailed("PC not reachable", "Couldn't send to ${pcName ?: "your PC"}. $reason")
            return SendOutcome.NotReachable(reason)
        }
        return when (val result = connection.sendAcked(id, frame, ackTimeoutMillis)) {
            AckResult.Acked -> {
                val arrival = clock.elapsedMillis() - tapAtMillis
                arrivals.record(arrival)
                log.i("link", "text sent ($source) $fingerprint isUrl=$isUrl; tap to ack $arrival ms")
                alerts.sent(pcName, arrival)
                SendOutcome.Delivered(arrival)
            }
            AckResult.NoConnection -> {
                log.w("link", "send ($source): the connection dropped before the send $fingerprint")
                alerts.sendFailed("PC not reachable", "Couldn't send to ${pcName ?: "your PC"}. The connection dropped.")
                SendOutcome.NotReachable("The connection dropped.")
            }
            AckResult.NoReply -> {
                log.w("link", "send ($source): no ack in $ackTimeoutMillis ms $fingerprint")
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

    companion object {
        /** protocol.md: acks are due within 5 seconds. */
        const val ACK_TIMEOUT_MILLIS = 5_000L

        /** How long a send waits for a connection that is being (re)started. */
        const val CONNECT_WAIT_MILLIS = 4_000L
    }
}
