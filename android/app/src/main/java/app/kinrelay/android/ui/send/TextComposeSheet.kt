package app.kinrelay.android.ui.send

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

/**
 * The text box. [onSend] gets what was typed; whether it goes as a text or, when it is exactly one web link, as a `link` is decided by
 * [SendPlan.typed] and the actions, not here. The Send button stays off until there is something to send.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TextComposeSheet(pcName: String, onDismiss: () -> Unit, onSend: (String) -> Unit) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var text by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    // One send per sheet: a second tap during the hide animation would queue the same text again.
    var sending by remember { mutableStateOf(false) }
    // The keyboard comes up with the sheet: the user opened it to type.
    LaunchedEffect(Unit) { focus.requestFocus() }
    fun close(then: () -> Unit = {}) {
        scope.launch { sheetState.hide() }.invokeOnCompletion {
            onDismiss()
            then()
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState, shape = MaterialTheme.shapes.extraLarge) {
        Column(
            modifier = Modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(start = 18.dp, end = 18.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SheetHeader(title = "Send text", subtitle = "To $pcName", onClose = { close() })
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                placeholder = { Text("Type or paste something") },
                minLines = 3,
                maxLines = 8,
                shape = MaterialTheme.shapes.large,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
            )
            Text(
                "Just one link? It is sent as a link.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 4.dp),
            )
            Button(
                onClick = {
                    if (!sending) {
                        sending = true
                        val toSend = text
                        close { onSend(toSend) }
                    }
                },
                enabled = !sending && SendPlan.typed(text) !is SendPlan.Typed.Empty,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(SendIcons.Send, contentDescription = null, modifier = Modifier.size(18.dp))
                Text("Send", modifier = Modifier.padding(start = 8.dp))
            }
        }
    }
}
