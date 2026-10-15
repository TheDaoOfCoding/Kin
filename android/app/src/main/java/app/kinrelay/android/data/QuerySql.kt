package app.kinrelay.android.data

/**
 * The queries that keep the database small, as constants: the DAOs use them, and `RetentionSqlTest` runs the same text on a real SQLite
 * (the unit tests have no Android SQLite, so a query written only inside an annotation could not be tested).
 *
 * What is kept for ever: every transfer row (a transfer ID is never used twice, so the ID must be remembered) and every text row. What is
 * dropped beyond the newest [QueuePolicy.HISTORY_LIMIT]: the file rows of finished transfers (names, URIs) and the bodies of finished
 * texts. A transfer that still owes the PC a `complete` or `cancel` keeps its file rows until that is acked.
 */
object QuerySql {
    const val DROP_OLD_TRANSFER_FILES =
        "DELETE FROM transfer_files WHERE transferId IN (" +
            "SELECT id FROM transfers WHERE status IN ('done', 'failed', 'canceled') AND owed IS NULL " +
            "AND id NOT IN (SELECT id FROM transfers ORDER BY createdAtMillis DESC LIMIT :keep))"

    const val DROP_OLD_TEXT_BODIES =
        "UPDATE texts SET body = NULL WHERE body IS NOT NULL AND status IN ('done', 'failed', 'canceled') " +
            "AND id NOT IN (SELECT id FROM texts ORDER BY createdAtMillis DESC LIMIT :keep)"

    /** The cache keys of transfers whose cached copies may still be needed: queued, active, or failed with files and not retried. */
    const val LIVE_LOCAL_IDS =
        "SELECT localId FROM transfers WHERE status IN ('queued', 'active') " +
            "OR (status = 'failed' AND retriedAs IS NULL AND EXISTS (SELECT 1 FROM transfer_files f WHERE f.transferId = transfers.id))"

    /** A retried item is one send, counted by the item that took its place. */
    const val OUTGOING_TRANSFER_OUTCOMES =
        "SELECT status AS status, errorCode AS errorCode, attempts AS attempts FROM transfers " +
            "WHERE direction = 'up' AND status IN ('done', 'failed') AND retriedAs IS NULL"

    const val OUTGOING_TEXT_OUTCOMES =
        "SELECT status AS status, errorCode AS errorCode, attempts AS attempts FROM texts " +
            "WHERE direction = 'up' AND status IN ('done', 'failed') AND retriedAs IS NULL"
}
