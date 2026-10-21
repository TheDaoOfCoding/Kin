package app.kinrelay.android.clipboard

import android.app.PendingIntent
import android.graphics.drawable.Icon
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import app.kinrelay.android.R
import app.kinrelay.android.graph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The "Send clipboard" quick settings tile. The tile can't read the clipboard itself (it has no focus), so it
 * starts [ClipboardSendActivity]: through a `PendingIntent` on Android 14+, where the `Intent` overload of
 * `startActivityAndCollapse` is gone for apps that target 34.
 */
class SendClipboardTile : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        val graph = applicationContext.graph
        graph.scope.launch {
            val pc = graph.pairedDevice()
            withContext(Dispatchers.Main) {
                val tile = qsTile ?: return@withContext
                tile.icon = Icon.createWithResource(this@SendClipboardTile, R.drawable.ic_stat_kin)
                tile.label = getString(R.string.send_clipboard)
                tile.state = if (pc == null) Tile.STATE_UNAVAILABLE else Tile.STATE_INACTIVE
                tile.subtitle = pc?.name ?: "Not paired"
                tile.updateTile()
            }
        }
    }

    override fun onClick() {
        super.onClick()
        if (isLocked) unlockAndRun(::launch) else launch()
    }

    private fun launch() {
        val intent = ClipboardSendActivity.intent(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val pending = PendingIntent.getActivity(this, 2, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            startActivityAndCollapse(pending)
        } else {
            // Android 10 to 13 only have this overload; the framework refuses it from Android 14, which is the branch above.
            @Suppress("DEPRECATION", "StartActivityAndCollapseDeprecated")
            startActivityAndCollapse(intent)
        }
    }
}
