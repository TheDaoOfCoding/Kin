package app.kinrelay.android.link

import org.junit.Assert.assertEquals
import org.junit.Test

class LinkNotificationFeedTest {
    private var clock = 1_000_000L
    private val queued = mutableListOf<Pair<Long, () -> Unit>>()
    private val posts = mutableListOf<String>()
    private val feed = LinkNotificationFeed(
        intervalMillis = 30_000,
        now = { clock },
        schedule = { delay, task -> queued += delay to task },
        post = { posts += it },
    )

    @Test
    fun aChangeOfStateIsPostedAtOnce() {
        feed.submit("Connected", "Connected · 80%")
        feed.submit("Looking", "Looking")
        assertEquals(listOf("Connected · 80%", "Looking"), posts)
    }

    @Test
    fun aBatteryOnlyChangeWaitsForTheThrottle() {
        feed.submit("Connected", "Connected · 80%")
        feed.submit("Connected", "Connected · 79%") // first throttle request: due now (nothing posted before it)
        feed.submit("Connected", "Connected · 78%") // within the interval: queued
        assertEquals(2, posts.size)
        assertEquals(1, queued.size)
        queued.single().second()
        assertEquals("Connected · 78%", posts.last())
    }

    @Test
    fun aQueuedCallbackDoesNothingAfterDestroy() {
        feed.submit("Connected", "Connected · 80%")
        feed.submit("Connected", "Connected · 79%")
        feed.submit("Connected", "Connected · 78%")
        val before = posts.size
        assertEquals(1, queued.size)
        feed.destroy()
        // The handler's queue was cleared in the service, but a callback that was already running or posted late must still be harmless.
        queued.single().second()
        assertEquals(before, posts.size)
    }

    @Test
    fun nothingIsPostedOrQueuedAfterDestroy() {
        feed.submit("Connected", "Connected · 80%")
        feed.destroy()
        feed.submit("Looking", "Looking")
        feed.submit("Connected", "Connected · 50%")
        assertEquals(listOf("Connected · 80%"), posts)
        assertEquals(0, queued.size)
    }

    @Test
    fun aThrottleDueAfterDestroyDoesNotSchedule() {
        feed.submit("Connected", "Connected · 80%")
        feed.submit("Connected", "Connected · 79%")
        feed.destroy()
        feed.submit("Connected", "Connected · 78%")
        assertEquals(0, queued.size)
    }

    @Test
    fun anUnchangedTextIsNotPostedAgain() {
        feed.submit("Connected", "Connected")
        feed.submit("Connected", "Connected")
        assertEquals(1, posts.size)
    }
}
