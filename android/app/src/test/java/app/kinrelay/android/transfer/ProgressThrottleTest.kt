package app.kinrelay.android.transfer

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProgressThrottleTest {
    @Test
    fun theFirstUpdateGoesOutThenOnePerInterval() {
        val t = ProgressThrottle(500)
        assertTrue(t.tryReport(1_000))
        assertFalse(t.tryReport(1_100))
        assertFalse(t.tryReport(1_499))
        assertTrue(t.tryReport(1_500))
        assertFalse(t.tryReport(1_501))
        assertTrue(t.tryReport(2_000))
    }

    @Test
    fun aThousandCallsInASecondGiveTwoUpdates() {
        val t = ProgressThrottle()
        val sent = (0 until 1_000).count { t.tryReport(10_000L + it) }
        assertEquals("one at 0 ms and one at 500 ms", 2, sent)
    }

    @Test
    fun aClockThatStepsBackDoesNotSilenceUpdates() {
        val t = ProgressThrottle(500)
        assertTrue(t.tryReport(10_000))
        assertTrue("the clock went back an hour: that is not 'just now'", t.tryReport(10_000 - 3_600_000))
        assertFalse(t.tryReport(10_000 - 3_600_000 + 10))
    }

    @Test
    fun updatesFromManyThreadsNeverBeatTheLimit() {
        val t = ProgressThrottle(500)
        val sent = java.util.concurrent.atomic.AtomicInteger()
        val threads = (1..8).map { Thread { repeat(10_000) { if (t.tryReport(5_000)) sent.incrementAndGet() } } }
        threads.forEach { it.start() }
        threads.forEach { it.join() }
        assertEquals("all at the same instant: exactly one", 1, sent.get())
    }
}
