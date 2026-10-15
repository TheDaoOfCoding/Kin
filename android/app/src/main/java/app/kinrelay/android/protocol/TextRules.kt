package app.kinrelay.android.protocol

/** The sender's size rule for text (protocol.md, "Text messages", "Size"). The shared vectors are `senderSize[]` of `protocol/text-rules.json`. */
object TextRules {
    enum class Plan {
        /** A `text` message. */
        TEXT,

        /** A `.txt` file through the ordinary file transfer. */
        FILE,

        /** Not sent at all: a sensitive text over the threshold (a file has no `sensitive` mark). */
        NOT_SENT,
    }

    /** What to do with an explicit text whose wire form is [wireBytes] bytes of UTF-8. */
    fun plan(wireBytes: Int, sensitive: Boolean): Plan = when {
        wireBytes <= MAX_TEXT_BYTES -> Plan.TEXT
        sensitive -> Plan.NOT_SENT
        else -> Plan.FILE
    }
}
