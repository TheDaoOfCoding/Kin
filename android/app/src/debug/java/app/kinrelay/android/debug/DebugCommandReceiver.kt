package app.kinrelay.android.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.kinrelay.android.graph
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
                        val url = intent.getStringExtra("url")
                        if (url == null) graph.log.w("debug", "CMD PAIR needs --es url <link>") else graph.pairing.start(url, context)
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
