package app.kinrelay.android.notifmirror

import android.content.ComponentName
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import app.kinrelay.android.graph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch

/**
 * The system binds this service once the user gives Kin notification access (a special access, not a permission Kin can ask for). It does nothing
 * until a notification changes: there is no timer, no polling and no wake lock (design 4.11). Each callback only queues the notification; one
 * coroutine takes them in order, so a post and the removal that follows can't swap places. The decisions are in [MirrorEngine].
 *
 * It lives in the same process as the link service and uses whatever connection exists: it **never starts `LinkService`** (a background start of
 * a foreground service isn't allowed on newer versions), and with no connection a notification is dropped, not kept.
 */
class MirrorListenerService : NotificationListenerService() {
    private sealed interface Event {
        class Posted(val item: SbnItem) : Event
        class Removed(val item: SbnItem, val reason: Int) : Event
    }

    private val events = Channel<Event>(Channel.UNLIMITED)
    private var worker: Job? = null
    private var rebound = false

    override fun onListenerConnected() {
        super.onListenerConnected()
        val graph = applicationContext.graph
        instance = this
        rebound = false
        worker?.cancel()
        worker = graph.scope.launch(Dispatchers.Default) {
            for (event in events) {
                try {
                    when (event) {
                        is Event.Posted -> graph.mirror.engine.onPosted(event.item)
                        is Event.Removed -> graph.mirror.engine.onRemoved(event.item, event.reason)
                    }
                } catch (e: Exception) {
                    // One bad notification must not stop the rest. The log has the class, never the notification.
                    graph.log.w("mirror", "mirror: a notification failed (${e.javaClass.simpleName})")
                }
            }
        }
        graph.mirror.listenerChanged(true)
    }

    override fun onListenerDisconnected() {
        val graph = applicationContext.graph
        if (instance === this) instance = null
        graph.mirror.listenerChanged(false)
        // The system unbinds an app whose access is revoked (nothing to do) but also, on some phones, one it just killed: ask once to be bound again.
        if (!rebound) {
            rebound = true
            runCatching { requestRebind(ComponentName(this, MirrorListenerService::class.java)) }
        }
        super.onListenerDisconnected()
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        events.close()
        worker?.cancel()
        super.onDestroy()
    }

    override fun onNotificationPosted(sbn: StatusBarNotification, rankingMap: RankingMap) {
        events.trySend(Event.Posted(SbnItem(sbn, rankingOf(rankingMap, sbn.key))))
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification, rankingMap: RankingMap, reason: Int) {
        events.trySend(Event.Removed(SbnItem(sbn, rankingOf(rankingMap, sbn.key)), reason))
    }

    // Not overridden on purpose: ranking changes, interruption filter changes and channel changes are ignored at once.

    private fun rankingOf(map: RankingMap?, key: String): Ranking? = map?.let { m -> Ranking().takeIf { m.getRanking(key, it) } }

    companion object {
        @Volatile private var instance: MirrorListenerService? = null

        /** The notifications that are active right now (a one-off call; nothing is cached), or null when the listener is not connected. */
        fun activeItems(): List<SbnItem>? {
            val service = instance ?: return null
            return try {
                val ranking = service.currentRanking
                service.activeNotifications.orEmpty().map { sbn -> SbnItem(sbn, ranking?.let { r -> Ranking().takeIf { r.getRanking(sbn.key, it) } }) }
            } catch (_: Exception) {
                null
            }
        }

        /** Test hygiene (the debug command `MIRROR_CLEAR_ALL`): cancels every notification of every app, as "Clear all" does. Nothing in the app calls it. */
        fun cancelAll() {
            // Kin's own notifications stay: cancelling the foreground service's notification stops the service (and, on Android 10, kills the process).
            runCatching {
                val service = instance ?: return
                service.activeNotifications.orEmpty().filter { it.packageName != service.packageName }.forEach { service.cancelNotification(it.key) }
            }
        }

        fun cancel(key: String) {
            runCatching { instance?.cancelNotification(key) }
        }
    }
}
