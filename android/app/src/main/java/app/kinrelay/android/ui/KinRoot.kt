package app.kinrelay.android.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import android.os.Build
import androidx.core.net.toUri
import app.kinrelay.android.DebugHooks
import app.kinrelay.android.setup.OemTable
import app.kinrelay.android.AppGraph
import app.kinrelay.android.BuildConfig
import app.kinrelay.android.PermissionSnapshot
import app.kinrelay.android.data.Stats
import app.kinrelay.android.debuginfo.ConnectionHistory
import app.kinrelay.android.debuginfo.LogExport
import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryStatus
import app.kinrelay.android.history.SuccessRate
import app.kinrelay.android.pairing.PairingConfirmation
import app.kinrelay.android.pairing.PairingUiState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val MAIN = 0
private const val SETUP = 1
private const val DEBUG = 2
private const val MIRROR_SETUP = 3
private const val MIRROR_APPS = 4

/**
 * The screens in one. When a link is waiting: Confirm pairing. With no PC paired: pairing (and Debug). With a PC: Home, History and Settings
 * as tabs, and, from Settings or after the first pairing, the setup (onboarding) and Debug as full screens.
 */
@Composable
fun KinRoot(
    graph: AppGraph,
    context: Context,
    permissions: PermissionSnapshot,
    onAskPermissions: () -> Unit,
    onAllowLocalNetwork: () -> Unit,
    onScan: () -> Unit,
    onPair: (String) -> Unit,
    onConfirmPairing: (PairingConfirmation) -> Unit,
    onCancelPairing: () -> Unit,
) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var screen by rememberSaveable { mutableIntStateOf(MAIN) }
    var setupOffered by rememberSaveable { androidx.compose.runtime.mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val devices by graph.database.devices().observeAll().collectAsState(initial = null)
    val identity by graph.identityFlow.collectAsState()
    val identityProblem by graph.identityProblem.collectAsState()
    val linkState by graph.link.state.collectAsState()
    val lastPing by graph.link.lastPing.collectAsState()
    val pairingState by graph.pairing.state.collectAsState()
    val loadedStats by graph.settings.stats.collectAsState(initial = null)
    val stats = loadedStats ?: Stats()
    val logLines by graph.log.recent.collectAsState()
    val history by graph.history.items.collectAsState(initial = emptyList())
    val pc = devices?.firstOrNull()
    val localNetworkAllowed = permissions.localNetwork

    // The setup is shown once, right after the first pairing (and never before the settings are known).
    LaunchedEffect(pc != null, loadedStats?.setupDone) {
        if (pc != null && loadedStats != null && !loadedStats!!.setupDone && !setupOffered) {
            setupOffered = true
            screen = SETUP
        }
    }
    BackHandler(enabled = screen != MAIN) { screen = MAIN }

    fun open(item: HistoryItem) {
        val text = item.text
        if (text != null) {
            context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Kin", text))
            Toast.makeText(context, "Copied", Toast.LENGTH_SHORT).show()
            return
        }
        val uri = item.openUri ?: return
        try {
            context.startActivity(
                Intent(Intent.ACTION_VIEW).setDataAndType(uri.toUri(), item.openMime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            )
        } catch (_: Exception) {
            Toast.makeText(context, "No app can open this file.", Toast.LENGTH_SHORT).show()
        }
    }

    Scaffold(modifier = Modifier.fillMaxSize()) { padding ->
        Column(modifier = Modifier.padding(padding)) {
            val confirm = pairingState as? PairingUiState.Confirm
            if (confirm != null) {
                // A pairing link from outside the app: nothing else is shown until the user decides.
                ConfirmPairingScreen(confirm.confirmation, onConfirm = { onConfirmPairing(confirm.confirmation) }, onCancel = onCancelPairing)
                return@Column
            }
            if (screen == SETUP && pc != null) {
                SetupScreen(
                    permissions = permissions, context = context, firstTime = loadedStats?.setupDone != true,
                    guide = OemTable.select(DebugHooks.manufacturer(context), Build.BRAND),
                    onAskPermissions = onAskPermissions, onOpenAppSettings = onAllowLocalNetwork,
                    onDone = {
                        scope.launch {
                            graph.settings.setSetupDone(true)
                            // Notification mirroring is offered once, right after the reliability setup (2B.2); it stays in Settings after that.
                            if (!graph.mirror.store.setupShown.first()) screen = MIRROR_SETUP
                        }
                        screen = MAIN
                        graph.link.userAction()
                    },
                )
                return@Column
            }
            if (screen == MIRROR_SETUP && pc != null) {
                MirrorSetupScreen(graph, context, permissions, onDone = { screen = MAIN })
                return@Column
            }
            if (screen == MIRROR_APPS && pc != null) {
                MirrorAppsScreen(graph, context, onBack = { screen = MAIN })
                return@Column
            }
            val success by produceState(SuccessRate.Result(0, 0), history) { value = graph.successRate() }
            val events by produceState(emptyList(), logLines.size) { value = withContext(Dispatchers.IO) { ConnectionHistory.parse(graph.log.allLines()) } }
            val debug: @Composable () -> Unit = {
                DebugScreen(
                    stats, logLines, success,
                    errors = history.filter { it.status == HistoryStatus.FAILED || it.status == HistoryStatus.EXPIRED },
                    events = events,
                    mirror = {
                        androidx.compose.foundation.layout.Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(6.dp)) {
                            Text("Notification mirroring", style = androidx.compose.material3.MaterialTheme.typography.titleMedium)
                            Text(graph.mirror.sender.stats.line() + " · announced ${graph.mirror.sender.announcedCount()} · last action ${graph.mirror.lastActionResult}", style = androidx.compose.material3.MaterialTheme.typography.bodySmall)
                            DebugHooks.MirrorTest(graph, context)
                        }
                    },
                    onPingInterval = { scope.launch { graph.settings.setPingIntervalSeconds(it) } },
                    onExport = {
                        scope.launch {
                            // Writing the zip can fail (no space, a log being rotated), and so can opening the share sheet: neither may crash the app.
                            try {
                                val uri = withContext(Dispatchers.IO) { LogExport.create(graph) }
                                context.startActivity(LogExport.shareIntent(uri))
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                graph.log.w("debug", "export logs failed: ${e.javaClass.simpleName}: ${e.message}")
                                Toast.makeText(context, "Couldn't export the logs. Check that the phone has free space and try again.", Toast.LENGTH_LONG).show()
                            }
                        }
                    },
                )
            }
            if (screen == DEBUG) {
                TextButton(onClick = { screen = MAIN }) { Text("‹ Back") }
                debug()
                return@Column
            }
            PrimaryTabRow(selectedTabIndex = tab) {
                Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("Kin") })
                if (pc != null) {
                    Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("History") })
                    Tab(selected = tab == 2, onClick = { tab = 2 }, text = { Text("Settings") })
                } else {
                    Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("Debug") })
                }
            }
            val now = System.currentTimeMillis()
            when {
                devices == null -> Unit
                pc == null && tab == 1 -> debug()
                pc == null -> OnboardingScreen(
                    state = pairingState,
                    localNetworkAllowed = localNetworkAllowed,
                    onAllowLocalNetwork = onAllowLocalNetwork,
                    identityProblem = identityProblem,
                    onScan = onScan,
                    onPair = onPair,
                )
                tab == 1 -> HistoryScreen(
                    items = history, nowMillis = now, onOpen = ::open,
                    onRetry = { graph.retryItem(it) }, onCancel = { graph.cancelItem(it) },
                )
                tab == 2 -> SettingsScreen(
                    context = context, pc = pc, deviceName = stats.deviceName.orEmpty(), systemName = graph.deviceInfo.name,
                    phoneShortId = identity?.shortId, versionName = BuildConfig.VERSION_NAME,
                    showLinkPreviewsOnLockScreen = stats.showLinkPreviewsOnLockScreen,
                    copyReceivedTextAutomatically = stats.copyReceivedTextAutomatically,
                    onShowLinkPreviewsOnLockScreen = { scope.launch { graph.settings.setShowLinkPreviewsOnLockScreen(it) } },
                    onCopyReceivedTextAutomatically = { scope.launch { graph.settings.setCopyReceivedTextAutomatically(it) } },
                    onSaveName = { graph.settings.setDeviceName(it) },
                    onOpenSetup = { screen = SETUP }, onOpenDebug = { screen = DEBUG },
                    onUnpair = { graph.link.unpair(context) },
                    mirrorCard = { MirrorSettingsCard(graph, context, permissions, onOpenSetup = { screen = MIRROR_SETUP }, onOpenApps = { screen = MIRROR_APPS }) },
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
                    recent = history, nowMillis = now, onSeeHistory = { tab = 1 },
                    onOpenItem = ::open, onRetryItem = { graph.retryItem(it) }, onCancelItem = { graph.cancelItem(it) },
                    mirrorCard = { MirrorHomeCard(graph, permissions, onOpenSetup = { screen = MIRROR_SETUP }) },
                )
            }
        }
    }
}
