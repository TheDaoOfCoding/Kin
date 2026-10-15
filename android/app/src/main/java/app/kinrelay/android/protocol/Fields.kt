package app.kinrelay.android.protocol

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Typed reads from a parsed [JsonObject], following the JSON conventions of protocol.md.
 * Each returns null when the field is missing, `null`, or of the wrong type.
 * Never call `.long` or `.boolean` on a primitive directly: kotlinx coerces quoted values.
 */
object Fields {
    const val MAX_SAFE_INTEGER = 9_007_199_254_740_991L
    private val INTEGER = Regex("0|[1-9][0-9]*")

    fun string(element: JsonElement?): String? =
        (element as? JsonPrimitive)?.takeIf { it.isString && it !is JsonNull }?.content

    fun nonEmptyString(element: JsonElement?): String? = string(element)?.takeIf { it.isNotEmpty() }

    /** An ID or `ref`: 1 to 64 Unicode code points. */
    fun id(element: JsonElement?): String? =
        string(element)?.takeIf { it.isNotEmpty() && it.codePointCount(0, it.length) <= 64 }

    private val PATH_ID = Regex("[A-Za-z0-9_-]{1,64}")

    /** A transfer ID or file ID: it goes in a URL path, so only `A-Z a-z 0-9 - _`, 1 to 64 characters. */
    fun isPathId(value: String): Boolean = PATH_ID.matches(value)

    /** A JSON integer: no quotes, sign, fraction or exponent, at most 2^53-1. */
    fun integer(element: JsonElement?): Long? {
        val primitive = element as? JsonPrimitive ?: return null
        if (primitive is JsonNull || primitive.isString) return null
        val text = primitive.content
        if (!INTEGER.matches(text) || text.length > 16) return null
        return text.toLong().takeIf { it <= MAX_SAFE_INTEGER }
    }

    fun boolean(element: JsonElement?): Boolean? {
        val primitive = element as? JsonPrimitive ?: return null
        if (primitive is JsonNull || primitive.isString) return null
        return when (primitive.content) {
            "true" -> true
            "false" -> false
            else -> null
        }
    }

    /** Present and not `null`. An explicit `null` counts as absent. */
    fun present(element: JsonElement?): JsonElement? = element?.takeIf { it !is JsonNull }
}
