package app.kinrelay.android.protocol

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject

/** One file of an `offer`. [name] is the sender's raw name: it is cleaned only when the file is saved. */
data class OfferFile(val fileId: String, val name: String, val size: Long, val mime: String)

/** A valid `offer`. [id] is the transfer ID. */
data class Offer(val id: String, val files: List<OfferFile>, val totalSize: Long)

/** What the receiver did with a frame. [category] is the name the test vectors use. */
sealed class Action(val category: String) {
    class HandshakeOk(val hello: Hello) : Action("handshake_ok")
    class MarkDelivered(val ref: String) : Action("mark_delivered")
    class MarkFailed(val ref: String, val code: String, val message: String) : Action("mark_failed")
    class ShowErrorOnly(val code: String, val message: String) : Action("show_error_only")
    class ShowPing(val id: String) : Action("show_ping_toast")
    class DeliverText(val id: String, val text: String, val isUrl: Boolean, val sensitive: Boolean) : Action("deliver_text")
    class SetClipboard(val id: String, val text: String) : Action("set_clipboard")

    /** A valid `offer`, acked: download the files. */
    class StartDownload(val offer: Offer) : Action("start_download")

    /** The PC (role) got a `complete` from the phone: the transfer is done. */
    class MarkTransferDone(val transferId: String) : Action("mark_transfer_done")

    /** A `cancel`: stop the transfer and delete its partial files. Unknown transfers are ignored by whoever handles this. */
    class CancelTransfer(val transferId: String, val reason: String) : Action("cancel_transfer")

    /** An `offer` the phone can't take (no space): an `error` went out instead of the ack. */
    class OfferRefused(val code: String) : Action("offer_refused")

    data object IgnoreDuplicate : Action("ignore_duplicate")
    data object RemovePeerAndClose : Action("remove_peer_and_close")
    data object CloseOnly : Action("close_only")

    /** The frame was invalid; an `error` reply went out instead. */
    data object Rejected : Action("rejected")
}

/** Frames to send back (in order), a WebSocket close code to use, and what happened. */
class Outcome(val replies: List<String>, val close: Int?, val action: Action)

/** Which side of the connection a [MessageReceiver] stands for. The direction of a message decides whether it is a known type. */
enum class ReceiverRole {
    /** The real app. Accepts `clipboard` and `offer`, refuses `complete` (only the phone sends it). */
    PHONE,

    /** Stands for the PC in the shared test vectors. Accepts `complete`, refuses `clipboard` and `offer`. */
    PC,
}

/**
 * The receiving side of the control channel for one connection: framing, validation order and
 * replies exactly as protocol.md, "Control channel". It does no I/O: the caller sends the replies,
 * closes the socket and acts on [Outcome.action].
 *
 * Handles `hello`, `ack`, `error`, `ping`, `unpair`, `text`, `clipboard` and, since 1.3, `offer`, `complete` and
 * `cancel`. A message type that only goes the other way (`clipboard` and `offer` from the phone, `complete` from the
 * PC) is `unsupported_type`.
 *
 * [offerGate] runs on a valid, new `offer` before it is acked: it returns null to accept, or an error code (the phone
 * says `disk_full`) to send an `error` *instead of* the ack, which ends the offer's life (protocol.md, Files: PC → phone).
 */
