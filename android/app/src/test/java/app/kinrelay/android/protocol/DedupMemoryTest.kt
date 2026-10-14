package app.kinrelay.android.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class DedupMemoryTest {
    private val hour = 3_600_000L

    @Test
    fun remembersTheFirstResponse() {
        val memory = DedupMemory({ 0L })
        assertNull(memory.lookup("a"))
        memory.record("a", "ack-a")
        assertEquals("ack-a", memory.lookup("a"))
    }

    @Test
    fun neverForgetsWithinTheNewest1000EvenWhenOld() {
        var now = 0L
        val memory = DedupMemory({ now })
        repeat(1000) { memory.record("id$it", "r") }
        now = 48 * hour
        memory.record("fresh", "r") // 1001 entries now; the oldest is past 24 h, so exactly one goes
        assertEquals(1000, memory.size)
        assertNull(memory.lookup("id0"))
        assertNotNull(memory.lookup("id1"))
    }

    @Test
    fun neverForgetsWithin24HoursEvenBeyond1000() {
        var now = 0L
        val memory = DedupMemory({ now })
        repeat(1500) { memory.record("id$it", "r"); now += 1000 }
        assertEquals(1500, memory.size)
        assertNotNull(memory.lookup("id0"))
    }

    @Test
    fun forgetsOnlyWhenBothOldAndBeyond1000() {
        var now = 0L
        val memory = DedupMemory({ now })
        repeat(1200) { memory.record("id$it", "r") }
        now = 25 * hour
        memory.record("x", "r")
        assertEquals(1000, memory.size)
        assertNull(memory.lookup("id0"))
        assertNotNull(memory.lookup("id201"))
        assertNotNull(memory.lookup("x"))
    }

    @Test
    fun reportsAdditionsAndRemovalsForPersistence() {
        var now = 0L
        val added = mutableListOf<String>()
        val removed = mutableListOf<String>()
        val memory = DedupMemory({ now }, onChange = { a, r -> a?.let { added += it.id }; removed += r })
        repeat(1001) { memory.record("id$it", "r") }
        assertEquals(1001, added.size)
        assertEquals(emptyList<String>(), removed)
        now = 30 * hour
        memory.record("late", "r")
        assertEquals(listOf("id0", "id1"), removed)
    }

    @Test
    fun startsFromPersistedEntries() {
        val memory = DedupMemory({ 10L }, listOf(DedupMemory.Entry("old", "ack-old", 1)))
        assertEquals("ack-old", memory.lookup("old"))
    }
}
