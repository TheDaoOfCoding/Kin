package app.kinrelay.android.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Writing the phone's clipboard. Writing from the background is allowed on every Android version. */
interface ClipboardPort {
    /** [sensitive] hides the content from the system's clipboard confirmation (Android 13+). */
    suspend fun write(text: String, sensitive: Boolean)
}

class AndroidClipboard(context: Context) : ClipboardPort {
    private val manager = context.applicationContext.getSystemService(ClipboardManager::class.java)

    override suspend fun write(text: String, sensitive: Boolean) {
        withContext(Dispatchers.Main) {
            val clip = ClipData.newPlainText("Kin", text)
            if (sensitive && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                clip.description.extras = PersistableBundle().apply { putBoolean(EXTRA_IS_SENSITIVE, true) }
            }
            manager.setPrimaryClip(clip)
        }
    }

    companion object {
        /** `ClipDescription.EXTRA_IS_SENSITIVE` (API 33), written out so it also compiles for older targets. */
        private const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"

        /**
         * The text on the clipboard, or null. Only the focused app may read it (Android 10+), so call this from
         * an activity that has window focus.
         */
        fun read(context: Context): String? {
            val manager = context.getSystemService(ClipboardManager::class.java)
            val clip = manager.primaryClip ?: return null
            if (clip.itemCount == 0) return null
            val description = clip.description
            val isText = description.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN) ||
                description.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML)
            if (!isText) return null
            return clip.getItemAt(0).coerceToText(context)?.toString()
        }
    }
}
