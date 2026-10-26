package app.kinrelay.android.media

/**
 * `PlaybackState` and `MediaController` values <-> the wire (design 4.2, vectors `androidMap[]`). The numbers are the framework's constants, written out so that
 * this stays testable on the JVM without Android classes (a test pins each one against the SDK's value on the emulator, see `MediaMappingTest`).
 */
object MediaMapping {
    const val STATE_NONE = 0
    const val STATE_STOPPED = 1
    const val STATE_PAUSED = 2
    const val STATE_PLAYING = 3
    const val STATE_FAST_FORWARDING = 4
    const val STATE_REWINDING = 5
    const val STATE_BUFFERING = 6
    const val STATE_ERROR = 7
    const val STATE_CONNECTING = 8
    const val STATE_SKIPPING_TO_PREVIOUS = 9
    const val STATE_SKIPPING_TO_NEXT = 10
    const val STATE_SKIPPING_TO_QUEUE_ITEM = 11

    const val ACTION_PAUSE = 2L
    const val ACTION_PLAY = 4L
    const val ACTION_SKIP_TO_PREVIOUS = 16L
    const val ACTION_SKIP_TO_NEXT = 32L
    const val ACTION_SEEK_TO = 256L
    const val ACTION_PLAY_PAUSE = 512L

    /** `PlaybackState.getPlaybackSpeed()` unknown or negative is read as 1.0 (design 3.4). */
    const val PLAYBACK_POSITION_UNKNOWN = -1L

    /**
     * Android's state to the wire, or null when the session is not a candidate (design 4.2: `STATE_NONE`, `STATE_STOPPED` and `STATE_ERROR` are not shown).
     * Fast-forwarding and rewinding are playing; skipping is buffering (the design lists only the common states).
     */
    fun playStateOf(androidState: Int): PlayState? = when (androidState) {
        STATE_PLAYING, STATE_FAST_FORWARDING, STATE_REWINDING -> PlayState.PLAYING
        STATE_PAUSED -> PlayState.PAUSED
        STATE_BUFFERING, STATE_CONNECTING, STATE_SKIPPING_TO_PREVIOUS, STATE_SKIPPING_TO_NEXT, STATE_SKIPPING_TO_QUEUE_ITEM -> PlayState.BUFFERING
        else -> null
    }

    /** The `actions` a session offers, in the wire's fixed order. `seek` is only offered with a duration (3.3: "only believed with a durationMs"). */
    fun actionsOf(mask: Long, hasDuration: Boolean): List<MediaAction> = buildList {
        if (mask and ACTION_PLAY != 0L) add(MediaAction.PLAY)
        if (mask and ACTION_PAUSE != 0L) add(MediaAction.PAUSE)
        if (mask and ACTION_PLAY_PAUSE != 0L) add(MediaAction.TOGGLE)
        if (mask and ACTION_SKIP_TO_NEXT != 0L) add(MediaAction.NEXT)
        if (mask and ACTION_SKIP_TO_PREVIOUS != 0L) add(MediaAction.PREV)
        if (hasDuration && mask and ACTION_SEEK_TO != 0L) add(MediaAction.SEEK)
    }

    /** The viewer's `PlaybackState` actions for what the source offers (the reverse of [actionsOf], vectors `toAndroid[]`): one bit per action, `toggle` is `ACTION_PLAY_PAUSE` alone. */
    fun maskOf(actions: Collection<MediaAction>): Long {
        var mask = 0L
        for (a in actions) {
            mask = mask or when (a) {
                MediaAction.PLAY -> ACTION_PLAY
                MediaAction.PAUSE -> ACTION_PAUSE
                MediaAction.TOGGLE -> ACTION_PLAY_PAUSE
                MediaAction.NEXT -> ACTION_SKIP_TO_NEXT
                MediaAction.PREV -> ACTION_SKIP_TO_PREVIOUS
                MediaAction.SEEK -> ACTION_SEEK_TO
            }
        }
        return mask
    }

    /**
     * The mask the viewer's session really sets: [maskOf], and when the source offers only `toggle` the play and pause bits as well (System UI draws the play/pause button from them, not
     * from `ACTION_PLAY_PAUSE`; a press is then sent as the `toggle` the source listed, see [playOrToggle]).
     */
    fun sessionMaskOf(actions: Collection<MediaAction>): Long {
        val mask = maskOf(actions)
        return if (MediaAction.TOGGLE in actions) mask or ACTION_PLAY or ACTION_PAUSE else mask
    }

    /** The viewer's `PlaybackState` state: a playing session at rate 0 behaves as paused (3.3). */
    fun androidStateOf(state: PlayState, rateMilli: Int = MediaWire.DEFAULT_RATE_MILLI): Int = when (state) {
        PlayState.PLAYING -> if (rateMilli <= 0) STATE_PAUSED else STATE_PLAYING
        PlayState.PAUSED -> STATE_PAUSED
        PlayState.BUFFERING -> STATE_BUFFERING
        PlayState.STOPPED -> STATE_STOPPED
    }

    /** `playbackSpeed` to `rateMilli` (vectors `androidMap[]`): rounded to thousandths and cut to 0..4,000; a negative speed is 0, a number that isn't one is 1.0. Whatever the state: a paused session at speed 0 sends 0. */
    fun rateMilliOf(speed: Float): Int {
        if (speed.isNaN() || speed.isInfinite()) return MediaWire.DEFAULT_RATE_MILLI
        return Math.round(speed * 1000f).coerceIn(0, MediaWire.MAX_RATE_MILLI)
    }

    /** What a viewer's play or pause button asks the source for: `play` or `pause` when listed, else the `toggle` the source listed (null: nothing is offered). */
    fun playOrToggle(actions: Set<MediaAction>): MediaCmdKind? = when {
        MediaAction.PLAY in actions -> MediaCmdKind.PLAY
        MediaAction.TOGGLE in actions -> MediaCmdKind.TOGGLE
        else -> null
    }

    fun pauseOrToggle(actions: Set<MediaAction>): MediaCmdKind? = when {
        MediaAction.PAUSE in actions -> MediaCmdKind.PAUSE
        MediaAction.TOGGLE in actions -> MediaCmdKind.TOGGLE
        else -> null
    }

    /** What the one play/pause button of the Home card sends: pause while [playing], else play; the source's `toggle` when that is all it lists (null: nothing is offered). */
    fun playPauseCommand(playing: Boolean, actions: Set<MediaAction>): MediaCmdKind? =
        if (playing) pauseOrToggle(actions) else playOrToggle(actions)

    /** The second line a phone viewer shows: `<artist> · <PC name>` (3.11), the PC name alone when there is no artist. */
    fun deviceLine(artist: String?, deviceName: String): String =
        if (artist.isNullOrEmpty()) deviceName else "$artist \u00B7 $deviceName"
}
