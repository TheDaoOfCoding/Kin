package app.kinrelay.android.media

import app.kinrelay.android.link.Clock

/** What the runner may do to the active session: the transport buttons. An exception is a refusal by the system or the app (`action_failed`). */
interface MediaTransport {
    fun play()
    fun pause()
    fun next()
    fun prev()
    fun seekTo(positionMs: Long)
}

/** The phone's media volume (design 4.4): `STREAM_MUSIC`, or the session's own for a remote (cast) route. Each call returns false or throws when the system refuses. */
interface MediaVolumeControl {
    fun setLevel(level: Int): Boolean
    fun adjust(delta: Int): Boolean
    fun setMute(mute: Boolean): Boolean

    /** The volume now (to report back as `media.volume`), or null when this device has no settable volume. */
    fun read(): MediaVolume?
}

/**
 * The active session as the runner needs it: its `sid`, what it offers now, its state (for `toggle`) and **its own controls**. All four come from one read of the active session, so the
 * `sid` the runner checks is the session the button then presses (a session switch between the check and the press can't redirect it).
 */
class ActiveSession(val sid: String, val actions: Set<MediaAction>, val state: PlayState, val transport: MediaTransport?)

/** What a command ended in. Success has **no reply** (the effect comes back as `media.state` / `media.volume`). */
sealed interface CmdOutcome {
    data object Done : CmdOutcome

    /** An `error` with the command's `ref`. */
    class Error(val code: String, val message: String) : CmdOutcome

    /** Over the budget and the one `rate_limited` answer of the last 10 s was already given: nothing is said. */
    data object Silent : CmdOutcome
}

/**
 * Runs a PC's `media.cmd` (design 3.6, 4.2, 4.4; vectors `cmdResult[]`). The answers, in this order: `disabled` (this phone's `share` is off), `rate_limited` (at most 20 per 10 s: a
 * bucket of 20 refilled 2 per second; the error itself at most once per 10 s), `not_found` (no active session), `stale` (not the active `sid`, or an action it doesn't list now),
 * `action_failed` (the system or the app refused). A volume command belongs to the device: it needs no session and is never `stale`.
 */
class MediaCommandRunner(
    private val active: () -> ActiveSession?,
    private val volume: MediaVolumeControl,
    private val shareOn: () -> Boolean,
    private val clock: Clock,
    private val stats: MediaStats = MediaStats(),
    /** The phone's volume after a volume command: sent to the PC as `media.volume`. */
    private val volumeChanged: (MediaVolume) -> Unit = {},
) {
    private val bucket = MediaLimits.sourceCmd(clock)
    private val errorSpacer = ErrorSpacer(MediaLimits.ERROR_SPACING_MILLIS, clock)

    fun run(cmd: MediaCmd): CmdOutcome {
        if (!shareOn()) return fail("disabled", "This phone isn't sharing its media.")
        if (!bucket.tryTake()) {
            stats.cmdError("rate_limited")
            return if (errorSpacer.mayAnswer()) CmdOutcome.Error("rate_limited", "Too many media commands.") else CmdOutcome.Silent
        }
        if (cmd.kind == MediaCmdKind.VOLUME) return runVolume(cmd)
        val session = active() ?: return fail("not_found", "Nothing is playing on this phone.")
        if (cmd.sid != session.sid) return fail("stale", "That isn't the session that is shown.")
        if (!offered(cmd.kind, session.actions)) return fail("stale", "The session doesn't offer that button now.")
        val t = session.transport ?: return fail("not_found", "The player is gone.")
        return try {
            when (cmd.kind) {
                MediaCmdKind.PLAY -> t.play()
                MediaCmdKind.PAUSE -> t.pause()
                MediaCmdKind.TOGGLE -> if (session.state == PlayState.PLAYING) t.pause() else t.play()
                MediaCmdKind.NEXT -> t.next()
                MediaCmdKind.PREV -> t.prev()
                MediaCmdKind.SEEK -> t.seekTo(cmd.seekMs ?: 0L)
                MediaCmdKind.VOLUME -> Unit
            }
            stats.cmdIn(cmd.kind.wire)
            CmdOutcome.Done
        } catch (_: Exception) {
            fail("action_failed", "The player refused.")
        }
    }

    private fun runVolume(cmd: MediaCmd): CmdOutcome {
        val ok = try {
            when {
                cmd.level != null -> volume.setLevel(cmd.level)
                cmd.delta != null -> volume.adjust(cmd.delta)
                cmd.mute != null -> volume.setMute(cmd.mute)
                else -> false
            }
        } catch (_: Exception) {
            // A SecurityException for Do Not Disturb, or any refusal: the command failed, nothing else changed (design 4.4).
            false
        }
        if (!ok) return fail("action_failed", "The phone refused to change its volume.")
        stats.cmdIn("volume")
        runCatching { volume.read() }.getOrNull()?.let(volumeChanged)
        return CmdOutcome.Done
    }

    /** A button is allowed when the session lists it: `play` also when it lists `toggle`, `pause` likewise, and `toggle` when it lists any of the three (vectors `cmdResult[]`). */
    private fun offered(kind: MediaCmdKind, actions: Set<MediaAction>): Boolean = when (kind) {
        MediaCmdKind.PLAY -> MediaAction.PLAY in actions || MediaAction.TOGGLE in actions
        MediaCmdKind.PAUSE -> MediaAction.PAUSE in actions || MediaAction.TOGGLE in actions
        MediaCmdKind.TOGGLE -> MediaAction.PLAY in actions || MediaAction.PAUSE in actions || MediaAction.TOGGLE in actions
        MediaCmdKind.NEXT -> MediaAction.NEXT in actions
        MediaCmdKind.PREV -> MediaAction.PREV in actions
        MediaCmdKind.SEEK -> MediaAction.SEEK in actions
        MediaCmdKind.VOLUME -> true
    }

    private fun fail(code: String, message: String): CmdOutcome {
        stats.cmdError(code)
        return CmdOutcome.Error(code, message)
    }
}
