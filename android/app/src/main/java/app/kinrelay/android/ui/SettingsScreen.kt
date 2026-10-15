package app.kinrelay.android.ui

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.setup.SetupActions
import kotlinx.coroutines.launch

/**
 * Settings (milestone 1.4): the device name that goes in `hello`, the setup (onboarding) again, the notification channels, unpair, and About.
 * There is nothing for images-to-clipboard (a PC feature: the phone has no clipboard images), and no "open links automatically" (out of scope).
 */
@Composable
fun SettingsScreen(
    context: Context,
    pc: DeviceEntity?,
    deviceName: String,
    systemName: String,
    phoneShortId: String?,
    versionName: String,
    onSaveName: suspend (String) -> Unit,
    onOpenSetup: () -> Unit,
    onOpenDebug: () -> Unit,
    onUnpair: suspend () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    var name by rememberSaveable(deviceName) { mutableStateOf(deviceName) }
    var saved by remember { mutableStateOf(false) }
    var confirmUnpair by remember { mutableStateOf(false) }
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Settings", style = MaterialTheme.typography.titleLarge)

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Device name", style = MaterialTheme.typography.titleMedium)
                Text("This is how your PC shows this phone. It is sent each time Kin connects, so a change takes effect at the next connection.", style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(value = name, onValueChange = { name = it; saved = false }, label = { Text("Name (empty: $systemName)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { scope.launch { onSaveName(name); saved = true } }, enabled = name.trim() != deviceName) { Text("Save") }
                    if (saved) Text("Saved.", modifier = Modifier.padding(top = 10.dp))
                }
            }
        }

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Connection", style = MaterialTheme.typography.titleMedium)
                Text("If your PC stops receiving, or Kin disconnects when the screen is off, check the notification, Nearby devices and battery settings again.", style = MaterialTheme.typography.bodySmall)
                Button(onClick = onOpenSetup, modifier = Modifier.fillMaxWidth()) { Text("Fix connection reliability") }
            }
        }

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Notifications", style = MaterialTheme.typography.titleMedium)
                Text("Kin has three channels, so you can choose the sound and the importance of each.", style = MaterialTheme.typography.bodySmall)
                OutlinedButton(onClick = { SetupActions.openNotificationSettings(context) }, modifier = Modifier.fillMaxWidth()) { Text("Notification settings") }
                OutlinedButton(onClick = { SetupActions.openChannelSettings(context, NotificationChannels.LINK) }, modifier = Modifier.fillMaxWidth()) { Text("Connection status") }
                OutlinedButton(onClick = { SetupActions.openChannelSettings(context, NotificationChannels.ARRIVALS) }, modifier = Modifier.fillMaxWidth()) { Text("Arrivals: text, files, pings") }
                OutlinedButton(onClick = { SetupActions.openChannelSettings(context, NotificationChannels.TRANSFERS) }, modifier = Modifier.fillMaxWidth()) { Text("File transfers: progress and results") }
            }
        }

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Clipboard", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Text you copy on the PC reaches this phone's clipboard by itself. Sending this phone's clipboard to the PC is manual (the quick tile, " +
                        "or the notification button), because Android doesn't let apps read the clipboard in the background.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        if (pc != null) {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Paired PC", style = MaterialTheme.typography.titleMedium)
                    Text(pc.name)
                    OutlinedButton(onClick = { confirmUnpair = true }, modifier = Modifier.fillMaxWidth()) { Text("Unpair") }
                }
            }
        }

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("About", style = MaterialTheme.typography.titleMedium)
                Text("Kin $versionName")
                Text("Protocol version 1", style = MaterialTheme.typography.bodySmall)
                Text("This phone: ${phoneShortId?.let(DeviceIds::display) ?: "…"}", style = MaterialTheme.typography.bodySmall)
                pc?.let { Text("PC: ${DeviceIds.shortIdOfDeviceId(it.deviceId)?.let(DeviceIds::display) ?: "?"}", style = MaterialTheme.typography.bodySmall) }
                TextButton(onClick = onOpenDebug) { Text("Debug") }
            }
        }
    }
    if (confirmUnpair && pc != null) {
        AlertDialog(
            onDismissRequest = { confirmUnpair = false },
            title = { Text("Unpair from ${pc.name}?") },
            text = { Text("Text, files and the clipboard stop syncing until you pair again. Anything still waiting to be sent to ${pc.name} is dropped. ${pc.name} is told, if it's connected.") },
            confirmButton = { TextButton(onClick = { confirmUnpair = false; scope.launch { onUnpair() } }) { Text("Unpair") } },
            dismissButton = { TextButton(onClick = { confirmUnpair = false }) { Text("Cancel") } },
        )
    }
}