class MessageReceiver(
    private val dedup: DedupMemory,
    private val role: ReceiverRole = ReceiverRole.PHONE,
    private val offerGate: (Offer) -> String? = { null },
) {
    var peer: Hello? = null
        private set

    val helloAccepted: Boolean get() = peer != null

    private val handled: Set<String> = buildSet {
        addAll(listOf("hello", "ack", "error", "ping", "unpair", "text", "cancel"))
        if (role == ReceiverRole.PHONE) addAll(listOf("clipboard", "offer")) else add("complete")
    }

    fun onBinary(): Outcome = Outcome(emptyList(), 1003, Action.CloseOnly)

    fun onText(text: String): Outcome {
        // 1. Size (UTF-8 bytes of the whole frame). Closes without a reply.
        if (Utf8.size(text) > MAX_FRAME_BYTES) return Outcome(emptyList(), 1009, Action.CloseOnly)

        // 2. One valid JSON object.
        val frame = try {
            StrictJson.parse(text) as? JsonObject
        } catch (_: StrictJson.InvalidJson) {
            null
        } ?: return rejected(ErrorCodes.BAD_REQUEST, "The message is not a valid JSON object.", ref = null)

        val ref = Fields.id(frame["id"])

        // 3. `type` present and a string.
        val type = Fields.string(frame["type"])
            ?: return rejected(ErrorCodes.BAD_REQUEST, "The message has no string type.", ref)

        // 4. Before a hello is accepted, anything else is a protocol error.
        if (!helloAccepted && type != "hello") {
            return rejected(ErrorCodes.BAD_REQUEST, "Send hello first.", ref, close = 1002)
        }

        // 5. Unknown or wrong-direction type.
        if (type !in handled) return rejected(ErrorCodes.UNSUPPORTED_TYPE, "Unsupported message type.", ref)

        return when (type) {
            "hello" -> onHello(frame, ref)
            "ack" -> onAck(frame)
            "error" -> onError(frame)
            "ping" -> onPing(frame, ref)
            "text" -> onText(frame, ref)
            "clipboard" -> onClipboard(frame, ref)
            "offer" -> onOffer(frame, ref)
            "complete" -> onComplete(frame, ref)
            "cancel" -> onCancel(frame, ref)
            else -> onUnpair(frame, ref)
        }
    }

    private fun onHello(frame: JsonObject, ref: String?): Outcome {
        // 6. protocol is checked before every other field.
        val protocol = Fields.integer(frame["protocol"])
        if (protocol != null && protocol != PROTOCOL_VERSION.toLong()) {
            return rejected(ErrorCodes.UNSUPPORTED_VERSION, "Protocol version $protocol is not supported. Update the app.", ref, close = 4002)
        }
        // 7. Required fields and types.
        val id = ref
        val name = Fields.nonEmptyString(frame["name"])
        val platform = Fields.nonEmptyString(frame["platform"])
        val appVersion = Fields.string(frame["appVersion"])
        if (protocol == null || id == null || name == null || platform == null || appVersion == null) {
            return rejected(ErrorCodes.BAD_REQUEST, "The hello is missing a field or has a wrong type.", ref)
        }
        // 8. A hello has no ack and its ID isn't remembered.
        val hello = Hello(id, name, platform, appVersion)
        peer = hello
        return Outcome(emptyList(), null, Action.HandshakeOk(hello))
    }

    private fun onAck(frame: JsonObject): Outcome {
        val ackRef = Fields.id(frame["ref"])
            ?: return rejected(ErrorCodes.BAD_REQUEST, "The ack has no valid ref.", ref = null)
        return Outcome(emptyList(), null, Action.MarkDelivered(ackRef))
    }

    private fun onError(frame: JsonObject): Outcome {
        val code = Fields.string(frame["code"])
        val message = Fields.string(frame["message"])
        val rawRef = Fields.present(frame["ref"])
        val errorRef = Fields.id(rawRef)
        if (code == null || message == null || (rawRef != null && errorRef == null)) {
            return rejected(ErrorCodes.BAD_REQUEST, "The error message is invalid.", ref = null)
        }
        val action = if (errorRef != null) Action.MarkFailed(errorRef, code, message) else Action.ShowErrorOnly(code, message)
        return Outcome(emptyList(), null, action)
    }

    private fun onPing(frame: JsonObject, ref: String?): Outcome {
        val sentAt = Fields.integer(frame["sentAt"])
        if (ref == null || sentAt == null) {
            return rejected(ErrorCodes.BAD_REQUEST, "The ping needs an id and a sentAt.", ref)
        }
        return processOnce(ref, Action.ShowPing(ref))
    }

    private fun onText(frame: JsonObject, ref: String?): Outcome {
        val text = Fields.nonEmptyString(frame["text"])
        val isUrl = Fields.boolean(frame["isUrl"])
        // `sensitive` is optional: absent or null is false, anything but a boolean is invalid.
        val rawSensitive = Fields.present(frame["sensitive"])
        val sensitive = if (rawSensitive == null) false else Fields.boolean(rawSensitive)
        if (ref == null || text == null || isUrl == null || sensitive == null) {
            return rejected(ErrorCodes.BAD_REQUEST, "The text message needs an id, a non-empty text and isUrl (and sensitive, if present, must be a boolean).", ref)
        }
        return processOnce(ref, Action.DeliverText(ref, text, isUrl, sensitive))
    }

    private fun onClipboard(frame: JsonObject, ref: String?): Outcome {
        val text = Fields.nonEmptyString(frame["text"])
        if (ref == null || text == null) {
            return rejected(ErrorCodes.BAD_REQUEST, "The clipboard message needs an id and a non-empty text.", ref)
        }
        return processOnce(ref, Action.SetClipboard(ref, text))
    }

    private fun onOffer(frame: JsonObject, ref: String?): Outcome {
        val offer = parseOffer(frame, ref)
            ?: return rejected(ErrorCodes.BAD_REQUEST, "The offer needs a transfer id, at least one file (fileId, name, size, mime) and totalSize.", ref)
        // A repeated offer gets its first response again and is not looked at again (protocol.md, de-duplication).
        dedup.lookup(offer.id)?.let { return Outcome(listOf(it), null, Action.IgnoreDuplicate) }
        // More than 800 files: an `error` instead of the ack, and the ID is not remembered, so a smaller offer may use it (protocol.md, Files: PC -> phone).
        if (offer.files.size > MAX_OFFER_FILES) {
            val error = Frames.error(ErrorCodes.TOO_LARGE, "An offer has at most $MAX_OFFER_FILES files.", offer.id)
            return Outcome(listOf(error), null, Action.OfferRefused(ErrorCodes.TOO_LARGE))
        }
        val refusal = offerGate(offer)
        if (refusal != null) {
            // An `error` in place of the ack ends the offer's life. It is remembered like any response.
            val error = Frames.error(refusal, "The phone can't take this transfer.", offer.id)
            dedup.record(offer.id, error)
            return Outcome(listOf(error), null, Action.OfferRefused(refusal))
        }
        return processOnce(offer.id, Action.StartDownload(offer))
    }

    private fun parseOffer(frame: JsonObject, ref: String?): Offer? {
        val id = ref?.takeIf { Fields.isPathId(it) } ?: return null
        val files = frame["files"] as? JsonArray ?: return null
        val totalSize = Fields.integer(frame["totalSize"]) ?: return null
        if (files.isEmpty()) return null
        val parsed = ArrayList<OfferFile>(files.size)
        val seen = HashSet<String>()
        for (entry in files) {
            val file = entry as? JsonObject ?: return null
            val fileId = Fields.string(file["fileId"])?.takeIf { Fields.isPathId(it) } ?: return null
            val name = Fields.string(file["name"]) ?: return null
            val size = Fields.integer(file["size"]) ?: return null
            val mime = Fields.string(file["mime"]) ?: return null
            if (!seen.add(fileId)) return null
            parsed += OfferFile(fileId, name, size, mime)
        }
        return Offer(id, parsed, totalSize)
    }

    private fun onComplete(frame: JsonObject, ref: String?): Outcome {
        val transferId = Fields.string(frame["transferId"])?.takeIf { Fields.isPathId(it) }
        if (ref == null || transferId == null) {
            return rejected(ErrorCodes.BAD_REQUEST, "The complete message needs an id and a transferId.", ref)
        }
        return processOnce(ref, Action.MarkTransferDone(transferId))
    }

    private fun onCancel(frame: JsonObject, ref: String?): Outcome {
        val transferId = Fields.string(frame["transferId"])?.takeIf { Fields.isPathId(it) }
        val reason = Fields.string(frame["reason"])
        if (ref == null || transferId == null || reason == null) {
            return rejected(ErrorCodes.BAD_REQUEST, "The cancel message needs an id, a transferId and a reason.", ref)
        }
        return processOnce(ref, Action.CancelTransfer(transferId, reason))
    }

    private fun onUnpair(frame: JsonObject, ref: String?): Outcome {
        if (ref == null) return rejected(ErrorCodes.BAD_REQUEST, "The unpair needs an id.", ref)
        return processOnce(ref, Action.RemovePeerAndClose, close = 1000, closeOnDuplicate = 1000)
    }

    /**
     * Acks a message, or repeats the stored response if its ID was already processed.
     * A repeated `unpair` is acked again and the connection is closed again ([closeOnDuplicate]), as the PC does.
     */
    private fun processOnce(id: String, action: Action, close: Int? = null, closeOnDuplicate: Int? = null): Outcome {
        dedup.lookup(id)?.let { return Outcome(listOf(it), closeOnDuplicate, Action.IgnoreDuplicate) }
        val ack = Frames.ack(id)
        dedup.record(id, ack)
        return Outcome(listOf(ack), close, action)
    }

    private fun rejected(code: String, message: String, ref: String?, close: Int? = null) =
        Outcome(listOf(Frames.error(code, message, ref)), close, Action.Rejected)
}
