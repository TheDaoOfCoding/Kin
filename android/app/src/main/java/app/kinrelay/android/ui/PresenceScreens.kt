package app.kinrelay.android.ui

import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
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
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinTint
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.tint
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.CompositionLocalProvider

/** What the "?" of the Pause card says: the long explanation the card itself no longer carries. */
private const val PAUSE_HELP =
    "Stops everything automatic (clipboard sync, mirroring, media, screenshots) for a while. " +
        "What you send yourself still arrives, quietly, in the history. Ringing still works."

/**
 * The Pause card (design 5.3) as one row: the pause icon, "Pause Kin", a "?" and one "Pause" button that opens the three choices. While paused the row says when it ends
 * ("Paused until 18:30") and its button is "Resume"; how many things arrived quietly meanwhile, and what the PC says about its own pause, show as small lines under it. Used on Home and
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
    var menu by remember { mutableStateOf(false) }
    val small = KinTheme.colors.tint(KinTint.HONEY).onContainer
    KinCard(modifier.fillMaxWidth(), tint = KinTint.HONEY) {
        Column(Modifier.padding(start = 14.dp, top = 10.dp, end = 10.dp, bottom = 10.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                TintSlot(KinIcons.Pausec, KinTint.HONEY, onCard = true)
                TitleWithHelp(
                    if (view.paused) PauseWords.pausedLine(PauseSupport.sentence(context, view.state)) else "Pause Kin",
                    PAUSE_HELP, Modifier.weight(1f),
                    style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold),
                )
                if (view.paused) {
                    KinButton("Resume", { scope.launch { graph.pause.resume() } }, style = KinButtonStyle.WARN, small = true)
                } else {
                    Box {
                        KinButton("Pause", { menu = true }, style = KinButtonStyle.PLAIN, small = true)
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            PauseChoice.entries.forEach { choice ->
                                DropdownMenuItem(text = { Text(PauseWords.choiceLabel(choice)) }, onClick = { menu = false; scope.launch { graph.pause.set(choice) } })
                            }
                        }
                    }
                }
            }
            if (view.paused) PauseWords.receivedWhilePaused(view.receivedWhilePaused)?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = small) }
            if (peer.connected && peer.peerPaused) {
                val until = peer.peerPausedUntilWall?.let { PauseSupport.time(context, it) }
                Text(if (peer.peerPausedUntilResumed || until == null) "Paused on the PC" else "Paused on the PC until $until", style = MaterialTheme.typography.bodySmall, color = small)
            }
        }
    }
}

/** The PC's status under its name on Home: what the persistent notification says, in the same words (the PC's pause, a laptop's battery). Empty when there is nothing to say. */
@Composable
fun PcStatusLine(graph: AppGraph, context: Context) {
    val peer by graph.presence.view.collectAsState()
    // 2B.5: while Kin holds Do Not Disturb for the PC's Focus, Home says so (and Settings has "Release now").
    val dnd by graph.dnd.view.collectAsState()
    if (dnd.holding) Text(app.kinrelay.android.dnd.DndText.holding(dnd.route, dnd.heldFilter), style = MaterialTheme.typography.bodyMedium)
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
    KinCard(modifier = modifier.fillMaxWidth(), container = KinTheme.colors.warningContainer) {
        CompositionLocalProvider(LocalContentColor provides KinTheme.colors.onWarningContainer) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Screenshots aren't being sent", style = MaterialTheme.typography.titleMedium)
                if (ui.needsAccess) {
                    Text("Needs access to all photos", style = MaterialTheme.typography.bodyLarge)
                    Text("Android gave Kin no access or only selected photos, which can't work in the background.", style = MaterialTheme.typography.bodySmall)
                    KinButton("Open Kin's settings", { app.kinrelay.android.setup.SetupActions.openAppDetails(context) }, style = KinButtonStyle.WARN, small = true)
                }
                if (ui.needsNotifications) {
                    Text("Needs notifications", style = MaterialTheme.typography.bodyLarge)
                    Text("Kin can't show the 5-second Cancel notification, so it sends nothing.", style = MaterialTheme.typography.bodySmall)
                    KinButton("Open notification settings", { app.kinrelay.android.setup.SetupActions.openNotificationSettings(context) }, style = KinButtonStyle.WARN, small = true)
                }
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
    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Status, pause and ring", style = MaterialTheme.typography.titleMedium)
            SettingSwitch(
                "Share this phone's battery with your PC", share,
                help = "Your PC's tray shows the level and whether it is charging, and warns you when it is low. Only your paired PC sees it. Kin keeps no history of it.",
            ) { scope.launch { graph.presenceStore.setShare(it) } }
            SettingSwitch(
                "Show your PC's status here", show,
                help = "A laptop's battery and whether Kin is paused on the PC show in the Kin notification and on Home.",
            ) { scope.launch { graph.presenceStore.setShow(it) } }
            TitleWithHelp("Pause", "Pause Kin from the quick settings tile, the notification or Home: 1 hour, until tomorrow 08:00, or until you resume.")
            OutlinedButton(
                onClick = { if (!PauseSupport.requestAddTile(context)) tileHint = true },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("Add the Pause tile") }
            if (tileHint) Text("Swipe down twice, tap the pencil (edit), and drag \"Pause Kin\" in.", style = MaterialTheme.typography.bodySmall)
            SettingSwitch(
                "Let my PC ring this phone", ringAllow,
                help = "Your PC can play a loud ring on the alarm volume, even when the phone is silent, to help you find it. It stops by itself after 30 seconds, and the Stop button is always there. It works while Kin is paused.",
            ) { scope.launch { graph.presenceStore.setRingAllow(it) } }
            if (ringAllow) {
                SettingSwitch(
                    "Ring at maximum volume", ringLoud,
                    help = "The alarm volume is raised for the ring and put back afterwards. Without this, the ring uses your alarm volume.",
                ) { scope.launch { graph.presenceStore.setRingLoud(it) } }
                if (!graph.ring.canFullScreen()) {
                    TitleWithHelp(
                        "Full Stop screen",
                        "On a locked phone Android shows the ring as a notification with a Stop button. Allow full-screen notifications for Kin to show the full Stop screen instead.",
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

    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Screenshots", style = MaterialTheme.typography.titleMedium)
            SettingSwitch(
                "Send screenshots to your PC", ui.screenshots,
                help = "Kin sends only screenshots taken after you turn this on, to your PC only. Each one waits 5 seconds with a Cancel button before it leaves the phone. " +
                    "On the PC it goes to Downloads\\Kin and onto the clipboard (kept out of Windows clipboard history). Apps that save screenshots somewhere else aren't seen.",
            ) { on ->
                if (on) pendingScreenshots = true else scope.launch { graph.shots.turnOff(screenshots = false, camera = null) }
            }
            SettingSwitch(
                "Also send new camera photos", ui.camera,
                help = "Photos you take from now on go to Downloads\\Kin on your PC, never onto its clipboard.",
            ) { on ->
                if (on) pendingScreenshots = false else scope.launch { graph.shots.turnOff(screenshots = null, camera = false) }
            }
            if (ui.needsAccess) {
                Text("Needs access to all photos", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.error)
                Text("Android gave Kin no access or only selected photos. Allow \"All photos\" for Kin.", style = MaterialTheme.typography.bodySmall)
                OutlinedButton(onClick = { app.kinrelay.android.setup.SetupActions.openAppDetails(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open Kin's settings") }
            }
            if (ui.needsNotifications) {
                Text("Needs notifications", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.error)
                Text("Allow Kin's notifications, including File transfers, so the 5-second Cancel can show.", style = MaterialTheme.typography.bodySmall)
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
