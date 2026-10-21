package app.kinrelay.android.transfer

import org.junit.Assert.assertEquals
import org.junit.Test

class MediaKindsTest {
    private val extensions = mapOf("jpg" to "image/jpeg", "mp4" to "video/mp4", "pdf" to "application/pdf", "exe" to "application/x-msdownload")
    private fun classify(name: String, mime: String) = MediaKinds.classify(name, mime) { extensions[it] }

    @Test
    fun photosVideosAndTheRestGoToTheirFolders() {
        assertEquals(Classified(SaveFolder.PICTURES, "image/jpeg"), classify("a.jpg", "image/jpeg"))
        assertEquals(Classified(SaveFolder.MOVIES, "video/mp4"), classify("a.MP4", "video/mp4"))
        assertEquals(Classified(SaveFolder.DOWNLOADS, "application/pdf"), classify("a.pdf", "application/pdf"))
    }

    @Test
    fun theExtensionBeatsTheSendersMimeType() {
        assertEquals(SaveFolder.DOWNLOADS, classify("trick.exe", "image/jpeg").folder)
        assertEquals(SaveFolder.PICTURES, classify("a.jpg", "application/octet-stream").folder)
    }

    @Test
    fun anUnknownExtensionFollowsTheSendersTypeIfItIsWellFormed() {
        assertEquals(SaveFolder.PICTURES, classify("a.heic", "image/heic").folder)
        assertEquals(SaveFolder.DOWNLOADS, classify("noext", "image jpeg").folder)
        assertEquals(Classified(SaveFolder.DOWNLOADS, MediaKinds.DEFAULT_MIME), classify("noext", ""))
    }
}
