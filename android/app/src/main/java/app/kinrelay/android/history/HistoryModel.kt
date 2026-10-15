package app.kinrelay.android.history

import app.kinrelay.android.data.OutcomeRow
import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.data.TextItemEntity
import app.kinrelay.android.data.TextSource
import app.kinrelay.android.data.TransferDirection
import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferStatus

enum class HistoryStatus { QUEUED, ACTIVE, DONE, FAILED, CANCELED, EXPIRED }

/** One line of the history screen: a transfer of files or a text, in either direction. */
data class HistoryItem(
    /** Unique in the list (`t:<id>` for transfers, `x:<id>` for texts). */
    val key: String,
    val id: String,
    val isText: Boolean,
    val outgoing: Boolean,
    val status: HistoryStatus,
    val timeMillis: Long,
    val title: String,
    val sizeBytes: Long,
    val errorCode: String?,
    val errorMessage: String?,
    val peerName: String,
    /** The text of a text item (to copy). Null for files, and never set for a sensitive text (those are not shown at all). */
    val text: String?,
    /** A received file that is saved: tapping the item opens it. */
    val openUri: String?,
    val openMime: String?,
    val canRetry: Boolean,
    val canCancel: Boolean,
    /** A failed upload or text that a user Retry already sent again as a new item (the new one is its own row; this one stays as it ended). */
    val retried: Boolean,
)

/** Builds the history from what is stored: both directions, newest first, never a sensitive text, never an automatic clipboard sync, capped. */
object HistoryBuilder {
    /**
     * The texts the history may show. A sensitive text is never shown, and neither is anything that came from the automatic clipboard sync
     * (those are not stored in the first place; this is the second lock on the door).
     */
    fun visible(texts: List<TextItemEntity>): List<TextItemEntity> =
        texts.filter { !it.sensitive && it.source != TextSource.AUTO_CLIPBOARD }

    fun build(
        transfers: List<TransferEntity>,
        files: List<TransferFileEntity>,
        texts: List<TextItemEntity>,
        limit: Int = QueuePolicy.HISTORY_LIMIT,
    ): List<HistoryItem> {
        val filesByTransfer = files.groupBy { it.transferId }
        val items = ArrayList<HistoryItem>(transfers.size + texts.size)
        for (t in transfers) items += fileItem(t, filesByTransfer[t.id].orEmpty())
        for (x in visible(texts)) items += textItem(x)
        return items.sortedWith(compareByDescending<HistoryItem> { it.timeMillis }.thenBy { it.key }).take(limit)
    }

    private fun status(status: String, errorCode: String?): HistoryStatus = when (status) {
        TransferStatus.QUEUED -> HistoryStatus.QUEUED
        TransferStatus.ACTIVE -> HistoryStatus.ACTIVE
        TransferStatus.DONE -> HistoryStatus.DONE
        TransferStatus.CANCELED -> HistoryStatus.CANCELED
        else -> if (errorCode == EXPIRED) HistoryStatus.EXPIRED else HistoryStatus.FAILED
    }

    private fun fileItem(t: TransferEntity, files: List<TransferFileEntity>): HistoryItem {
        val outgoing = t.direction == TransferDirection.UP
        val status = status(t.status, t.errorCode)
        val names = files.sortedBy { it.ordinal }.map { it.finalName ?: it.name }
        val title = when {
            names.isEmpty() -> "Files"
            names.size == 1 -> names[0]
            else -> "${names.size} files: ${names.take(2).joinToString(", ")}${if (names.size > 2) ", …" else ""}"
        }
        val saved = files.filter { it.finished && it.localUri != null }.minByOrNull { it.ordinal }
        val retried = outgoing && t.retriedAs != null
        return HistoryItem(
            key = "t:${t.id}", id = t.id, isText = false, outgoing = outgoing, status = status, timeMillis = t.createdAtMillis, title = title,
            sizeBytes = t.totalBytes, errorCode = t.errorCode.takeIf { t.status == TransferStatus.FAILED || t.status == TransferStatus.CANCELED },
            errorMessage = t.message.takeIf { t.status == TransferStatus.FAILED || t.status == TransferStatus.CANCELED }, peerName = t.peerName,
            text = null, openUri = if (!outgoing && status == HistoryStatus.DONE) saved?.localUri else null, openMime = saved?.mime,
            canRetry = outgoing && t.status == TransferStatus.FAILED && !retried && t.errorCode !in SOURCE_GONE,
            canCancel = t.status == TransferStatus.QUEUED || t.status == TransferStatus.ACTIVE,
            retried = retried,
        )
    }

    private fun textItem(x: TextItemEntity): HistoryItem {
        val outgoing = x.direction == TransferDirection.UP
        val status = status(x.status, x.errorCode)
        val failedLike = x.status == TransferStatus.FAILED || x.status == TransferStatus.CANCELED
        return HistoryItem(
            key = "x:${x.id}", id = x.id, isText = true, outgoing = outgoing, status = status, timeMillis = x.createdAtMillis,
            title = preview(x.body), sizeBytes = x.byteSize, errorCode = x.errorCode.takeIf { failedLike }, errorMessage = x.message.takeIf { failedLike },
            peerName = x.peerName, text = x.body, openUri = null, openMime = null,
            canRetry = outgoing && x.status == TransferStatus.FAILED && x.body != null && x.retriedAs == null,
            canCancel = outgoing && x.status == TransferStatus.QUEUED,
            retried = outgoing && x.retriedAs != null,
        )
    }

    /** One line of a text, at most [limit] characters (cut on a code point). */
    fun preview(body: String?, limit: Int = 80): String {
        if (body == null) return "Text"
        val flat = body.replace(Regex("\\s+"), " ").trim()
        if (flat.isEmpty()) return "Text"
        if (flat.codePointCount(0, flat.length) <= limit) return flat
        return flat.substring(0, flat.offsetByCodePoints(0, limit)) + "…"
    }

    const val EXPIRED = "expired"

    /**
     * Failures after which the files of an upload can't be sent again, so Retry is not offered: it expired (its cached copies were deleted),
     * the shared file can't be read any more, or the PC it was meant for is no longer paired (the copies went with it).
     */
    private val SOURCE_GONE = setOf(EXPIRED, "source_lost", "peer_changed")
}

/**
 * The first-try success rate of targets.md #1 ("at least 99% of sends succeed on the first try"): of the sends from this phone that ended
 * (delivered, or failed), how many were delivered by the first attempt. Cancelled items and queued ones that haven't ended don't count, and
 * neither do failures that aren't a send gone wrong: an item that **expired** in the queue, one whose PC was **unpaired**, and one that
 * was meant for a PC that is **no longer paired**.
 */
object SuccessRate {
    data class Result(val firstTry: Int, val total: Int) {
        /** 0..100, or null when nothing has ended yet. */
        val percent: Double? get() = if (total == 0) null else firstTry * 100.0 / total
        fun label(): String = percent?.let { String.format(java.util.Locale.US, "%.1f%% (%d of %d)", it, firstTry, total) } ?: "no sends yet"
    }

    private val NOT_A_SEND = setOf(HistoryBuilder.EXPIRED, "unpaired", "peer_changed")

    fun compute(vararg groups: List<OutcomeRow>): Result {
        var firstTry = 0
        var total = 0
        for (row in groups.flatMap { it }) {
            when (row.status) {
                TransferStatus.DONE -> { total++; if (row.attempts <= 1) firstTry++ }
                TransferStatus.FAILED -> if (row.errorCode !in NOT_A_SEND) total++
            }
        }
        return Result(firstTry, total)
    }
}
