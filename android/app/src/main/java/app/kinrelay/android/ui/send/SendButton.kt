package app.kinrelay.android.ui.send

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.AppGraph
import app.kinrelay.android.link.LinkState

/**
 * The big "Send" button of Home and everything behind it: the sheet with the tiles, the text box and the two pickers. All the state lives
 * here, so Home only has to place it. The colours, shapes and type come from the theme.
 */
@Composable
fun SendButton(graph: AppGraph, context: Context, pcName: String, state: LinkState, modifier: Modifier = Modifier) {
    val actions = remember(graph, context) { SendActions(graph, context) }
    var sheetOpen by rememberSaveable { mutableStateOf(false) }
    var composeOpen by rememberSaveable { mutableStateOf(false) }

    val filePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris: List<Uri> -> actions.sendFiles(uris) }
    val mediaPicker = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris: List<Uri> -> actions.sendFiles(uris) }

    Surface(
        onClick = { sheetOpen = true },
        modifier = modifier.fillMaxWidth().height(64.dp),
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.primary,
        contentColor = MaterialTheme.colorScheme.onPrimary,
    ) {
        Row(
            modifier = Modifier.padding(start = 8.dp, end = 18.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(
                modifier = Modifier.size(48.dp).background(MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.16f), CircleShape),
                contentAlignment = Alignment.Center,
            ) { Icon(SendIcons.Send, contentDescription = null, modifier = Modifier.size(28.dp)) }
            Column(modifier = Modifier.weight(1f)) {
                Text("Send", style = MaterialTheme.typography.titleLarge)
                Text(
                    SendPlan.buttonSubtitle(state),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.8f),
                )
            }
            Icon(SendIcons.Chevron, contentDescription = null)
        }
    }

    if (sheetOpen) {
        SendSheet(
            pcName = pcName,
            status = SendPlan.sheetStatus(state),
            onDismiss = { sheetOpen = false },
            onText = { composeOpen = true },
            onPaste = actions::paste,
            onFile = { filePicker.launch(arrayOf("*/*")) },
            onMedia = { mediaPicker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) },
        )
    }
    if (composeOpen) {
        TextComposeSheet(pcName = pcName, onDismiss = { composeOpen = false }, onSend = actions::sendTyped)
    }
}
