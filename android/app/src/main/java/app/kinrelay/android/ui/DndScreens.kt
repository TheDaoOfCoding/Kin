package app.kinrelay.android.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.AppGraph
import app.kinrelay.android.dnd.DndText
import app.kinrelay.android.dnd.DndWire
import kotlinx.coroutines.launch

/**
 * Settings: "Focus and Do Not Disturb" (2B.5, design 4.2 and 4.3). Two switches, both **off** by default:
 * - "Silence this phone while my PC is in Focus", with "How much" (Priority only, the default, or Alarms only: never total silence). Turning it on with no access opens the system's **Do Not Disturb
 *   access** screen (the narrowest access: it can turn Do Not Disturb on and off and cannot read notifications; notification access, if already given for mirroring, covers it too). Nothing is
 *   asked at start-up. While Kin holds the phone silent the row says so, with "Release now".
 * - "Tell my PC when this phone is on Do Not Disturb": needs no access at all.
 * Both switches go through the feature resolver.
 */
@Composable
fun DndSettingsCard(graph: AppGraph, context: Context, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val view by graph.dnd.view.collectAsState()
    val peer by graph.presence.view.collectAsState()
    // Back from the system's access screen: look at the access again (idempotent).
    LaunchedEffect(Unit) { graph.dnd.refreshAccess() }
    val needsUpdate = peer.connected && !peer.peerListsPresence
    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Focus and Do Not Disturb", style = MaterialTheme.typography.titleMedium)
            SettingSwitch(DndText.FOLLOW_TITLE, DndText.FOLLOW_DESCRIPTION, view.followOn) { on ->
                scope.launch { graph.dndStore.setFollow(on) }
                // The access is asked for here, when the user turns the feature on and none exists yet (never at start-up).
                if (on && !view.access) openAccessScreen(context)
            }
            if (view.followOn) {
                Text("How much", style = MaterialTheme.typography.titleSmall)
                LevelRow(DndText.PRIORITY_LABEL, DndText.PRIORITY_NOTE, view.followFilter != DndWire.ALARMS) { scope.launch { graph.dndStore.setFollowFilter(DndWire.PRIORITY) } }
                LevelRow(DndText.ALARMS_LABEL, DndText.ALARMS_NOTE, view.followFilter == DndWire.ALARMS) { scope.launch { graph.dndStore.setFollowFilter(DndWire.ALARMS) } }
                Text(DndText.access(view.access, view.holding), style = MaterialTheme.typography.bodySmall, color = if (view.access) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.error)
                if (!view.access) {
                    Text(DndText.ACCESS_EXPLANATION, style = MaterialTheme.typography.bodySmall)
                    OutlinedButton(onClick = { openAccessScreen(context) }, modifier = Modifier.fillMaxWidth()) { Text(DndText.ACCESS_BUTTON) }
                }
                if (needsUpdate) Text(DndText.UPDATE_PC, style = MaterialTheme.typography.bodySmall)
            }
            if (view.holding) {
                // The level actually held (the record), not the "How much" setting, which may have changed since the request.
                Text(DndText.holding(view.route, view.heldFilter), style = MaterialTheme.typography.bodyMedium)
                OutlinedButton(onClick = { graph.dnd.releaseNow() }, modifier = Modifier.fillMaxWidth()) { Text("Release now") }
            }
            if (view.followOn || view.holding) Text(DndText.FORCE_STOP_NOTE, style = MaterialTheme.typography.bodySmall)
            SettingSwitch(DndText.SHARE_TITLE, DndText.SHARE_DESCRIPTION, view.shareOn) { on -> scope.launch { graph.dndStore.setShare(on) } }
        }
    }
}

@Composable
private fun LevelRow(label: String, note: String, selected: Boolean, onSelect: () -> Unit) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        RadioButton(selected = selected, onClick = onSelect)
        Column(modifier = Modifier.weight(1f)) {
            Text(label, style = MaterialTheme.typography.bodyLarge)
            Text(note, style = MaterialTheme.typography.bodySmall)
        }
    }
}

/** The system's "Do Not Disturb access" list (Kin is in it because it declares `ACCESS_NOTIFICATION_POLICY`); the narrowest access, never notification access. */
private fun openAccessScreen(context: Context) {
    val intent = Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(intent)
    } catch (_: ActivityNotFoundException) {
        // A phone with no such screen: the app's own settings page is the nearest place.
        app.kinrelay.android.setup.SetupActions.openAppDetails(context)
    }
}
