package app.kinrelay.android.alerts

import app.kinrelay.android.log.KinLog
import java.util.concurrent.atomic.AtomicLong

/** What [AlertInbound] needs from the phone: the history row and the notification. The real ones are [AlertHub]'s; the tests record the calls. */
interface AlertPorts {
    /** Writes the history row of kind `alert` (the PC's name, the id, the cleaned text and title). */
    suspend fun record(id: String, pcName: String, alert: AlertMessage)

    /** Posts or updates "From your PC": one aggregated notification, no buttons, nothing on the clipboard, nothing opened. False when nothing was posted (the notification permission is denied). */
    fun show(pcName: String, alert: AlertMessage): Boolean
}

/** Counts only (design 8, T1): never a text or a title. */
class AlertStats {
    val received = AtomicLong()
    val shown = AtomicLong()
    val quiet = AtomicLong()

    fun line(): String = "alerts: received ${received.get()}, shown ${shown.get()}, quiet ${quiet.get()}"
}

/**
 * What the phone does with a valid, acked `alert` (design 4.4 of 2B.5): **record it in the history** (always, a row of kind `alert`), then **show** it as a notification on the `kin_alerts`
 * channel, or, **while this phone is paused, show nothing** (2B.4 3.5, quiet: the history row and the count "N received while paused" are all there is). It never touches the clipboard, never opens
 * anything and has no buttons. Android's own Do Not Disturb applies to the notification as to any other: Kin never overrides it. The log gets lengths and counts, never a text or a title.
 */
class AlertInbound(
    private val ports: AlertPorts,
    /** This phone is paused (the wall-clock gate). */
    private val paused: () -> Boolean,
    /** A manual arrival was handled quietly because of the pause ("3 received while paused"). */
    private val onQuiet: () -> Unit,
    private val log: KinLog,
    val stats: AlertStats = AlertStats(),
) {
    suspend fun onAlert(id: String, pcName: String, alert: AlertMessage) {
        stats.received.incrementAndGet()
        // The history row first: it is the one record of an alert, shown or not.
        try {
            ports.record(id, pcName, alert)
        } catch (e: Exception) {
            log.w("alerts", "alert: the history row could not be written (${e.javaClass.simpleName})")
        }
        if (paused()) {
            stats.quiet.incrementAndGet()
            onQuiet()
            log.i("alerts", "alert handled quietly: this phone is paused (${alert.text.length} chars). ${stats.line()}")
            return
        }
        try {
            if (ports.show(pcName, alert)) stats.shown.incrementAndGet()
        } catch (e: Exception) {
            log.w("alerts", "alert: the notification could not be posted (${e.javaClass.simpleName})")
        }
        log.i("alerts", "alert received (${alert.text.length} chars, title ${alert.title?.length ?: 0} chars, waited ${alert.waitedMs?.let { it / 1000 } ?: 0} s). ${stats.line()}")
    }
}
