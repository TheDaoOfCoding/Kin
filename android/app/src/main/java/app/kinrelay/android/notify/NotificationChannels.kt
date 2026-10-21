package app.kinrelay.android.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context

/**
 * The notification channels: connection status (low, silent), arrivals (default: text, files and pings from the PC), transfers (low: progress, and the results of what the phone sent),
 * and, from 2B.4 and 2B.5, ring, alerts and the screenshot hold. The channels of earlier builds are deleted.
 */
object NotificationChannels {
    const val LINK = "link"
    const val ARRIVALS = "arrivals"
    const val TRANSFERS = "transfers"

    /** "Ring my phone" (2B.4): the Stop notification. High importance, but **no sound and no vibration of its own**: the ring player does both (on the alarm stream). Created once. */
    const val RING = "kin_ring"

    /** "From your PC" (2B.5): alerts a script or the browser sent, and the PC's low battery. Default importance, because the user asked to be told. Android's Do Not Disturb applies as to any channel. */
    const val ALERTS = "kin_alerts"

    /**
     * The 5-second hold of an auto-sent screenshot (2B.5 review round 2): high importance so it comes up as a heads-up with its Cancel visible, but **no sound and no vibration**. Created once.
     * Sending and its result stay on the quiet *File transfers* channel.
     */
    const val SHOT_HOLD = "kin_shot_hold"

    private val OBSOLETE = listOf("pings", "text", "problems", "transfer_results")

    fun ensure(context: Context) {
        val manager = context.applicationContext.getSystemService(NotificationManager::class.java)
        OBSOLETE.forEach { manager.deleteNotificationChannel(it) }
        manager.createNotificationChannel(
            NotificationChannel(LINK, "Connection status", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shows whether your PC is connected"
                setShowBadge(false)
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(ARRIVALS, "Arrivals", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "Text, links, photos and files your PC sent, and pings"
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(TRANSFERS, "File transfers", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Progress of files going to or coming from your PC, and whether what you sent arrived"
                setShowBadge(false)
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(ALERTS, "From your PC", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "Alerts your PC sends (a script that finished, a download that finished) and your PC's low battery"
                lockscreenVisibility = android.app.Notification.VISIBILITY_PRIVATE
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(SHOT_HOLD, "Screenshot about to be sent", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "The 5 seconds before Kin sends a screenshot or photo by itself, with a button (or a tap) to cancel. Silent."
                setSound(null, null)
                enableVibration(false)
                setShowBadge(false)
            }
        )
        manager.createNotificationChannel(
            NotificationChannel(RING, "Ring my phone", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "The Stop button while your PC rings this phone. The ring itself plays on the alarm volume."
                setSound(null, null)
                enableVibration(false)
                setShowBadge(false)
                lockscreenVisibility = android.app.Notification.VISIBILITY_PUBLIC
            }
        )
    }
}
