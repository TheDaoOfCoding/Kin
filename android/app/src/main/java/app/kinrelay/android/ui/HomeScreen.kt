package app.kinrelay.android.ui

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.pairing.PairingUiState
import app.kinrelay.android.pause.PauseWords
import app.kinrelay.android.ui.theme.ArtSlot
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinLogo
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.LinkLook
import app.kinrelay.android.ui.theme.PairArt
import kotlinx.coroutines.launch

/** Which face the status card shows: asleep while the user's Pause is on, happy while connected, looking around otherwise. */
/**
 * The Pause words the PC card shows, or null when the link has a problem the user must see: Pause leaves the link alone, so a PC that
 * forgot the phone, a version mismatch or a lost Wi-Fi still happen while paused, and they win over the sleeping face.
 */
fun pauseOnCard(state: LinkState, pausedText: String?): String? = when (state) {
    LinkState.PairAgain, LinkState.UpdateApp, LinkState.Replaced, LinkState.NotOnWifi, LinkState.PermissionNeeded -> null
    else -> pausedText
}

fun linkLook(state: LinkState, paused: Boolean): LinkLook = when {
    paused -> LinkLook.PAUSED
    state is LinkState.Connected -> LinkLook.CONNECTED
    else -> LinkLook.CONNECTING
}

/**
 * Home: the PC's status card, the Send button's place ([sendButton]), the feature cards (mirroring, pause, media) and the recent items.
 * [pausedText] is the words for the user's own Pause ("Until 14:30") while it is on, else null. "Ping PC" is on the Debug screen.
 */
@Composable
fun HomeScreen(
    pc: DeviceEntity,
    localNetworkAllowed: Boolean,
    onAllowLocalNetwork: () -> Unit,
    identityProblem: String?,
    pairingState: PairingUiState,
    onDismissPairing: () -> Unit,
    state: LinkState,
    onReconnect: () -> Unit,
    onUnpair: suspend () -> Unit,
    modifier: Modifier = Modifier,
    pausedText: String? = null,
    recent: List<HistoryItem> = emptyList(),
    nowMillis: Long = System.currentTimeMillis(),
    onSeeHistory: () -> Unit = {},
    onOpenItem: (HistoryItem) -> Unit = {},
    onRetryItem: (HistoryItem) -> Unit = {},
    onCancelItem: (HistoryItem) -> Unit = {},
    /** The big Send button (built apart), shown right under the PC's status card. Empty until it exists. */
    sendButton: @Composable () -> Unit = {},
    /** The "Mirroring notifications" card (2B.2): empty while mirroring is off. */
    mirrorCard: @Composable () -> Unit = {},
    /** The "Media" card (2B.3): empty while both directions are off. */
    mediaCard: @Composable () -> Unit = {},
    /** The PC's status in the status card, under the pill (2B.4): a laptop's battery and its pause. */
    statusLine: @Composable () -> Unit = {},
    /** The Pause card (2B.4). */
    pauseCard: @Composable () -> Unit = {},
    /** "Needs access to all photos" (2B.4): only while auto-send is on and the access is missing. */
    shotsCard: @Composable () -> Unit = {},
    /** Room left under the last item for the floating nav. */
    bottomPadding: Dp = 0.dp,
) {
    val scope = rememberCoroutineScope()
    var confirmRemove by remember { mutableStateOf(false) }
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        AppBar()
        // A pairing that fails while a PC is paired (a stale link, a used code) used to be visible only in the log.
        when (pairingState) {
            is PairingUiState.Failed -> NoticeCard(
                title = "Pairing didn't work", text = pairingState.message, tone = NoticeTone.ERROR,
                action = { TextButton(onClick = onDismissPairing) { Text("Dismiss", color = KinTheme.colors.onErrorContainer) } },
            )
            is PairingUiState.Working -> Text(pairingState.message, style = MaterialTheme.typography.bodyLarge, color = KinTheme.colors.onSurface)
            PairingUiState.Idle, is PairingUiState.Confirm -> Unit
        }
        if (identityProblem != null) IdentityProblemCard(identityProblem)
        if (!localNetworkAllowed) LocalNetworkCard(onAllowLocalNetwork)
        StatusCard(pc.name, state, pauseOnCard(state, pausedText), statusLine, onReconnect)
        if (state is LinkState.PairAgain) KinButton("Remove this PC", { confirmRemove = true }, style = KinButtonStyle.PLAIN, icon = KinIcons.Unlink, block = true)
        sendButton()
        mirrorCard()
        pauseCard()
        mediaCard()
        shotsCard()
        if (recent.isNotEmpty()) {
            Spacer(Modifier.height(2.dp))
            SectionHead("Recent", action = "See all", onAction = onSeeHistory)
            KinCard {
                recent.take(3).forEachIndexed { index, item ->
                    if (index > 0) HorizontalDivider(color = KinTheme.colors.outlineVariant)
                    HistoryRow(item, nowMillis, onOpenItem, onRetryItem, onCancelItem)
                }
            }
        }
        Spacer(Modifier.height(bottomPadding))
    }
    if (confirmRemove) {
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Remove ${pc.name}?") },
            text = { Text(UiText.unlinkText(pc.name)) },
            confirmButton = { TextButton(onClick = { confirmRemove = false; scope.launch { onUnpair() } }) { Text("Remove") } },
            dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text("Cancel") } },
        )
    }
}

