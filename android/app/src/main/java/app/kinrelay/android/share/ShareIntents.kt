package app.kinrelay.android.share

import android.content.Intent
import android.net.Uri
import androidx.core.content.IntentCompat

/**
 * Reads the files out of a share intent. Only `content:` URIs are taken: any app can send an intent to the exported share
 * activity, and a `file:` URI could point into Kin's own private folder (its identity key, its database). Android has refused to hand
 * out `file:` URIs between apps since 7.0, so this costs real senders nothing.
 */
object ShareIntents {
    /** The shared URIs in order, without duplicates: the clip data first, then `EXTRA_STREAM` (a single URI or a list). */
    fun streams(intent: Intent): List<Uri> {
        val found = LinkedHashSet<Uri>()
        try {
            intent.clipData?.let { clip -> for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let(found::add) }
            IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let(found::add)
            IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { found.addAll(it) }
        } catch (e: RuntimeException) {
            // A sender put something unreadable in the extras: treat it as no files.
        }
        return found.filter { it.scheme == "content" }
    }

    /** True if the intent carries something that isn't a `content:` URI (a `file:` URI, say), which is ignored. */
    fun hasRefusedStreams(intent: Intent): Boolean = try {
        val all = LinkedHashSet<Uri>()
        intent.clipData?.let { clip -> for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let(all::add) }
        IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let(all::add)
        all.any { it.scheme != "content" }
    } catch (e: RuntimeException) {
        false
    }
}
