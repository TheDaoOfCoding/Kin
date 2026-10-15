package app.kinrelay.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
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
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.link.PingResult
import app.kinrelay.android.pairing.PairingUiState
import kotlinx.coroutines.launch

@Composable
fun HomeScreen(
    pc: DeviceEntity,
    phoneShortId: String?,
    localNetworkAllowed: Boolean,
    onAllowLocalNetwork: () -> Unit,
    identityProblem: String?,
    pairingState: PairingUiState,
    onDismissPairing: () -> Unit,
    state: LinkState,
    lastPing: PingResult?,
    onPing: suspend () -> Unit,
    onReconnect: () -> Unit,
    onUnpair: suspend () -> Unit,
    modifier: Modifier = Modifier,
    recent: List<app.kinrelay.android.history.HistoryItem> = emptyList(),
    nowMillis: Long = System.currentTimeMillis(),
    onSeeHistory: () -> Unit = {},
    onOpenItem: (app.kinrelay.android.history.HistoryItem) -> Unit = {},
    onRetryItem: (app.kinrelay.android.history.HistoryItem) -> Unit = {},
    onCancelItem: (app.kinrelay.android.history.HistoryItem) -> Unit = {},
) {
    val scope = rememberCoroutineScope()
    var pinging by remember { mutableStateOf(false) }
    var confirmUnpair by remember { mutableStateOf(false) }
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Kin", style = MaterialTheme.typography.displayMedium)
        // A pairing that fails while a PC is paired (a stale link, a used code) used to be visible only in the log.
        when (pairingState) {
            is PairingUiState.Failed -> Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Pairing didn't work", style = MaterialTheme.typography.titleMedium)
                    Text(pairingState.message, color = MaterialTheme.colorScheme.error)
                    TextButton(onClick = onDismissPairing) { Text("Dismiss") }
                }
            }
            is PairingUiState.Working -> Text(pairingState.message, style = MaterialTheme.typography.bodyLarge)
            PairingUiState.Idle, is PairingUiState.Confirm -> Unit
        }
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(pc.name, style = MaterialTheme.typography.titleLarge)
                Text(UiText.status(state), style = MaterialTheme.typography.bodyLarge)
                val pcShort = DeviceIds.shortIdOfDeviceId(pc.deviceId)?.let(DeviceIds::display) ?: "?"
                Text("PC ID: $pcShort", style = MaterialTheme.typography.bodySmall)
                Text("This phone: ${phoneShortId?.let(DeviceIds::display) ?: "…"}", style = MaterialTheme.typography.bodySmall)
            }
        }
        if (identityProblem != null) IdentityProblemCard(identityProblem)
        if (!localNetworkAllowed) LocalNetworkCard(onAllowLocalNetwork)
        Button(
            onClick = { scope.launch { pinging = true; onPing(); pinging = false } },
            enabled = !pinging,
            modifier = Modifier.fillMaxWidth(),
        ) { Text(if (pinging) "Pinging…" else "Ping PC") }
        if (lastPing != null) Text(UiText.ping(lastPing), style = MaterialTheme.typography.titleMedium)
        if (state is LinkState.Replaced || state is LinkState.Connecting) {
            OutlinedButton(onClick = onReconnect, modifier = Modifier.fillMaxWidth()) { Text("Reconnect now") }
        }
        if (recent.isNotEmpty()) {
            Text("Recent", style = MaterialTheme.typography.titleMedium)
            recent.take(3).forEach { HistoryRow(it, nowMillis, onOpenItem, onRetryItem, onCancelItem, modifier = Modifier.padding(horizontal = 0.dp)) }
            TextButton(onClick = onSeeHistory) { Text("See all history") }
        }
        OutlinedButton(onClick = { confirmUnpair = true }, modifier = Modifier.fillMaxWidth()) {
            Text(if (state is LinkState.PairAgain) "Remove this PC" else "Unpair")
        }
    }
    if (confirmUnpair) {
        AlertDialog(
            onDismissRequest = { confirmUnpair = false },
            title = { Text(if (state is LinkState.PairAgain) "Remove ${pc.name}?" else "Unpair from ${pc.name}?") },
            text = { Text("Text and the clipboard stop syncing until you pair again. ${pc.name} is told, if it's connected.") },
            confirmButton = {
                TextButton(onClick = { confirmUnpair = false; scope.launch { onUnpair() } }) {
                    Text(if (state is LinkState.PairAgain) "Remove" else "Unpair")
                }
            },
            dismissButton = { TextButton(onClick = { confirmUnpair = false }) { Text("Cancel") } },
        )
    }
}

/** Android 17 won't let the app reach the PC without the "Nearby devices" permission. */
@Composable
fun LocalNetworkCard(onAllow: () -> Unit, modifier: Modifier = Modifier) {
    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Nearby devices permission needed", style = MaterialTheme.typography.titleMedium)
            Text("Kin isn't allowed to reach devices on your network, so it can't connect to your PC. Turn on Nearby devices for Kin in the app's permissions.")
            Button(onClick = onAllow) { Text("Allow in settings") }
        }
    }
}

/** The Keystore couldn't be used right now. Nothing was deleted; Kin tries again by itself. */
@Composable
fun IdentityProblemCard(problem: String, modifier: Modifier = Modifier) {
    Card(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Kin can't use its secure storage right now", style = MaterialTheme.typography.titleMedium)
            Text(problem, style = MaterialTheme.typography.bodyMedium)
            Text("Your pairing is kept. Restarting the phone usually fixes this.", style = MaterialTheme.typography.bodySmall)
        }
    }
}
