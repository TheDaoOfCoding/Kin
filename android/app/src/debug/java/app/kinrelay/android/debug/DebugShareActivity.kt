package app.kinrelay.android.debug

import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import app.kinrelay.android.KinApp
import app.kinrelay.android.share.ShareActivity
import android.util.Log

/**
 * Debug builds only: starts the real [ShareActivity] with a share intent for the given files, as another app's "Share" would, so a
 * script can test the whole share path with no second app:
 *
 *   adb shell am start -n app.kinrelay.android/.debug.DebugShareActivity --es files "a.jpg,b.bin" [--es action SEND|SEND_MULTIPLE]
 *
 * The files are those made by the `MAKE_FILES` command (name of the file only). The default action is `SEND_MULTIPLE` for more than one
 * file and `SEND` for one. The intent carries `FLAG_GRANT_READ_URI_PERMISSION`, `EXTRA_STREAM` and the clip data, like a real share.
 * It needs the DUMP permission to start, which the adb shell has and no normal app has.
 */
class DebugShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val names = intent.getStringExtra("files")?.split(',')?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty()
        if (names.isEmpty()) {
            Log.w(KinApp.LOG_TAG, "CMD SHARE needs --es files <name,name>")
            finish()
            return
        }
        val uris: List<Uri> = names.map { DebugFiles.uriOf(this, it) }
        val multiple = when (intent.getStringExtra("action")) {
            "SEND" -> false
            "SEND_MULTIPLE" -> true
            else -> uris.size > 1
        }
        val share = Intent(this, ShareActivity::class.java).apply {
            action = if (multiple) Intent.ACTION_SEND_MULTIPLE else Intent.ACTION_SEND
            type = intent.getStringExtra("type") ?: "*/*"
            if (multiple) putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris)) else putExtra(Intent.EXTRA_STREAM, uris[0])
            clipData = ClipData.newRawUri("files", uris[0]).also { clip -> uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) } }
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        Log.i(KinApp.LOG_TAG, "CMD SHARE ${share.action} ${uris.size} files")
        startActivity(share)
        finish()
    }
}
