package app.kinrelay.android.debug

import android.app.Activity
import android.util.Log
import app.kinrelay.android.KinApp
import app.kinrelay.android.clipboard.AndroidClipboard
import app.kinrelay.android.clipboard.ClipboardGuard

/**
 * Debug builds only: reads the phone's real clipboard, which a background component can't do, and prints it to
 * logcat (not to the log file) as `CMD CLIPBOARD len=<code points> sha=<8 hex> text=<json string>`:
 *
 *   adb shell am start -n app.kinrelay.android/.debug.DebugClipboardActivity
 *   adb logcat -d -s KinRelay | findstr "CMD CLIPBOARD"
 *
 * It needs the DUMP permission to start, which the adb shell has and no normal app has.
 */
class DebugClipboardActivity : Activity() {
    private var done = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        val text = AndroidClipboard.read(this)
        val line = if (text == null) {
            "CMD CLIPBOARD empty"
        } else {
            "CMD CLIPBOARD ${ClipboardGuard.fingerprint(text)} text=${quote(text)}"
        }
        Log.i(KinApp.LOG_TAG, line)
        finish()
    }

    private fun quote(text: String): String = buildString {
        append('"')
        for (c in text) when {
            c == '"' -> append("\\\"")
            c == '\\' -> append("\\\\")
            c == '\n' -> append("\\n")
            c == '\r' -> append("\\r")
            c == '\t' -> append("\\t")
            c.code < 0x20 -> append("\\u%04x".format(c.code))
            else -> append(c)
        }
        append('"')
    }
}
