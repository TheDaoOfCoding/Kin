package app.kinrelay.android.ui

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import app.kinrelay.android.AppGraph
import app.kinrelay.android.PermissionSnapshot
import app.kinrelay.android.notifmirror.AppClass
import app.kinrelay.android.notifmirror.MirrorPolicy
import app.kinrelay.android.notifmirror.PickerApp
import app.kinrelay.android.notifmirror.PickerModel
import app.kinrelay.android.notifmirror.RedactionMatcher
import app.kinrelay.android.notifmirror.RestrictedSettings
import app.kinrelay.android.setup.SetupActions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinTint

/** The apps that have a launcher icon (the `<queries>` entry), as picker candidates. No `QUERY_ALL_PACKAGES`. */
internal fun launchableApps(context: Context): List<PickerModel.Candidate> {
    val packages = context.packageManager
    val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
    val found = try {
        if (Build.VERSION.SDK_INT >= 33) {
            packages.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            packages.queryIntentActivities(intent, 0)
        }
    } catch (_: Exception) {
        emptyList()
    }
    return found.map { PickerModel.Candidate(it.activityInfo.packageName, it.loadLabel(packages).toString()) }
        .filter { it.packageName != context.packageName }
}

/** Calls [onResume] every time the screen comes back to the front (after the system settings, say). */
@Composable
private fun OnResume(context: Context, onResume: () -> Unit) {
    val owner = context as? LifecycleOwner ?: return
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_RESUME) onResume() }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }
}

/**
 * The list of apps (design 4.3): messaging apps first and pre-ticked on the first run, then everything else, then the "likely sensitive" ones, which
 * are off, marked with a warning, and ask before they are turned on. Each ticked app has a small switch for "also show notifications I've hidden on the lock screen".
 */
@Composable
fun AppPickerList(
    apps: List<PickerApp>,
    allowed: Set<String>,
    hiddenAllowed: Set<String>,
    onAllowed: (String, Boolean) -> Unit,
    onHiddenAllowed: (String, Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    var filter by rememberSaveable { mutableStateOf("") }
    var confirming by remember { mutableStateOf<PickerApp?>(null) }
    val shown = apps.filter { filter.isBlank() || it.label.contains(filter, ignoreCase = true) || it.packageName.contains(filter, ignoreCase = true) }
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(value = filter, onValueChange = { filter = it }, label = { Text("Search apps") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        LazyColumn(modifier = Modifier.weight(1f, fill = false)) {
            fun section(title: String, rows: List<PickerApp>) {
                if (rows.isEmpty()) return
                item(key = "h-$title") { Text(title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 12.dp, bottom = 4.dp)) }
                items(rows, key = { it.packageName }) { app ->
                    val on = app.packageName in allowed
                    Column(modifier = Modifier.fillMaxWidth()) {
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(app.label, style = MaterialTheme.typography.bodyLarge)
                                if (app.appClass == AppClass.FLAGGED) Text("⚠ Likely sensitive: codes, balances or passwords", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                            }
                            Checkbox(checked = on, onCheckedChange = { want ->
                                if (want && app.appClass == AppClass.FLAGGED) confirming = app else onAllowed(app.packageName, want)
                            })
                        }
                        if (on) {
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().padding(start = 16.dp)) {
                                Text("Also show notifications I've hidden on the lock screen", style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
                                Switch(checked = app.packageName in hiddenAllowed, onCheckedChange = { onHiddenAllowed(app.packageName, it) })
                            }
                        }
                    }
                }
            }
            section("Messaging apps", shown.filter { it.appClass == AppClass.MESSAGING })
            section("Other apps", shown.filter { it.appClass == AppClass.OTHER })
            section("Likely sensitive (off unless you turn them on)", shown.filter { it.appClass == AppClass.FLAGGED })
        }
    }
    confirming?.let { app ->
        AlertDialog(
            onDismissRequest = { confirming = null },
            title = { Text("Turn on ${app.label}?") },
            text = { Text("Notifications from ${app.label} may contain codes, balances or passwords, and they'll appear on your PC. Kin sends only the app's name, never the title, the text or a code, and your PC shows \"Private notification\".") },
            confirmButton = { TextButton(onClick = { onAllowed(app.packageName, true); confirming = null }) { Text("Turn on") } },
            dismissButton = { TextButton(onClick = { confirming = null }) { Text("Cancel") } },
        )
    }
}

