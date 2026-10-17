package app.kinrelay.android.media

import android.content.Context
import android.media.AudioManager
import android.media.VolumeProvider
import android.media.session.MediaController

/**
 * The phone's media volume (design 4.4). If the active session is a remote (cast) route, the volume is **the session's own**: `playbackInfo` current/max, set with `setVolumeTo` and
 * `adjustVolume`, only if it isn't fixed. Otherwise it is `STREAM_MUSIC`: `AudioManager`, set with no `FLAG_SHOW_UI` (the PC is where the user is looking). A fixed-volume device reports
 * none at all. **The lowest level Kin sets by `level` or `delta` is index 1** (only `mute` silences, with `ADJUST_MUTE`), and every call may throw a `SecurityException` (Do Not Disturb on
 * some versions): the runner turns that into `action_failed`. Kin never touches the ringer mode, `STREAM_RING`, `STREAM_NOTIFICATION`, `STREAM_ALARM` or Do Not Disturb.
 */
class AndroidVolume(context: Context, private val activeController: () -> MediaController?) : MediaVolumeControl {
    private val audio: AudioManager = context.applicationContext.getSystemService(AudioManager::class.java)

    private fun remote(): MediaController.PlaybackInfo? =
        activeController()?.playbackInfo?.takeIf { it.playbackType == MediaController.PlaybackInfo.PLAYBACK_TYPE_REMOTE }

    override fun read(): MediaVolume? {
        remote()?.let { info ->
            if (info.volumeControl == VolumeProvider.VOLUME_CONTROL_FIXED || info.maxVolume <= 0) return null
            return MediaVolume(MediaVolumeMath.levelOf(info.currentVolume, info.maxVolume), muted = false)
        }
        if (audio.isVolumeFixed) return null
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        return MediaVolume(MediaVolumeMath.levelOf(audio.getStreamVolume(AudioManager.STREAM_MUSIC), max), audio.isStreamMute(AudioManager.STREAM_MUSIC))
    }

    override fun setLevel(level: Int): Boolean {
        val controller = activeController()
        remote()?.let { info ->
            if (info.volumeControl != VolumeProvider.VOLUME_CONTROL_ABSOLUTE || info.maxVolume <= 0) return false
            controller?.setVolumeTo(MediaVolumeMath.indexOfLevel(level, info.maxVolume), 0)
            return true
        }
        if (audio.isVolumeFixed) return false
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        // Setting a level undoes a mute, as the volume buttons do.
        if (audio.isStreamMute(AudioManager.STREAM_MUSIC)) audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_UNMUTE, 0)
        audio.setStreamVolume(AudioManager.STREAM_MUSIC, MediaVolumeMath.indexOfLevel(level, max), 0)
        return true
    }

    override fun adjust(delta: Int): Boolean {
        val controller = activeController()
        remote()?.let { info ->
            if (info.volumeControl == VolumeProvider.VOLUME_CONTROL_FIXED || info.maxVolume <= 0) return false
            if (info.volumeControl == VolumeProvider.VOLUME_CONTROL_ABSOLUTE) controller?.setVolumeTo(MediaVolumeMath.indexAfterDelta(info.currentVolume, delta, info.maxVolume), 0)
            else controller?.adjustVolume(if (delta > 0) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER, 0)
            return true
        }
        if (audio.isVolumeFixed) return false
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        if (audio.isStreamMute(AudioManager.STREAM_MUSIC)) audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_UNMUTE, 0)
        audio.setStreamVolume(AudioManager.STREAM_MUSIC, MediaVolumeMath.indexAfterDelta(audio.getStreamVolume(AudioManager.STREAM_MUSIC), delta, max), 0)
        return true
    }

    override fun setMute(mute: Boolean): Boolean {
        if (remote() != null || audio.isVolumeFixed) return false
        audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, if (mute) AudioManager.ADJUST_MUTE else AudioManager.ADJUST_UNMUTE, 0)
        return true
    }
}
