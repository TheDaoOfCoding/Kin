package app.kinrelay.android.ui

import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import app.kinrelay.android.AppGraph
import app.kinrelay.android.PermissionSnapshot
import app.kinrelay.android.features.FeatureIds
import app.kinrelay.android.pause.PauseChoice
import app.kinrelay.android.pause.PauseSupport
import app.kinrelay.android.pause.PauseWords
import app.kinrelay.android.presence.PresenceText
import kotlinx.coroutines.launch

/**
 * The Pause card (design 5.3): the three choices and Resume, with the end of the pause, how many things arrived quietly meanwhile, and what the PC says about its own pause. Used on Home and
 * by the tile's long-press screen ([app.kinrelay.android.pause.PauseSettingsActivity]).
 */
@Composable
fun PauseCard(graph: AppGraph, context: Context, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    // The published view says when to look again; what it shows is the wall clock now (a pause that ended while the phone slept shows as ended).
    graph.pause.view.collectAsState().value
    LaunchedEffect(Unit) { graph.pause.refresh() }
    val view = graph.pause.viewNow()
    val peer by graph.presence.view.collectAsState()
    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Pause Kin", style = MaterialTheme.typography.titleMedium)
            if (view.paused) {
                Text(PauseSupport.sentence(context, view.state), style = MaterialTheme.typography.bodyLarge)
                PauseWords.receivedWhilePaused(view.receivedWhilePaused)?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                Text(
                    "Clipboard sync, notification mirroring, media and screenshots are off. Text, links and files you send still arrive here, quietly, in the history. Ringing still works.",
                    style = MaterialTheme.typography.bodySmall,
                )
                Button(onClick = { scope.launch { graph.pause.resume() } }, modifier = Modifier.fillMaxWidth()) { Text("Resume") }
            } else {
                Text(
                    "Stops everything automatic (clipboard sync, mirroring, media, screenshots) for a while. What you send yourself still arrives, quietly.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PauseChoice.entries.forEach { choice ->
                    OutlinedButton(onClick = { scope.launch { graph.pause.set(choice) } }, contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 10.dp)) {
                        Text(PauseWords.choiceLabel(choice), style = MaterialTheme.typography.labelMedium)
                    }
                }
            }
            if (peer.connected && peer.peerPaused) {
                val until = peer.peerPausedUntilWall?.let { PauseSupport.time(context, it) }
                Text(if (peer.peerPausedUntilResumed || until == null) "Paused on the PC" else "Paused on the PC until $until", style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

/** The PC's status under its name on Home: what the persistent notification says, in the same words (the PC's pause, a laptop's battery). Empty when there is nothing to say. */
@Composable
fun PcStatusLine(graph: AppGraph, context: Context) {
    val peer by graph.presence.view.collectAsState()
    if (!peer.connected || !peer.showOn) return
    val pause = if (peer.peerPaused) {
        app.kinrelay.android.presence.PeerPauseText(peer.peerPausedUntilResumed, peer.peerPausedUntilWall?.let { PauseSupport.time(context, it) })
    } else {
        null
    }
    val line = listOfNotNull(peer.battery?.let { "PC battery " + PresenceText.battery(it) }, pause?.let { PresenceText.pause(it) }).joinToString(" · ")
    if (line.isNotEmpty()) Text(line, style = MaterialTheme.typography.bodyMedium)
}

/** Home: a card only while auto-send is on and Android doesn't give Kin all photos (design 5.6: "Needs access to all photos"; a partial or revoked grant can't work in the background). */
@Composable
fun ShotsHomeCard(graph: AppGraph, context: Context, permissions: PermissionSnapshot, modifier: Modifier = Modifier) {
    val ui by graph.shots.ui.collectAsState()
    LaunchedEffect(permissions.photos, permissions.notifications) { graph.shots.reevaluate() }
    if (!ui.available || !(ui.needsAccess || ui.needsNotifications)) return
    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Screenshots aren't being sent", style = MaterialTheme.typography.titleMedium)
            if (ui.needsAccess) {
                Text("Needs access to all photos", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.error)
                Text("Android gave Kin no access or only selected photos, which can't work in the background.", style = MaterialTheme.typography.bodySmall)
                OutlinedButton(onClick = { app.kinrelay.android.setup.SetupActions.openAppDetails(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open Kin's settings") }
            }
            if (ui.needsNotifications) {
                Text("Needs notifications", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.error)
                Text("Kin can't show the 5-second Cancel notification, so it sends nothing.", style = MaterialTheme.typography.bodySmall)
                OutlinedButton(onClick = { app.kinrelay.android.setup.SetupActions.openNotificationSettings(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification settings") }
            }
        }
    }
}

/**
 * Settings: "Presence and ring" (design 5.2, 5.4): share the battery, show the PC's status, let the PC ring this phone (and how loud), the Pause tile, and the full-screen Stop screen on a locked
 * phone. Every switch goes through the feature resolver.
 */
@Composable
fun PresenceSettingsCard(graph: AppGraph, context: Context, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val share by graph.features.flow(FeatureIds.PRESENCE_SHARE).collectAsState(initial = true)
    val show by graph.features.flow(FeatureIds.PRESENCE_SHOW).collectAsState(initial = true)
    val ringAllow by graph.features.flow(FeatureIds.RING_ALLOW).collectAsState(initial = true)
    val ringLoud by graph.presenceStore.ringLoud.collectAsState(initial = true)
    var tileHint by remember { mutableStateOf(false) }
    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Status, pause and ring", style = MaterialTheme.typography.titleMedium)
            SettingSwitch(
                "Share this phone's battery with your PC",
                "Your PC's tray shows the level and whether it is charging, and warns you when it is low. Only your paired PC sees it; Kin keeps no history of it.",
                share,
            ) { scope.launch { graph.presenceStore.setShare(it) } }
            SettingSwitch(
                "Show your PC's status here",
                "A laptop's battery and whether Kin is paused on the PC show in the Kin notification and on Home.",
                show,
            ) { scope.launch { graph.presenceStore.setShow(it) } }
            Text("Pause", style = MaterialTheme.typography.titleSmall)
            Text(
                "Pause Kin from the quick settings tile, the notification (\"Pause 1 h\") or Home: 1 hour, until tomorrow 08:00, or until you resume.",
                style = MaterialTheme.typography.bodySmall,
            )
            OutlinedButton(
                onClick = { if (!PauseSupport.requestAddTile(context)) tileHint = true },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("Add the Pause tile") }
            if (tileHint) Text("Swipe down twice, tap the pencil (edit), and drag \"Pause Kin\" in.", style = MaterialTheme.typography.bodySmall)
            Text("Ring my phone", style = MaterialTheme.typography.titleSmall)
            SettingSwitch(
                "Let my PC ring this phone",
                "Your PC can play a loud ring on the alarm volume, even when the phone is silent, to help you find it. It stops by itself after 30 seconds, and the Stop button is always there. It works while Kin is paused.",
                ringAllow,
            ) { scope.launch { graph.presenceStore.setRingAllow(it) } }
            if (ringAllow) {
                SettingSwitch(
                    "Ring at maximum volume",
                    "On: the alarm volume is raised for the ring and put back afterwards. Off: it uses your alarm volume.",
                    ringLoud,
                ) { scope.launch { graph.presenceStore.setRingLoud(it) } }
                if (!graph.ring.canFullScreen()) {
                    Text(
                        "On a locked phone Android shows the ring as a notification with a Stop button. To show the full Stop screen instead, allow full-screen notifications for Kin.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                    OutlinedButton(
                        onClick = {
                            context.startActivity(
                                Intent(Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT, "package:${context.packageName}".toUri()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                            )
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text("Show the Stop screen on a locked phone") }
                }
            }
        }
    }
}

/**
 * Settings: "Screenshots" (design 5.6): off by default. Turning a switch on says what is read, asks for the photo permission **now**, takes a baseline (so nothing that already exists is sent) and
 * refuses if the PC cannot keep a screenshot out of its clipboard history. A grant that is partial or revoked shows "Needs access to all photos".
 */
@Composable
fun ShotsSettingsCard(graph: AppGraph, context: Context, permissions: PermissionSnapshot, modifier: Modifier = Modifier) {
    val ui by graph.shots.ui.collectAsState()
    if (!ui.available) return
    val scope = rememberCoroutineScope()
    val peer by graph.presence.view.collectAsState()
    var confirmKind by remember { mutableStateOf<Boolean?>(null) } // true: screenshots, false: camera photos
    var message by remember { mutableStateOf<String?>(null) }
    // Back from the system's permission screen (or a revoked grant): look again, never trust an old answer.
    LaunchedEffect(permissions.photos, permissions.notifications) { graph.shots.reevaluate() }

    fun turnOn(screenshots: Boolean) {
        scope.launch {
            // The 5-second hold with its Cancel must be visible, or nothing would be cancellable: without notifications the switch stays off.
            if (!graph.shots.holdVisible()) {
                message = "Needs notifications: allow Kin's notifications (including File transfers) so the 5-second Cancel can show."
                graph.shots.reevaluate()
                return@launch
            }
            val ok = graph.shots.turnOn(screenshots = if (screenshots) true else null, camera = if (screenshots) null else true)
            message = if (ok) null else "Kin couldn't turn this on. Check that your PC is connected and updated to Kin 0.2.0."
        }
    }

    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val screenshots = confirmKind
        confirmKind = null
        if (!granted || screenshots == null) {
            message = "Kin needs access to all photos to see new screenshots."
            graph.shots.reevaluate()
        } else {
            turnOn(screenshots)
        }
    }
    var pendingScreenshots by remember { mutableStateOf<Boolean?>(null) }

    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Screenshots", style = MaterialTheme.typography.titleMedium)
            SettingSwitch(
                "Send screenshots to your PC",
                "Kin watches for new screenshots and sends only ones taken after you turn this on, to your PC only. A screenshot waits 5 seconds with a Cancel button before it leaves the phone. " +
                    "On the PC it goes to Downloads\\Kin and onto the clipboard (kept out of Windows clipboard history). It can't see apps that save screenshots somewhere else.",
                ui.screenshots,
            ) { on ->
                if (on) pendingScreenshots = true else scope.launch { graph.shots.turnOff(screenshots = false, camera = null) }
            }
            SettingSwitch(
                "Also send new camera photos",
                "Photos you take from now on go to Downloads\\Kin on your PC, never onto its clipboard.",
                ui.camera,
            ) { on ->
                if (on) pendingScreenshots = false else scope.launch { graph.shots.turnOff(screenshots = null, camera = false) }
            }
            if (ui.needsAccess) {
                Text("Needs access to all photos", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.error)
                Text(
                    "Android gave Kin no access or only selected photos, which can't work in the background. Allow \"All photos\" for Kin.",
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedButton(onClick = { app.kinrelay.android.setup.SetupActions.openAppDetails(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open Kin's settings") }
            }
            if (ui.needsNotifications) {
                Text("Needs notifications", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.error)
                Text(
                    "Kin can't show the 5-second \"Sending screenshot\" notification with its Cancel button, so it sends nothing. Allow Kin's notifications, including the File transfers channel.",
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedButton(onClick = { app.kinrelay.android.setup.SetupActions.openNotificationSettings(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification settings") }
            }
            if (peer.connected && !peer.peerListsPresence) {
                Text("Your PC needs Kin 0.2.0 to keep screenshots out of its clipboard history.", style = MaterialTheme.typography.bodySmall)
            }
            message?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
        }
    }
    pendingScreenshots?.let { screenshots ->
        AlertDialog(
            onDismissRequest = { pendingScreenshots = null },
            title = { Text(if (screenshots) "Send screenshots to your PC?" else "Send camera photos to your PC?") },
            text = {
                Text(
                    if (screenshots) {
                        "Kin will read the photo library to see new screenshots. It sends only screenshots taken after you turn this on, to your PC only, after a 5-second wait with a Cancel button. " +
                            "Anything on the screen, a password or a code, can end up in the PC's Downloads\\Kin folder and on its clipboard until you copy something else."
                    } else {
                        "Kin will read the photo library to see new camera photos. It sends only photos taken after you turn this on, to your PC only, after a 5-second wait with a Cancel button."
                    },
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    pendingScreenshots = null
                    if (permissions.photos) {
                        // Already granted: no permission dialog, so no pending kind to remember (a stale one would turn on the wrong switch on a later result).
                        confirmKind = null
                        turnOn(screenshots)
                    } else {
                        confirmKind = screenshots
                        ask.launch(graph.shots.permissionToRequest())
                    }
                }) { Text("Turn on") }
            },
            dismissButton = { TextButton(onClick = { pendingScreenshots = null }) { Text("Cancel") } },
        )
    }
}
