package app.kinrelay.android.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.SystemClock
import app.kinrelay.android.clipboard.AndroidClipboard
import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.graph
import app.kinrelay.android.link.SendSource
import app.kinrelay.android.pairing.PairingIntake
import app.kinrelay.android.pairing.PairingSource
import app.kinrelay.android.transfer.ShareSource
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * adb hooks for the integration tests (debug builds only). Results are log lines with the
 * `KinRelay` logcat tag, starting with `CMD`, so a script can wait for them:
 *
 *   adb shell am broadcast -a app.kinrelay.android.debug.PING -n app.kinrelay.android/.debug.DebugCommandReceiver
 *
 * See android/README.md for every command.
 */
class DebugCommandReceiver : BroadcastReceiver() {
    /** The handler the service uses for what the PC sends, built over the real clipboard, notifications and settings. */
    private fun incoming(context: Context) = context.graph.let { graph ->
        app.kinrelay.android.link.IncomingHandler(
            AndroidClipboard(context), graph.notifications, graph.clipboardGuard, graph.log,
            copyAutomatically = { graph.settings.copyReceivedTextAutomatically() },
            lockScreenLinks = { graph.settings.showLinkPreviewsOnLockScreen() },
        )
    }

    override fun onReceive(context: Context, intent: Intent) {
        val graph = context.graph
        val command = intent.action.orEmpty().substringAfterLast('.')
        val pending = goAsync()
        graph.scope.launch {
            try {
                when (command) {
                    "PING" -> {
                        graph.link.ensureRunning(context)
                        graph.log.i("debug", "CMD PING result=${graph.link.ping()}")
                    }
                    "STATE" -> {
                        val device = graph.pairedDevice()
                        val stats = graph.settings.stats.first()
                        graph.log.i(
                            "debug",
                            "CMD STATE link=${graph.link.state.value} paired=${device?.name} pcId=${device?.deviceId} " +
                                "lastAddresses=${device?.lastAddresses} lastPing=${graph.link.lastPing.value} " +
                                "lastArrivalMs=${stats.lastArrivalMillis} clipboardGuard=${graph.clipboardGuard.describe()} " +
                                "identityProblem=${graph.identityProblem.value} pairing=${graph.pairing.state.value.javaClass.simpleName} " +
                                "connects=${stats.connects} disconnects=${stats.disconnects} kills=${stats.kills} successRate=${graph.successRate().label()} " +
                                "phoneId=${graph.identityFlow.value?.deviceId} wakeLockHeld=${graph.wakeLocks?.isHeld()}",
                        )
                    }
                    "CONNECT" -> {
                        graph.link.ensureRunning(context)
                        graph.link.userAction()
                        graph.log.i("debug", "CMD CONNECT done")
                    }
                    "UNPAIR" -> {
                        graph.link.unpair(context)
                        graph.log.i("debug", "CMD UNPAIR done")
                    }
                    "PAIR" -> {
                        // The shell is the "user": same as pasting the link in the app, so no confirmation screen.
                        val url = intent.getStringExtra("url")
                        if (url == null) {
                            graph.log.w("debug", "CMD PAIR needs --es url <link>")
                        } else {
                            val decision = graph.pairing.intake(url, PairingSource.IN_APP)
                            if (decision is PairingIntake.PairNow) graph.pairing.start(decision.target, context)
                            graph.log.i("debug", "CMD PAIR ${decision?.javaClass?.simpleName}")
                        }
                    }
                    "SEND_TEXT" -> {
                        // `--es textfile <name>`: the text is the UTF-8 content of files/debug-share/<name> (adb can't carry 200 KiB or a bare CR/LF in an argument)
                        val text = intent.getStringExtra("textfile")?.let { java.io.File(DebugFiles.dir(context), DebugFiles.fileName(it)).readText() } ?: intent.getStringExtra("text")
                        if (text == null) {
                            graph.log.w("debug", "CMD SEND_TEXT needs --es text <text>")
                        } else {
                            val result = graph.sender.send(text, SystemClock.elapsedRealtime(), SendSource.DEBUG, sensitive = intent.getBooleanExtra("sensitive", false))
                            graph.log.i("debug", "CMD SEND_TEXT result=$result")
                        }
                    }
                    // A `link` or `text` as if the PC had sent it (2B.1 tests; the desktop's own link send may not be built yet): the same
                    // IncomingHandler the service uses, so the notification, its tap, Copy, grouping and the settings are the real ones.
                    // `--es url <url> [--es title <title>] [--es id <id>]`
                    "LINK_FROM_PC" -> {
                        val url = intent.getStringExtra("url")
                        if (url == null) {
                            graph.log.w("debug", "CMD LINK_FROM_PC needs --es url <url>")
                        } else {
                            val shown = incoming(context).onLink(
                                "DESK-PC", intent.getStringExtra("id") ?: app.kinrelay.android.protocol.MessageIds.newId(), url,
                                intent.getStringExtra("title")?.let(app.kinrelay.android.protocol.LinkWire::cleanTitle), null,
                            )
                            graph.log.i("debug", "CMD LINK_FROM_PC shown=${shown != null}")
                        }
                    }
                    // `--es text <text> [--ez sensitive true]`
                    "TEXT_FROM_PC" -> {
                        val text = intent.getStringExtra("text")
                        if (text == null) {
                            graph.log.w("debug", "CMD TEXT_FROM_PC needs --es text <text>")
                        } else {
                            incoming(context).onText(
                                "DESK-PC", intent.getStringExtra("id") ?: app.kinrelay.android.protocol.MessageIds.newId(), text,
                                isUrl = app.kinrelay.android.protocol.UrlRules.isUrl(text), sensitive = intent.getBooleanExtra("sensitive", false),
                            )
                            graph.log.i("debug", "CMD TEXT_FROM_PC done")
                        }
                    }
                    // `--ez lockscreen true|false`, `--ez copytext true|false` (either or both)
                    "SET_LINK_PREFS" -> {
                        if (intent.hasExtra("lockscreen")) graph.settings.setShowLinkPreviewsOnLockScreen(intent.getBooleanExtra("lockscreen", false))
                        if (intent.hasExtra("copytext")) graph.settings.setCopyReceivedTextAutomatically(intent.getBooleanExtra("copytext", true))
                        graph.log.i("debug", "CMD SET_LINK_PREFS lockscreen=${graph.settings.showLinkPreviewsOnLockScreen()} copytext=${graph.settings.copyReceivedTextAutomatically()}")
                    }
                    // `--es url <url> [--es title <title>]`: the share sheet's path without the sheet (a ShareActivity intent works too)
                    "SEND_LINK" -> {
                        val url = intent.getStringExtra("url")
                        if (url == null) {
                            graph.log.w("debug", "CMD SEND_LINK needs --es url <url>")
                        } else {
                            val result = graph.sender.sendLink(url, intent.getStringExtra("title"), SystemClock.elapsedRealtime(), SendSource.DEBUG)
                            graph.log.i("debug", "CMD SEND_LINK result=$result")
                        }
                    }
                    // 2B.2 notification mirroring. `--es pkg <p>[,<p>] [--ez on false]`: the allow list (also `com.android.shell`, the package of
                    // `adb shell cmd notification post`).
                    "MIRROR_ALLOW" -> {
                        val packages = intent.getStringExtra("pkg")?.split(',')?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty().toSet()
                        val current = graph.mirror.store.current().allowed
                        graph.mirror.store.setAllowed(if (intent.getBooleanExtra("on", true)) current + packages else current - packages)
                        graph.log.i("debug", "CMD MIRROR_ALLOW allowed=${graph.mirror.store.current().allowed.sorted()}")
                    }
                    "MIRROR_HIDDEN" -> {
                        val packages = intent.getStringExtra("pkg")?.split(',')?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty().toSet()
                        val current = graph.mirror.store.current().hiddenAllowed
                        graph.mirror.store.setHiddenAllowed(if (intent.getBooleanExtra("on", true)) current + packages else current - packages)
                        graph.log.i("debug", "CMD MIRROR_HIDDEN hiddenAllowed=${graph.mirror.store.current().hiddenAllowed.sorted()}")
                    }
                    "MIRROR_ENABLE" -> {
                        graph.mirror.store.setEnabled(intent.getBooleanExtra("on", true))
                        graph.log.i("debug", "CMD MIRROR_ENABLE enabled=${graph.mirror.store.current().enabled}")
                    }
                    "MIRROR_DISMISS_PREF" -> {
                        graph.mirror.store.setDismissOnPhone(intent.getBooleanExtra("on", true))
                        graph.log.i("debug", "CMD MIRROR_DISMISS_PREF dismissOnPhone=${graph.mirror.store.current().dismissOnPhone}")
                    }
                    // `--es kind a|b|c|d|e|f|g [--es tag <t>]`: the debug test notifications (design 4.14), posted from Kin on the channel kin_debug_mirror.
                    "MIRROR_POST" -> {
                        val kind = intent.getStringExtra("kind")?.firstOrNull() ?: 'a'
                        graph.log.i("debug", "CMD MIRROR_POST kind=$kind posted=${MirrorTestPoster.post(context, kind, intent.getStringExtra("tag") ?: "t")}")
                    }
                    // `--es tag <t> --ei id <n>`: cancel one of Kin's own test notifications, as an app removing its notification would.
                    "MIRROR_CANCEL" -> {
                        context.getSystemService(android.app.NotificationManager::class.java).cancel(intent.getStringExtra("tag") ?: "t", intent.getIntExtra("id", 1))
                        graph.log.i("debug", "CMD MIRROR_CANCEL done")
                    }
                    // Clears the shade (the listener's `cancelAllNotifications`): `adb shell cmd notification post` is limited to 50 notifications per package.
                    "MIRROR_CLEAR_ALL" -> {
                        app.kinrelay.android.notifmirror.MirrorListenerService.cancelAll()
                        graph.log.i("debug", "CMD MIRROR_CLEAR_ALL done")
                    }
                    // Counts and settings, never content.
                    "MIRROR_STATE" -> {
                        val policy = graph.mirror.store.current()
                        val stats = graph.mirror.sender.stats
                        graph.log.i(
                            "debug",
                            "CMD MIRROR_STATE enabled=${policy.enabled} apps=${policy.allowed.size} hiddenAllowed=${policy.hiddenAllowed.size} dismissOnPhone=${policy.dismissOnPhone} " +
                                "access=${graph.mirror.accessGranted()} listener=${graph.mirror.listenerConnected} pcAcceptsNotif=${graph.mirror.pcAcceptsNotif.value} " +
                                "online=${graph.mirror.sender.isOnline()} sent=${stats.sent.get()} removed=${stats.removed.get()} identical=${stats.identical.get()} " +
                                "actions=${stats.actions.get()} dismissed=${stats.dismissed.get()} dropped={${listOf("not_allowed", "own", "other_profile", "ongoing", "summary", "local_only", "media", "category", "importance", "secret", "empty", "offline", "rate").joinToString(",") { "$it:${stats.dropped(it)}" }}} " +
                                "announced=${graph.mirror.sender.announcedCount()} lastAction=${graph.mirror.lastActionResult} replies=${MirrorTestLog.replies} lastReplySha=${MirrorTestLog.lastReplySha} marksRead=${MirrorTestLog.marksRead} sdk=${android.os.Build.VERSION.SDK_INT}",
                        )
                    }
                    "SET_CLIPBOARD" -> {
                        val text = intent.getStringExtra("textfile")?.let { java.io.File(DebugFiles.dir(context), DebugFiles.fileName(it)).readText() } ?: intent.getStringExtra("text")
                        if (text == null) {
                            graph.log.w("debug", "CMD SET_CLIPBOARD needs --es text <text>")
                        } else {
                            AndroidClipboard(context).write(text, sensitive = intent.getBooleanExtra("sensitive", false))
                            graph.log.i("debug", "CMD SET_CLIPBOARD ${ClipboardGuard.fingerprint(text)}")
                        }
                    }
                    "MAKE_FILES" -> {
                        val spec = intent.getStringExtra("spec")
                        if (spec == null) {
                            graph.log.w("debug", "CMD MAKE_FILES needs --es spec <name:size|name@WxH,...>")
                        } else {
                            DebugFiles.make(context, spec).forEach { graph.log.i("debug", "CMD MAKE_FILES $it") }
                        }
                    }
                    "SEND_FILES" -> {
                        // Straight to the transfer engine (no share activity): the files are those made by MAKE_FILES.
                        val names = intent.getStringExtra("files")?.split(',')?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty()
                        if (names.isEmpty()) {
                            graph.log.w("debug", "CMD SEND_FILES needs --es files <name,name>")
                        } else {
                            graph.link.ensureRunning(context)
                            val id = graph.transfers.enqueueUpload(names.map { ShareSource(DebugFiles.uriOf(context, it).toString()) })
                            graph.log.i("debug", "CMD SEND_FILES transfer=$id")
                        }
                    }
                    "TRANSFERS" -> {
                        val lines = graph.transfers.summary(20)
                        graph.log.i("debug", "CMD TRANSFERS count=${lines.size} wakeLockHeld=${graph.wakeLocks?.isHeld()}")
                        lines.forEach { graph.log.i("debug", "CMD TRANSFER $it") }
                    }
                    "RECEIVED" -> DebugChecks.received(context).forEach { graph.log.i("debug", "CMD RECEIVED $it") }
                    "CANCEL_TRANSFER" -> {
                        val id = intent.getStringExtra("id")
                        if (id == null) graph.log.w("debug", "CMD CANCEL_TRANSFER needs --es id <transfer id>") else { graph.transfers.cancel(id); graph.log.i("debug", "CMD CANCEL_TRANSFER $id requested") }
                    }
                    "RETRY_TRANSFER" -> {
                        val id = intent.getStringExtra("id")
                        if (id == null) graph.log.w("debug", "CMD RETRY_TRANSFER needs --es id <transfer id>") else { graph.transfers.retry(id); graph.log.i("debug", "CMD RETRY_TRANSFER $id requested") }
                    }
                    "RETRY_QUEUED" -> {
                        graph.transfers.retryQueuedUploads()
                        graph.log.i("debug", "CMD RETRY_QUEUED done")
                    }
                    "DELETE_RECEIVED" -> DebugChecks.deleteReceived(context).forEach { graph.log.i("debug", "CMD DELETE_RECEIVED $it") }
                    "MAKE_ORPHANS" -> DebugChecks.makeOrphans(context).forEach { graph.log.i("debug", "CMD MAKE_ORPHANS $it") }
                    "SWEEP" -> {
                        graph.transfers.sweepOrphans()
                        graph.log.i("debug", "CMD SWEEP done")
                    }
                    "EMPTY_ENTRY_CHECK" -> DebugChecks.emptyEntryCheck(context).forEach { graph.log.i("debug", "CMD EMPTY_ENTRY_CHECK $it") }
                    "APPEND_CHECK" -> DebugChecks.appendCheck(context).forEach { graph.log.i("debug", "CMD APPEND_CHECK $it") }
                    "TEXTS" -> {
                        // The text queue and history, without any text (id, direction, status, attempts, source, flags, size).
                        val items = graph.database.texts().recent(50)
                        graph.log.i("debug", "CMD TEXTS count=${items.size}")
                        items.forEach { graph.log.i("debug", "CMD TEXT ${it.id} ${it.direction} ${it.status} attempts=${it.attempts} src=${it.source} sensitive=${it.sensitive} bytes=${it.byteSize} err=${it.errorCode} bodyKept=${it.body != null} retriedAs=${it.retriedAs} kind=${it.kind} titleChars=${it.title?.length ?: 0}") }
                    }
                    "CANCEL_TEXT" -> {
                        val id = intent.getStringExtra("id")
                        if (id == null) graph.log.w("debug", "CMD CANCEL_TEXT needs --es id <message id>") else graph.log.i("debug", "CMD CANCEL_TEXT $id canceled=${graph.textOutbox.cancel(id)}")
                    }
                    "RETRY_TEXT" -> {
                        val id = intent.getStringExtra("id")
                        val item = id?.let { graph.history.items.first().firstOrNull { h -> h.id == it } }
                        if (item == null) graph.log.w("debug", "CMD RETRY_TEXT needs --es id <message id>") else { graph.retryItem(item); graph.log.i("debug", "CMD RETRY_TEXT $id requested") }
                    }
                    "EXPORT_LOGS" -> {
                        // The same zip as Settings > Debug > Export logs, without the share sheet: it is in cache/exports/ (run-as to read it).
                        val uri = app.kinrelay.android.debuginfo.LogExport.create(graph)
                        val zip = java.io.File(context.cacheDir, app.kinrelay.android.debuginfo.LogExport.DIR).listFiles()?.maxByOrNull { it.lastModified() }
                        graph.log.i("debug", "CMD EXPORT_LOGS uri=$uri file=${zip?.name} bytes=${zip?.length()}")
                    }
                    "FLUSH" -> {
                        val peer = graph.pairedDevice()
                        val connection = peer?.let { graph.link.await(1_000, it.deviceId) }
                        graph.log.i("debug", "CMD FLUSH ${connection?.let { graph.flusher.flush(it) } ?: "no connection"}")
                    }
                    // `--el ahead <ms>`: judge the queue as if the clock were that far ahead (a text made now is 7 days old at 604800000)
                    "EXPIRE" -> graph.log.i("debug", "CMD EXPIRE expired=${graph.flusher.expire(intent.getLongExtra("ahead", 0L))}")
                    "SET_PING_INTERVAL" -> {
                        val seconds = intent.getIntExtra("seconds", -1)
                        if (seconds > 0) {
                            graph.settings.setPingIntervalSeconds(seconds)
                            graph.log.i("debug", "CMD SET_PING_INTERVAL ${seconds}s (applies at the next connection)")
                        } else {
                            graph.log.w("debug", "CMD SET_PING_INTERVAL needs --ei seconds <n>")
                        }
                    }
                }
            } finally {
                pending.finish()
            }
        }
    }
}
