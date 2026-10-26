package app.kinrelay.android.ui

import android.content.Context
import android.os.Build
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.PermissionSnapshot
import app.kinrelay.android.setup.OemGuide
import app.kinrelay.android.setup.OemTable
import app.kinrelay.android.setup.SetupActions

/**
 * The setup, also called onboarding (milestone 1.4): notifications (Android 13+), Nearby devices (Android 17), the battery settings, and the
 * steps for the phone's maker. It is shown once after the first pairing and can be opened again any time from Settings ("Fix connection
 * reliability"): each step shows whether it is done, so it works as a check-up too.
 */
@Composable
fun SetupScreen(
    permissions: PermissionSnapshot,
    context: Context,
    firstTime: Boolean,
    onAskPermissions: () -> Unit,
    onOpenAppSettings: () -> Unit,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
    guide: OemGuide? = OemTable.select(Build.MANUFACTURER, Build.BRAND),
) {
    Column(
        modifier = modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(if (firstTime) "Keep Kin connected" else "Fix connection reliability", style = MaterialTheme.typography.headlineMedium)
        Text("Phones stop background apps to save battery, so Kin needs these few settings to stay connected.", style = MaterialTheme.typography.bodyLarge)

        if (permissions.notificationsExist) {
            StepCard(
                title = "Notifications",
                done = permissions.notifications,
                text = "Kin shows what arrives from your PC, and whether it's connected.",
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = onAskPermissions) { Text("Allow") }
                    TextButton(onClick = onOpenAppSettings) { Text("App settings") }
                }
            }
        }
        if (permissions.localNetworkExists) {
            StepCard(
                title = "Nearby devices",
                done = permissions.localNetwork,
                text = "Android 17 asks before an app may talk to devices on your Wi-Fi, such as your PC. Without it Kin can't connect.",
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = onAskPermissions) { Text("Allow") }
                    TextButton(onClick = onOpenAppSettings) { Text("App settings") }
                }
            }
        }
        StepCard(
            title = "Battery",
            done = permissions.batteryUnrestricted,
            text = "Open the battery list, find Kin and set it to “Not optimised” (on some phones “Unrestricted”). Then Android leaves Kin's connection alone.",
        ) {
            Button(onClick = { SetupActions.openBatteryOptimisationList(context) }) { Text("Open battery settings") }
        }

        OemCard(guide, context)

        Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Done") }
        if (firstTime) TextButton(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Skip for now") }
    }
}

@Composable
private fun OemCard(guide: OemGuide?, context: Context) {
    var note by remember { mutableStateOf<String?>(null) }
    KinCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(guide?.title ?: "Your phone", style = MaterialTheme.typography.titleMedium)
            if (guide == null) {
                Text("Your phone's maker has no extra settings that we know of. The battery step above is all it needs.")
                OemTable.GENERAL_STEPS.forEach { Text("• $it") }
            } else {
                Text("Phones from this maker stop background apps more eagerly. Do these steps once:")
                guide.steps.forEachIndexed { i, step -> Text("${i + 1}. $step") }
                OutlinedButton(
                    onClick = {
                        note = when (val result = SetupActions.openOem(context, guide)) {
                            is SetupActions.OemOpen.Opened -> null
                            SetupActions.OemOpen.FellBack -> "This phone doesn't have that settings screen, so Kin's own settings page opened. Follow the steps above."
                        }
                    },
                ) { Text("Open ${guide.targets.first().label.lowercase()} settings") }
                note?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
            }
        }
    }
}

@Composable
private fun StepCard(title: String, done: Boolean, text: String, actions: @Composable () -> Unit) {
    KinCard(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(if (done) "✓ Done" else "To do", color = if (done) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error)
            }
            Text(text)
            if (!done) actions()
        }
    }
}
