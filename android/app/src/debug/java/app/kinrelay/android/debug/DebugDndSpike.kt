package app.kinrelay.android.debug

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.content.ContextCompat
import app.kinrelay.android.AppGraph
import app.kinrelay.android.notifmirror.MirrorListenerService

/**
 * Spike S5 of 2B.5 (design 11, Android step 1), debug builds only, **emulator only**. Each op prints one `CMD DND_SPIKE ...` line:
 *
 *   DND_SPIKE --es op read
 *   DND_SPIKE --es op set_nm --ei filter N          NotificationManager.setInterruptionFilter (route 1: Do Not Disturb access)
 *   DND_SPIKE --es op set_listener --ei filter N    NotificationListenerService.requestInterruptionFilter (route 2: notification access)
 *   DND_SPIKE --es op rules                         NotificationManager.getAutomaticZenRules (names and states; needs policy access)
 *   DND_SPIKE --es op watch [--ez on false]         a dynamic receiver for ACTION_INTERRUPTION_FILTER_CHANGED that logs each change
 *
 * Filters: 1 all, 2 priority, 3 none (total silence), 4 alarms.
 */
object DebugDndSpike {
    private var receiver: BroadcastReceiver? = null

    fun handle(intent: Intent, context: Context, graph: AppGraph) {
        val nm = context.getSystemService(NotificationManager::class.java)
        when (val op = intent.getStringExtra("op") ?: "read") {
            "read" -> graph.log.i(
                "debug",
                "CMD DND_SPIKE read filter=${nm.currentInterruptionFilter} policyAccess=${nm.isNotificationPolicyAccessGranted} listenerConnected=${MirrorListenerService.isConnected()}",
            )
            "set_nm" -> {
                val filter = intent.getIntExtra("filter", 1)
                val result = try {
                    nm.setInterruptionFilter(filter)
                    "ok"
                } catch (e: Exception) {
                    "${e.javaClass.simpleName}: ${e.message}"
                }
                graph.log.i("debug", "CMD DND_SPIKE set_nm filter=$filter result=$result now=${nm.currentInterruptionFilter}")
            }
            "set_listener" -> {
                val filter = intent.getIntExtra("filter", 1)
                val ok = MirrorListenerService.requestFilter(filter)
                graph.log.i("debug", "CMD DND_SPIKE set_listener filter=$filter called=$ok now=${nm.currentInterruptionFilter}")
            }
            "rules" -> {
                val result = try {
                    nm.automaticZenRules.entries.joinToString(" | ") { (id, r) -> "id=$id name=${r.name} enabled=${r.isEnabled} filter=${r.interruptionFilter} owner=${r.owner?.packageName}" }
                } catch (e: Exception) {
                    "${e.javaClass.simpleName}: ${e.message}"
                }
                graph.log.i("debug", "CMD DND_SPIKE rules $result")
            }
            "watch" -> {
                if (intent.getBooleanExtra("on", true)) {
                    if (receiver == null) {
                        val r = object : BroadcastReceiver() {
                            override fun onReceive(c: Context, i: Intent) {
                                val first = nm.currentInterruptionFilter
                                val zen = android.provider.Settings.Global.getInt(c.contentResolver, "zen_mode", -1)
                                graph.log.i("debug", "CMD DND_SPIKE broadcast filter=$first zen_mode=$zen")
                            }
                        }
                        ContextCompat.registerReceiver(context.applicationContext, r, IntentFilter(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
                        receiver = r
                    }
                } else {
                    receiver?.let { context.applicationContext.unregisterReceiver(it) }
                    receiver = null
                }
                graph.log.i("debug", "CMD DND_SPIKE watch registered=${receiver != null}")
            }
            else -> graph.log.w("debug", "CMD DND_SPIKE unknown op $op")
        }
    }
}
