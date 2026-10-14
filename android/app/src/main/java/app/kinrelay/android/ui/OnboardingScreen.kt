package app.kinrelay.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.pairing.PairingUiState

@Composable
fun OnboardingScreen(
    state: PairingUiState,
    onScan: () -> Unit,
    onPair: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    var link by rememberSaveable { mutableStateOf("") }
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Kin", style = MaterialTheme.typography.displayMedium)
        Text("Pair with your PC", style = MaterialTheme.typography.titleLarge)
        Text(
            "On your PC open Kin and choose “Pair new phone”, then scan the code it shows.",
            style = MaterialTheme.typography.bodyLarge,
        )
        Button(onClick = onScan, enabled = state !is PairingUiState.Working, modifier = Modifier.fillMaxWidth()) {
            Text("Scan code")
        }
        Text("Or paste a pairing link", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(
            value = link,
            onValueChange = { link = it },
            label = { Text("kinrelay://pair?…") },
            modifier = Modifier.fillMaxWidth(),
        )
        Button(
            onClick = { onPair(link) },
            enabled = link.isNotBlank() && state !is PairingUiState.Working,
            modifier = Modifier.fillMaxWidth(),
        ) { Text("Pair") }
        when (state) {
            PairingUiState.Idle -> Unit
            is PairingUiState.Working -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp))
                Text(state.message)
            }
            is PairingUiState.Failed -> Text(state.message, color = MaterialTheme.colorScheme.error)
        }
    }
}
