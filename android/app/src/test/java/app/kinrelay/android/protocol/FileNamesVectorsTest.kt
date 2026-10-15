package app.kinrelay.android.protocol

import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.array
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Every case of `protocol/file-names.json`: the cleaning rules and the create-new (collision) cases. */
class FileNamesVectorsTest {
    private val vectors = Vectors.load("file-names.json")

    private fun names(case: JsonObject, key: String): List<String> = case.array(key).map { it.str() }

    @Test
    fun everyCleaningCase() {
        val cases = vectors.cases("cases")
        assertTrue("there are ${cases.size} cases", cases.size >= 200)
        for (case in cases) {
            val name = case.getValue("name").str()
            val actual = FileNames.clean(case.getValue("input").str(), names(case, "existing"))
            assertEquals("cases/$name", case.getValue("expected").str(), actual)
            assertTrue("cases/$name: at most ${FileNames.MAX_NAME_BYTES} bytes", Utf8.size(actual) <= FileNames.MAX_NAME_BYTES)
        }
    }

    /**
     * `createNew[]`: a create call that says "already exists" for the names in `onDisk` (case-insensitively, like NTFS). Both
     * `operation`s (`create`, and `move_part`: `name.ext.part` moved to its final name) follow the same rules, so the same loop answers
     * them: the phone's move of a downloaded file is a create of the final name. The 100-candidate limit cases are in both.
     */
    @Test
    fun everyCreateNewCase() {
        val cases = vectors.cases("createNew")
        assertTrue(cases.size >= 12)
        assertEquals(setOf("create", "move_part"), cases.map { it["operation"]?.jsonPrimitive?.content ?: "create" }.toSet())
        val limitCases = cases.filter { it.getValue("name").str().let { n -> "100-names" in n || "99th-suffix" in n } }
        assertEquals("the limit cases", 5, limitCases.size)
        for (case in cases) {
            val name = case.getValue("name").str()
            val onDisk = names(case, "onDisk").map { it.lowercase() }.toSet()
            val failure = case["createFails"]?.jsonPrimitive?.content
            val result = FileNames.createNew(case.getValue("input").str(), names(case, "existing")) { candidate ->
                when {
                    candidate.lowercase() in onDisk -> FileNames.Created.AlreadyExists
                    failure != null -> FileNames.Created.Failed(failure)
                    else -> FileNames.Created.Name(candidate)
                }
            }
            val expected = case.getValue("expected")
            if (expected is JsonObject) {
                assertEquals("createNew/$name: fails", "internal", expected.getValue("fails").jsonPrimitive.content)
                assertTrue("createNew/$name: $result", result is FileNames.CreateResult.Failed)
            } else {
                assertEquals("createNew/$name", FileNames.CreateResult.Ok(expected.jsonPrimitive.content), result)
            }
        }
    }

    @Test
    fun theCleanedNameKeepsTheRealExtensionAndNeverHasAPath() {
        assertEquals("c.txt", FileNames.clean("a/b\\c.txt", emptyList()))
        assertEquals("photo (1).jpg", FileNames.clean("photo.jpg", listOf("PHOTO.JPG")))
        assertEquals("file", FileNames.clean("", emptyList()))
    }
}
