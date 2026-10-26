package app.kinrelay.android.ui

import android.content.Context
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.data.Appearance
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.data.KinSkin
import app.kinrelay.android.data.KinThemeMode
import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.notify.NotificationChannels
import app.kinrelay.android.setup.SetupActions
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinPalette
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.KinTint
import app.kinrelay.android.ui.theme.lipBehind
import app.kinrelay.android.ui.theme.tint
import kotlinx.coroutines.launch

/**
 * A setting with a switch and a short title. [help] is the explanation, behind a small "?" next to the title; [note] is a line of live state
 * under the title, only for something the user must act on or wait for (the switch already shows on and off).
 */
@Composable
internal fun SettingSwitch(title: String, checked: Boolean, help: String? = null, note: String? = null, onChecked: (Boolean) -> Unit) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(modifier = Modifier.weight(1f)) {
            TitleWithHelp(title, help)
            if (note != null) Text(note, style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant)
        }
        Switch(checked = checked, onCheckedChange = onChecked, colors = kinSwitchColors())
    }
}

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
    showLinkPreviewsOnLockScreen: Boolean,
    copyReceivedTextAutomatically: Boolean,
    onShowLinkPreviewsOnLockScreen: (Boolean) -> Unit,
    onCopyReceivedTextAutomatically: (Boolean) -> Unit,
    onSaveName: suspend (String) -> Unit,
    onOpenSetup: () -> Unit,
    onOpenDebug: () -> Unit,
    onUnpair: suspend () -> Unit,
    appearance: Appearance,
    onSkin: (KinSkin) -> Unit,
    onThemeMode: (KinThemeMode) -> Unit,
    modifier: Modifier = Modifier,
    /** Room left under the last card for the floating nav. */
    bottomPadding: Dp = 0.dp,
    /** "Show notifications on your PC" (2B.2). */
    mirrorCard: @Composable () -> Unit = {},
    /** "Media" (2B.3): the four switches. */
    mediaCard: @Composable () -> Unit = {},
    /** "Status, pause and ring" (2B.4). */
    presenceCard: @Composable () -> Unit = {},
    /** "Screenshots" (2B.4): auto-send, opt-in. Empty in a build without it. */
    shotsCard: @Composable () -> Unit = {},
    /** "Focus and Do Not Disturb" (2B.5): both switches off by default. */
    dndCard: @Composable () -> Unit = {},
) {
    val scope = rememberCoroutineScope()
    var name by rememberSaveable(deviceName) { mutableStateOf(deviceName) }
    var justSaved by remember { mutableStateOf(false) }
    var confirmUnpair by remember { mutableStateOf(false) }
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        PageTitle("Settings", Modifier.padding(top = 12.dp, bottom = 2.dp))

        GroupLabel("Appearance")
        AppearanceCard(appearance, onSkin, onThemeMode)

        KinCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                TitleWithHelp("Device name", "This is how your PC shows this phone. A change takes effect the next time Kin connects.", style = MaterialTheme.typography.titleMedium)
                OutlinedTextField(value = name, onValueChange = { name = it; justSaved = false }, label = { Text("Name (empty: $systemName)") }, singleLine = true, shape = KinTheme.style.shapes.small, modifier = Modifier.fillMaxWidth())
                // Nothing to save, so no button; after a save it stays as a disabled "Saved" until the next edit.
                val save = nameSaveState(name, deviceName, justSaved)
                save.label?.let { KinButton(it, { scope.launch { onSaveName(name); justSaved = true } }, enabled = save == NameSaveState.SAVE) }
            }
        }

        KinCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Connection", style = MaterialTheme.typography.titleMedium)
                Text("If your PC stops receiving, check these settings again.", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant)
                KinButton("Fix connection reliability", onOpenSetup, block = true)
            }
        }

        KinCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Notifications", style = MaterialTheme.typography.titleMedium)
                Text("Choose the sound and importance of each kind.", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant)
                KinButton("Notification settings", { SetupActions.openNotificationSettings(context) }, style = KinButtonStyle.PLAIN, block = true)
                KinButton("Connection status", { SetupActions.openChannelSettings(context, NotificationChannels.LINK) }, style = KinButtonStyle.PLAIN, block = true)
                KinButton("Arrivals: text, files, pings", { SetupActions.openChannelSettings(context, NotificationChannels.ARRIVALS) }, style = KinButtonStyle.PLAIN, block = true)
                KinButton("File transfers: progress and results", { SetupActions.openChannelSettings(context, NotificationChannels.TRANSFERS) }, style = KinButtonStyle.PLAIN, block = true)
                KinButton("From your PC: alerts and its battery", { SetupActions.openChannelSettings(context, NotificationChannels.ALERTS) }, style = KinButtonStyle.PLAIN, block = true)
            }
        }

        mirrorCard()

        mediaCard()

        presenceCard()

        dndCard()

        shotsCard()

        KinCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                TitleWithHelp(
                    "Links and text from your PC",
                    "A link from your PC opens with one tap on its notification. Kin never opens or copies one by itself.",
                    style = MaterialTheme.typography.titleMedium,
                )
                SettingSwitch(
                    "Show link previews on the lock screen",
                    showLinkPreviewsOnLockScreen,
                    help = "Without this, a locked phone shows only \"Link from your PC\", with no page title or address. Android's own lock-screen setting for sensitive notifications can hide them too.",
                    onChecked = onShowLinkPreviewsOnLockScreen,
                )
                SettingSwitch(
                    "Copy received text automatically",
                    copyReceivedTextAutomatically,
                    help = "Text you send from your PC goes straight to this phone's clipboard. Without this, the notification has a Copy button.",
                    onChecked = onCopyReceivedTextAutomatically,
                )
            }
        }

        if (pc != null) {
            KinCard(Modifier.fillMaxWidth(), onClick = { confirmUnpair = true }) {
                Row(Modifier.padding(horizontal = 14.dp, vertical = 11.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconSlot(KinIcons.Unlink, KinTheme.colors.onErrorContainer, KinTheme.colors.errorContainer, size = 40.dp)
                    Column(Modifier.weight(1f)) {
                        Text("Unlink ${pc.name}", style = MaterialTheme.typography.titleSmall.copy(fontSize = 15.sp), color = KinTheme.colors.error)
                        Text("You can pair again any time", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant)
                    }
                    Icon(KinIcons.ChevR, contentDescription = null, tint = KinTheme.colors.onSurfaceSubtle, modifier = Modifier.size(20.dp))
                }
            }
        }

        KinCard(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("About", style = MaterialTheme.typography.titleMedium)
                Text("Kin $versionName")
                Text("Protocol version 1", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant)
                Text("This phone: ${phoneShortId?.let(DeviceIds::display) ?: "…"}", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant)
                pc?.let { Text("PC: ${DeviceIds.shortIdOfDeviceId(it.deviceId)?.let(DeviceIds::display) ?: "?"}", style = MaterialTheme.typography.bodySmall, color = KinTheme.colors.onSurfaceVariant) }
                TextButton(onClick = onOpenDebug) { Text("Debug", color = KinTheme.colors.primaryText) }
            }
        }
        Spacer(Modifier.height(bottomPadding))
    }
    if (confirmUnpair && pc != null) {
        AlertDialog(
            onDismissRequest = { confirmUnpair = false },
            title = { Text("Unlink from ${pc.name}?") },
            text = { Text(UiText.unlinkText(pc.name)) },
            confirmButton = { TextButton(onClick = { confirmUnpair = false; scope.launch { onUnpair() } }) { Text("Unlink", color = KinTheme.colors.error) } },
            dismissButton = { TextButton(onClick = { confirmUnpair = false }) { Text("Cancel") } },
        )
    }
}

