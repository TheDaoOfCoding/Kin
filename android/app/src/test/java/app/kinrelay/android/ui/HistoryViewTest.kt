package app.kinrelay.android.ui

import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.ui.theme.LinkLook
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pure parts of the redesigned History and Home: the days, the filters, the picture of a row, the words under it, the faces. */
class HistoryViewTest {
    private val zone = ZoneId.of("UTC")
    private val now = ZonedDateTime.of(2026, 10, 26, 19, 0, 0, 0, zone).toInstant().toEpochMilli()

    private fun at(day: Int, hour: Int) = ZonedDateTime.of(2026, 10, day, hour, 0, 0, 0, zone).toInstant().toEpochMilli()

    private fun item(
        key: String, time: Long, outgoing: Boolean = true, status: HistoryStatus = HistoryStatus.DONE, isText: Boolean = false,
        title: String = "photo.jpg", size: Long = 1_200_000, mime: String? = null,
    ) = HistoryItem(
        key = key, id = key, isText = isText, outgoing = outgoing, status = status, timeMillis = time, title = title, sizeBytes = size,
        errorCode = null, errorMessage = null, peerName = "Desk PC", text = null, openUri = null, openMime = mime,
        canRetry = false, canCancel = false, retried = false,
    )

    @Test
    fun theListIsSplitIntoDaysInTheSameOrder() {
        val items = listOf(item("a", at(26, 18)), item("b", at(26, 9)), item("c", at(25, 21)), item("d", at(20, 12)), item("e", at(20, 8)))
        val groups = HistoryView.group(items, now, zone, Locale.UK)
        assertEquals(listOf("Today", "Yesterday", "Tue 20 Oct"), groups.map { it.label })
        assertEquals(listOf(listOf("a", "b"), listOf("c"), listOf("d", "e")), groups.map { g -> g.items.map { it.key } })
    }

    @Test
    fun noItemsMakeNoGroups() = assertTrue(HistoryView.group(emptyList(), now, zone).isEmpty())

    @Test
    fun eachFilterKeepsWhatItSays() {
        val sent = item("sent", 1)
        val received = item("received", 2, outgoing = false)
        val failedSend = item("failed", 3, status = HistoryStatus.FAILED)
        val expired = item("expired", 4, outgoing = false, status = HistoryStatus.EXPIRED)
        val waiting = item("waiting", 5, status = HistoryStatus.QUEUED)
        val all = listOf(sent, received, failedSend, expired, waiting)
        fun keys(f: HistoryFilter) = all.filter(f::matches).map { it.key }
        assertEquals(listOf("sent", "received", "failed", "expired", "waiting"), keys(HistoryFilter.ALL))
        assertEquals(listOf("sent", "waiting"), keys(HistoryFilter.SENT))
        assertEquals(listOf("received"), keys(HistoryFilter.RECEIVED))
        assertEquals(listOf("failed", "expired"), keys(HistoryFilter.FAILED))
    }

    @Test
    fun aRowPicturesATextAnImageOrAFile() {
        assertEquals(HistoryView.Kind.TEXT, HistoryView.kind(item("t", 1, isText = true, title = "hello")))
        assertEquals(HistoryView.Kind.IMAGE, HistoryView.kind(item("i", 1, mime = "image/png", title = "x")))
        assertEquals(HistoryView.Kind.IMAGE, HistoryView.kind(item("j", 1, title = "IMG_1842.JPG")))
        assertEquals(HistoryView.Kind.FILE, HistoryView.kind(item("f", 1, title = "lease.pdf")))
        assertEquals(HistoryView.Kind.FILE, HistoryView.kind(item("g", 1, title = "3 files: a.png, b.png", mime = "application/zip")))
    }

    @Test
    fun theLineUnderARowSaysWhatHappenedWhenAndHowBig() {
        assertEquals("Sent to Desk PC · 18:00 · 1.2 MB", HistoryText.line(item("a", at(26, 18)), now, zone))
        assertEquals("Received from Desk PC · 18:00 · 1.2 MB", HistoryText.line(item("b", at(26, 18), outgoing = false), now, zone))
        assertEquals("Sending… · 18:00 · 1.2 MB", HistoryText.line(item("c", at(26, 18), status = HistoryStatus.ACTIVE), now, zone))
        assertEquals("Failed · 18:00 · 1.2 MB", HistoryText.line(item("d", at(26, 18), status = HistoryStatus.FAILED), now, zone))
        // A text has no size worth showing.
        assertEquals("Sent to Desk PC · 18:00", HistoryText.line(item("e", at(26, 18), isText = true, size = 12), now, zone))
    }

    @Test
    fun theFacesFollowThePauseAndTheLink() {
        val connected = LinkState.Connected("Desk PC", PairingAddress("192.168.1.5", 4747))
        assertEquals(LinkLook.CONNECTED, linkLook(connected, paused = false))
        assertEquals(LinkLook.PAUSED, linkLook(connected, paused = true))
        assertEquals(LinkLook.CONNECTING, linkLook(LinkState.Connecting(), paused = false))
        assertEquals(LinkLook.CONNECTING, linkLook(LinkState.NotOnWifi, paused = false))
        assertEquals(LinkLook.PAUSED, linkLook(LinkState.PairAgain, paused = true))
    }

    @Test
    fun theStatePillIsShortAndOnlyTheStatesThatNeedASentenceHaveOne() {
        val connected = LinkState.Connected("Desk PC", PairingAddress("192.168.1.5", 4747))
        assertEquals("Connected on Wi-Fi", UiText.pill(connected))
        assertEquals("Looking for your PC…", UiText.pill(LinkState.Connecting()))
        assertNull(UiText.detail(connected))
        assertNull(UiText.detail(LinkState.NotOnWifi))
        assertEquals("Next try in 5 s", UiText.detail(LinkState.Connecting(5_000)))
        assertEquals(UiText.status(LinkState.PairAgain), UiText.detail(LinkState.PairAgain))
        val states = listOf(connected, LinkState.Connecting(), LinkState.NotOnWifi, LinkState.Stopped, LinkState.Replaced, LinkState.UpdateApp, LinkState.PairAgain, LinkState.PermissionNeeded)
        for (state in states) assertTrue("$state: ${UiText.pill(state)}", UiText.pill(state).length <= 22)
    }

    @Test
    fun noUserVisibleStringOfTheNewScreensHasAnEmDash() {
        val dir = java.io.File("src/main/java/app/kinrelay/android/ui")
        val files = listOf("HomeScreen", "HistoryScreen", "OnboardingScreen", "SettingsScreen", "KinComponents", "HistoryView", "DebugScreen", "UiText", "MediaScreens", "MirrorScreens", "PresenceScreens", "DndScreens")
        for (name in files) {
            // A dash in a comment is fine; one in a string literal reaches the user.
            val strings = java.io.File(dir, "$name.kt").readLines().filterNot { it.trim().startsWith("//") || it.trim().startsWith("*") || it.trim().startsWith("/**") }
            for (line in strings) assertFalse("$name: $line", Regex("\"[^\"]*—[^\"]*\"").containsMatchIn(line))
        }
    }
}
