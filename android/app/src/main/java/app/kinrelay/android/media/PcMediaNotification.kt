package app.kinrelay.android.media

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.media.session.MediaSession
import android.os.Build
import androidx.core.content.ContextCompat
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R

/**
 * The `MediaStyle` notification that shows the PC's media on the phone (design 4.5). One fixed ID on its own channel (`kin_pc_media`: low importance, no
 * sound, no vibration, no badge). It is posted from the process the link service already holds: **no new foreground service** and no media playback type.
 * With no `POST_NOTIFICATIONS` (Android 13+) nothing is posted and [post] says so.
 */
class PcMediaNotification(context: Context) {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)

    init {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Your PC's media", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shows what your PC is playing, with buttons that control it"
                setShowBadge(false)
                setSound(null, null)
                enableVibration(false)
            }
        )
    }

    fun canPost(): Boolean = Build.VERSION.SDK_INT < 33 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun broadcast(action: String, code: Int): PendingIntent =
        PendingIntent.getBroadcast(
            context, code, Intent(context, PcMediaReceiver::class.java).setAction(action),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    /** Posts or updates the notification. False when the app may not post notifications: the caller then hides the session too (it would take the media keys with no card to show for it). */
    fun post(token: MediaSession.Token, view: PcMediaView, art: Bitmap?, showOnLockScreen: Boolean): Boolean {
        if (!canPost()) return false
        val playing = view.state == PlayState.PLAYING
        val actions = ArrayList<Notification.Action>(3)
        val compact = ArrayList<Int>(3)
        if (MediaAction.PREV in view.actions) actions += action(R.drawable.ic_media_previous, "Previous", ACTION_PREV, 1)
        if (MediaMapping.playOrToggle(view.actions) != null || MediaMapping.pauseOrToggle(view.actions) != null) {
            compact += actions.size
            actions += if (playing) action(R.drawable.ic_media_pause, "Pause", ACTION_PAUSE, 2) else action(R.drawable.ic_media_play, "Play", ACTION_PLAY, 3)
        }
        if (MediaAction.NEXT in view.actions) actions += action(R.drawable.ic_media_next, "Next", ACTION_NEXT, 4)
        if (compact.isEmpty() && actions.isNotEmpty()) compact += 0
        val style = Notification.MediaStyle().setMediaSession(token)
        if (compact.isNotEmpty()) style.setShowActionsInCompactView(*compact.take(3).toIntArray())
        val builder = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stat_kin)
            .setContentTitle(view.titleLine)
            .setContentText(view.deviceLine)
            .setSubText(view.subText)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setOnlyAlertOnce(true)
            .setLocalOnly(true)
            .setOngoing(playing)
            .setShowWhen(false)
            .setVisibility(if (showOnLockScreen) Notification.VISIBILITY_PUBLIC else Notification.VISIBILITY_PRIVATE)
            .setContentIntent(
                PendingIntent.getActivity(
                    context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
            )
            .setDeleteIntent(broadcast(ACTION_DISMISSED, 5))
            .setStyle(style)
        if (!showOnLockScreen) {
            // What a locked phone shows when it hides sensitive content (the phone's own lock-screen setting decides): the device, never the title.
            builder.setPublicVersion(
                Notification.Builder(context, CHANNEL_ID).setSmallIcon(R.drawable.ic_stat_kin).setContentTitle("Media from ${view.deviceName}").build()
            )
        }
        art?.let { builder.setLargeIcon(it) }
        // The buttons also go in the notification itself: Android 13+ builds them from the session's PlaybackState, older versions only know these.
        actions.forEach { builder.addAction(it) }
        manager.notify(ID, builder.build())
        return true
    }

    fun cancel() {
        manager.cancel(ID)
    }

    private fun action(icon: Int, label: String, action: String, code: Int): Notification.Action =
        Notification.Action.Builder(Icon.createWithResource(context, icon), label, broadcast(action, code)).build()

    companion object {
        const val CHANNEL_ID = "kin_pc_media"
        const val ID = 30
        const val ACTION_PLAY = "app.kinrelay.android.media.PLAY"
        const val ACTION_PAUSE = "app.kinrelay.android.media.PAUSE"
        const val ACTION_NEXT = "app.kinrelay.android.media.NEXT"
        const val ACTION_PREV = "app.kinrelay.android.media.PREV"
        const val ACTION_DISMISSED = "app.kinrelay.android.media.DISMISSED"
    }
}
