package app.kinrelay.android.history

import java.util.Locale

/**
 * What a history item is, for the Filter. The same table as the PC's (`FILE_TYPES` in `desktop/src-tauri/src/history.rs`): plan 01, contract C3.
 * Both apps keep their own copy as data and test the same sample names, so the two stay equal. The enum order is the order of the chips.
 */
enum class HistoryType(val label: String) {
    PHOTO("Photos"), VIDEO("Videos"), DOCUMENT("Documents"), APP("Apps"), OTHER("Other files"), TEXT("Texts"), LINK("Links"),
}

object HistoryTypes {
    private val photo = setOf("jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "avif", "tif", "tiff")
    private val video = setOf("mp4", "mov", "mkv", "webm", "avi", "m4v", "3gp", "wmv")
    private val document = setOf("pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp", "epub")
    private val app = setOf("apk", "apks", "xapk")

    private const val APK_MIME = "application/vnd.android.package-archive"

    /** The type of one file: the MIME first (when it says photo, video or app), then the last extension, else [HistoryType.OTHER]. */
    fun classify(name: String, mime: String?): HistoryType {
        val m = mime?.trim()?.lowercase(Locale.ROOT).orEmpty()
        when {
            m.startsWith("image/") -> return HistoryType.PHOTO
            m.startsWith("video/") -> return HistoryType.VIDEO
            m == APK_MIME -> return HistoryType.APP
        }
        val dot = name.lastIndexOf('.')
        val ext = if (dot < 0) "" else name.substring(dot + 1).lowercase(Locale.ROOT)
        return when (ext) {
            in photo -> HistoryType.PHOTO
            in video -> HistoryType.VIDEO
            in document -> HistoryType.DOCUMENT
            in app -> HistoryType.APP
            else -> HistoryType.OTHER
        }
    }
}
