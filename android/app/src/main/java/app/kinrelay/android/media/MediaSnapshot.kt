package app.kinrelay.android.media

/**
 * The active session of this phone as the sender sees it: plain values, no Android classes (so it can be tested on the JVM, like `NotifSnapshot`). The position is **raw**, as `PlaybackState` has
 * it: [position] at [positionUpdatedAt] (both on `SystemClock.elapsedRealtime()`) and [speed]; the sender extrapolates to the moment it sends (design 3.4). [artHash] / [iconHash] name images
 * the sender has encoded; the bytes travel separately as `media.image`.
 */
data class MediaSnapshot(
    val sid: String,
    val app: String,
    val state: PlayState,
    val title: String?,
    val artist: String?,
    val album: String?,
    val durationMs: Long?,
    val position: Long,
    val positionUpdatedAt: Long,
    val speed: Float,
    val actions: List<MediaAction>,
    val artHash: String?,
    val iconHash: String?,
    val volume: MediaVolume?,
) {
    /** The position for [now] (null: unknown, no progress bar). */
    fun positionAt(now: Long): Long? = MediaPosition.sourceAndroid(state, position, positionUpdatedAt, speed, now, durationMs)

    /** The `media.state` this snapshot is at [now]: strings cleaned and cut again (senders must not exceed the limits), the rate from the speed, an unknown or live duration absent. */
    fun toWire(now: Long, withArt: Boolean = true, withIcon: Boolean = true): MediaState = MediaState(
        sid = sid,
        app = MediaWire.clean(app, MediaWire.APP_MAX) ?: "Media",
        state = state,
        title = MediaWire.clean(title, MediaWire.TITLE_MAX),
        artist = MediaWire.clean(artist, MediaWire.ARTIST_MAX),
        album = MediaWire.clean(album, MediaWire.ALBUM_MAX),
        durationMs = durationMs?.takeIf { it in 1..MediaWire.MAX_DURATION_MS },
        positionMs = positionAt(now),
        rateMilli = MediaMapping.rateMilliOf(speed),
        actions = actions,
        artHash = artHash.takeIf { withArt },
        iconHash = iconHash.takeIf { withIcon },
        volume = volume,
    )

    /**
     * Whether the PC would see the same thing: every field equal, and the position within 1,500 ms of what the extrapolation of the other would be at [now] (3.4: a position the extrapolation
     * explains is no change; one it doesn't is a seek). Used so that a callback that changed nothing visible sends nothing.
     */
    fun sameAs(other: MediaSnapshot, now: Long): Boolean {
        if (copy(position = 0, positionUpdatedAt = 0, speed = 0f) != other.copy(position = 0, positionUpdatedAt = 0, speed = 0f)) return false
        if (MediaMapping.rateMilliOf(speed) != MediaMapping.rateMilliOf(other.speed)) return false
        val a = positionAt(now)
        val b = other.positionAt(now)
        return when {
            a == null || b == null -> a == b
            else -> Math.abs(a - b) <= MediaLimits.POSITION_JUMP_MILLIS
        }
    }
}
