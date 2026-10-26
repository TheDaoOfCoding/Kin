package app.kinrelay.android.ui

import android.content.Context
import android.os.SystemClock
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.AppGraph
import app.kinrelay.android.PermissionSnapshot
import app.kinrelay.android.media.MediaUiText
import app.kinrelay.android.media.SliderSendGate
import app.kinrelay.android.notifmirror.RestrictedSettings
import kotlinx.coroutines.launch
import app.kinrelay.android.ui.theme.ArtSlot
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.MediaArt
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.layout.size

/** The least time between two values a drag of the PC's volume slider sends: 5 in 10 s at most, a quarter of the sender's 20 per 10 s, so the last value has room. */
private const val SLIDER_SEND_GAP_MILLIS = 400L

/** A switch with a title and a line under it (the same look as the mirror card). */
@Composable
private fun MediaSwitch(title: String, line: String, checked: Boolean, onChecked: (Boolean) -> Unit) {
    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(line, style = MaterialTheme.typography.bodySmall)
        }
        Switch(checked = checked, onCheckedChange = onChecked)
    }
}

/**
 * The "Media" card of Settings (design 4.7, user decisions 1 to 3): four switches, all off by default. "Show this phone's media on your PC" is one switch for every app. Reading other
 * apps' media needs the same notification access as mirroring (no mirroring switch needed): turning the switch on without it opens the system's access screen and shows the
 * restricted-setting steps of 2B.2 until access is there.
 */
@Composable
fun MediaSettingsCard(graph: AppGraph, context: Context, permissions: PermissionSnapshot, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val ui by graph.media.ui.collectAsState()
    val access = permissions.notificationAccess
    // Back from the system's access screen: the hub looks again.
    LaunchedEffect(access) { graph.media.refreshAccess() }
    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Media", style = MaterialTheme.typography.titleMedium)
            Text(
                "Controls for what plays on one device, on the other: pause, skip and volume. Both are off until you turn them on. Kin keeps no title, artist or picture: it only passes them on.",
                style = MaterialTheme.typography.bodySmall,
            )
            MediaSwitch(
                "Show this phone's media on your PC", MediaUiText.shareLine(ui, access), ui.policy.share,
            ) { on ->
                scope.launch { graph.media.store.setShare(on) }
                if (on && !access) RestrictedSettings.openNotificationAccess(context)
            }
            if (ui.policy.share && !access) {
                RestrictedSettingsCard(context)
                OutlinedButton(onClick = { RestrictedSettings.openNotificationAccess(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification access") }
            }
            MediaSwitch("Show your PC's media on this phone", MediaUiText.showLine(ui), ui.policy.show) { on -> scope.launch { graph.media.store.setShow(on) } }
            MediaSwitch(
                "Show PC media on the lock screen",
                "Off: Kin marks the PC's media as private, so a locked phone should show only \"Media from your PC\" when Android is set to hide sensitive notification content. Some phones show the controls anyway: that is the phone's own lock-screen setting. On: it shows the title too.",
                ui.policy.showOnLockScreen,
            ) { scope.launch { graph.media.store.setShowOnLockScreen(it) } }
            MediaSwitch(
                "Use the phone's volume keys for the PC",
                "On: while your PC is playing, the volume keys change the PC's volume instead of this phone's.",
                ui.policy.useVolumeKeys,
            ) { scope.launch { graph.media.store.setUseVolumeKeys(it) } }
        }
    }
}

/** The row on Home while either direction is on: one line, and the PC's volume slider while its media is shown. Draws nothing when both are off. */
@Composable
fun MediaHomeCard(graph: AppGraph, permissions: PermissionSnapshot, modifier: Modifier = Modifier) {
    val ui by graph.media.ui.collectAsState()
    val line = MediaUiText.homeLine(ui, permissions.notificationAccess) ?: return
    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                ArtSlot(50.dp, 50.dp, shape = KinTheme.style.avatarShape) { MediaArt(Modifier.size(50.dp)) }
                Column(Modifier.weight(1f)) {
                    Text("Media", style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold))
                    Text(line, style = MaterialTheme.typography.bodyMedium, color = KinTheme.colors.onSurfaceVariant)
                }
            }
            val volume = ui.pcVolume
            if (volume != null) {
                var level by remember(volume.level) { mutableFloatStateOf(volume.level.toFloat()) }
                // A drag sends at most a few values a second (the phone may send 20 commands per 10 s), and **the value where the finger lifts always goes**: a long drag can't lose it.
                val gate = remember { SliderSendGate(SLIDER_SEND_GAP_MILLIS) }
                Text("${ui.pcName ?: "PC"} volume${if (volume.muted) " (muted)" else ""}", style = MaterialTheme.typography.bodySmall)
                Slider(
                    value = level,
                    onValueChange = {
                        level = it
                        if (gate.mayCommit(SystemClock.elapsedRealtime())) graph.media.setPcVolume(it.toInt())
                    },
                    onValueChangeFinished = {
                        gate.reset()
                        graph.media.setPcVolume(level.toInt())
                    },
                    valueRange = 0f..100f,
                )
            }
        }
    }
}
