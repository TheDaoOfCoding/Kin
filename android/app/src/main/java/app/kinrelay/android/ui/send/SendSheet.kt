package app.kinrelay.android.ui.send

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.kinrelay.android.ui.theme.KinTheme
import kotlinx.coroutines.launch

/**
 * The sheet behind the Send button: four tiles (Text, Paste, File, Media) and a Folder row that is not built yet. A tap on a tile closes the
 * sheet and starts what the tile does ([onText] opens the text box, [onPaste] reads the clipboard, [onFile] and [onMedia] open the system pickers).
 * [status] is the line under the title: whether it arrives now or waits.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendSheet(
    pcName: String,
    status: String,
    onDismiss: () -> Unit,
    onText: () -> Unit,
    onPaste: () -> Unit,
    onFile: () -> Unit,
    onMedia: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    // One tile per sheet: a second tap during the hide animation would paste twice or open a second picker.
    var acting by remember { mutableStateOf(false) }
    // Slide the sheet away first, then do what the tile says (a picker or the text box opens over the app, not over a half-closed sheet).
    fun choose(action: () -> Unit) {
        if (acting) return
        acting = true
        scope.launch { sheetState.hide() }.invokeOnCompletion {
            onDismiss()
            action()
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState, shape = MaterialTheme.shapes.extraLarge) {
        Column(
            modifier = Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 18.dp, end = 18.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SheetHeader(title = "Send to $pcName", subtitle = status, onClose = { scope.launch { sheetState.hide() }.invokeOnCompletion { onDismiss() } })
            // The feature tints: four pastels in Warm, one plain container in Simple (the tokens decide, not this file).
            val kin = KinTheme.colors
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                SendTile("Text", "A note or a link", SendIcons.Text, kin.tintPeach, kin.onTintPeach, kin.tintPeachIcon, Modifier.weight(1f)) { choose(onText) }
                SendTile("Paste", "From the clipboard", SendIcons.Paste, kin.tintLilac, kin.onTintLilac, kin.tintLilacIcon, Modifier.weight(1f)) { choose(onPaste) }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                SendTile("File", "Any type, any size", SendIcons.File, kin.tintSage, kin.onTintSage, kin.tintSageIcon, Modifier.weight(1f)) { choose(onFile) }
                SendTile("Media", "Photos and videos", SendIcons.Media, kin.tintHoney, kin.onTintHoney, kin.tintHoneyIcon, Modifier.weight(1f)) { choose(onMedia) }
            }
            ComingSoonRow("Folder", "Send a whole folder at once", SendIcons.Folder)
            Text(
                "Tip: in any app, tap Share, then Kin.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 4.dp),
            )
        }
    }
}

@Composable
internal fun SheetHeader(title: String, subtitle: String, onClose: () -> Unit) {
    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleLarge)
            Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        IconButton(onClick = onClose) { Icon(SendIcons.Close, contentDescription = "Close") }
    }
}

@Composable
private fun SendTile(title: String, subtitle: String, icon: ImageVector, container: Color, content: Color, iconTint: Color, modifier: Modifier, onClick: () -> Unit) {
    Surface(onClick = onClick, modifier = modifier, shape = MaterialTheme.shapes.extraLarge, color = container, contentColor = content) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Icon(icon, contentDescription = null, tint = iconTint, modifier = Modifier.size(28.dp))
            Column {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(subtitle, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

/** A feature that is planned and not built: shown so the user knows it is coming, and never clickable. */
@Composable
private fun ComingSoonRow(title: String, subtitle: String, icon: ImageVector) {
    Surface(
        modifier = Modifier.fillMaxWidth().alpha(0.6f).semantics { disabled() },
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceVariant,
        contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
    ) {
        Row(modifier = Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(28.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(subtitle, style = MaterialTheme.typography.bodySmall)
            }
            Text("Coming soon", style = MaterialTheme.typography.labelMedium)
        }
    }
}
