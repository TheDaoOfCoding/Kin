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
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.pairing.PairingConfirmation

/**
 * Shown for every pairing link that arrives from outside the app. Anyone on the network can send one, so nothing
 * is contacted and nothing is replaced until the user taps Confirm.
 */
@Composable
fun ConfirmPairingScreen(
    confirmation: PairingConfirmation,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Kin", style = MaterialTheme.typography.displayMedium)
        Text("Pair with this PC?", style = MaterialTheme.typography.titleLarge)
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(confirmation.pcName, style = MaterialTheme.typography.titleLarge)
                Text("PC ID: ${confirmation.shortId}", style = MaterialTheme.typography.titleMedium)
                Text("The name comes from the link. The ID is worked out from it.", style = MaterialTheme.typography.bodySmall)
            }
        }
        Text(
            "Only continue if you just started pairing on your own PC and its ID is the one shown there. A link from a web page or a message can come from anyone.",
            style = MaterialTheme.typography.bodyLarge,
        )
        confirmation.replacing?.let {
            Text("This will replace $it.", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.error)
        }
        if (confirmation.alreadyPaired) {
            Text("This PC is already paired. Pairing again only refreshes it.", style = MaterialTheme.typography.bodyMedium)
        }
        Button(onClick = onConfirm, modifier = Modifier.fillMaxWidth()) { Text("Confirm") }
        OutlinedButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) { Text("Cancel") }
    }
}
