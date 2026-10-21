package app.kinrelay.android.debug

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import app.kinrelay.android.AppGraph
import app.kinrelay.android.alerts.AlertMessage
import app.kinrelay.android.dnd.DndFilter
import app.kinrelay.android.presence.Battery
import app.kinrelay.android.protocol.MessageIds

/**
 * The 2B.5 debug commands (design 11), debug builds only. Every one prints a `CMD ...` line with numbers and words, never a text. They change the **emulator** only (`cmd notification set_dnd`,
 * `allow_dnd` and `allow_listener` are the shell's side of the same tests: the emulator's, never a real phone's).
 *
 *   DND_STATE                                          the switches, the access, the hold, the filters, the follower's memory
 *   DND_SET [--ez follow b] [--ez share b] [--es filter priority|alarms]
 *   DND_RELEASE_NOW                                    "Release now" of Settings
 *   DND_REFRESH                                        look at the access again (as after the system's access screen)
 *   DND_USER --es filter all|priority|alarms|none      a stand-in for the user's own Do Not Disturb on **API 29**, where `cmd notification` has no `set_dnd`
 *                                                      (needs `cmd notification allow_dnd app.kinrelay.android`; on API 35+ it would make Kin's own rule: use `set_dnd` there)
 *   FOCUS_FAKE --es event connect|snapshot|close|unlink|start [--ez on b] [--ez presence b]
 *                                                      feeds the follower what a PC would, without a PC: `connect` (the hello; presence = whether it lists `presence`, default true),
 *                                                      `snapshot --ez on true|false` (the PC's `focus`), `close`, `unlink`, `start`
 *   ALERT_STATE                                        counts of alerts (received, shown, quiet)
 *   ALERT_FROM_PC --es text T [--es title T] [--el waitedMs N] [--es id I]
 *                                                      an `alert` as if the PC had sent it, through the real inbound path (history row, notification unless paused)
 *   PC_BATTERY_FAKE --ei level N --ez charging b       a battery of the PC through the same nudge a `presence` snapshot feeds
 */
object DebugDnd {
    private val COMMANDS = setOf("DND_STATE", "DND_SET", "DND_RELEASE_NOW", "DND_REFRESH", "DND_USER", "FOCUS_FAKE", "ALERT_STATE", "ALERT_FROM_PC", "PC_BATTERY_FAKE")

    fun handles(command: String): Boolean = command in COMMANDS

    suspend fun handle(command: String, intent: Intent, context: Context, graph: AppGraph) {
        when (command) {
            "DND_STATE" -> graph.log.i("debug", "CMD DND_STATE ${graph.dnd.debugLine()}")
            "DND_SET" -> {
                if (intent.hasExtra("follow")) graph.dndStore.setFollow(intent.getBooleanExtra("follow", false))
                if (intent.hasExtra("share")) graph.dndStore.setShare(intent.getBooleanExtra("share", false))
                intent.getStringExtra("filter")?.let { graph.dndStore.setFollowFilter(it) }
                kotlinx.coroutines.delay(500)
                graph.log.i("debug", "CMD DND_SET ${graph.dnd.debugLine()}")
            }
            "DND_RELEASE_NOW" -> {
                graph.dnd.releaseNow()
                kotlinx.coroutines.delay(500)
                graph.log.i("debug", "CMD DND_RELEASE_NOW ${graph.dnd.debugLine()}")
            }
            "DND_REFRESH" -> {
                graph.dnd.refreshAccess()
                kotlinx.coroutines.delay(500)
                graph.log.i("debug", "CMD DND_REFRESH ${graph.dnd.debugLine()}")
            }
            "DND_USER" -> {
                val nm = context.getSystemService(NotificationManager::class.java)
                val filter = when (intent.getStringExtra("filter")) {
                    "priority" -> DndFilter.PRIORITY
                    "alarms" -> DndFilter.ALARMS
                    "none" -> DndFilter.NONE
                    else -> DndFilter.ALL
                }
                val result = try {
                    nm.setInterruptionFilter(filter)
                    "ok"
                } catch (e: Exception) {
                    e.javaClass.simpleName
                }
                kotlinx.coroutines.delay(500)
                graph.log.i("debug", "CMD DND_USER filter=$filter result=$result now=${nm.currentInterruptionFilter}")
            }
            "FOCUS_FAKE" -> {
                when (val event = intent.getStringExtra("event") ?: "snapshot") {
                    "connect" -> graph.dnd.onConnect(intent.getBooleanExtra("presence", true))
                    "snapshot" -> graph.dnd.onSnapshot(intent.getBooleanExtra("on", true))
                    "close" -> graph.dnd.onClose()
                    "unlink" -> graph.dnd.onUnlink()
                    "start" -> graph.dnd.onServiceStart()
                    else -> graph.log.w("debug", "CMD FOCUS_FAKE unknown event $event")
                }
                kotlinx.coroutines.delay(800)
                graph.log.i("debug", "CMD FOCUS_FAKE ${graph.dnd.debugLine()}")
            }
            "ALERT_STATE" -> graph.log.i("debug", "CMD ALERT_STATE ${graph.alerts.debugLine()}")
            "ALERT_FROM_PC" -> {
                val text = intent.getStringExtra("text")
                if (text == null) {
                    graph.log.w("debug", "CMD ALERT_FROM_PC needs --es text <text>")
                } else {
                    val alert = AlertMessage(text, intent.getStringExtra("title"), if (intent.hasExtra("waitedMs")) intent.getLongExtra("waitedMs", 0L) else null)
                    graph.alerts.onAlert(intent.getStringExtra("id") ?: MessageIds.newId(), "DESK-PC", alert)
                    graph.log.i("debug", "CMD ALERT_FROM_PC ${graph.alerts.debugLine()}")
                }
            }
            "PC_BATTERY_FAKE" -> {
                val level = intent.getIntExtra("level", 14)
                val charging = intent.getBooleanExtra("charging", false)
                val nudged = graph.pcBatteryNudge.onPcBattery("debug-pc", Battery(level, charging), graph.presence.view.value.showOn, graph.pause.paused())
                nudged?.let { graph.alerts.showPcBattery("DESK-PC", it) }
                graph.log.i("debug", "CMD PC_BATTERY_FAKE nudged=${nudged != null}")
            }
        }
    }
}
