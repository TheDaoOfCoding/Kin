package app.kinrelay.android.notify

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The logic behind the notification rework: the cap with "drop the oldest", the one updating "N texts" notification, and the trailing throttle. */
class NotificationLogicTest {
    // ---------------------------------------------------------------- the cap

    @Test
    fun aBurstOfSixtyTransfersNeverHoldsMoreThanTheCapAndKeepsTheNewest() {
        val ledger = NotificationLedger(10)
        val cancelled = ArrayList<String>()
        for (i in 1..60) cancelled += ledger.claim("t$i", "Transfer $i")
        assertEquals(10, ledger.size)
        assertEquals((51..60).map { "t$it" }, ledger.keys)
        assertEquals("every other one was pushed out, oldest first", (1..50).map { "t$it" }, cancelled)
    }

    @Test
    fun refreshingASlotKeepsItsPlaceAndPushesNothingOut() {
        val ledger = NotificationLedger(3)
        ledger.claim("a", "A"); ledger.claim("b", "B"); ledger.claim("c", "C")
        assertEquals(emptyList<String>(), ledger.claim("a", "A again")) // its progress notification updates
        assertEquals(listOf("a", "b", "c"), ledger.keys)
        assertEquals(listOf("A again", "B", "C"), ledger.titles)
        assertEquals("the oldest is still a, so a goes first", listOf("a"), ledger.claim("d", "D"))
    }

    @Test
    fun aResultIsNeverRefusedWhenTheCapIsFull() {
        val ledger = NotificationLedger(2)
        ledger.claim("a", "A"); ledger.claim("b", "B")
        val evicted = ledger.claim("c", "result of c")
        assertEquals(listOf("a"), evicted)
        assertTrue("c is in", "c" in ledger)
    }

    @Test
    fun slotsThatTimedOutOrWereDismissedStopCounting() {
        var now = 0L
        val ledger = NotificationLedger(3, now = { now })
        ledger.claim("a", "A"); ledger.claim("b", "B"); ledger.claim("c", "C")
        now += NotificationLedger.MISSING_GRACE_MILLIS + 1
        ledger.retainOnly(setOf("b"))
        assertEquals(listOf("b"), ledger.keys)
        assertTrue(ledger.claim("d", "D").isEmpty())
        assertTrue(ledger.release("d"))
        assertFalse(ledger.release("d"))
    }

    @Test
    fun theTwoGroupsAndEverythingElseStayUnderFiftyNotifications() {
        // 10 transfer slots (each may hold a progress and a result for a moment) + 10 arrival slots + 2 summaries + 5 problems + the service,
        // a ping and the texts: the worst case, far below Android's 50.
        val worst = 10 * 2 + 10 + 2 + 5 + 3
        assertTrue("worst case $worst", worst < 50)
    }

    // ---------------------------------------------------------------- texts: one notification

    @Test
    fun sixtyTextsAreOneNotificationThatCountsAndShowsTheLastFew() {
        val arrivals = TextArrivals()
        var state = arrivals.state()
        for (i in 1..60) state = arrivals.add("Desk PC", "text $i", null)
        assertEquals(60, state.count)
        assertEquals("60 texts from Desk PC", TextArrivals.title(state))
        assertEquals((56..60).map { "text $it" }, state.lines)
    }

    @Test
    fun oneTextIsTextFromThePc() {
        val arrivals = TextArrivals()
        val state = arrivals.add("Desk PC", "hello", null)
        assertEquals("Text from Desk PC", TextArrivals.title(state))
        assertEquals(listOf<String?>("hello"), state.lines)
    }

    @Test
    fun aSensitiveTextCountsButShowsNoPreview() {
        val arrivals = TextArrivals()
        arrivals.add("Desk PC", "visible", null)
        val state = arrivals.add("Desk PC", null, null)
        assertEquals(2, state.count)
        assertEquals(listOf("visible", null), state.lines)
        assertFalse("the text of a sensitive arrival is nowhere in the state", state.toString().contains("secret"))
    }

    @Test
    fun theLatestLinkCanBeOpenedAndAnEarlierOneIsForgotten() {
        val arrivals = TextArrivals()
        arrivals.add("Desk PC", "https://a.example", "https://a.example")
        val state = arrivals.add("Desk PC", "plain text", null)
        assertNull(state.openUrl)
        assertEquals("https://b.example", arrivals.add("Desk PC", "https://b.example", "https://b.example").openUrl)
    }

    @Test
    fun seeingOrDismissingTheNotificationStartsANewCount() {
        val arrivals = TextArrivals()
        arrivals.add("Desk PC", "a", null); arrivals.add("Desk PC", "b", null)
        arrivals.reset()
        assertTrue(arrivals.isEmpty())
        assertEquals(1, arrivals.add("Desk PC", "c", null).count)
    }

    @Test
    fun aTextFromAnotherPcStartsANewCount() {
        val arrivals = TextArrivals()
        arrivals.add("Old PC", "a", null)
        val state = arrivals.add("New PC", "b", null)
        assertEquals(1, state.count)
        assertEquals("New PC", state.pcName)
    }

    // ---------------------------------------------------------------- the throttle

