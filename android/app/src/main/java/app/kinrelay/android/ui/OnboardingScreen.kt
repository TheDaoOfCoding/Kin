package app.kinrelay.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.pairing.PairingUiState
import app.kinrelay.android.ui.theme.ArtSlot
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinLogo
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.LinkLook
import app.kinrelay.android.ui.theme.PairArt

/**
 * The welcome screen, shown while no PC is paired. The pairing itself is unchanged: scan the code the PC shows, or paste its link. [onOpenDebug] is
 * the way to the Debug screen with no PC (it used to be a tab).
 */
@Composable
fun OnboardingScreen(
    state: PairingUiState,
    localNetworkAllowed: Boolean,
    onAllowLocalNetwork: () -> Unit,
    identityProblem: String?,
    onScan: () -> Unit,
    onPair: (String) -> Unit,
    modifier: Modifier = Modifier,
    onOpenDebug: () -> Unit = {},
) {
    val colors = KinTheme.colors
    var link by rememberSaveable { mutableStateOf("") }
    val working = state is PairingUiState.Working
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 26.dp, vertical = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            KinLogo(42.dp)
            Text("Kin", style = MaterialTheme.typography.headlineMedium.copy(fontSize = 24.sp), color = colors.onSurface)
        }
        ArtSlot(292.dp, 208.dp, Modifier.padding(top = 22.dp, bottom = 14.dp), KinTheme.style.shapes.large) {
            PairArt(LinkLook.CONNECTED, Modifier.size(292.dp, 208.dp), plainStroke = 4.dp)
        }
        Text("Your devices are family.\nKin treats them that way.", style = MaterialTheme.typography.headlineMedium, color = colors.onSurface, textAlign = TextAlign.Center)
        Text(
            "Send text, links, photos and files between your phone and PC, and see your phone's notifications on the big screen. It all stays on your Wi-Fi.",
            style = MaterialTheme.typography.bodyLarge, color = colors.onSurfaceVariant, textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(6.dp))
        if (identityProblem != null) IdentityProblemCard(identityProblem)
        if (!localNetworkAllowed) LocalNetworkCard(onAllowLocalNetwork)
        KinButton("Scan code", onScan, icon = KinIcons.Laptop, block = true, enabled = !working)
        Text(
            "On your PC open Kin and choose “Pair new phone”, then scan the code it shows.",
            style = MaterialTheme.typography.bodyMedium, color = colors.onSurfaceSubtle, textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(10.dp))
        Text("Or paste a pairing link", style = MaterialTheme.typography.titleMedium, color = colors.onSurface, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(
            value = link,
            onValueChange = { link = it },
            label = { Text("kinrelay://pair?…") },
            shape = KinTheme.style.shapes.small,
            modifier = Modifier.fillMaxWidth(),
        )
        KinButton("Pair", { onPair(link) }, style = KinButtonStyle.PLAIN, block = true, enabled = link.isNotBlank() && !working)
        when (state) {
            PairingUiState.Idle, is PairingUiState.Confirm -> Unit
            is PairingUiState.Working -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp))
                Text(state.message, color = colors.onSurface)
            }
            is PairingUiState.Failed -> Text(state.message, color = colors.error)
        }
        TextButton(onClick = onOpenDebug) { Text("Debug", color = colors.onSurfaceSubtle) }
    }
}
