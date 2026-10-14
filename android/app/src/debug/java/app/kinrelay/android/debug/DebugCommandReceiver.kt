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
                                "phoneId=${graph.identityFlow.value?.deviceId}",
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
                        val text = intent.getStringExtra("text")
                        if (text == null) {
                            graph.log.w("debug", "CMD SEND_TEXT needs --es text <text>")
                        } else {
                            val result = graph.sender.send(text, SystemClock.elapsedRealtime(), SendSource.DEBUG)
                            graph.log.i("debug", "CMD SEND_TEXT result=$result")
                        }
                    }
                    "SET_CLIPBOARD" -> {
                        val text = intent.getStringExtra("text")
                        if (text == null) {
                            graph.log.w("debug", "CMD SET_CLIPBOARD needs --es text <text>")
                        } else {
                            AndroidClipboard(context).write(text, sensitive = false)
                            graph.log.i("debug", "CMD SET_CLIPBOARD ${ClipboardGuard.fingerprint(text)}")
                        }
                    }
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
