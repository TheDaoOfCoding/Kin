package app.kinrelay.android.media

import app.kinrelay.android.protocol.LinkWire

/**
 * The PC's media as the phone holds it **in memory only** (design 4.5, 7): the latest `media.state` (a full snapshot that replaces the one before), the moment it arrived on the phone's
 * own monotonic clock (the anchor of 3.4), the last few images and the PC's volume. Pure: no Android classes. Nothing here is stored or logged.
 */
class PcMediaModel {
    @Volatile private var state: MediaState? = null
    @Volatile private var pcVolume: MediaVolume? = null
    private var anchor = 0L
    private val images = LinkedHashMap<String, ByteArray>()

    val current: MediaState? get() = state

    /**
     * The PC's volume as last reported, **kept apart from the session**: the PC's volume belongs to the device (design 4.4), so a `media.volume` that arrives while the PC has no media
     * session is kept, and a `media.clear` doesn't lose it. A fixed-volume PC sends none (null).
     */
    val volume: MediaVolume? get() = pcVolume

    /** An image that passed every check: kept by its hash, the last [MAX_IMAGES] only. */
    fun onImage(image: MediaImage) {
        images.remove(image.hash)
        images[image.hash] = image.bytes
        while (images.size > MAX_IMAGES) images.remove(images.keys.first())
    }

    fun onState(next: MediaState, now: Long) {
        state = next
        pcVolume = next.volume
        anchor = now
    }

    /** `media.volume`: the PC's volume changed (nothing else did). Kept whether or not the PC has a session. */
    fun onVolume(volume: MediaVolume) {
        pcVolume = volume
    }

    /** `media.clear`: the PC has no session any more; its volume is still what it was. */
    fun clear() {
        state = null
    }

    /** The connection closed (or `show` went off): everything goes, images and volume included. */
    fun reset() {
        state = null
        pcVolume = null
        images.clear()
    }

    /** What to show at [now]: the position moved on from the anchor, the state a viewer shows (3.4), the art the source sent for this session (none if its image never arrived). */
    fun view(deviceName: String, now: Long): PcMediaView? {
        val s = state ?: return null
        val shown = MediaPosition.shown(s.state, s.positionMs, s.rateMilli, s.durationMs, now - anchor)
        return PcMediaView(
            sid = s.sid,
            app = s.app,
            state = shown.state,
            title = s.title,
            artist = s.artist,
            album = s.album,
            durationMs = s.durationMs,
            positionMs = shown.positionMs,
            rateMilli = s.rateMilli,
            actions = s.actions.toSet(),
            art = s.artHash?.let(images::get),
            volume = pcVolume,
            deviceName = deviceName,
        )
    }

    /**
     * Milliseconds from [now] until a playing state with no new message passes the end by more than 5 s and must be shown paused at the end (3.4), or null when it never will (not playing,
     * no duration or position, rate 0). The hub arms **one** `delay` for it, not a timer.
     */
    fun millisUntilEnd(now: Long): Long? {
        val s = state ?: return null
        val duration = s.durationMs ?: return null
        val position = s.positionMs ?: return null
        val rate = s.rateMilli.coerceIn(0, MediaWire.MAX_RATE_MILLI)
        if (s.state != PlayState.PLAYING || rate == 0) return null
        val over = duration + MediaLimits.END_GRACE_MILLIS - position
        val due = if (over < 0) 0L else over * 1000 / rate + 1
        return maxOf(0L, due - (now - anchor))
    }

    companion object {
        const val MAX_IMAGES = 8

        /** The PC's name for the second line: cleaned, 40 code points, "PC" when it has none. */
        fun deviceName(raw: String?): String = raw?.let { LinkWire.cleanText(it, MediaWire.APP_MAX) } ?: "PC"
    }
}
