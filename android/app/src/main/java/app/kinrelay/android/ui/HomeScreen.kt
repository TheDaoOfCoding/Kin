package app.kinrelay.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
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
import kotlinx.coroutines.launch

@Composable
fun HomeScreen(
    pc: DeviceEntity,
    phoneShortId: String?,
    localNetworkAllowed: Boolean,
    onAllowLocalNetwork: () -> Unit,
    state: LinkState,
    lastPing: PingResult?,
    onPing: suspend () -> Unit,
    onReconnect: () -> Unit,
    onUnpair: suspend () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    var pinging by remember { mutableStateOf(false) }
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Kin", style = MaterialTheme.typography.displayMedium)
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(pc.name, style = MaterialTheme.typography.titleLarge)
                Text(UiText.status(state), style = MaterialTheme.typography.bodyLarge)
                val pcShort = DeviceIds.shortIdOfDeviceId(pc.deviceId)?.let(DeviceIds::display) ?: "?"
                Text("PC ID: $pcShort", style = MaterialTheme.typography.bodySmall)
                Text("This phone: ${phoneShortId?.let(DeviceIds::display) ?: "…"}", style = MaterialTheme.typography.bodySmall)
            }
        }
        if (!localNetworkAllowed) {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Kin isn't allowed to reach devices on your network, so it can't connect to your PC.")
                    Button(onClick = onAllowLocalNetwork) { Text("Allow in settings") }
                }
            }
        }
        Button(
            onClick = { scope.launch { pinging = true; onPing(); pinging = false } },
            enabled = !pinging,
            modifier = Modifier.fillMaxWidth(),
        ) { Text(if (pinging) "Pinging…" else "Ping PC") }
        if (lastPing != null) Text(UiText.ping(lastPing), style = MaterialTheme.typography.titleMedium)
        if (state is LinkState.Replaced || state is LinkState.Connecting) {
            OutlinedButton(onClick = onReconnect, modifier = Modifier.fillMaxWidth()) { Text("Reconnect now") }
        }
        OutlinedButton(onClick = { scope.launch { onUnpair() } }, modifier = Modifier.fillMaxWidth()) {
            Text(if (state is LinkState.PairAgain) "Remove this PC" else "Unpair")
        }
    }
}
