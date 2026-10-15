package app.kinrelay.android.transfer

/** Where a received file goes and which MIME type it is saved with. */
data class Classified(val folder: SaveFolder, val mime: String)

/**
 * Photos go to `Pictures/Kin`, videos to `Movies/Kin` and everything else to `Download/Kin` (android.md). The MIME type in an
 * `offer` is never trusted (protocol.md): when the file's extension says what it is, the extension wins, so a `.exe` that claims
 * `image/jpeg` is saved as a download, not put in the gallery. The sender's MIME type decides only when the extension tells nothing.
 */
object MediaKinds {
    private val MIME_SHAPE = Regex("[a-z0-9][a-z0-9!#$&^_.+-]*/[a-z0-9][a-z0-9!#$&^_.+-]*")

    fun classify(cleanName: String, senderMime: String, mimeForExtension: (String) -> String?): Classified {
        val extension = cleanName.substringAfterLast('.', "").lowercase()
        val fromExtension = if (extension.isEmpty()) null else mimeForExtension(extension)?.lowercase()?.takeIf { MIME_SHAPE.matches(it) }
        val sender = senderMime.trim().lowercase().takeIf { MIME_SHAPE.matches(it) }
        val mime = fromExtension ?: sender ?: DEFAULT_MIME
        val folder = when {
            mime.startsWith("image/") -> SaveFolder.PICTURES
            mime.startsWith("video/") -> SaveFolder.MOVIES
            else -> SaveFolder.DOWNLOADS
        }
        return Classified(folder, mime)
    }

    const val DEFAULT_MIME = "application/octet-stream"
}
