package app.kinrelay.android.pairing

import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingUrlVectorsTest {
    private val vectors = Vectors.load("pairing-urls.json")

    @Test
    fun validUrlsParseToTheExpectedTarget() {
        val cases = vectors.cases("valid")
        assertTrue(cases.size >= 30)
        for (case in cases) {
            val name = case.getValue("name").str()
            val expected = case.getValue("parsed").jsonObject
            val result = PairingUrl.parse(case.getValue("url").str())
            assertTrue("$name: parses (got $result)", result is PairingParse.Ok)
            val target = (result as PairingParse.Ok).target
            assertEquals("$name: version", expected.getValue("version").jsonPrimitive.int, target.version)
            assertEquals("$name: addresses",
                expected.getValue("addresses").jsonArray.map { it.jsonObject.getValue("host").str() to it.jsonObject.getValue("port").jsonPrimitive.int },
                target.addresses.map { it.host to it.port })
            assertEquals("$name: id", expected.getValue("id").str(), target.id)
            assertEquals("$name: token", expected.getValue("token").str(), target.token)
            val expectedName = expected.getValue("name").let { if (it is JsonNull) null else it.str() }
            assertEquals("$name: name", expectedName, target.name)
        }
    }

    @Test
    fun invalidUrlsAreRejectedWithTheExpectedCategoryAndParameter() {
        val cases = vectors.cases("invalid")
        assertTrue(cases.size >= 80)
        for (case in cases) {
            val name = case.getValue("name").str()
            val expected = case.getValue("error").jsonObject
            val result = PairingUrl.parse(case.getValue("url").str())
            assertTrue("$name: is rejected (got $result)", result is PairingParse.Error)
            val error = result as PairingParse.Error
            assertEquals("$name: category", expected.getValue("category").str(), error.category.name.lowercase())
            assertEquals("$name: param", expected["param"]?.str(), error.param)
        }
    }

    @Test
    fun theDeepLinkTheEndToEndScriptsUseParses() {
        val id = "yetOwJHO-3liWauefLv2Zs8_NNIkRYvy7_i5RBG-fP8"
        val result = PairingUrl.parse("kinrelay://pair?v=1&a=10.0.2.2:47821&id=$id&tk=AAECAwQFBgcICQoLDA0ODw&n=Desk%20PC")
        assertEquals(PairingAddress("10.0.2.2", 47821), ((result as PairingParse.Ok).target.addresses.single()))
        assertEquals("Desk PC", result.target.name)
    }
}