/** The logo and name. The right side is left empty on purpose (the "On your Wi-Fi only" chip was cluttering the page). */
@Composable
private fun AppBar() {
    val colors = KinTheme.colors
    Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(top = 6.dp), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            KinLogo()
            Text("Kin", style = MaterialTheme.typography.titleLarge, color = colors.onSurface)
        }
    }
}

/** The PC's card: "Paired with", its name, the state in a pill, and the PC and phone drawn with faces that follow the link. */
@Composable
private fun StatusCard(pcName: String, state: LinkState, pausedText: String?, statusLine: @Composable () -> Unit, onReconnect: () -> Unit) {
    val colors = KinTheme.colors
    val look = KinTheme.style
    val shape = look.shapes.large
    val face = linkLook(state, pausedText != null)
    val detail = if (pausedText != null) null else UiText.detail(state)
    Row(
        Modifier
            .fillMaxWidth()
            .shadow(look.cardElevation, shape, clip = false, ambientColor = look.shadowColor, spotColor = look.shadowColor)
            .let { if (look.hairline != Color.Transparent) it.border(BorderStroke(1.dp, look.hairline), shape) else it }
            .clip(shape)
            .background(Brush.linearGradient(listOf(colors.heroStart, colors.heroEnd), start = Offset.Zero, end = Offset(600f, 400f)))
            .heightIn(min = 122.dp)
            .padding(start = 18.dp, top = 14.dp, end = 14.dp, bottom = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text("Paired with", style = MaterialTheme.typography.labelMedium, color = colors.onSurfaceVariant)
            Text(pcName, style = MaterialTheme.typography.headlineSmall, color = colors.onSurface, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 1.dp, bottom = 8.dp))
            StatePill(face, pausedText?.let(PauseWords::pausedLine) ?: UiText.pill(state))
            if (detail != null) Text(detail, style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
            CompositionLocalProvider(LocalContentColor provides colors.onSurfaceVariant) {
                Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) { statusLine() }
            }
            if (pausedText == null && (state is LinkState.Replaced || state is LinkState.Connecting)) {
                KinButton("Reconnect now", onReconnect, style = KinButtonStyle.PLAIN, small = true, modifier = Modifier.padding(top = 6.dp))
            }
        }
        ArtSlot(132.dp, 94.dp) { PairArt(face, Modifier.size(132.dp, 94.dp)) }
    }
}

/** The state in words, with a dot: green while connected, blinking while it looks for the PC, amber while paused. */
@Composable
private fun StatePill(look: LinkLook, text: String) {
    val colors = KinTheme.colors
    val blink = if (look == LinkLook.CONNECTING) {
        rememberInfiniteTransition(label = "blink").animateFloat(
            initialValue = 1f, targetValue = 1f,
            animationSpec = infiniteRepeatable(keyframes { durationMillis = 1200; 1f at 0; 0.35f at 600; 1f at 1200 }, RepeatMode.Restart),
            label = "blink",
        ).value
    } else {
        1f
    }
    val dot = when (look) {
        LinkLook.CONNECTED -> colors.success
        LinkLook.CONNECTING -> colors.onSurfaceSubtle
        LinkLook.PAUSED -> colors.warning
    }
    Row(
        Modifier.heightIn(min = 28.dp).clip(KinTheme.style.full).background(colors.chipContainer).padding(start = 10.dp, end = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(7.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(9.dp).alpha(blink).clip(KinTheme.style.full).background(dot))
        Text(text, style = MaterialTheme.typography.labelMedium, color = colors.onSurface, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

enum class NoticeTone { WARNING, ERROR }

/** A card that says something is wrong, in the warning (amber) or the error colours, with an optional action. */
@Composable
fun NoticeCard(title: String, text: String, tone: NoticeTone, modifier: Modifier = Modifier, action: @Composable () -> Unit = {}) {
    val colors = KinTheme.colors
    val container = if (tone == NoticeTone.ERROR) colors.errorContainer else colors.warningContainer
    val on = if (tone == NoticeTone.ERROR) colors.onErrorContainer else colors.onWarningContainer
    KinCard(modifier.fillMaxWidth(), container = container) {
        CompositionLocalProvider(LocalContentColor provides on) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(text, style = MaterialTheme.typography.bodyMedium)
                action()
            }
        }
    }
}

/** Android 17 won't let the app reach the PC without the "Nearby devices" permission. */
@Composable
fun LocalNetworkCard(onAllow: () -> Unit, modifier: Modifier = Modifier) {
    NoticeCard(
        "Nearby devices permission needed",
        "Kin isn't allowed to reach devices on your network, so it can't connect to your PC. Turn on Nearby devices for Kin in the app's permissions.",
        NoticeTone.WARNING, modifier,
    ) { KinButton("Allow in settings", onAllow, style = KinButtonStyle.WARN, small = true) }
}

/** The Keystore couldn't be used right now. Nothing was deleted; Kin tries again by itself. */
@Composable
fun IdentityProblemCard(problem: String, modifier: Modifier = Modifier) {
    NoticeCard("Kin can't use its secure storage right now", "$problem\n\nYour pairing is kept. Restarting the phone usually fixes this.", NoticeTone.ERROR, modifier)
}
