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
                                "connects=${stats.connects} disconnects=${stats.disconnects} kills=${stats.kills} " +
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
