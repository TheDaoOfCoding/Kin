package app.kinrelay.android.shots

/** Which kind of image an auto-send is; [wire] is the `origin` of `POST /v1/transfers`. */
enum class ShotKind(val wire: String) {
    SCREENSHOT("screenshot"),
    CAMERA("camera"),
}

/**
 * One MediaStore row, **only** the columns the picker may read (design 5.6): `_ID`, `RELATIVE_PATH`, `MIME_TYPE`, `IS_PENDING`, `DATE_ADDED`, `SIZE`, `OWNER_PACKAGE_NAME`. Never `DATA`,
 * EXIF, location or a thumbnail.
 */
data class ShotRow(
    val id: Long,
    val relativePath: String?,
    val mime: String?,
    val isPending: Boolean,
    val dateAddedSeconds: Long,
    val size: Long,
    val ownerPackage: String?,
    /** `DATE_TAKEN` in milliseconds (0 when the row has none): a copied old screenshot has a fresh `DATE_ADDED` but keeps its old date taken. */
    val dateTakenMillis: Long = 0,
)

/** What the picker needs besides the row: the switches, the pause and what was already handled. */
data class ShotConfig(
    val screenshotsOn: Boolean,
    val cameraOn: Boolean,
    val paused: Boolean,
    val baselineId: Long,
    val handled: Set<Long> = emptySet(),
    /** The wall-clock second at which the baseline was taken (0 when unknown): an image taken before it is not new. */
    val baselineTimeSeconds: Long = 0,
)

sealed interface ShotDecision {
    data class Send(val kind: ShotKind) : ShotDecision

    /** [reason] is one of the `SKIP_*` names (they are the counts in the log). */
    data class Skip(val reason: String) : ShotDecision
}

/**
 * Decides per row whether it is sent (design 5.6, vectors `shot[]`). The rules, in this order:
 *
 * 1 `_ID` at or below the baseline, or already handled, or `DATE_TAKEN` before the moment the baseline was taken (`old`); 2 `IS_PENDING = 1` (`pending`: the observer fires again when it clears); 3 a MIME type that is not an image type (`image/` plus anything) or `SIZE` 0 (`not_image`); 4 `DATE_ADDED`
 * more than 15 s before now (`stale`: a gallery re-index or a restore, not a screenshot); 5 Kin's own folders (`own`: received files must never be sent back); 6 a screenshot
 * (a path with "screenshot" in it, any case, or `OWNER_PACKAGE_NAME` `com.android.systemui`) only if the screenshots switch is on (`off`); 7 a camera photo (`DCIM/Camera`) only if the camera switch is on
 * (`off`); 8 anything else is never sent (`other`: downloads, WhatsApp images, memes, edited copies); 9 paused (`paused`; the caller moves the baseline up to this `_ID`).
 * The rate limit (10) is the caller's: a bucket of 5, refilled 1 per 20 s.
 */
object ShotPicker {
    const val SKIP_OLD = "old"
    const val SKIP_PENDING = "pending"
    const val SKIP_NOT_IMAGE = "not_image"
    const val SKIP_STALE = "stale"
    const val SKIP_OWN = "own"
    const val SKIP_OFF = "off"
    const val SKIP_OTHER = "other"
    const val SKIP_PAUSED = "paused"
    const val SKIP_RATE = "rate"
    const val SKIP_OFFLINE = "offline"

    /** The hold notification could not be shown (notifications off, permission denied, channel blocked): the image is not sent, there'd be no Cancel. */
    const val SKIP_NO_HOLD = "no_hold"

    const val STALE_SECONDS = 15L
    const val SYSTEM_UI = "com.android.systemui"
    private val OWN_FOLDERS = listOf("Pictures/Kin/", "Download/Kin/")
    private const val CAMERA_FOLDER = "DCIM/Camera"

    fun decide(row: ShotRow, nowSeconds: Long, config: ShotConfig): ShotDecision {
        if (row.id <= config.baselineId || row.id in config.handled) return ShotDecision.Skip(SKIP_OLD)
        // A copied old screenshot (a restore, a file manager) is a new row with an old date taken: taken before the feature was turned on, so not new.
        if (row.dateTakenMillis > 0 && config.baselineTimeSeconds > 0 && row.dateTakenMillis / 1000 < config.baselineTimeSeconds) return ShotDecision.Skip(SKIP_OLD)
        if (row.isPending) return ShotDecision.Skip(SKIP_PENDING)
        if (row.mime?.startsWith("image/", ignoreCase = true) != true || row.size <= 0) return ShotDecision.Skip(SKIP_NOT_IMAGE)
        if (nowSeconds - row.dateAddedSeconds > STALE_SECONDS) return ShotDecision.Skip(SKIP_STALE)
        val path = row.relativePath.orEmpty()
        if (OWN_FOLDERS.any { path.startsWith(it, ignoreCase = true) }) return ShotDecision.Skip(SKIP_OWN)
        val kind = when {
            path.contains("screenshot", ignoreCase = true) || row.ownerPackage == SYSTEM_UI -> {
                if (!config.screenshotsOn) return ShotDecision.Skip(SKIP_OFF)
                ShotKind.SCREENSHOT
            }
            path.startsWith(CAMERA_FOLDER, ignoreCase = true) -> {
                if (!config.cameraOn) return ShotDecision.Skip(SKIP_OFF)
                ShotKind.CAMERA
            }
            else -> return ShotDecision.Skip(SKIP_OTHER)
        }
        if (config.paused) return ShotDecision.Skip(SKIP_PAUSED)
        return ShotDecision.Send(kind)
    }
}
