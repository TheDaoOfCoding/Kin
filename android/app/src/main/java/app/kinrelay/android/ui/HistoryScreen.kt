package app.kinrelay.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus

/**
 * The history (milestone 1.4): what was sent to and received from the PC, both directions, newest first (the last 500). A sensitive text and
 * the automatic clipboard sync never appear. Tap a finished item to open the file or copy the text; **Retry** on a failed send, **Cancel**
 * on one that still waits.
 */
@Composable
fun HistoryScreen(
    items: List<HistoryItem>,
    nowMillis: Long,
    onOpen: (HistoryItem) -> Unit,
    onRetry: (HistoryItem) -> Unit,
    onCancel: (HistoryItem) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (items.isEmpty()) {
        Column(modifier = modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("History", style = MaterialTheme.typography.titleLarge)
            Text("Nothing yet. What you send to your PC and what it sends to you shows up here.")
        }
        return
    }
    LazyColumn(modifier = modifier.fillMaxSize()) {
        item { Text("History", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 8.dp)) }
        items(items, key = { it.key }) { item ->
            HistoryRow(item, nowMillis, onOpen, onRetry, onCancel)
            HorizontalDivider()
        }
    }
}

@Composable
fun HistoryRow(
    item: HistoryItem,
    nowMillis: Long,
    onOpen: (HistoryItem) -> Unit,
    onRetry: (HistoryItem) -> Unit,
    onCancel: (HistoryItem) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tappable = item.status == HistoryStatus.DONE && (item.openUri != null || item.text != null)
    Column(
        modifier = modifier.fillMaxWidth().let { if (tappable) it.clickable { onOpen(item) } else it }.padding(horizontal = 16.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(if (item.outgoing) "↑" else "↓", style = MaterialTheme.typography.titleMedium)
            Text(item.title, style = MaterialTheme.typography.bodyLarge, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            Text(HistoryText.status(item), style = MaterialTheme.typography.labelLarge, color = statusColor(item.status))
        }
        Text(
            "${HistoryText.direction(item)} · ${HistoryText.time(item.timeMillis, nowMillis)} · ${HistoryText.size(item.sizeBytes)}" +
                if (item.isText) " · text" else "",
            style = MaterialTheme.typography.bodySmall,
        )
        if (item.status == HistoryStatus.FAILED || item.status == HistoryStatus.EXPIRED) {
            val reason = item.errorMessage ?: item.errorCode
            if (reason != null) {
                Text(
                    if (item.errorCode != null && !reason.contains(item.errorCode)) "$reason (${item.errorCode})" else reason,
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error,
                )
            }
        }
        if (item.retried) Text(if (item.isText) "Sent again as a new message." else "Sent again as a new transfer.", style = MaterialTheme.typography.bodySmall)
        if (tappable) Text(if (item.text != null) "Tap to copy" else "Tap to open", style = MaterialTheme.typography.bodySmall)
        if (item.canRetry || item.canCancel) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (item.canRetry) TextButton(onClick = { onRetry(item) }) { Text("Retry") }
                if (item.canCancel) TextButton(onClick = { onCancel(item) }) { Text("Cancel") }
            }
        }
    }
}

@Composable
private fun statusColor(status: HistoryStatus) = when (status) {
    HistoryStatus.FAILED, HistoryStatus.EXPIRED -> MaterialTheme.colorScheme.error
    HistoryStatus.DONE -> MaterialTheme.colorScheme.primary
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}
