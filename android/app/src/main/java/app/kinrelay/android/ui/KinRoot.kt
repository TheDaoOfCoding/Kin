package app.kinrelay.android.ui

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import app.kinrelay.android.AppGraph
import app.kinrelay.android.data.Stats
import app.kinrelay.android.pairing.PairingConfirmation
import app.kinrelay.android.pairing.PairingUiState
import kotlinx.coroutines.launch

/** The screens in one: Confirm pairing (when a link is waiting), Onboarding or Home (depending on whether a PC is paired), and Debug. */
@Composable
fun KinRoot(
    graph: AppGraph,
    context: Context,
    localNetworkAllowed: Boolean,
    onAllowLocalNetwork: () -> Unit,
    onScan: () -> Unit,
    onPair: (String) -> Unit,
    onConfirmPairing: (PairingConfirmation) -> Unit,
    onCancelPairing: () -> Unit,
) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val scope = rememberCoroutineScope()
    val devices by graph.database.devices().observeAll().collectAsState(initial = null)
    val identity by graph.identityFlow.collectAsState()
    val identityProblem by graph.identityProblem.collectAsState()
    val linkState by graph.link.state.collectAsState()
    val lastPing by graph.link.lastPing.collectAsState()
    val pairingState by graph.pairing.state.collectAsState()
    val stats by graph.settings.stats.collectAsState(initial = Stats())
    val logLines by graph.log.recent.collectAsState()

    Scaffold(modifier = Modifier.fillMaxSize()) { padding ->
        Column(modifier = Modifier.padding(padding)) {
            val confirm = pairingState as? PairingUiState.Confirm
            if (confirm != null) {
                // A pairing link from outside the app: nothing else is shown until the user decides.
                ConfirmPairingScreen(confirm.confirmation, onConfirm = { onConfirmPairing(confirm.confirmation) }, onCancel = onCancelPairing)
                return@Column
            }
            PrimaryTabRow(selectedTabIndex = tab) {
                Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("Kin") })
                Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("Debug") })
            }
            if (tab == 1) {
                DebugScreen(stats, logLines, onPingInterval = { scope.launch { graph.settings.setPingIntervalSeconds(it) } })
            } else {
                val pc = devices?.firstOrNull()
                when {
                    devices == null -> Unit
                    pc == null -> OnboardingScreen(
                        state = pairingState,
                        localNetworkAllowed = localNetworkAllowed,
                        onAllowLocalNetwork = onAllowLocalNetwork,
                        identityProblem = identityProblem,
                        onScan = onScan,
                        onPair = onPair,
                    )
                    else -> HomeScreen(
                        pc = pc,
                        phoneShortId = identity?.shortId,
                        localNetworkAllowed = localNetworkAllowed,
                        onAllowLocalNetwork = onAllowLocalNetwork,
                        identityProblem = identityProblem,
                        pairingState = pairingState,
                        onDismissPairing = { graph.pairing.reset() },
                        state = linkState,
                        lastPing = lastPing,
                        onPing = { graph.link.ping() },
                        onReconnect = { graph.link.userAction() },
                        onUnpair = { graph.link.unpair(context) },
                    )
                }
            }
        }
    }
}
