package app.kinrelay.android.ui

import android.content.Context
import android.graphics.BitmapFactory
import android.os.SystemClock
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.kinrelay.android.AppGraph
import app.kinrelay.android.PermissionSnapshot
import app.kinrelay.android.media.MediaCmdKind
import app.kinrelay.android.media.MediaUiText
import app.kinrelay.android.media.SliderSendGate
import app.kinrelay.android.notifmirror.RestrictedSettings
import app.kinrelay.android.ui.theme.ArtSlot
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.MediaArt
import kotlinx.coroutines.launch

/** The least time between two values a drag of the PC's volume slider sends: 5 in 10 s at most, a quarter of the sender's 20 per 10 s, so the last value has room. */
private const val SLIDER_SEND_GAP_MILLIS = 400L

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
            TitleWithHelp(
                "Media",
                "Control what plays on one device from the other: pause, skip and volume. Kin keeps no title, artist or picture, it only passes them on.",
                style = MaterialTheme.typography.titleMedium,
            )
            SettingSwitch(
                "Show this phone's media on your PC", ui.policy.share,
                help = "Lets your PC show and control what this phone plays. It needs notification access.",
                note = MediaUiText.shareLine(ui, access),
            ) { on ->
                scope.launch { graph.media.store.setShare(on) }
                if (on && !access) RestrictedSettings.openNotificationAccess(context)
            }
            if (ui.policy.share && !access) {
                RestrictedSettingsCard(context)
                OutlinedButton(onClick = { RestrictedSettings.openNotificationAccess(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification access") }
            }
            SettingSwitch(
                "Show your PC's media on this phone", ui.policy.show,
                help = "Shows what your PC plays on this phone, with pause, skip and volume controls.",
                note = MediaUiText.showLine(ui),
            ) { on -> scope.launch { graph.media.store.setShow(on) } }
            SettingSwitch(
                "Show PC media on the lock screen", ui.policy.showOnLockScreen,
                help = "Without this, Kin marks the PC's media as private, so a locked phone shows only \"Media from your PC\". Some phones show the controls anyway: that is the phone's own lock-screen setting.",
            ) { scope.launch { graph.media.store.setShowOnLockScreen(it) } }
            SettingSwitch(
                "Use the phone's volume keys for the PC", ui.policy.useVolumeKeys,
                help = "While your PC is playing, the volume keys change the PC's volume instead of this phone's.",
            ) { scope.launch { graph.media.store.setUseVolumeKeys(it) } }
        }
    }
}

/**
 * The card on Home while either direction is on. While the PC's media is shown it is one row (the album art or the record, the title, "on <PC>", previous, play/pause and next)
 * with the PC's volume slider under it; otherwise one line says what is going on. Draws nothing when both directions are off.
 */
@OptIn(ExperimentalMaterial3Api::class) // Slider.Track's gap and stop-indicator options
@Composable
fun MediaHomeCard(graph: AppGraph, permissions: PermissionSnapshot, modifier: Modifier = Modifier) {
    val ui by graph.media.ui.collectAsState()
    val line = MediaUiText.homeLine(ui, permissions.notificationAccess) ?: return
    val now = ui.pcNow
    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (now != null) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    ArtSlot(46.dp, 46.dp, shape = KinTheme.style.avatarShape) {
                        // Decoded once per picture: the hub keeps the same bytes until the source sends another.
                        val bitmap = remember(now.art) { now.art?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }?.asImageBitmap() }
                        if (bitmap != null) Image(bitmap, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.size(46.dp)) else MediaArt(Modifier.size(46.dp))
                    }
                    Column(Modifier.weight(1f)) {
                        Text(now.title, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text("on ${ui.pcName ?: "your PC"}", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    TransportButton(KinIcons.Prev, "Previous", now.canPrev) { graph.media.pcTransport(MediaCmdKind.PREV) }
                    TransportButton(if (now.playing) KinIcons.Pause else KinIcons.Play, if (now.playing) "Pause" else "Play", now.canPlayPause, main = true) { graph.media.pcPlayPause() }
                    TransportButton(KinIcons.Next, "Next", now.canNext) { graph.media.pcTransport(MediaCmdKind.NEXT) }
                }
            } else {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    ArtSlot(50.dp, 50.dp, shape = KinTheme.style.avatarShape) { MediaArt(Modifier.size(50.dp)) }
                    Column(Modifier.weight(1f)) {
                        Text("Media", style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold))
                        Text(line, style = MaterialTheme.typography.bodyMedium, color = KinTheme.colors.onSurfaceVariant)
                    }
                }
            }
            val volume = ui.pcVolume
            if (volume != null) {
                var level by remember(volume.level) { mutableFloatStateOf(volume.level.toFloat()) }
                // A drag sends at most a few values a second (the phone may send 20 commands per 10 s), and **the value where the finger lifts always goes**: a long drag can't lose it.
                val gate = remember { SliderSendGate(SLIDER_SEND_GAP_MILLIS) }
                Text("${ui.pcName ?: "PC"} volume${if (volume.muted) " (muted)" else ""}", style = MaterialTheme.typography.bodySmall)
                val colors = KinTheme.colors
                val sliderColors = SliderDefaults.colors(thumbColor = colors.primary, activeTrackColor = colors.primary, inactiveTrackColor = colors.surfaceContainerHigh)
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
                    colors = sliderColors,
                    // Material's own thumb is a tall thin bar; this is a plain round one in the theme colour.
                    thumb = { Box(Modifier.size(22.dp).shadow(2.dp, CircleShape).clip(CircleShape).background(colors.primary)) },
                    track = { state -> SliderDefaults.Track(state, colors = sliderColors, thumbTrackGapSize = 0.dp, drawStopIndicator = null) },
                )
            }
        }
    }
}

/** A round transport button of the media row: [main] (play/pause) is filled in the theme colour, the others are plain icons. Greyed and inert when the PC doesn't offer it. */
@Composable
private fun TransportButton(icon: ImageVector, description: String, enabled: Boolean, main: Boolean = false, size: Dp = if (main) 42.dp else 36.dp, onClick: () -> Unit) {
    val colors = KinTheme.colors
    Box(
        Modifier
            .size(size)
            .alpha(if (enabled) 1f else 0.38f)
            .clip(CircleShape)
            .let { if (main) it.background(colors.primary) else it }
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = description, tint = if (main) colors.onPrimary else colors.onSurface, modifier = Modifier.size(if (main) 22.dp else 20.dp))
    }
}
