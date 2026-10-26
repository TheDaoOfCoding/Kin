package app.kinrelay.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.data.Stats
import app.kinrelay.android.debuginfo.ConnectionEvent
import app.kinrelay.android.debuginfo.LatencySummary
import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.SuccessRate
import app.kinrelay.android.link.PingResult
import kotlinx.coroutines.launch

private val PING_CHOICES = listOf(60 to "1 min", 240 to "4 min", 600 to "10 min")

/**
 * The Debug screen (milestone 1.4): the first-try success rate (targets.md #1), recent errors with their codes, ping round trip and
 * tap-to-ack latency, the connect and disconnect history, the kill count, and **Export logs** (a zip of the logs and a state dump with no
 * secrets and no text, handed to the share sheet).
 */
@Composable
fun DebugScreen(
    stats: Stats,
    logLines: List<String>,
    success: SuccessRate.Result,
    errors: List<HistoryItem>,
    events: List<ConnectionEvent>,
    onPingInterval: (Int) -> Unit,
    onExport: () -> Unit,
    /** "Ping PC" (it used to be on Home): one round trip to the PC, and the last answer. */
    onPing: suspend () -> Unit = {},
    lastPing: PingResult? = null,
    modifier: Modifier = Modifier,
    /** The notification-mirroring counters and, in a debug build, the "Mirror test" section (2B.2). */
    mirror: @Composable () -> Unit = {},
) {
    val rtt = LatencySummary.of(stats.recentRtts)
    val arrival = LatencySummary.of(stats.recentArrivals.map { it.toDouble() })
    val scope = rememberCoroutineScope()
    var pinging by remember { mutableStateOf(false) }
    LazyColumn(modifier = modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item { Text("Debug", style = MaterialTheme.typography.titleLarge) }
        item { Text("First-try success: ${success.label()}", style = MaterialTheme.typography.titleMedium) }
        item { Text("Target: at least 99% of sends succeed on the first try.", style = MaterialTheme.typography.bodySmall) }
        item { Text("Connects: ${stats.connects}   Disconnects: ${stats.disconnects}   Service kills: ${stats.kills}") }
        item { Text("Ping round trip: ${latency(rtt) { UiText.formatMillis(it) }}") }
        item { Text("Tap to ack (text sent): ${latency(arrival) { "${Math.round(it)} ms" }}") }
        item { Text("WebSocket ping every (applies at the next connection):", style = MaterialTheme.typography.bodySmall) }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PING_CHOICES.forEach { (seconds, label) ->
                    FilterChip(selected = stats.pingIntervalSeconds == seconds, onClick = { onPingInterval(seconds) }, label = { Text(label) })
                }
            }
        }
        item {
            KinButton(if (pinging) "Pinging…" else "Ping PC", { scope.launch { pinging = true; onPing(); pinging = false } }, style = KinButtonStyle.PLAIN, block = true, enabled = !pinging)
        }
        if (lastPing != null) item { Text(UiText.ping(lastPing), style = MaterialTheme.typography.titleMedium) }
        item { KinButton("Export logs", onExport, block = true) }
        item { mirror() }

        item { Text("Recent errors", style = MaterialTheme.typography.titleMedium) }
        if (errors.isEmpty()) item { Text("None.", style = MaterialTheme.typography.bodySmall) }
        items(errors.take(15), key = { "e" + it.key }) { e ->
            Text(
                "${e.errorCode ?: "?"} · ${HistoryText.time(e.timeMillis, System.currentTimeMillis())} · ${e.errorMessage ?: ""}",
                fontFamily = FontFamily.Monospace, fontSize = 11.sp, lineHeight = 13.sp,
            )
        }

        item { Text("Connections", style = MaterialTheme.typography.titleMedium) }
        if (events.isEmpty()) item { Text("Nothing logged yet.", style = MaterialTheme.typography.bodySmall) }
        items(events.take(20)) { e ->
            Text(
                "${e.time.take(19).replace('T', ' ')} ${e.kind.name.lowercase().replace('_', ' ')}: ${e.detail.take(90)}",
                fontFamily = FontFamily.Monospace, fontSize = 10.sp, lineHeight = 12.sp,
            )
        }

        item { Text("Log", style = MaterialTheme.typography.titleMedium) }
        items(logLines.asReversed().take(60)) { line ->
            Text(line, fontFamily = FontFamily.Monospace, fontSize = 10.sp, lineHeight = 12.sp)
        }
    }
}

private fun latency(s: LatencySummary, format: (Double) -> String): String =
    if (s.count == 0) "none yet" else "last ${format(s.last!!)}, median ${format(s.median!!)} (n=${s.count})"
