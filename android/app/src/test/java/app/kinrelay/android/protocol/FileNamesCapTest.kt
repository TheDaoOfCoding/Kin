package app.kinrelay.android.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The limit of the create-new loop (protocol.md, File names, 1.3 changelog 43): the plain name and `(1)` to `(99)`, 100 candidates, then `internal`. */
class FileNamesCapTest {
    @Test
    fun aSystemThatRefusesEveryNameIsGivenUpOnAfterHundredCandidates() {
        val tried = ArrayList<String>()
        val result = FileNames.createNew("a.txt", emptyList()) { name ->
            tried += name
            FileNames.Created.AlreadyExists
        }
        assertTrue("$result", result is FileNames.CreateResult.Failed)
        assertEquals("the plain name and (1) to (99)", 100, tried.size)
        assertEquals("a.txt", tried.first())
        assertEquals("a (99).txt", tried.last())
    }

    @Test
    fun suffixNinetyNineIsTheLastCandidate() {
        val existing = listOf("a.txt") + (1..98).map { "a ($it).txt" }
        val result = FileNames.createNew("a.txt", existing) { FileNames.Created.Name(it) }
        assertEquals(FileNames.CreateResult.Ok("a (99).txt"), result)
    }

    @Test
    fun whenTheFolderAlreadyHoldsEveryCandidateNothingIsCreated() {
        val existing = listOf("a.txt") + (1..99).map { "a ($it).txt" }
        var calls = 0
        val result = FileNames.createNew("a.txt", existing) { calls++; FileNames.Created.Name(it) }
        assertTrue("$result", result is FileNames.CreateResult.Failed)
        assertEquals(0, calls)
    }

    @Test
    fun candidatesSkippedBecauseTheyAreKnownCountLikeTheOnesTheCreateRefuses() {
        // 50 names are in `existing`, and the create refuses 50 more: 100 candidates in all, then the file fails.
        val existing = listOf("a.txt") + (1..49).map { "a ($it).txt" }
        var calls = 0
        val result = FileNames.createNew("a.txt", existing) { calls++; FileNames.Created.AlreadyExists }
        assertTrue("$result", result is FileNames.CreateResult.Failed)
        assertEquals(50, calls)
    }

    @Test
    fun anExtensionlessNameAndALongNameStopAtTheSameLimit() {
        var calls = 0
        val long = "x".repeat(190) + ".txt"
        for (name in listOf("notes", long)) {
            calls = 0
            val result = FileNames.createNew(name, emptyList()) { calls++; FileNames.Created.AlreadyExists }
            assertTrue("$name: $result", result is FileNames.CreateResult.Failed)
            assertEquals(name, 100, calls)
        }
    }

    @Test
    fun cleanItselfHasNoLimit() {
        // `clean` is the vectors' function: any number of taken names still gets a free one. Only `createNew` gives up.
        val existing = listOf("a.txt") + (1..150).map { "a ($it).txt" }
        assertEquals("a (151).txt", FileNames.clean("a.txt", existing))
    }
}