/** "Appearance": which skin, and light, dark or the phone's own. Both apply at once and are kept. */
@Composable
private fun AppearanceCard(appearance: Appearance, onSkin: (KinSkin) -> Unit, onThemeMode: (KinThemeMode) -> Unit) {
    val dark = KinTheme.style.dark
    KinCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 14.dp, top = 14.dp, end = 14.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Skin", style = MaterialTheme.typography.titleSmall.copy(fontSize = 14.sp), modifier = Modifier.padding(horizontal = 2.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SkinChoice("Warm", "Friendly, the default", KinSkin.WARM, dark, appearance.skin == KinSkin.WARM, { onSkin(KinSkin.WARM) }, Modifier.weight(1f))
                SkinChoice("Simple", "Plain and quiet", KinSkin.SIMPLE, dark, appearance.skin == KinSkin.SIMPLE, { onSkin(KinSkin.SIMPLE) }, Modifier.weight(1f))
            }
            Spacer(Modifier.height(6.dp))
            Text("Theme", style = MaterialTheme.typography.titleSmall.copy(fontSize = 14.sp), modifier = Modifier.padding(horizontal = 2.dp))
            KinSegmented(
                listOf(
                    Triple(KinThemeMode.SYSTEM, "System", KinIcons.Auto),
                    Triple(KinThemeMode.LIGHT, "Light", KinIcons.Sun),
                    Triple(KinThemeMode.DARK, "Dark", KinIcons.Moon),
                ),
                appearance.mode, onThemeMode,
            )
        }
    }
}

