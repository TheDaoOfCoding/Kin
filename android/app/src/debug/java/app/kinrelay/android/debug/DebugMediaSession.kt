package app.kinrelay.android.debug

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Shader
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.core.graphics.createBitmap
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * Debug builds only (design 4.9): a silent test player of Kin's own for the phone -> PC direction. **A `MediaSession` with no `MediaPlayer`, no `AudioTrack`
 * and no audio focus**: nothing on the emulator or the PC can make a sound from it. Its extras carry `kin.debug_media = true`, which is the one exception to
 * "Kin never reads its own sessions". Its callback only counts the commands it gets (no titles). The art is a generated 640 x 640 gradient, so the
 * downscale to 256 px is exercised. Release builds contain none of this.
 */
object DebugMediaSession {
    const val EXTRA = "kin.debug_media"
    private const val TAG = "kin_debug_media"

    private val main = Handler(Looper.getMainLooper())
    private var session: MediaSession? = null
    private var title = "Kin test song"
    private var artist = "Kin test artist"
    private var durationMs = 180_000L
    private var withArt = true

    /** Commands the session received, by name (`play`, `pause`, `next`, `prev`, `seek`), and the last one with its argument. Counts only. */
    val commands = ConcurrentHashMap<String, AtomicLong>()
    @Volatile var lastCommand: String = "none"
        private set

    val isRunning: Boolean get() = session != null

    private fun note(name: String, detail: String = name) {
        commands.computeIfAbsent(name) { AtomicLong() }.incrementAndGet()
        lastCommand = detail
    }

    private val callback = object : MediaSession.Callback() {
        override fun onPlay() = note("play")
        override fun onPause() = note("pause")
        override fun onSkipToNext() = note("next")
        override fun onSkipToPrevious() = note("prev")
        override fun onSeekTo(pos: Long) = note("seek", "seek $pos")
    }

    @Synchronized
    fun start(context: Context, title: String?, artist: String?, durationMs: Long?, art: Boolean = true) {
        stop()
        title?.let { this.title = it }
        artist?.let { this.artist = it }
        durationMs?.let { this.durationMs = it }
        withArt = art
        commands.clear()
        lastCommand = "none"
        val s = MediaSession(context.applicationContext, TAG)
        s.setExtras(Bundle().apply { putBoolean(EXTRA, true) })
        s.setCallback(callback, main)
        session = s
        setMetadata(s)
        setState("paused", 0L, 1000, ALL_ACTIONS)
        s.isActive = true
    }

    @Synchronized
    fun setState(state: String, positionMs: Long, rateMilli: Int, actionMask: Long) {
        val s = session ?: return
        val androidState = when (state) {
            "playing" -> PlaybackState.STATE_PLAYING
            "paused" -> PlaybackState.STATE_PAUSED
            "buffering" -> PlaybackState.STATE_BUFFERING
            else -> PlaybackState.STATE_STOPPED
        }
        s.setPlaybackState(
            PlaybackState.Builder().setActions(actionMask).setState(androidState, positionMs, rateMilli / 1000f, SystemClock.elapsedRealtime()).build()
        )
    }

    /** A change of the title and artist while the session runs (a track change). */
    @Synchronized
    fun setTrack(title: String?, artist: String?, durationMs: Long?) {
        val s = session ?: return
        title?.let { this.title = it }
        artist?.let { this.artist = it }
        durationMs?.let { this.durationMs = it }
        setMetadata(s)
    }

    private fun setMetadata(s: MediaSession) {
        val b = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
            .putString(MediaMetadata.METADATA_KEY_ALBUM, "Kin test album")
            .putLong(MediaMetadata.METADATA_KEY_DURATION, durationMs)
        if (withArt) b.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, gradient(title.hashCode()))
        s.setMetadata(b.build())
    }

    @Synchronized
    fun stop() {
        val s = session ?: return
        session = null
        runCatching { s.isActive = false }
        runCatching { s.release() }
    }

    /** A 640 x 640 gradient from a hue made of [seed]: big enough that the 256 px cap and the 24,576-byte cap both matter. */
    private fun gradient(seed: Int): Bitmap {
        val size = 640
        val bitmap = createBitmap(size, size)
        val hue = ((seed % 360) + 360) % 360
        val paint = Paint().apply {
            shader = LinearGradient(0f, 0f, size.toFloat(), size.toFloat(), Color.HSVToColor(floatArrayOf(hue.toFloat(), 0.8f, 0.9f)), Color.HSVToColor(floatArrayOf(((hue + 120) % 360).toFloat(), 0.9f, 0.4f)), Shader.TileMode.CLAMP)
        }
        Canvas(bitmap).drawRect(0f, 0f, size.toFloat(), size.toFloat(), paint)
        return bitmap
    }

    /** A small generated JPEG (200 x 200, what a PC's `media.image` carries) for the fake PC session; no file, no real picture. */
    fun pcArtJpeg(): ByteArray {
        val bitmap = createBitmap(200, 200)
        Canvas(bitmap).drawRect(0f, 0f, 200f, 200f, Paint().apply { shader = LinearGradient(0f, 0f, 200f, 200f, Color.MAGENTA, Color.BLUE, Shader.TileMode.CLAMP) })
        return java.io.ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, 80, it) }.toByteArray()
    }

    /** play, pause, play/pause, next, previous, seek. */
    const val ALL_ACTIONS = 4L or 2L or 512L or 32L or 16L or 256L
}
