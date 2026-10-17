package app.kinrelay.android.media

/** `state` of a `media.state` (protocol 3.3). Only these four exist on the wire. */
enum class PlayState(val wire: String) {
    PLAYING("playing"),
    PAUSED("paused"),
    BUFFERING("buffering"),
    STOPPED("stopped"),
    ;

    companion object {
        fun fromWire(value: String): PlayState? = entries.firstOrNull { it.wire == value }
    }
}

/** An entry of `actions`: what the source's session offers right now (design 3.3). A later version adds more; unknown entries are ignored. */
enum class MediaAction(val wire: String) {
    PLAY("play"),
    PAUSE("pause"),
    TOGGLE("toggle"),
    NEXT("next"),
    PREV("prev"),
    SEEK("seek"),
    ;

    companion object {
        fun fromWire(value: String): MediaAction? = entries.firstOrNull { it.wire == value }
    }
}

/** The volume of the source device, 0 to 100 (design 3.3: `volume {level, muted}`). */
data class MediaVolume(val level: Int, val muted: Boolean)

/** One `media.state`, the full snapshot of the source's active session. Every optional field is null when absent. [positionMs] is the position at the moment of sending (3.4). */
data class MediaState(
    val sid: String,
    val app: String,
    val state: PlayState,
    val title: String? = null,
    val artist: String? = null,
    val album: String? = null,
    val durationMs: Long? = null,
    val positionMs: Long? = null,
    val rateMilli: Int = 1000,
    val actions: List<MediaAction> = emptyList(),
    val artHash: String? = null,
    val iconHash: String? = null,
    val volume: MediaVolume? = null,
)

/** `cmd` of a `media.cmd`. */
enum class MediaCmdKind(val wire: String) {
    PLAY("play"),
    PAUSE("pause"),
    TOGGLE("toggle"),
    NEXT("next"),
    PREV("prev"),
    SEEK("seek"),
    VOLUME("volume"),
    ;

    companion object {
        fun fromWire(value: String): MediaCmdKind? = entries.firstOrNull { it.wire == value }
    }
}

/** What `media.cmd` carries beyond the verb. For `volume`, exactly one of [level], [delta] and [mute] is set; for `seek`, [seekMs]. */
data class MediaCmd(
    val kind: MediaCmdKind,
    val sid: String? = null,
    val seekMs: Long? = null,
    val level: Int? = null,
    val delta: Int? = null,
    val mute: Boolean? = null,
)

/** `media.status`: [show] "I am showing your media", [share] "I am willing to share mine", [access] phone only (`granted` or `missing`). */
data class MediaStatus(val show: Boolean, val share: Boolean, val access: String? = null)

/** Image bytes of a `media.image` that passed every check (hash, magic, size). */
class MediaImage(val hash: String, val kind: String, val bytes: ByteArray)

/** The messages of `media.*` as the app takes them, already validated and cleaned by the receiver (design 3.3 and 3.9). */
sealed interface MediaMsg {
    val id: String

    data class Status(override val id: String, val status: MediaStatus) : MediaMsg
    data class State(override val id: String, val state: MediaState) : MediaMsg
    class Image(override val id: String, val image: MediaImage) : MediaMsg
    data class Clear(override val id: String) : MediaMsg
    data class Volume(override val id: String, val volume: MediaVolume) : MediaMsg
    data class Cmd(override val id: String, val cmd: MediaCmd) : MediaMsg
}
