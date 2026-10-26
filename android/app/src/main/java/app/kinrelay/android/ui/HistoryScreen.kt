package app.kinrelay.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus
import app.kinrelay.android.ui.theme.EmptyArt
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.KinTint

/**
 * The history (milestone 1.4): what was sent to and received from the PC, both directions, newest first (the last 500), in days. A sensitive text
 * and the automatic clipboard sync never appear. Tap a finished item to open the file or copy the text; **Retry** on a failed send, **Cancel**
 * on one that still waits. [onSend] is the "Send something" button of the empty state; without it the button is left out.
 */
@Composable
fun HistoryScreen(
    items: List<HistoryItem>,
    nowMillis: Long,
    onOpen: (HistoryItem) -> Unit,
    onRetry: (HistoryItem) -> Unit,
    onCancel: (HistoryItem) -> Unit,
    modifier: Modifier = Modifier,
    onSend: (() -> Unit)? = null,
    /** Room left under the list for the floating nav. */
    bottomPadding: Dp = 0.dp,
) {
    var filter by rememberSaveable { mutableStateOf(HistoryFilter.ALL) }
    Column(modifier.fillMaxSize().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        PageTitle("History", Modifier.padding(top = 12.dp, bottom = 2.dp))
        if (items.isEmpty()) {
            HistoryEmpty(
                title = "Nothing here yet", text = "Things you send and receive will show up here, newest first.", onSend = onSend,
                modifier = Modifier.weight(1f).padding(bottom = bottomPadding),
            )
            return@Column
        }
        FilterRow(filter) { filter = it }
        val shown = items.filter(filter::matches)
        if (shown.isEmpty()) {
            HistoryEmpty(
                title = "Nothing here", text = "No ${filter.label.lowercase()} items yet.", onSend = null,
                modifier = Modifier.weight(1f).padding(bottom = bottomPadding),
            )
            return@Column
        }
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            groups(HistoryView.group(shown, nowMillis), nowMillis, onOpen, onRetry, onCancel)
            item { Spacer(Modifier.height(bottomPadding)) }
        }
    }
}

/** The empty state: the happy box (Warm) or a plain icon (Simple), a title, a line, and a button to send the first thing. */
@Composable
private fun HistoryEmpty(title: String, text: String, onSend: (() -> Unit)?, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().padding(horizontal = 22.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
        EmptyArt(Modifier.size(170.dp, 128.dp))
        Spacer(Modifier.height(14.dp))
        Text(title, style = MaterialTheme.typography.titleLarge.copy(fontSize = 19.sp), color = KinTheme.colors.onSurface, textAlign = TextAlign.Center)
        Text(text, style = MaterialTheme.typography.bodyMedium, color = KinTheme.colors.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 6.dp, bottom = 18.dp))
        if (onSend != null) KinButton("Send something", onSend, icon = KinIcons.Send)
    }
}

@Composable
private fun FilterRow(selected: HistoryFilter, onSelect: (HistoryFilter) -> Unit) {
    val colors = KinTheme.colors
    val shape = KinTheme.style.full
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        HistoryFilter.entries.forEach { filter ->
            val on = filter == selected
            Row(
                Modifier
                    .heightIn(min = 34.dp)
                    .clip(shape)
                    .let { if (on) it.background(colors.primaryContainer) else it.background(colors.surface).border(BorderStroke(1.dp, colors.outline), shape) }
                    .selectable(selected = on, role = Role.Tab, onClick = { onSelect(filter) })
                    .padding(horizontal = 14.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                if (on) Icon(KinIcons.Check, contentDescription = null, tint = colors.onPrimaryContainer, modifier = Modifier.size(15.dp))
                Text(filter.label, style = MaterialTheme.typography.labelLarge.copy(fontSize = 13.sp), color = if (on) colors.onPrimaryContainer else colors.onSurfaceVariant)
            }
        }
    }
}

private fun LazyListScope.groups(
    groups: List<HistoryGroup>,
    nowMillis: Long,
    onOpen: (HistoryItem) -> Unit,
    onRetry: (HistoryItem) -> Unit,
    onCancel: (HistoryItem) -> Unit,
) {
    groups.forEach { group ->
        item(key = "g:" + group.label) { GroupLabel(group.label) }
        item(key = "c:" + group.label) {
            KinCard {
                group.items.forEachIndexed { index, item ->
                    if (index > 0) HorizontalDivider(color = KinTheme.colors.outlineVariant)
                    HistoryRow(item, nowMillis, onOpen, onRetry, onCancel)
                }
            }
        }
    }
}

/**
 * One row of the history, in the design's list style: a round picture, the title and what happened under it, and at the end a badge, a Cancel or a
 * Retry. The same row is used by "Recent" on Home. Put rows in a [KinCard] with a divider between them.
 */
@Composable
fun HistoryRow(
    item: HistoryItem,
    nowMillis: Long,
    onOpen: (HistoryItem) -> Unit,
    onRetry: (HistoryItem) -> Unit,
    onCancel: (HistoryItem) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = KinTheme.colors
    val tappable = item.status == HistoryStatus.DONE && (item.openUri != null || item.text != null)
    val failed = item.status == HistoryStatus.FAILED || item.status == HistoryStatus.EXPIRED
    val (icon, tint) = when (HistoryView.kind(item)) {
        HistoryView.Kind.TEXT -> KinIcons.Chat to KinTint.LILAC
        HistoryView.Kind.IMAGE -> KinIcons.Image to KinTint.PEACH
        HistoryView.Kind.FILE -> KinIcons.File to KinTint.SAGE
    }
    Row(
        modifier
            .fillMaxWidth()
            .let { if (tappable) it.clickable(onClickLabel = if (item.text != null) "Copy" else "Open") { onOpen(item) } else it }
            .padding(horizontal = 12.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        KinAvatar(icon, tint, error = failed)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(item.title, style = MaterialTheme.typography.titleSmall, color = colors.onSurface, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                Icon(if (item.outgoing) KinIcons.Up else KinIcons.Down, contentDescription = null, tint = colors.onSurfaceVariant, modifier = Modifier.size(13.dp))
                Text(HistoryText.line(item, nowMillis), style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (failed) {
                val reason = item.errorMessage ?: item.errorCode
                if (reason != null) {
                    Text(
                        if (item.errorCode != null && !reason.contains(item.errorCode)) "$reason (${item.errorCode})" else reason,
                        style = MaterialTheme.typography.bodySmall, color = colors.error,
                    )
                }
            }
            if (item.retried) Text(if (item.isText) "Sent again as a new message." else "Sent again as a new transfer.", style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
        }
        RowEnd(item, failed, onRetry, onCancel)
    }
}

/** What sits at the end of a row: Retry for a failed send, Cancel while it waits or moves, the badge when it is done, else a word. */
@Composable
private fun RowEnd(item: HistoryItem, failed: Boolean, onRetry: (HistoryItem) -> Unit, onCancel: (HistoryItem) -> Unit) {
    val colors = KinTheme.colors
    when {
        item.canRetry -> KinButton("Retry", { onRetry(item) }, style = KinButtonStyle.SOFT, icon = KinIcons.Retry, small = true)
        item.canCancel -> KinIconButton(KinIcons.X, "Cancel", { onCancel(item) })
        item.status == HistoryStatus.DONE -> DoneBadge(HistoryText.status(item))
        failed -> KinChip(HistoryText.status(item), colors.errorContainer, colors.onErrorContainer)
        item.status == HistoryStatus.CANCELED -> KinChip(HistoryText.status(item), colors.surfaceContainerHigh, colors.onSurfaceVariant)
        else -> Unit
    }
}
