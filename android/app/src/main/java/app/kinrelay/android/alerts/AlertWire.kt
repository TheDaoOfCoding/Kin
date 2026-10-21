package app.kinrelay.android.alerts

import app.kinrelay.android.link.Clock
import app.kinrelay.android.media.MediaBucket
import app.kinrelay.android.protocol.Fields
import app.kinrelay.android.protocol.LinkWire
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** A valid `alert` as the phone keeps it: the text and title cleaned like `link.title`, and how long it waited in the PC's queue (null = it did not wait). */
data class AlertMessage(val text: String, val title: String?, val waitedMs: Long?)

/**
 * `alert` (2B.5, changelog 59): a PC to phone message for scripts ("tell me when it's done") and the browser's "Download finished". It is a text-class message: acked, de-duplicated by `id`, queued
 * on the PC with an expiry the sender chose (local: nothing about it crosses the wire). The vectors are `alerts.json` (`alert[]`, `alertRate[]`) and the `alert-*` cases of `messages.json`.
 */
object AlertWire {
    /** "I understand `alert`": the phone lists it in every `hello` (it is the only receiver). */
    const val CAPABILITY = "alert"
    const val TYPE = "alert"

    const val FRAME_BYTES = 4 * 1024
    const val TEXT_MAX_CODE_POINTS = 300
    const val TITLE_MAX_CODE_POINTS = 60

    /** `waitedMs` over 7 days is cut to 7 days. */
    const val WAITED_MAX_MILLIS = 604_800_000L

    /** The phone shows "· 3 h ago" only when the alert waited longer than 2 minutes. */
    const val WAITED_SHOWN_OVER_MILLIS = 120_000L

    /** Result of reading an `alert`: [Ok] with the message, or [Bad] (`bad_request`, with the reason). */
    sealed interface Parsed {
        data class Ok(val alert: AlertMessage) : Parsed
        data class Bad(val message: String) : Parsed
    }

    /**
     * Reads the fields of an `alert` frame (already a valid object with a valid `id`). `text` is required, a string, and must not be empty after the five cleaning steps of `link.title` (cut to
     * [TEXT_MAX_CODE_POINTS]); `title` is optional, a string, cleaned and cut to [TITLE_MAX_CODE_POINTS] (empty after cleaning is absent); `waitedMs` is an optional integer from 0, cut to 7 days.
     * `null` counts as absent, an unknown field is ignored (an alert never has buttons or priority), a wrong type is [Parsed.Bad].
     */
    fun parse(frame: JsonObject): Parsed {
        val rawText = Fields.string(frame["text"]) ?: return Parsed.Bad("An alert needs a text (a string).")
        val text = LinkWire.cleanText(rawText, TEXT_MAX_CODE_POINTS) ?: return Parsed.Bad("The alert's text is empty.")
        val title = when (val raw = Fields.present(frame["title"])) {
            null -> null
            else -> LinkWire.cleanText(Fields.string(raw) ?: return Parsed.Bad("An alert's title must be a string."), TITLE_MAX_CODE_POINTS)
        }
        val waited = when (val raw = Fields.present(frame["waitedMs"])) {
            null -> null
            else -> minOf(waitedMillis(raw) ?: return Parsed.Bad("waitedMs must be an integer from 0."), WAITED_MAX_MILLIS)
        }
        return Parsed.Ok(AlertMessage(text, title, waited))
    }

    /**
     * An integer from 0. One written with digits beyond 2^53-1 is still "a very long wait", which is cut to 7 days anyway, so it is cut rather than refused (2B.5 review; no vector says otherwise:
     * the largest one is exactly 2^53-1). A fraction, an exponent, a sign, a string or any other type is still refused.
     */
    private fun waitedMillis(raw: JsonElement): Long? {
        Fields.integer(raw)?.let { return it }
        val primitive = raw as? JsonPrimitive ?: return null
        if (primitive is JsonNull || primitive.isString) return null
        return if (BEYOND_SAFE.matches(primitive.content)) WAITED_MAX_MILLIS else null
    }

    private val BEYOND_SAFE = Regex("[1-9][0-9]{15,}")
}

/**
 * The phone's alert bucket (design 3.2, vectors `alertRate[]`): capacity 10, one token back per 6 s, starts full. A **new valid** alert takes one; a repeat of a known id and an invalid frame cost
 * nothing (the receiver asks only after its other checks), and a refused alert is not remembered. Per process, not per connection: a runaway script can't get more by reconnecting.
 */
class AlertBudget(clock: Clock) {
    private val bucket = MediaBucket(CAPACITY, REFILL_MILLIS, clock)

    fun tryTake(): Boolean = bucket.tryTake()

    companion object {
        const val CAPACITY = 10
        const val REFILL_MILLIS = 6_000L
    }
}
