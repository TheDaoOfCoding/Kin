package app.kinrelay.android.history

import app.kinrelay.android.data.OutcomeRow
import app.kinrelay.android.data.TextItemEntity
import app.kinrelay.android.data.TextSource
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.ui.HistoryText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HistoryTest {
    private fun text(
        id: String, at: Long, direction: String = "up", status: String = "done", body: String? = "hello", sensitive: Boolean = false,
        source: String = TextSource.SHARE, errorCode: String? = null,
    ) = TextItemEntity(id, direction, "pc", "Desk PC", status, at, at + 1, body, (body ?: "").length.toLong(), sensitive, false, source, errorCode, errorCode?.let { "message of $it" }, 1)

    private fun transfer(id: String, at: Long, direction: String = "up", status: String = "done", errorCode: String? = null, retriedAs: String? = null, total: Long = 100) =
        TransferEntity(id, direction, "pc", "Desk PC", "file", status, at, at + 1, total, errorCode, errorCode?.let { "message of $it" }, false, retriedAs = retriedAs)

    private fun file(id: String, name: String, ordinal: Int = 0, uri: String? = null, finished: Boolean = false, mime: String = "image/jpeg") =
        TransferFileEntity(id, "f${ordinal + 1}", ordinal, name, name, mime, 10, 0, uri, null, finished)

    // ---------------------------------------------------------------- filtering

    @Test
    fun aSensitiveTextIsNeverShown() {
        val shown = HistoryBuilder.build(emptyList(), emptyList(), listOf(text("secret", 5, sensitive = true, body = null), text("plain", 4)))
        assertEquals(listOf("x:plain"), shown.map { it.key })
    }

    @Test
    fun anAlertFromThePcIsARowThatShowsItsTitleAndText() {
        // 2B.5: kind `alert` in the texts table (no schema change); the row reads like a text, with the title first.
        val alert = text("a1", 5, direction = "down", body = "Scene 12 finished", source = TextSource.PC)
            .copy(kind = app.kinrelay.android.data.TextKind.ALERT, title = "Render")
        val plain = text("a2", 4, direction = "down", body = "Done", source = TextSource.PC).copy(kind = app.kinrelay.android.data.TextKind.ALERT)
        val shown = HistoryBuilder.build(emptyList(), emptyList(), listOf(alert, plain))
        assertEquals(listOf("Render: Scene 12 finished", "Done"), shown.map { it.title })
        assertTrue(shown.all { it.isText && !it.outgoing && it.status == HistoryStatus.DONE })
        assertEquals("Scene 12 finished", shown.first().text)
    }

    @Test
    fun anAutomaticClipboardSyncIsNeverShownEvenIfSomethingStoredIt() {
        val shown = HistoryBuilder.build(emptyList(), emptyList(), listOf(text("auto", 9, source = TextSource.AUTO_CLIPBOARD), text("manual", 8, source = TextSource.CLIPBOARD)))
        assertEquals(listOf("x:manual"), shown.map { it.key })
    }

    @Test
    fun aSensitiveTextsBodyNeverReachesTheItemEvenWhenItIsKept() {
        val items = HistoryBuilder.build(emptyList(), emptyList(), listOf(text("s", 1, sensitive = true, body = "hunter2")))
        assertTrue(items.isEmpty())
    }

    @Test
    fun bothDirectionsAreShownNewestFirstAndTheListIsCapped() {
        val texts = (1..300).map { text("t$it", it.toLong()) }
        val transfers = (1..300).map { transfer("f$it", 1_000L + it, direction = if (it % 2 == 0) "down" else "up") }
        val shown = HistoryBuilder.build(transfers, emptyList(), texts, limit = 500)
        assertEquals(500, shown.size)
        assertEquals("newest first", shown.map { it.timeMillis }.sortedDescending(), shown.map { it.timeMillis })
        assertTrue(shown.any { it.outgoing } && shown.any { !it.outgoing })
        assertEquals("f300", shown.first().id)
    }

    // ---------------------------------------------------------------- what an item offers

    @Test
    fun aFailedOutgoingItemCanBeRetriedAQueuedOneCanBeCanceled() {
        val items = HistoryBuilder.build(
            listOf(transfer("failed-up", 1, status = "failed", errorCode = "connection_lost"), transfer("queued-up", 2, status = "queued"), transfer("failed-down", 3, direction = "down", status = "failed", errorCode = "internal")),
            emptyList(),
            listOf(text("failed-text", 4, status = "failed", errorCode = "internal"), text("queued-text", 5, status = "queued")),
        ).associateBy { it.id }
        assertTrue(items.getValue("failed-up").canRetry)
        assertFalse(items.getValue("failed-up").canCancel)
        assertTrue(items.getValue("queued-up").canCancel)
        assertFalse(items.getValue("queued-up").canRetry)
        assertFalse("an incoming failure is not retried from here", items.getValue("failed-down").canRetry)
        assertTrue(items.getValue("failed-text").canRetry)
        assertTrue(items.getValue("queued-text").canCancel)
        assertEquals(HistoryStatus.FAILED, items.getValue("failed-text").status)
        assertEquals("internal", items.getValue("failed-text").errorCode)
    }

    @Test
    fun anExpiredItemShowsAsExpiredAndCanBeRetried() {
        val items = HistoryBuilder.build(emptyList(), emptyList(), listOf(text("old", 1, status = "failed", errorCode = "expired")))
        assertEquals(HistoryStatus.EXPIRED, items.single().status)
        assertEquals("expired", items.single().errorCode)
        assertEquals("Expired", HistoryText.status(items.single()))
        assertTrue(items.single().canRetry)
    }

    @Test
    fun aTransferThatWasRetriedAsANewOneHasNoRetryButTheNewOneIsItsOwnItem() {
        val items = HistoryBuilder.build(
            listOf(transfer("old", 1, status = "failed", errorCode = "not_found", retriedAs = "new"), transfer("new", 2, status = "done")),
            emptyList(), emptyList(),
        ).associateBy { it.id }
        assertFalse(items.getValue("old").canRetry)
        assertTrue(items.getValue("old").retried)
        assertEquals(HistoryStatus.DONE, items.getValue("new").status)
    }

    @Test
    fun aDoneReceivedFileOpensAndADoneTextCopies() {
        val items = HistoryBuilder.build(
            listOf(transfer("got", 1, direction = "down")),
            listOf(file("got", "photo.jpg", uri = "content://media/1", finished = true)),
            listOf(text("in", 2, direction = "down", body = "text from the PC"), text("out", 3)),
        ).associateBy { it.id }
        assertEquals("content://media/1", items.getValue("got").openUri)
        assertEquals("image/jpeg", items.getValue("got").openMime)
        assertEquals("text from the PC", items.getValue("in").text)
        assertNull(items.getValue("in").openUri)
        assertNull("a sent file is on the PC: nothing to open here", HistoryBuilder.build(listOf(transfer("sent", 1)), listOf(file("sent", "a.jpg", uri = "content://x", finished = true)), emptyList()).single().openUri)
    }

    @Test
    fun theTitleSaysWhatItIs() {
        val items = HistoryBuilder.build(
            listOf(transfer("one", 1), transfer("many", 2)),
            listOf(file("one", "a.jpg"), file("many", "b.jpg", 0), file("many", "c.jpg", 1), file("many", "d.jpg", 2)),
            listOf(text("long", 3, body = "line one\n\n   line two " + "x".repeat(200))),
        ).associateBy { it.id }
        assertEquals("a.jpg", items.getValue("one").title)
        assertEquals("3 files: b.jpg, c.jpg, …", items.getValue("many").title)
        assertTrue(items.getValue("long").title.startsWith("line one line two x"))
        assertTrue(items.getValue("long").title.length <= 81)
    }

    // ---------------------------------------------------------------- numbers

    @Test
    fun sizesAndTimesAreReadable() {
        assertEquals("0 B", HistoryText.size(0))
        assertEquals("999 B", HistoryText.size(999))
        assertEquals("1.5 KB", HistoryText.size(1_500))
        assertEquals("3.4 MB", HistoryText.size(3_400_000))
        assertEquals("2.00 GB", HistoryText.size(2_000_000_000))
        val zone = java.time.ZoneOffset.UTC
        val noon = java.time.LocalDateTime.of(2026, 10, 15, 12, 3).toInstant(zone).toEpochMilli()
        assertEquals("12:03", HistoryText.time(noon, noon + 3_600_000, zone))
        assertTrue(HistoryText.time(noon, noon + 2 * 86_400_000L, zone).endsWith("12:03"))
        assertTrue(HistoryText.time(noon, noon + 2 * 86_400_000L, zone).startsWith("15"))
    }

    // ---------------------------------------------------------------- the first-try success rate

    @Test
    fun theSuccessRateCountsFirstTryDeliveriesAmongTheSendsThatEnded() {
        val rows = listOf(
            OutcomeRow("done", null, 1), OutcomeRow("done", null, 1), OutcomeRow("done", null, 1), // first try
            OutcomeRow("done", null, 3), // delivered, but not on the first try
            OutcomeRow("failed", "connection_lost", 5),
        )
        val result = SuccessRate.compute(rows)
        assertEquals(3, result.firstTry)
        assertEquals(5, result.total)
        assertEquals(60.0, result.percent!!, 0.0001)
        assertEquals("60.0% (3 of 5)", result.label())
    }

    @Test
    fun anExpiredAnUnpairedAndAWrongPcItemAreNotCountedAsSendsGoneWrong() {
        val result = SuccessRate.compute(
            listOf(
                OutcomeRow("done", null, 1), OutcomeRow("failed", "expired", 0), OutcomeRow("failed", "unpaired", 0), OutcomeRow("failed", "peer_changed", 0),
                OutcomeRow("failed", "not_found", 2),
            ),
        )
        assertEquals(1, result.firstTry)
        assertEquals("one delivered and one that really failed", 2, result.total)
    }

    @Test
    fun theSuccessRateOfNothingIsNoNumberAndSeveralGroupsAddUp() {
        assertNull(SuccessRate.compute(emptyList()).percent)
        assertEquals("no sends yet", SuccessRate.compute().label())
        val both = SuccessRate.compute(listOf(OutcomeRow("done", null, 1)), listOf(OutcomeRow("done", null, 1), OutcomeRow("done", null, 2)))
        assertEquals("2 of 3", "${both.firstTry} of ${both.total}")
        assertEquals("99.0% (99 of 100)", SuccessRate.Result(99, 100).label())
        assertEquals("a row with no attempts counted (a row from before 1.4) is a first try", 1, SuccessRate.compute(listOf(OutcomeRow("done", null, 0))).firstTry)
    }

    // ---------------------------------------------------------------- Retry only where the files can be sent again (1.4 review)

    @Test
    fun anExpiredUploadHasNoRetryBecauseItsCacheIsGone() {
        val items = HistoryBuilder.build(listOf(transfer("exp", 5, status = "failed", errorCode = "expired")), emptyList(), emptyList())
        assertEquals(HistoryStatus.EXPIRED, items.single().status)
        assertFalse(items.single().canRetry)
    }

    @Test
    fun anUploadWhoseSourceCantBeRecoveredHasNoRetry() {
        for (code in listOf("source_lost", "peer_changed")) {
            val item = HistoryBuilder.build(listOf(transfer("t-$code", 5, status = "failed", errorCode = code)), emptyList(), emptyList()).single()
            assertFalse("no Retry after $code", item.canRetry)
        }
    }

    @Test
    fun anUploadThatFailedForTheNetworkOrThePcCanStillBeRetried() {
        for (code in listOf("connection_lost", "not_found", "disk_full", "source_changed")) {
            val item = HistoryBuilder.build(listOf(transfer("t-$code", 5, status = "failed", errorCode = code)), emptyList(), emptyList()).single()
            assertTrue("Retry after $code", item.canRetry)
        }
    }

    @Test
    fun anExpiredTextStillHasRetryItsBodyIsKept() {
        val item = HistoryBuilder.build(emptyList(), emptyList(), listOf(text("x", 5, status = "failed", errorCode = "expired"))).single()
        assertTrue(item.canRetry)
    }

    @Test
    fun aFailedTextThatWasRetriedStaysInTheHistoryAndPointsAtTheNewOne() {
        val failed = text("old", 5, status = "failed", errorCode = "internal").copy(retriedAs = "new")
        val fresh = text("new", 9, status = "queued")
        val items = HistoryBuilder.build(emptyList(), emptyList(), listOf(failed, fresh))
        assertEquals("both rows are shown, newest first", listOf("x:new", "x:old"), items.map { it.key })
        val old = items.single { it.id == "old" }
        assertTrue("it says it was sent again", old.retried)
        assertFalse("and can't be retried a second time", old.canRetry)
        assertEquals(HistoryStatus.FAILED, old.status)
        assertTrue(items.single { it.id == "new" }.canCancel)
    }
}
