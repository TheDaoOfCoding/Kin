package app.kinrelay.android.link

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** 2B.5 review round 2, B1: the link notification must not come back after the service is gone. */
class LinkShutdownTest {
    private val posts = mutableListOf<String>()
    private val feed = LinkNotificationFeed(intervalMillis = 30_000, now = { 0L }, schedule = { _, _ -> }, post = { posts += it })

    @Test
    fun theStatesReportedWhileStoppingAreNotPosted() {
        feed.submit("Connected to PC", "Connected to PC")
        val steps = mutableListOf<String>()
        LinkShutdown.run(
            feed = feed,
            // What the service's collector does when the runtime says Stopped (on its own thread, so it can run at any moment during the stop).
            stopRuntime = { steps += "stop"; feed.submit("Looking for PC", "Looking for PC") },
            markStopped = { steps += "marked"; feed.submit("Looking for PC again", "Looking for PC again") },
            removeNotification = { steps += "removed" },
        )
        assertEquals(listOf("stop", "marked", "removed"), steps)
        assertEquals(listOf("Connected to PC"), posts)
    }

    @Test
    fun theNotificationIsRemovedLastAndEvenWithoutAFeed() {
        val steps = mutableListOf<String>()
        LinkShutdown.run(feed = null, stopRuntime = { steps += "stop" }, markStopped = { steps += "marked" }, removeNotification = { steps += "removed" })
        assertEquals(listOf("stop", "marked", "removed"), steps)
    }

    @Test
    fun theFeedIsDestroyedBeforeTheRuntimeIsStopped() {
        var postedDuringStop = true
        LinkShutdown.run(
            feed = feed,
            stopRuntime = {
                val before = posts.size
                feed.submit("Looking for PC", "Looking for PC")
                postedDuringStop = posts.size != before
            },
            markStopped = {},
            removeNotification = {},
        )
        assertTrue(!postedDuringStop)
    }
}
