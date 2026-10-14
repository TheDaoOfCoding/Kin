package app.kinrelay.android.testing

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** Loads the shared vectors of `protocol/` (a Gradle test resource directory). */
object Vectors {
    fun load(file: String): JsonObject {
        val text = javaClass.classLoader!!.getResource(file)!!.readText(Charsets.UTF_8)
        return Json.parseToJsonElement(text).jsonObject
    }

    /** `*` at the end of a pattern means "every name that starts with this" (protocol/README.md). */
    fun inScope(name: String, patterns: List<String>): Boolean =
        patterns.any { if (it.endsWith("*")) name.startsWith(it.dropLast(1)) else name == it }

    fun hex(text: String): ByteArray = ByteArray(text.length / 2) { text.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
}

fun JsonElement.str(): String = jsonPrimitive.content

fun JsonObject.cases(name: String): List<JsonObject> = getValue(name).jsonArray.map { it.jsonObject }

fun JsonObject.string(name: String): String? = (this[name] as? JsonPrimitive)?.takeIf { it.isString }?.content

fun JsonObject.array(name: String): JsonArray = getValue(name).jsonArray
