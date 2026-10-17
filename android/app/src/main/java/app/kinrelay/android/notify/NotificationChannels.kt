package app.kinrelay.android.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context

/**
 * The three notification channels (milestone 1.4): connection status (low, silent), arrivals (default: text, files and pings from the PC),
 * and transfers (low: progress, and the results of what the phone sent). The channels of earlier builds are deleted.
 */
object NotificationChannels {
    const val LINK = "link"
    const val ARRIVALS = "arrivals"
    const val TRANSFERS = "transfers"

    /** "Ring my phone" (2B.4): the Stop notification. High importance, but **no sound and no vibration of its own**: the ring player does both (on the alarm stream). Created once. */
    const val RING = "kin_ring"

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
                description = "Progress of files going to or coming from your PC, whether what you sent arrived, and the 5-second hold of a screenshot Kin sends by itself"
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
