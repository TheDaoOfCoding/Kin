package app.kinrelay.android.media

/** What [SourceSwitch] drives: [MediaSource], or a fake in the unit tests. */
interface SourceControl {
    val isRunning: Boolean

    /** False when the notification access is missing (nothing was registered). */
    fun start(): Boolean

    fun stop()
}

/**
 * Starts and stops the phone's session reader as the hub's conditions change (design 4.2), and **tells the sender when the reader stops**: whatever the PC was showing for this phone's
 * session has no source any more (notification access revoked while sharing, the PC stopped showing, the connection went), so the sender's held session is dropped and a `media.clear`
 * follows after its 1.5 s debounce (if the PC was showing something and `share` is still on; with `share` off the sender already sent its clear).
 */
class SourceSwitch(private val source: () -> SourceControl, private val sender: MediaSender) {
    /** What [apply] did. */
    enum class Result { STARTED, START_FAILED, STOPPED, UNCHANGED }

    @Synchronized
    fun apply(want: Boolean): Result {
        val s = source()
        return when {
            want && !s.isRunning -> if (s.start()) Result.STARTED else Result.START_FAILED
            !want && s.isRunning -> {
                s.stop()
                sender.clear()
                Result.STOPPED
            }
            else -> Result.UNCHANGED
        }
    }
}
