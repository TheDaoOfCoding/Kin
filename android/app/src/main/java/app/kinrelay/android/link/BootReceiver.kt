package app.kinrelay.android.link

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import app.kinrelay.android.graph
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/**
 * Restarts the service after a reboot, and after the app was updated (`adb install -r`, the Play Store, an installer): an update kills the
 * process and its foreground service, and nothing else would bring it back until the app is opened. `ACTION_MY_PACKAGE_REPLACED`, like
 * `BOOT_COMPLETED`, is one of the broadcasts that may start a foreground service from the background (Android 12+ restriction exemptions), and
 * Android 15's ban on starting some foreground service types from a boot receiver does not list `connectedDevice`. Whether it works on a given
 * phone is logged, not assumed: the same lines, tagged `boot`, say what arrived and what was done.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action
        if (!restartsService(action)) return
        val graph = context.graph
        graph.log.i("boot", "$action received (sdk ${Build.VERSION.SDK_INT})")
        val pending = goAsync()
        graph.scope.launch {
            try {
                if (graph.pairedDevice() == null) {
                    graph.log.i("boot", "no paired PC; not starting the service")
                } else {
                    LinkService.start(context)
                    graph.log.i("boot", "service start requested")
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                graph.log.e("boot", "restarting the service failed: ${e.javaClass.simpleName}: ${e.message}")
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        /** The broadcasts that bring the service back: a reboot and an update of this app. Anything else is ignored. */
        fun restartsService(action: String?): Boolean = action == Intent.ACTION_BOOT_COMPLETED || action == Intent.ACTION_MY_PACKAGE_REPLACED
    }
}
