package app.kinrelay.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.data.Stats

private val PING_CHOICES = listOf(60 to "1 min", 240 to "4 min", 600 to "10 min")

@Composable
fun DebugScreen(
    stats: Stats,
    logLines: List<String>,
    onPingInterval: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Debug", style = MaterialTheme.typography.titleLarge)
        Text("Connects: ${stats.connects}   Disconnects: ${stats.disconnects}   Service kills: ${stats.kills}")
        Text("Last ping: ${stats.lastRttMillis?.let { UiText.formatMillis(it) } ?: "none yet"}")
        Text("Last send (tap to ack): ${stats.lastArrivalMillis?.let { "$it ms" } ?: "none yet"}")
        Text("WebSocket ping every (applies at the next connection):", style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PING_CHOICES.forEach { (seconds, label) ->
                FilterChip(
                    selected = stats.pingIntervalSeconds == seconds,
                    onClick = { onPingInterval(seconds) },
                    label = { Text(label) },
                )
            }
        }
        LazyColumn(reverseLayout = true, modifier = Modifier.fillMaxSize()) {
            items(logLines.asReversed()) { line ->
                Text(line, fontFamily = FontFamily.Monospace, fontSize = 10.sp, lineHeight = 12.sp)
            }
        }
    }
}
