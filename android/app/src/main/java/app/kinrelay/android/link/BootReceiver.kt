package app.kinrelay.android.link

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import app.kinrelay.android.graph
import kotlinx.coroutines.launch

/**
 * Restarts the service after a reboot. Whether a `connectedDevice` foreground service may start
 * from here on Android 15+ is logged, not assumed.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        val graph = context.graph
        graph.log.i("boot", "BOOT_COMPLETED received (sdk ${Build.VERSION.SDK_INT})")
        val pending = goAsync()
        graph.scope.launch {
            try {
                if (graph.pairedDevice() == null) {
                    graph.log.i("boot", "no paired PC; not starting the service")
                } else {
                    LinkService.start(context)
                    graph.log.i("boot", "service start requested")
                }
            } finally {
                pending.finish()
            }
        }
    }
}