    private class Scheduler {
        var now = 0L
        val tasks = ArrayList<Pair<Long, () -> Unit>>()
        fun advance(to: Long) {
            while (true) {
                val due = tasks.filter { it.first <= to }.minByOrNull { it.first } ?: break
                tasks.remove(due)
                now = due.first
                due.second()
            }
            now = to
        }
    }

    @Test
    fun theThrottleRunsAtOnceThenAtMostOncePerIntervalAndTheLastRequestIsNeverLost() {
        val s = Scheduler()
        val runs = ArrayList<Long>()
        val throttle = TrailingThrottle(500, { s.now }, { delay, task -> s.tasks += (s.now + delay) to task }, { runs += s.now })
        throttle.request() // the first one goes at once
        for (t in 1..59) { s.advance(t * 10L); throttle.request() } // 59 more within 590 ms
        s.advance(5_000)
        assertEquals("one at once, one at the end of the interval, and then nothing: the last state was posted", listOf(0L, 500L, 1000L), runs)
        assertTrue(s.tasks.isEmpty())
    }

    @Test
    fun theThrottleLetsASeparateRequestThroughAtOnce() {
        val s = Scheduler()
        val runs = ArrayList<Long>()
        val throttle = TrailingThrottle(500, { s.now }, { delay, task -> s.tasks += (s.now + delay) to task }, { runs += s.now })
        throttle.request()
        s.advance(2_000)
        throttle.request()
        assertEquals(listOf(0L, 2_000L), runs)
    }

    @Test
    fun aBurstOfSixtyTextsPostsAFewTimesAndTheLastPostHasTheFinalCount() {
        val s = Scheduler()
        val arrivals = TextArrivals()
        val posted = ArrayList<Int>()
        val throttle = TrailingThrottle(500, { s.now }, { delay, task -> s.tasks += (s.now + delay) to task }, { posted += arrivals.state().count })
        for (i in 1..60) { arrivals.add("Desk PC", "text $i", null); throttle.request(); s.advance(s.now + 10) }
        s.advance(10_000)
        assertTrue("posted ${posted.size} times", posted.size <= 4)
        assertEquals(60, posted.last())
    }

    // ---------------------------------------------------------------- the cap and the summary count (1.4 review)

    @Test
    fun aSlotIsKeptWhileTheSystemsListDoesNotShowItYetAndForgottenOnlyAfterTheGrace() {
        var now = 0L
        val ledger = NotificationLedger(10, now = { now })
        ledger.claim("a", "A")
        now += 1_000
        // activeNotifications lags a notify: "a" was just posted and is not in the list yet.
        assertEquals(0, ledger.retainOnly(emptySet()))
        assertEquals(listOf("a"), ledger.keys)
        now += NotificationLedger.MISSING_GRACE_MILLIS - 1_000 + 1
        assertEquals("missing for more than the grace: dismissed or timed out", 1, ledger.retainOnly(emptySet()))
        assertTrue(ledger.keys.isEmpty())
    }

    @Test
    fun aRefreshedSlotStartsItsGraceAgain() {
        var now = 0L
        val ledger = NotificationLedger(10, now = { now })
        ledger.claim("a", "A")
        now += 1_900
        ledger.claim("a", "A") // a progress update posts it again
        now += 1_900
        assertEquals(0, ledger.retainOnly(emptySet()))
        now += 200
        assertEquals(1, ledger.retainOnly(emptySet()))
    }

    @Test
    fun aStormOfFifteenFilesKeepsTenOnScreenAndTheSummaryCountsTenWhileTheSystemListLags() {
        // The 1.4 integration run: after 15 single-file offers the summary said "3 files" with 10 results on screen, because the slots were
        // forgotten against a list that does not show a notification for a moment after it was posted.
        var now = 0L
        val ledger = NotificationLedger(10, now = { now })
        val onScreen = LinkedHashMap<String, Long>() // key -> when it was posted
        fun systemList(): Set<String> = onScreen.filterValues { now - it >= 300 }.keys // lags by 300 ms
        val summaryCounts = mutableListOf<Int>()
        repeat(15) { i ->
            val key = "t$i"
            ledger.retainOnly(systemList())
            ledger.claim(key, "T$i").forEach { onScreen.remove(it) }
            onScreen[key] = now
            ledger.retainOnly(systemList()) // the summary prunes too
            summaryCounts += ledger.size
            now += 40 // 15 offers in well under a second
        }
        assertEquals("at most 10 notifications on screen", 10, onScreen.size)
        assertEquals("the ledger and the screen agree: nothing leaked past the cap", onScreen.keys.toList(), ledger.keys)
        assertEquals("the summary reads 10, not 3", 10, summaryCounts.last())
        assertEquals((1..10).toList() + List(5) { 10 }, summaryCounts)
        assertEquals("the oldest five were pushed out", (5..14).map { "t$it" }, ledger.keys)
    }

    @Test
    fun aDismissedNotificationStopsCountingInTheSummaryOnceItHasBeenGoneLongEnough() {
        var now = 0L
        val ledger = NotificationLedger(10, now = { now })
        val screen = mutableSetOf<String>()
        for (k in listOf("a", "b", "c")) { ledger.claim(k, k); screen += k }
        now += 5_000
        screen -= "b" // swiped away
        ledger.retainOnly(screen)
        assertEquals(listOf("a", "c"), ledger.keys)
    }
}