/** The picker's rows: the launchable apps (listed once, off the main thread) and the ticked apps that have no launcher icon, sorted and classified. Null while loading. */
@Composable
private fun pickerApps(context: Context, policy: MirrorPolicy, firstRun: Boolean): List<PickerApp>? {
    val candidates by produceState<List<PickerModel.Candidate>?>(null) { value = withContext(Dispatchers.IO) { launchableApps(context) } }
    val loaded = candidates ?: return null
    return remember(loaded, policy.allowed, firstRun) { PickerModel.build(loaded, emptySet(), policy, firstRun) }
}

/** "Which apps?" on its own, from Settings. Each change is saved at once. */
@Composable
fun MirrorAppsScreen(graph: AppGraph, context: Context, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val policy by graph.mirror.store.policy.collectAsState(initial = MirrorPolicy())
    val apps = pickerApps(context, policy, firstRun = false)
    Column(modifier = modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = onBack) { Text("‹ Back") }
        Text("Apps that show notifications on your PC", style = MaterialTheme.typography.titleLarge)
        Text("Nothing is read from an app that isn't ticked. Turning an app off removes its notifications from your PC.", style = MaterialTheme.typography.bodySmall)
        if (apps == null) {
            Text("Loading apps…")
        } else {
            AppPickerList(
                apps, policy.allowed, policy.hiddenAllowed,
                onAllowed = { pkg, on -> scope.launch { graph.mirror.store.toggleAllowed(pkg, on) } },
                onHiddenAllowed = { pkg, on -> scope.launch { graph.mirror.store.toggleHiddenAllowed(pkg, on) } },
                modifier = Modifier.weight(1f),
            )
        }
    }
}

/**
 * The opt-in setup (design 4.2): 1 what it does, 2 pick apps, 3 give access, 4 the result. Nothing is read before access is granted. When access is
 * not granted after the user comes back from the system screen, the "Restricted setting" steps are shown (Android gives no direct signal for it).
 */
