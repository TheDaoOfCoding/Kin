package app.kinrelay.android.protocol

import kotlinx.serialization.json.JsonObject

/** What the receiver did with a frame. [category] is the name the test vectors use. */
sealed class Action(val category: String) {
    class HandshakeOk(val hello: Hello) : Action("handshake_ok")
    class MarkDelivered(val ref: String) : Action("mark_delivered")
    class MarkFailed(val ref: String, val code: String, val message: String) : Action("mark_failed")
    class ShowErrorOnly(val code: String, val message: String) : Action("show_error_only")
    class ShowPing(val id: String) : Action("show_ping_toast")
    data object IgnoreDuplicate : Action("ignore_duplicate")
    data object RemovePeerAndClose : Action("remove_peer_and_close")
    data object CloseOnly : Action("close_only")

    /** The frame was invalid; an `error` reply went out instead. */
    data object Rejected : Action("rejected")
}

/** Frames to send back (in order), a WebSocket close code to use, and what happened. */
class Outcome(val replies: List<String>, val close: Int?, val action: Action)

/**
 * The receiving side of the control channel for one connection: framing, validation order and
 * replies exactly as protocol.md, "Control channel". It does no I/O: the caller sends the replies,
 * closes the socket and acts on [Outcome.action].
 *
 * Milestone 1.1 handles `hello`, `ack`, `error`, `ping` and `unpair`. The later message types
 * (`text`, `clipboard`, `offer`, `complete`, `cancel`) are answered with `unsupported_type`
 * until their milestones, the same as an unknown type.
 */
class MessageReceiver(private val dedup: DedupMemory) {
    var peer: Hello? = null
        private set

    val helloAccepted: Boolean get() = peer != null

    private val handled = setOf("hello", "ack", "error", "ping", "unpair")

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

        // 5. Unknown, wrong-direction or not yet supported type.
        if (type !in handled) return rejected(ErrorCodes.UNSUPPORTED_TYPE, "Unsupported message type.", ref)

        return when (type) {
            "hello" -> onHello(frame, ref)
            "ack" -> onAck(frame)
            "error" -> onError(frame)
            "ping" -> onPing(frame, ref)
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

    private fun onUnpair(frame: JsonObject, ref: String?): Outcome {
        if (ref == null) return rejected(ErrorCodes.BAD_REQUEST, "The unpair needs an id.", ref)
        return processOnce(ref, Action.RemovePeerAndClose, close = 1000)
    }

    /** Acks a message, or repeats the stored response if its ID was already processed. */
    private fun processOnce(id: String, action: Action, close: Int? = null): Outcome {
        dedup.lookup(id)?.let { return Outcome(listOf(it), null, Action.IgnoreDuplicate) }
        val ack = Frames.ack(id)
        dedup.record(id, ack)
        return Outcome(listOf(ack), close, action)
    }

    private fun rejected(code: String, message: String, ref: String?, close: Int? = null) =
        Outcome(listOf(Frames.error(code, message, ref)), close, Action.Rejected)
}
