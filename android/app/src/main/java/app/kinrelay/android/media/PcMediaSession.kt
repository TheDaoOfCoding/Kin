package app.kinrelay.android.media

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioAttributes
import android.media.MediaMetadata
import android.media.VolumeProvider
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.app.PendingIntent
import android.content.Intent
import app.kinrelay.android.MainActivity
import app.kinrelay.android.log.KinLog

/** What the phone's own buttons (the notification, the quick-settings card, a headset) ask of the PC. The session itself has no local effect: the PC's next `media.state` moves the display. */
interface PcMediaControls {
    fun transport(kind: MediaCmdKind)
    fun seek(positionMs: Long)
    fun volumeTo(level: Int)
    fun volumeAdjust(delta: Int)
}

/**
 * The PC's media on the phone (design 4.5): **one framework `MediaSession`** that plays nothing (no `MediaPlayer`, no audio focus, no `mediaPlayback` foreground
 * service), its metadata, its `PlaybackState`, and the `MediaStyle` notification that makes System UI draw it in the quick-settings carousel and on the lock
 * screen. Every callback sends one `media.cmd` through [PcMediaControls] and changes nothing here. Updated only when a `media.state` arrives, never on a timer.
 */
class PcMediaSession(context: Context, private val controls: PcMediaControls, private val log: KinLog) : PcMediaDisplay {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val notification = PcMediaNotification(this.context)
    private var session: MediaSession? = null
    private var provider: VolumeProvider? = null
    private var lastArtKey: Any? = null
    private var lastArt: Bitmap? = null

    /** The actions the source offers now, read by the callbacks (they pick `play` or `toggle`). */
    @Volatile private var actions: Set<MediaAction> = emptySet()

    override val isShown: Boolean get() = session != null

    private val callback = object : MediaSession.Callback() {
        override fun onPlay() = transport(MediaMapping.playOrToggle(actions))

        override fun onPause() = transport(MediaMapping.pauseOrToggle(actions))

        override fun onSkipToNext() = transport(MediaCmdKind.NEXT.takeIf { MediaAction.NEXT in actions })

        override fun onSkipToPrevious() = transport(MediaCmdKind.PREV.takeIf { MediaAction.PREV in actions })

        override fun onSeekTo(pos: Long) {
            if (MediaAction.SEEK in actions) controls.seek(pos.coerceAtLeast(0L))
        }

        private fun transport(kind: MediaCmdKind?) {
            if (kind != null) controls.transport(kind)
        }
    }

    /**
     * Shows [view] (or updates what is shown). Returns whether the notification was posted. **Without the notification permission nothing is posted and no session is kept** (design 4.5,
     * step 7): a session with no card would take the headset and volume keys for a PC the user can't see.
     */
    override fun show(view: PcMediaView, showOnLockScreen: Boolean, volumeKeys: Boolean): Boolean {
        if (!notification.canPost()) {
            hide()
            return false
        }
        actions = view.actions
        val s = session ?: createSession().also { session = it }
        s.setMetadata(metadata(view))
        s.setPlaybackState(playbackState(view))
        applyVolume(s, view.volume, volumeKeys)
        if (!s.isActive) s.isActive = true
        val posted = notification.post(s.sessionToken, view, art(view), showOnLockScreen)
        if (!posted) hide()
        return posted
    }

    /** Only the volume moved (a `media.volume`). */
    override fun updateVolume(volume: MediaVolume?) {
        val p = provider ?: return
        if (volume != null) p.currentVolume = volume.level
    }

    /** Removes everything: the session, the notification, the art (design 4.5, step 7). */
    override fun hide() {
        notification.cancel()
        val s = session ?: return
        session = null
        provider = null
        lastArt = null
        lastArtKey = null
        actions = emptySet()
        runCatching { s.isActive = false }
        runCatching { s.release() }
    }

    /** Test hygiene and the debug `MEDIA_PC_CMD`: the session's own callbacks, exactly what a carousel button calls (no input injection). */
    fun controller(): android.media.session.MediaController? = session?.let { android.media.session.MediaController(context, it.sessionToken) }

    private fun createSession(): MediaSession = MediaSession(context, SESSION_TAG).also {
        it.setCallback(callback, main)
        it.setSessionActivity(
            PendingIntent.getActivity(
                context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        )
    }

    private fun metadata(view: PcMediaView): MediaMetadata {
        val b = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, view.titleLine)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, view.deviceLine)
            .putString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE, view.titleLine)
            .putString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE, view.deviceLine)
        view.album?.let { b.putString(MediaMetadata.METADATA_KEY_ALBUM, it) }
        view.durationMs?.let { b.putLong(MediaMetadata.METADATA_KEY_DURATION, it) }
        art(view)?.let {
            b.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it)
            b.putBitmap(MediaMetadata.METADATA_KEY_ART, it)
        }
        return b.build()
    }

    private fun playbackState(view: PcMediaView): PlaybackState {
        val position = view.positionMs ?: PlaybackState.PLAYBACK_POSITION_UNKNOWN
        return PlaybackState.Builder()
            .setActions(MediaMapping.sessionMaskOf(view.actions))
            .setState(MediaMapping.androidStateOf(view.state, view.rateMilli), position, view.rateMilli / 1000f, SystemClock.elapsedRealtime())
            .build()
    }

    /** The art of this view, decoded once per image (a bounded decode: [BoundedImage]). */
    private fun art(view: PcMediaView): Bitmap? {
        val bytes = view.art ?: return null
        if (lastArtKey === bytes) return lastArt
        lastArtKey = bytes
        lastArt = BoundedImage.decode(bytes)
        return lastArt
    }

    private fun applyVolume(s: MediaSession, volume: MediaVolume?, volumeKeys: Boolean) {
        if (!volumeKeys || volume == null) {
            if (provider != null) {
                provider = null
                s.setPlaybackToLocal(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build())
            }
            return
        }
        val existing = provider
        if (existing != null) {
            existing.currentVolume = volume.level
            return
        }
        val p = object : VolumeProvider(VOLUME_CONTROL_ABSOLUTE, 100, volume.level) {
            override fun onSetVolumeTo(volume: Int) = controls.volumeTo(volume.coerceIn(0, 100))

            override fun onAdjustVolume(direction: Int) {
                if (direction != 0) controls.volumeAdjust(direction.coerceIn(-1, 1) * VOLUME_KEY_STEP)
            }
        }
        provider = p
        s.setPlaybackToRemote(p)
    }

    private companion object {
        const val SESSION_TAG = "kin-pc-media"
        const val VOLUME_KEY_STEP = 5
    }
}

/** Decodes the art a peer sent without trusting its size (design 4.5: bounds first, rejected over 512 px or 2 MiB decoded). */
object BoundedImage {
    const val MAX_SIDE = 512
    const val MAX_DECODED_BYTES = 2 * 1024 * 1024

    fun decode(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0 || bounds.outWidth > MAX_SIDE || bounds.outHeight > MAX_SIDE) return null
        if (bounds.outWidth.toLong() * bounds.outHeight * 4 > MAX_DECODED_BYTES) return null
        return try {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
        } catch (_: Exception) {
            null
        } catch (_: OutOfMemoryError) {
            null
        }
    }
}
