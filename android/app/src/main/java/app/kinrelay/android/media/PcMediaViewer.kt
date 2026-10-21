package app.kinrelay.android.media

import app.kinrelay.android.link.Clock

/** What the viewer draws on: the framework session and notification ([PcMediaSession]), or a fake in the unit tests. */
interface PcMediaDisplay {
    val isShown: Boolean

    /** Shows or updates the card. False when the notification could not be posted (no `POST_NOTIFICATIONS`): nothing may stay on screen then. */
    fun show(view: PcMediaView, showOnLockScreen: Boolean, volumeKeys: Boolean): Boolean

    fun updateVolume(volume: MediaVolume?)

    fun hide()
}

/**
 * The PC-media card as one piece of state (design 4.5, 7): the [PcMediaModel], what the user swiped away, the connection it belongs to, and the rules that decide whether the [PcMediaDisplay]
 * shows, updates or hides. **No Android classes**, so it is unit-tested. The hub feeds it the PC's messages and the user's choices; it is synchronized on itself.
 *
 * [display] makes the real display on first use (a phone that never shows the PC's media never creates it), [displayIfCreated] doesn't make one. [scheduleEnd] gets the milliseconds
 * until a playing state with no new message must be drawn paused at the end (3.4), or null to cancel: the hub owns the one `delay`.
 */
class PcMediaViewer(
    private val clock: Clock,
    private val display: () -> PcMediaDisplay,
    private val displayIfCreated: () -> PcMediaDisplay?,
    private val scheduleEnd: (Long?) -> Unit,
    private val deviceName: () -> String,
) {
    private class Dismissed(val sid: String, val title: String?)

    private val model = PcMediaModel()
    private var dismissed: Dismissed? = null
    private var connection: Any? = null

    /** False after the last draw if Android would not post the notification (the card is then not shown at all). */
    @Volatile var notificationsAllowed = true
        private set

    /** The PC's session as the model holds it. Readable from any thread (the buttons run on the main thread). */
    val current: MediaState? get() = model.current

    /** The PC's volume, known with or without a session. */
    val volume: MediaVolume? get() = model.volume

    val isShown: Boolean get() = displayIfCreated()?.isShown == true

    /**
     * A connection is ready (also a later `hello` of the same connection). **A different connection object means a new PC session: nothing of the old one may stay** (a fast reconnect can
     * come before the old link's close is seen): the model is reset and the card hidden. True when the connection changed.
     */
    @Synchronized
    fun onLinkReady(next: Any): Boolean {
        if (connection === next) return false
        connection = next
        forgetAll()
        return true
    }

    /** The connection ended: nothing about the PC or its session is kept (design 3.1, 3.7). */
    @Synchronized
    fun onLinkClosed() {
        connection = null
        forgetAll()
    }

    /** `show` went off: the card goes and the model with it. */
    @Synchronized
    fun forgetAll() {
        dismissed = null
        hide(forget = true)
    }

    @Synchronized
    fun onState(state: MediaState, policy: MediaPolicy) {
        model.onState(state, clock.elapsedMillis())
        render(policy)
    }

    @Synchronized
    fun onImage(image: MediaImage, policy: MediaPolicy) {
        model.onImage(image)
        // The art may come after the state that names it was shown (it does not: an image goes first): redraw only if this is the one on screen.
        if (model.current?.artHash == image.hash) render(policy)
    }

    @Synchronized
    fun onClear() {
        model.clear()
        dismissed = null
        hide(forget = false)
    }

    @Synchronized
    fun onVolume(volume: MediaVolume) {
        model.onVolume(volume)
        displayIfCreated()?.takeIf { it.isShown }?.updateVolume(volume)
    }

    /** The user swiped the notification away: it stays away until the PC plays again or the track or player changes (design 4.5, step 5). */
    @Synchronized
    fun dismiss() {
        model.current?.let { dismissed = Dismissed(it.sid, it.title) }
        // The notification is gone already: the session goes too, so no card is left that the user just removed.
        hide(forget = false)
    }

    /** Debug and tests: a swipe no longer counts. */
    @Synchronized
    fun forgetDismissal() {
        dismissed = null
    }

    /**
     * Draws what the model holds. Nothing is drawn while `show` is off, after the user swiped the notification away, or without a session. **If Android won't post the notification the
     * session is hidden too** (design 4.5, step 7): a session with no card would still take the headset and volume keys and the user couldn't see why.
     */
    @Synchronized
    fun render(policy: MediaPolicy) {
        if (!policy.show) return
        val view = model.view(deviceName(), clock.elapsedMillis()) ?: return hide(forget = false)
        dismissed?.let { d ->
            if (view.sid == d.sid && view.title == d.title && view.state != PlayState.PLAYING) return
            dismissed = null
        }
        val shown = display()
        val posted = shown.show(view, policy.showOnLockScreen, policy.useVolumeKeys && view.volume != null)
        notificationsAllowed = posted
        if (!posted) {
            // Whatever the display kept of this attempt goes (hiding is idempotent): no session without its card.
            scheduleEnd(null)
            shown.hide()
            return
        }
        scheduleEnd(model.millisUntilEnd(clock.elapsedMillis()))
    }

    /** Removes the session and the notification (design 4.5, step 7). [forget]: also the model (the switch went off or the connection closed). */
    @Synchronized
    fun hide(forget: Boolean) {
        scheduleEnd(null)
        if (forget) model.reset()
        displayIfCreated()?.takeIf { it.isShown }?.hide()
    }
}
