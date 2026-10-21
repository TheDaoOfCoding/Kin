package app.kinrelay.android.notifmirror

import app.kinrelay.android.link.Clock
import app.kinrelay.android.protocol.ErrorCodes

/** How firing a `PendingIntent` ended. [Sent] means `send` returned without an exception: nothing more is known (API 29 and 30 and a blocked start say nothing). */
enum class FireResult { SENT, CANCELED, FAILED }

/** The live notifications as the handler needs them (`NotificationListenerService.getActiveNotifications` in the app, a fake in the unit tests). */
interface ActionPort<T> {
    /** The notifications that are active right now, or null when the listener is not connected (access lost). A one-off call: nothing is cached. */
    fun active(): List<T>?

    fun key(sbn: T): String

    /** Only the cheap fields (package, channel, user): the allow-list check that must come before [snapshot] reads any content. */
    fun quick(sbn: T): QuickInfo
    fun snapshot(sbn: T): NotifSnapshot

    /** Fires the action at [index] of the live notification; with [replyText] it fills the free-form `RemoteInput` first. */
    fun fire(sbn: T, index: Int, replyText: String?): FireResult

    /** `cancelNotification(key)`. */
    fun cancel(sbn: T)
}

/** What goes back to the PC for a `notif.action`: an `ack`, or an `error` with the code and a message that holds no content. */
sealed interface ActionOutcome {
    data object Ack : ActionOutcome
    data class Error(val code: String, val message: String) : ActionOutcome
}

object NotifErrors {
    const val NOT_FOUND = "not_found"
    const val STALE = "stale"
    const val ACTION_FAILED = "action_failed"
    const val DISABLED = "disabled"
    const val RATE_LIMITED = "rate_limited"
    const val BAD_REQUEST = ErrorCodes.BAD_REQUEST
}

/**
 * `notif.action` and `notif.dismiss` from the PC (design 3.7, 4.7, 4.8). **The notification is looked up fresh** (the in-memory `nid` to key map,
 * else every active key hashed again), the action is checked against what the notification holds **now** (its index and its cleaned title: the
 * label the PC was shown), and a button is never pressed that is not the one the user saw. Nothing about the label or the reply text is logged.
 */
class ActionHandler<T>(
    private val port: ActionPort<T>,
    private val policy: suspend () -> MirrorPolicy,
    private val filter: MirrorFilter,
    private val keyOf: (nid: String) -> String?,
    private val clock: Clock,
    private val stats: MirrorStats = MirrorStats(),
    private val onSelfCancel: (nid: String) -> Unit = {},
) {
    private val times = ArrayDeque<Long>()

    private fun find(nid: String, active: List<T>): T? {
        keyOf(nid)?.let { key -> active.firstOrNull { port.key(it) == key }?.let { return it } }
        return active.firstOrNull { Nid.of(port.key(it)) == nid }
    }

    suspend fun onAction(request: NotifWire.ActionRequest): ActionOutcome {
        stats.actions.incrementAndGet()
        val now = clock.elapsedMillis()
        val allowed = synchronized(times) {
            while (times.isNotEmpty() && now - times.first() >= RATE_WINDOW_MILLIS) times.removeFirst()
            (times.size < RATE_LIMIT).also { if (it) times.addLast(now) }
        }
        if (!allowed) return error(NotifErrors.RATE_LIMITED, "Too many actions. Try again in a moment.")

        val current = policy()
        // Mirroring off: nothing about the live notifications is looked at, so "off" says nothing about what is on the phone.
        if (!current.enabled) return error(NotifErrors.DISABLED, "Mirroring is off for this notification.")
        val active = port.active() ?: return error(NotifErrors.DISABLED, "Notification access is off on the phone.")
        // 1. The notification. One that Kin would not have mirrored (app not ticked, own, other profile) answers as a missing one, before any content is read.
        val sbn = find(request.nid, active)?.takeIf { filter.pre(port.quick(it), current) == null } ?: return error(NotifErrors.NOT_FOUND, "That notification is gone.")
        val snapshot = port.snapshot(sbn)
        // 2. Mirroring on, access held, the app allowed, and nothing that is sent without actions (flagged, hidden, redacted).
        val decision = filter.decide(snapshot, current)
        if (decision !is FilterDecision.Send || decision.sensitive || snapshot.redacted) {
            return error(NotifErrors.DISABLED, "Mirroring is off for this notification.")
        }
        // 3. The action is the one the PC showed.
        // For a reply the label is the hint of the post: the cleaned label of its free-form RemoteInput (empty when there is none). For a button, its title.
        val isReply = request.text != null
        val action = snapshot.actions.firstOrNull { it.index == request.index }
        val freeForm = action?.remoteInputs?.firstOrNull { it.freeForm }
        val shown = if (isReply && freeForm != null) freeForm.label else action?.title
        // Both sides are in toast form (U+2028 and the like are spaces); a button whose label is empty in that form was never offered.
        val own = NotifWire.cleanLabel(shown, NotifWire.LABEL_MAX).orEmpty()
        if (action == null || !action.hasIntent || (!isReply && own.isEmpty()) || own != request.label) {
            return error(NotifErrors.STALE, "The notification changed. Look at it again.")
        }
        // 4. A reply needs a free-form RemoteInput; a plain button must have none.
        val hasFreeForm = freeForm != null
        if (isReply && !hasFreeForm) return error(NotifErrors.BAD_REQUEST, "That action can't take a reply.")
        if (!isReply && action.remoteInputs.isNotEmpty()) return error(NotifErrors.BAD_REQUEST, "That action needs a reply.")
        if (action.isActivity == true) return error(NotifErrors.ACTION_FAILED, "That action opens an app, which the phone can't do from the background.")
        // 5. Fire it.
        return when (port.fire(sbn, action.index, request.text)) {
            FireResult.SENT -> ActionOutcome.Ack
            FireResult.CANCELED -> error(NotifErrors.ACTION_FAILED, "The notification was cancelled.")
            FireResult.FAILED -> error(NotifErrors.ACTION_FAILED, "The app didn't take it.")
        }
    }

    /** `notif.dismiss`: cancels the notification when mirroring is on, "Dismiss on the phone too" is on, and it is clearable. No reply either way. Returns true when it was cancelled. */
    suspend fun onDismiss(nid: String): Boolean {
        val current = policy()
        if (!current.enabled || !current.dismissOnPhone) return false
        val active = port.active() ?: return false
        val sbn = find(nid, active)?.takeIf { filter.pre(port.quick(it), current) == null } ?: return false
        val snapshot = port.snapshot(sbn)
        if (snapshot.notClearable || snapshot.ongoing || snapshot.foregroundService) return false
        onSelfCancel(nid)
        port.cancel(sbn)
        stats.dismissed.incrementAndGet()
        return true
    }

    private fun error(code: String, message: String) = ActionOutcome.Error(code, message)

    companion object {
        const val RATE_LIMIT = 10
        const val RATE_WINDOW_MILLIS = 10_000L
    }
}
