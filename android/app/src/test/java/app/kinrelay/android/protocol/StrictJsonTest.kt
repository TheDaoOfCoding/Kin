package app.kinrelay.android.protocol

import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class StrictJsonTest {
    private val messages = Vectors.load("messages.json")

    private fun rejects(text: String) = try {
        StrictJson.parse(text)
        false
    } catch (_: StrictJson.InvalidJson) {
        true
    }

    private fun rawCases(prefix: String, section: String) =
        messages.cases(section).filter { it.getValue("name").str().startsWith(prefix) && it["raw"] != null }

    @Test
    fun theAppDecoderRejectsEveryStrictAndNestingVector() {
        val cases = rawCases("strict-", "invalid") + rawCases("nesting-", "invalid")
        assertTrue("found ${cases.size} cases", cases.size >= 27)
        for (case in cases) {
            assertTrue("${case.getValue("name").str()} must be rejected", rejects(case.getValue("raw").str()))
        }
    }

    @Test
    fun theAppDecoderAcceptsDepth64AndBracketsInStrings() {
        val cases = rawCases("nesting-depth-64", "valid") + rawCases("brackets-inside-strings", "valid")
        assertTrue(cases.size >= 3)
        for (case in cases) {
            assertFalse("${case.getValue("name").str()} must be accepted", rejects(case.getValue("raw").str()))
        }
    }

    @Test
    fun lonesurrogateEscapesAreRejectedAndPairsAreNot() {
        val cases = messages.cases("invalid").filter { val n = it.getValue("name").str(); n.startsWith("text-lone-") || n.startsWith("lone-surrogate") || n.startsWith("text-high-surrogate") || n.startsWith("text-reversed") }
        assertTrue(cases.size >= 6)
        for (case in cases) assertTrue(case.getValue("name").str(), rejects(case.getValue("raw").str()))
        assertFalse(rejects("{\"a\":\"\\ud83d\\ude00\"}"))
    }

    @Test
    fun duplicateKeysTheLastValueWins() {
        val tree = StrictJson.parse("""{"type":"ping","id":"first","type":"text","id":"last"}""").jsonObject
        assertEquals("text", tree.getValue("type").jsonPrimitive.content)
        assertEquals("last", tree.getValue("id").jsonPrimitive.content)
    }

    @Test
    fun otherRfc8259ViolationsAreRejected() {
        val bad = listOf(
            "", " ", "{", "}", "{\"a\"}", "{\"a\":}", "{\"a\":1,}", "[1,]", "[,1]", "{,}", "{\"a\":1 \"b\":2}",
            "{\"a\":01}", "{\"a\":-}", "{\"a\":1e}", "{\"a\":1.e1}", "{\"a\":--1}", "{\"a\":+1}", "{\"a\":.5}", "{\"a\":1.}",
            "{\"a\":0x10}", "{\"a\":tru}", "{\"a\":truee}", "{\"a\":True}", "{\"a\":NULL}", "{\"a\":'x'}", "{a:1}",
            "{\"a\":\"\\x\"}", "{\"a\":\"\\u12\"}", "{\"a\":\"line\nbreak\"}", "{\"a\":\"tab\there\"}", "{\"a\":1}{\"b\":2}",
            "{\"a\":1} x", "\u00A0{}", "\uFEFF{}", "{\"a\":1}//c", "/*c*/{}", "{\"a\":[1,2}", "[}", "{\"a\":\"unterminated}",
        )
        for (text in bad) assertTrue("must reject: ${text.replace("\n", "\\n")}", rejects(text))
    }

    @Test
    fun validRfc8259TextIsAccepted() {
        val good = listOf(
            "{}", "[]", " \t\r\n{ } ", "{\"a\":0}", "{\"a\":-0}", "{\"a\":1.5e+10}", "{\"a\":1E-2}", "{\"a\":-12.0}",
            "{\"a\":true,\"b\":false,\"c\":null}", "{\"a\":\"\\\" \\\\ \\/ \\b \\f \\n \\r \\t \\u00e9\"}",
            "\"just a string\"", "123", "[[[]]]", "{\"a\":{\"b\":[1,{\"c\":2}]}}", "{\"emoji\":\"😀\"}",
        )
        for (text in good) assertFalse("must accept: $text", rejects(text))
    }

    @Test
    fun deepNestingIsRejectedWithoutOverflowingTheStack() {
        assertTrue(rejects("[".repeat(100_000)))
        assertTrue(rejects("{\"a\":".repeat(100_000)))
        assertFalse(rejects("[".repeat(64) + "]".repeat(64)))
        assertTrue(rejects("[".repeat(65) + "]".repeat(65)))
    }

    // --- field access ---

    private fun obj(text: String): JsonObject = StrictJson.parse(text).jsonObject

    @Test
    fun integersAreReadFromTheTextNotFromKotlinxCoercion() {
        val tree = obj("""{"ok":42,"zero":0,"max":9007199254740991,"over":9007199254740992,"neg":-1,"frac":1.0,"exp":1e3,"quoted":"10","big":99999999999999999999,"bool":true,"nul":null}""")
        assertEquals(42L, Fields.integer(tree["ok"]))
        assertEquals(0L, Fields.integer(tree["zero"]))
        assertEquals(9_007_199_254_740_991L, Fields.integer(tree["max"]))
        for (key in listOf("over", "neg", "frac", "exp", "quoted", "big", "bool", "nul", "missing")) assertNull(key, Fields.integer(tree[key]))
    }

    @Test
    fun booleansAndStringsAreStrict() {
        val tree = obj("""{"t":true,"f":false,"qt":"true","one":1,"s":"x","n":5,"nul":null}""")
        assertEquals(true, Fields.boolean(tree["t"]))
        assertEquals(false, Fields.boolean(tree["f"]))
        for (key in listOf("qt", "one", "s", "nul", "missing")) assertNull(key, Fields.boolean(tree[key]))
        assertEquals("x", Fields.string(tree["s"]))
        for (key in listOf("n", "t", "nul", "missing")) assertNull(key, Fields.string(tree[key]))
        assertEquals("", Fields.string(JsonPrimitive("")))
        assertNull(Fields.nonEmptyString(JsonPrimitive("")))
    }

    @Test
    fun idsAreOneToSixtyFourCodePoints() {
        assertEquals("a", Fields.id(JsonPrimitive("a")))
        assertNull(Fields.id(JsonPrimitive("")))
        assertEquals(64, Fields.id(JsonPrimitive("x".repeat(64)))?.length)
        assertNull(Fields.id(JsonPrimitive("x".repeat(65))))
        val astral = "\uD83D\uDE00".repeat(64)
        assertEquals(astral, Fields.id(JsonPrimitive(astral)))
        assertNull(Fields.id(JsonPrimitive(astral + "a")))
    }

    /** Not an assertion: prints which vectors kotlinx accepts on its own (recorded in the results file). */
    @Test
    fun measureWhichVectorsKotlinxAloneAccepts() {
        val lenient = Json { isLenient = false }
        val cases = rawCases("strict-", "invalid") + rawCases("nesting-", "invalid") + messages.cases("invalid").filter { it["raw"] != null && it.getValue("name").str().let { n -> n.startsWith("lone-") || n.startsWith("text-lone") || n.startsWith("text-high") || n.startsWith("text-reversed") } }
        val accepted = cases.filter { runCatching { lenient.parseToJsonElement(it.getValue("raw").str()) }.isSuccess }.map { it.getValue("name").str() }
        println("KOTLINX_ALONE_ACCEPTS ${accepted.size}/${cases.size}: $accepted")
        println("KOTLINX_ALONE_REJECTS: ${(cases.map { it.getValue("name").str() } - accepted.toSet())}")
    }

    @Test
    fun kotlinxAloneWouldHaveAcceptedWhatTheValidatorRejects() {
        // Documents why the raw text is validated first (see the results file for the measured list).
        val lenientAccepts = listOf("""{"a":yes}""", """{"a":01}""", """{"a":NaN}""", """{"a":+1}""")
            .filter { runCatching { Json.parseToJsonElement(it) }.isSuccess }
        assertTrue("kotlinx accepted: $lenientAccepts", lenientAccepts.isNotEmpty())
        for (text in lenientAccepts) assertTrue(text, rejects(text))
    }
}
