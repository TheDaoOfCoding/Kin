package app.kinrelay.android.ui

import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus
import app.kinrelay.android.history.HistoryType
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
        names: List<String> = emptyList(), types: Set<HistoryType> = emptySet(), text: String? = null, isLink: Boolean = false,
    ) = HistoryItem(
        key = key, id = key, isText = isText, outgoing = outgoing, status = status, timeMillis = time, title = title, sizeBytes = size,
        errorCode = null, errorMessage = null, peerName = "Desk PC", text = text, openUri = null, openMime = mime,
        canRetry = false, canCancel = false, retried = false, names = names, types = types, isLink = isLink,
    )

    /** A file transfer with the given names and types. */
    private fun files(key: String, time: Long, vararg names: String, types: Set<HistoryType>, outgoing: Boolean = true, size: Long = 100, status: HistoryStatus = HistoryStatus.DONE) =
        item(key, time, outgoing = outgoing, status = status, title = names.first(), size = size, names = names.toList(), types = types)

    private fun textRow(key: String, time: Long, title: String, body: String = title, outgoing: Boolean = true, size: Long = 12) =
        item(key, time, outgoing = outgoing, isText = true, title = title, size = size, text = body, types = setOf(HistoryType.TEXT))

    private fun linkRow(key: String, time: Long, title: String, outgoing: Boolean = true) =
        item(key, time, outgoing = outgoing, isText = true, title = title, size = 40, text = "https://example.com/$key", types = setOf(HistoryType.LINK), isLink = true)

    private val photo = setOf(HistoryType.PHOTO)
    private val pdf = setOf(HistoryType.DOCUMENT)

    private fun keys(items: List<HistoryItem>, query: HistoryQuery) = HistoryView.apply(items, query, Locale.UK).map { it.key }

    /** A mixed history, newest first: a photo, a pdf, a mixed transfer, a video, an apk, a text, a link, and a failed and a waiting send. */
    private val mixed = listOf(
        files("photo", 90, "IMG_1.JPG", types = photo, size = 500),
        files("pdf", 80, "Report.pdf", types = pdf, outgoing = false, size = 900),
        files("mixed", 70, "b.png", "z.pdf", "c.zip", types = setOf(HistoryType.PHOTO, HistoryType.DOCUMENT, HistoryType.OTHER), size = 300),
        files("video", 60, "clip.MOV", types = setOf(HistoryType.VIDEO), outgoing = false, size = 5_000),
        files("apk", 50, "kin.apk", types = setOf(HistoryType.APP), size = 20),
        textRow("text", 40, "hello world", outgoing = false),
        linkRow("link", 30, "Kin site"),
        files("failed", 20, "lost.png", types = photo, status = HistoryStatus.FAILED, size = 50),
        files("waiting", 10, "later.pdf", types = pdf, status = HistoryStatus.QUEUED, size = 70),
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
    fun theDefaultQueryShowsEverythingNewestFirst() {
        assertTrue(HistoryQuery().isDefault)
        assertEquals(mixed.map { it.key }, keys(mixed.shuffled(java.util.Random(7)), HistoryQuery()))
    }

    @Test
    fun eachTypeKeepsTheItemsWithAFileOrTextOfThatType() {
        fun of(type: HistoryType) = keys(mixed, HistoryQuery(types = setOf(type)))
        assertEquals(listOf("photo", "mixed", "failed"), of(HistoryType.PHOTO))
        assertEquals(listOf("video"), of(HistoryType.VIDEO))
        assertEquals(listOf("pdf", "mixed", "waiting"), of(HistoryType.DOCUMENT))
        assertEquals(listOf("apk"), of(HistoryType.APP))
        assertEquals(listOf("mixed"), of(HistoryType.OTHER))
        assertEquals(listOf("text"), of(HistoryType.TEXT))
        assertEquals(listOf("link"), of(HistoryType.LINK))
    }

    @Test
    fun severalTypesAreOrAndTheGroupsAreAnd() {
        assertEquals(listOf("video", "text", "link"), keys(mixed, HistoryQuery(types = setOf(HistoryType.VIDEO, HistoryType.TEXT, HistoryType.LINK))))
        // Photos OR Documents, AND received, AND the words "re".
        val q = HistoryQuery(search = "re", types = setOf(HistoryType.PHOTO, HistoryType.DOCUMENT), direction = HistoryDirection.RECEIVED)
        assertEquals(listOf("pdf"), keys(mixed, q))
    }

    @Test
    fun directionCountsFailedAndWaitingItemsWhereTheyWent() {
        assertEquals(mixed.map { it.key }, keys(mixed, HistoryQuery(direction = HistoryDirection.BOTH)))
        assertEquals(listOf("photo", "mixed", "apk", "link", "failed", "waiting"), keys(mixed, HistoryQuery(direction = HistoryDirection.SENT)))
        assertEquals(listOf("pdf", "video", "text"), keys(mixed, HistoryQuery(direction = HistoryDirection.RECEIVED)))
        val expired = files("expired", 5, "old.pdf", types = pdf, outgoing = false, status = HistoryStatus.EXPIRED)
        assertEquals(listOf("pdf", "video", "text", "expired"), keys(mixed + expired, HistoryQuery(direction = HistoryDirection.RECEIVED)))
    }

    @Test
    fun searchLooksInEveryNameTheFullTextAndTheTitleIgnoringCase() {
        // "z.pdf" is the second file of "mixed": the title shows only "b.png".
        assertEquals(listOf("mixed"), keys(mixed, HistoryQuery(search = "Z.PDF")))
        assertEquals(listOf("mixed"), keys(mixed, HistoryQuery(search = "c.zip")))
        val long = textRow("long", 1, "the start of it", body = "the start of it and, much later, a needle")
        assertEquals(listOf("long"), keys(mixed + long, HistoryQuery(search = "NEEDLE")))
        assertEquals(listOf("link"), keys(mixed, HistoryQuery(search = "kin site")))
        assertEquals(listOf("link"), keys(mixed, HistoryQuery(search = "example.com/link")))
        assertEquals("words with spaces around them are trimmed", listOf("link"), keys(mixed, HistoryQuery(search = "  kin site ")))
        assertEquals(mixed.size, keys(mixed, HistoryQuery(search = "   ")).size)
        assertTrue(keys(mixed, HistoryQuery(search = "nothing like this")).isEmpty())
    }

    @Test
    fun newestAndOldestAreOppositeAndTiesAreStable() {
        assertEquals(mixed.map { it.key }.reversed(), keys(mixed, HistoryQuery(sort = HistorySort.OLDEST)))
        val tie = listOf(item("b", 5), item("a", 5))
        assertEquals(listOf("a", "b"), keys(tie, HistoryQuery()))
        assertEquals(listOf("b", "a"), keys(tie, HistoryQuery(sort = HistorySort.OLDEST)))
    }

    @Test
    fun nameSortsByTheFirstFileNameOrWhatATextRowShowsIgnoringCase() {
        val list = listOf(
            files("c", 1, "banana.png", types = photo),
            files("d", 2, "Apple.png", types = photo),
            textRow("e", 3, "cherry note"),
            linkRow("f", 4, "Date palm"),
            files("g", 5, "apple.png", types = photo),
            files("h", 6, "ápple.png", "zzz.png", types = photo),
        )
        // The three apples are equal at the primary level (case and accents ignored): newest first among them. Then banana, cherry, Date.
        assertEquals(listOf("h", "g", "d", "c", "e", "f"), keys(list, HistoryQuery(sort = HistorySort.NAME)))
    }

    @Test
    fun sizeSortsLargestFirstAndTextsAndLinksGoAfterAllFilesNewestFirst() {
        val list = listOf(
            textRow("t1", 100, "old text", size = 9_000_000),
            files("small", 50, "s.png", types = photo, size = 10),
            files("big", 40, "b.png", types = photo, size = 800),
            linkRow("l1", 120, "a link"),
            files("tie1", 30, "t1.png", types = photo, size = 300),
            files("tie2", 60, "t2.png", types = photo, size = 300),
        )
        assertEquals(listOf("big", "tie2", "tie1", "small", "l1", "t1"), keys(list, HistoryQuery(sort = HistorySort.SIZE)))
    }

    @Test
    fun daysAreOnlyForNewestAndOldest() {
        assertTrue(HistoryView.byDay(HistorySort.NEWEST))
        assertTrue(HistoryView.byDay(HistorySort.OLDEST))
        assertFalse(HistoryView.byDay(HistorySort.NAME))
        assertFalse(HistoryView.byDay(HistorySort.SIZE))
        // Oldest first keeps the days in order, the oldest day first.
        val sorted = HistoryView.apply(listOf(item("a", at(26, 18)), item("c", at(25, 21)), item("d", at(20, 12))), HistoryQuery(sort = HistorySort.OLDEST), Locale.UK)
        assertEquals(listOf("Tue 20 Oct", "Yesterday", "Today"), HistoryView.group(sorted, now, zone, Locale.UK).map { it.label })
    }

    @Test
    fun chipsAreOneChoiceEachAndTheXResetsOnlyThatChoice() {
        assertTrue(HistoryView.chips(HistoryQuery()).isEmpty())
        val query = HistoryQuery(search = "tax", types = setOf(HistoryType.LINK, HistoryType.PHOTO), sort = HistorySort.SIZE, direction = HistoryDirection.SENT)
        val chips = HistoryView.chips(query)
        assertEquals(listOf("Photos", "Links", "Sent", "Size", "tax"), chips.map { it.label })
        assertEquals(query.copy(types = setOf(HistoryType.LINK)), chips[0].remove(query))
        assertEquals(query.copy(types = setOf(HistoryType.PHOTO)), chips[1].remove(query))
        assertEquals(query.copy(direction = HistoryDirection.BOTH), chips[2].remove(query))
        assertEquals(query.copy(sort = HistorySort.NEWEST), chips[3].remove(query))
        assertEquals(query.copy(search = ""), chips[4].remove(query))
        assertEquals(listOf("Oldest"), HistoryView.chips(HistoryQuery(sort = HistorySort.OLDEST)).map { it.label })
        assertEquals("a long search is cut", 25, HistoryView.chips(HistoryQuery(search = "x".repeat(60))).single().label.length)
        assertTrue("blank words make no chip", HistoryView.chips(HistoryQuery(search = "  ")).isEmpty())
    }

    @Test
    fun theQuerySurvivesARotation() {
        val scope = androidx.compose.runtime.saveable.SaverScope { true }
        fun roundTrip(q: HistoryQuery) = with(historyQuerySaver) { restore(with(scope) { save(q) }!!) }
        val q = HistoryQuery(search = "a,b", types = setOf(HistoryType.APP, HistoryType.TEXT), sort = HistorySort.NAME, direction = HistoryDirection.RECEIVED)
        assertEquals(q, roundTrip(q))
        assertEquals(HistoryQuery(), roundTrip(HistoryQuery()))
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
        val files = listOf("HomeScreen", "HistoryScreen", "HistoryFilterSheet", "OnboardingScreen", "SettingsScreen", "KinComponents", "HistoryView", "DebugScreen", "UiText", "MediaScreens", "MirrorScreens", "PresenceScreens", "DndScreens")
        for (name in files) {
            // A dash in a comment is fine; one in a string literal reaches the user.
            val strings = java.io.File(dir, "$name.kt").readLines().filterNot { it.trim().startsWith("//") || it.trim().startsWith("*") || it.trim().startsWith("/**") }
            for (line in strings) assertFalse("$name: $line", Regex("\"[^\"]*—[^\"]*\"").containsMatchIn(line))
        }
    }
}
