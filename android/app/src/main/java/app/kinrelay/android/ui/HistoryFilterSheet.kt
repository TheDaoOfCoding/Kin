package app.kinrelay.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.Saver
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.history.HistoryType
import app.kinrelay.android.ui.send.SheetHeader
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinTheme
import kotlinx.coroutines.launch

/** Keeps a [HistoryQuery] across a rotation (`rememberSaveable`); never to disk, so a restart starts with no filter. */
val historyQuerySaver: Saver<HistoryQuery, List<String>> = Saver(
    save = { q -> listOf(q.search, q.types.joinToString(",") { it.name }, q.sort.name, q.direction.name) },
    restore = { saved ->
        HistoryQuery(
            search = saved[0],
            types = saved[1].split(',').mapNotNull { name -> HistoryType.entries.firstOrNull { it.name == name } }.toSet(),
            sort = HistorySort.entries.firstOrNull { it.name == saved[2] } ?: HistorySort.NEWEST,
            direction = HistoryDirection.entries.firstOrNull { it.name == saved[3] } ?: HistoryDirection.BOTH,
        )
    },
)

/**
 * The History filter: a bottom sheet (like the Send sheet) with a search box, the seven types as toggles, Sort and Direction as one choice each.
 * Every tap changes [query] at once, so the list behind the sheet follows; "Done" closes it. [shown] of [total] says what is left.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun HistoryFilterSheet(query: HistoryQuery, shown: Int, total: Int, onChange: (HistoryQuery) -> Unit, onDismiss: () -> Unit) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    fun close() { scope.launch { sheetState.hide() }.invokeOnCompletion { onDismiss() } }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState, shape = KinTheme.style.shapes.extraLarge) {
        Column(
            modifier = Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 18.dp, end = 18.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SheetHeader(title = "Filter", subtitle = if (shown == total) "$total items" else "$shown of $total items", onClose = ::close)
            OutlinedTextField(
                value = query.search, onValueChange = { onChange(query.copy(search = it)) },
                placeholder = { Text("Search names and text") },
                leadingIcon = { Icon(KinIcons.Search, contentDescription = null, modifier = Modifier.size(20.dp)) },
                singleLine = true, shape = KinTheme.style.shapes.small, modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            )
            Section("Type") {
                HistoryType.entries.forEach { type ->
                    val on = type in query.types
                    ChoiceChip(type.label, on, Role.Checkbox) { onChange(query.copy(types = if (on) query.types - type else query.types + type)) }
                }
            }
            Section("Sort") {
                HistorySort.entries.forEach { sort -> ChoiceChip(sort.label, sort == query.sort, Role.RadioButton) { onChange(query.copy(sort = sort)) } }
            }
            Section("Direction") {
                HistoryDirection.entries.forEach { d -> ChoiceChip(d.label, d == query.direction, Role.RadioButton) { onChange(query.copy(direction = d)) } }
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (!query.isDefault) KinButton("Clear", { onChange(HistoryQuery()) }, style = KinButtonStyle.SOFT)
                KinButton("Done", ::close, modifier = Modifier.weight(1f), block = true)
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Section(title: String, chips: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(title, style = MaterialTheme.typography.labelLarge, color = KinTheme.colors.onSurfaceVariant)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { chips() }
    }
}

/** A choice of the sheet: filled with a tick when on, an outline when off. */
@Composable
private fun ChoiceChip(label: String, on: Boolean, role: Role, onClick: () -> Unit) {
    val colors = KinTheme.colors
    val shape = KinTheme.style.full
    Row(
        Modifier
            .heightIn(min = 36.dp)
            .clip(shape)
            .let { if (on) it.background(colors.primaryContainer) else it.background(colors.surface).border(BorderStroke(1.dp, colors.outline), shape) }
            .selectable(selected = on, role = role, onClick = onClick)
            .padding(horizontal = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (on) Icon(KinIcons.Check, contentDescription = null, tint = colors.onPrimaryContainer, modifier = Modifier.size(15.dp))
        Text(label, style = MaterialTheme.typography.labelLarge.copy(fontSize = 13.sp), color = if (on) colors.onPrimaryContainer else colors.onSurfaceVariant)
    }
}