@Composable
fun MirrorSetupScreen(
    graph: AppGraph,
    context: Context,
    permissions: PermissionSnapshot,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    var step by rememberSaveable { mutableIntStateOf(1) }
    var openedSettings by rememberSaveable { mutableStateOf(false) }
    // The saved choices are read once; after that the ticks on this screen are the truth until "Next" saves them.
    val saved by produceState<MirrorPolicy?>(null) { value = graph.mirror.store.current() }
    var allowed by rememberSaveable { mutableStateOf<Set<String>?>(null) }
    var hidden by rememberSaveable { mutableStateOf<Set<String>?>(null) }
    val firstRun = saved?.allowed?.isEmpty() != false
    val apps = saved?.let { pickerApps(context, it, firstRun) }
    // The first list defines what is ticked at the start (messaging apps on the first run, the saved apps otherwise).
    LaunchedEffect(apps) {
        if (apps != null && allowed == null) {
            allowed = apps.filter { it.allowed }.map { it.packageName }.toSet()
            hidden = saved?.hiddenAllowed.orEmpty()
        }
    }
    // Coming back from the system's Notification access screen: granted moves on; not granted shows the steps.
    OnResume(context) { if (openedSettings && step == 3) step = 4 }
    LaunchedEffect(permissions.notificationAccess, step) { if (permissions.notificationAccess && step == 3) step = 4 }

    Column(modifier = modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        when (step) {
            1 -> Column(modifier = Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Show notifications on your PC", style = MaterialTheme.typography.headlineMedium)
                Text(
                    "Notifications from the apps you pick appear on your PC's screen, and nowhere else. Kin stores none of it, and you can reply and dismiss from the PC. It's off until you finish these steps.",
                    style = MaterialTheme.typography.bodyLarge,
                )
                Text("A mirrored notification's text lives in Windows' Action Center until it expires or you clear it.", style = MaterialTheme.typography.bodyMedium)
                if (RedactionMatcher.codesHidden(Build.VERSION.SDK_INT)) {
                    Text(
                        "On Android 15 and later, Android hides verification codes from apps like Kin. Your PC will say \"Code hidden by Android\" instead of offering a Copy button.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                Button(onClick = { step = 2 }, modifier = Modifier.fillMaxWidth()) { Text("Choose apps") }
                TextButton(onClick = { scope.launch { graph.mirror.store.setSetupShown(true); onDone() } }) { Text("Not now") }
            }
            2 -> {
                Text("Which apps?", style = MaterialTheme.typography.headlineMedium)
                Text("Messaging apps are ticked for you. Nothing from an app that isn't ticked is ever read. You can change this any time.", style = MaterialTheme.typography.bodyMedium)
                if (apps == null || allowed == null) {
                    Text("Loading apps…")
                } else {
                    AppPickerList(
                        apps, allowed.orEmpty(), hidden.orEmpty(),
                        onAllowed = { pkg, on -> allowed = if (on) allowed.orEmpty() + pkg else allowed.orEmpty() - pkg },
                        onHiddenAllowed = { pkg, on -> hidden = if (on) hidden.orEmpty() + pkg else hidden.orEmpty() - pkg },
                        modifier = Modifier.weight(1f),
                    )
                }
                Button(
                    onClick = {
                        scope.launch {
                            graph.mirror.store.setAllowed(allowed.orEmpty())
                            graph.mirror.store.setHiddenAllowed(hidden.orEmpty().intersect(allowed.orEmpty()))
                            step = 3
                        }
                    },
                    enabled = allowed != null, modifier = Modifier.fillMaxWidth(),
                ) { val count = allowed.orEmpty().size; Text("Next ($count ${if (count == 1) "app" else "apps"})") }
                TextButton(onClick = { step = 1 }) { Text("‹ Back") }
            }
            3 -> Column(modifier = Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Give Kin notification access", style = MaterialTheme.typography.headlineMedium)
                Text("Android keeps this in its own settings. Tap the button, find Kin in the list, and turn it on.", style = MaterialTheme.typography.bodyLarge)
                if (remember { RestrictedSettings.hint(context) }) {
                    Text(
                        "Android may ask for one extra step, because Kin was installed from a file and not from a store. If it does, the next screen shows you what to do.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                Button(onClick = { openedSettings = true; RestrictedSettings.openNotificationAccess(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification access") }
                TextButton(onClick = { step = 2 }) { Text("‹ Back") }
            }
            else -> Column(modifier = Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (permissions.notificationAccess) {
                    Text("Done", style = MaterialTheme.typography.headlineMedium)
                    Text("Notifications from ${allowed.orEmpty().size} apps will appear on your PC. It also needs the setting \"Show phone notifications\" turned on in Kin on your PC.", style = MaterialTheme.typography.bodyLarge)
                    Button(onClick = { scope.launch { graph.mirror.store.setEnabled(true); graph.mirror.store.setSetupShown(true); onDone() } }, modifier = Modifier.fillMaxWidth()) { Text("Turn it on") }
                } else {
                    RestrictedSettingsCard(context)
                    Button(onClick = { RestrictedSettings.openNotificationAccess(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification access") }
                    TextButton(onClick = { scope.launch { graph.mirror.store.setSetupShown(true); onDone() } }) { Text("Not now") }
                }
            }
        }
    }
}

/** The numbered "Restricted setting" steps, with a button for Kin's App info page (design 4.2). */
@Composable
fun RestrictedSettingsCard(context: Context, modifier: Modifier = Modifier) {
    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Notification access isn't on yet", style = MaterialTheme.typography.titleMedium)
            Text("If Android says \"Restricted setting\", that is because Kin was installed from a file. Allowing it takes four steps:", style = MaterialTheme.typography.bodyMedium)
            RestrictedSettings.STEPS.forEachIndexed { i, text -> Text("${i + 1}. $text", style = MaterialTheme.typography.bodyMedium) }
            OutlinedButton(onClick = { SetupActions.openAppDetails(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open app info") }
        }
    }
}

/** The "Show notifications on your PC" card of Settings. */
@Composable
fun MirrorSettingsCard(
    graph: AppGraph,
    context: Context,
    permissions: PermissionSnapshot,
    onOpenSetup: () -> Unit,
    onOpenApps: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val policy by graph.mirror.store.policy.collectAsState(initial = MirrorPolicy())
    val pcAccepts by graph.mirror.pcAcceptsNotif.collectAsState()
    KinCard(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Show notifications on your PC", style = MaterialTheme.typography.titleMedium)
            SettingSwitch(
                "Mirror notifications", policy.enabled,
                help = "Notifications from the apps you pick appear on your PC and nowhere else. Kin stores none of their content. " +
                    "To stop Android sharing notifications with Kin at all, turn Kin off in the system's Notification access list." +
                    if (RedactionMatcher.codesHidden(Build.VERSION.SDK_INT)) " Android hides verification codes from apps like Kin, so your PC says \"Code hidden by Android\"." else "",
                note = mirrorNote(policy, permissions.notificationAccess, pcAccepts),
            ) { on ->
                if (on && (!permissions.notificationAccess || policy.allowed.isEmpty())) onOpenSetup() else scope.launch { graph.mirror.store.setEnabled(on) }
            }
            if (!permissions.notificationAccess) {
                OutlinedButton(onClick = onOpenSetup, modifier = Modifier.fillMaxWidth()) { Text("Set up notification access") }
            } else if (policy.enabled) {
                OutlinedButton(onClick = { RestrictedSettings.openNotificationAccess(context) }, modifier = Modifier.fillMaxWidth()) { Text("Open notification access") }
            }
            OutlinedButton(onClick = onOpenApps, modifier = Modifier.fillMaxWidth()) { Text("Apps (${policy.allowed.size} allowed)") }
            SettingSwitch(
                "Dismiss on the phone too", policy.dismissOnPhone,
                help = "When you close a mirrored notification on your PC, it is dismissed here as well.",
            ) { scope.launch { graph.mirror.store.setDismissOnPhone(it) } }
        }
    }
}

/** What Settings says under the "Mirror notifications" switch: only a problem or a wait (null when it is off or all is well: the switch and Home say so). */
internal fun mirrorNote(policy: MirrorPolicy, access: Boolean, pcAccepts: Boolean): String? =
    if (!policy.enabled || (access && policy.allowed.isNotEmpty() && pcAccepts)) null else mirrorStatusLine(policy, access, pcAccepts)

/** What Home says about mirroring (the PC side is told by `notif.status`; this is the phone's own view). */
internal fun mirrorStatusLine(policy: MirrorPolicy, access: Boolean, pcAccepts: Boolean): String = when {
    !policy.enabled -> "Off"
    !access -> "Paused: Kin doesn't have notification access"
    policy.allowed.isEmpty() -> "On, but no app is ticked"
    !pcAccepts -> "On. Your PC needs Kin 0.2.0 and \"Show phone notifications\" turned on"
    else -> "On for ${policy.allowed.size} ${if (policy.allowed.size == 1) "app" else "apps"}"
}

/** The row on Home while mirroring is on, and the warning while it is paused (access lost). Null when there is nothing to say. */
@Composable
fun MirrorHomeCard(graph: AppGraph, permissions: PermissionSnapshot, onOpenSetup: () -> Unit, modifier: Modifier = Modifier) {
    val policy by graph.mirror.store.policy.collectAsState(initial = MirrorPolicy())
    val pcAccepts by graph.mirror.pcAcceptsNotif.collectAsState()
    if (!policy.enabled) return
    if (!permissions.notificationAccess) {
        FeatureCard(KinTint.LILAC, KinIcons.Bell, "Notification access is off", modifier) { small ->
            Text("Mirroring is paused until you give Kin notification access again.", style = MaterialTheme.typography.bodyMedium, color = small)
            KinButton("Fix", onOpenSetup, style = KinButtonStyle.PLAIN, small = true)
        }
    } else {
        FeatureCard(KinTint.LILAC, KinIcons.Bell, "Mirroring notifications", modifier) { small ->
            Text(mirrorStatusLine(policy, true, pcAccepts), style = MaterialTheme.typography.bodyMedium, color = small)
        }
    }
}