/** One skin to pick: a small preview drawn in that skin's own colours and corners (in the light or dark the app shows now), and its name. */
@Composable
private fun SkinChoice(name: String, hint: String, skin: KinSkin, dark: Boolean, selected: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val here = KinTheme.colors
    val c = KinPalette.colors(skin, dark)
    val look = KinPalette.style(skin, dark)
    val shape = look.shapes.medium
    val ring = if (selected) BorderStroke(2.dp, here.primaryText) else BorderStroke(1.5.dp, here.outlineVariant)
    Column(
        modifier.clip(shape).border(ring, shape).selectable(selected = selected, role = Role.RadioButton, onClick = onClick).padding(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(
            Modifier.fillMaxWidth().height(74.dp).clip(look.shapes.small).background(c.background).border(BorderStroke(1.dp, c.outlineVariant), look.shapes.small).padding(8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                Modifier.fillMaxWidth().height(22.dp).clip(look.shapes.extraSmall).background(Brush.linearGradient(listOf(c.heroStart, c.heroEnd), Offset.Zero, Offset(300f, 100f))).padding(horizontal = 7.dp),
                horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically,
            ) { Box(Modifier.size(9.dp).clip(look.full).background(c.success)) }
            Box(Modifier.fillMaxWidth(0.72f).height(14.dp).lipBehind(look.full, c.primaryEdge, look.pressLipSmall).clip(look.full).background(c.primary))
            Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                KinTint.entries.forEach { tint ->
                    val t = c.tint(tint)
                    Box(Modifier.size(16.dp).clip(look.iconShape).background(t.container).border(BorderStroke(1.5.dp, t.icon), look.iconShape))
                }
            }
        }
        Row(Modifier.padding(horizontal = 2.dp, vertical = 2.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(18.dp).border(BorderStroke(2.dp, if (selected) here.primaryText else here.outline), KinTheme.style.full), contentAlignment = Alignment.Center) {
                if (selected) Box(Modifier.size(8.dp).clip(KinTheme.style.full).background(here.primaryText))
            }
            Column {
                Text(name, style = MaterialTheme.typography.titleSmall.copy(fontSize = 14.sp, fontWeight = FontWeight.Bold))
                Text(hint, style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Normal), color = here.onSurfaceVariant)
            }
        }
    }
}
