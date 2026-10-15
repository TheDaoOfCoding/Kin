package app.kinrelay.android.history

import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.data.TextDao
import app.kinrelay.android.data.TransferDao
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flowOn

/** The history as a live list: transfers (with their files) and texts, both directions, rebuilt when the database changes. */
class HistoryRepository(transfers: TransferDao, texts: TextDao, limit: Int = QueuePolicy.HISTORY_LIMIT) {
    val items: Flow<List<HistoryItem>> = combine(
        transfers.observeRecent(limit), transfers.observeRecentFiles(limit), texts.observeRecent(limit),
    ) { t, f, x -> HistoryBuilder.build(t, f, x, limit) }.flowOn(Dispatchers.Default)
}
