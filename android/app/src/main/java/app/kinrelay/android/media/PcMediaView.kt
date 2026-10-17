package app.kinrelay.android.media

/**
 * What the phone shows of the PC's media (design 4.5): the source's `media.state`, the position as of the moment it is drawn, and the art bytes the source
 * sent. Built by [PcMediaModel] and handed to [PcMediaSession], which only translates it into framework objects. [positionMs] null means "unknown": no bar.
 */
class PcMediaView(
    val sid: String,
    val app: String,
    val state: PlayState,
    val title: String?,
    val artist: String?,
    val album: String?,
    val durationMs: Long?,
    val positionMs: Long?,
    val rateMilli: Int,
    val actions: Set<MediaAction>,
    /** The art as JPEG or PNG bytes (checked when it arrived), or null. */
    val art: ByteArray?,
    val volume: MediaVolume?,
    /** The PC's name as the user knows it (cleaned): the second line says `<artist> · <PC>`, the sub text `<app> on <PC>`. */
    val deviceName: String,
) {
    /** The title line: the title, else the app name (design 3.11). */
    val titleLine: String get() = title ?: app

    val deviceLine: String get() = MediaMapping.deviceLine(artist, deviceName)

    val subText: String get() = "$app on $deviceName"
}
