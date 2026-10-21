package app.kinrelay.android.pause

import android.graphics.drawable.Icon
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import app.kinrelay.android.R
import app.kinrelay.android.graph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The "Pause Kin" quick settings tile (design 5.3). **Tap**: not paused is pause for 1 hour, paused is resume. **Long-press** opens [PauseSettingsActivity] (`ACTION_QS_TILE_PREFERENCES`) with the
 * three choices. `STATE_ACTIVE` while paused with the end as the subtitle; unavailable with "Not paired" when no PC is paired. On a locked phone pausing works and resuming asks for the unlock first (the notification's button
 * follows the same rule). The tile follows the stored pause, so it shows a pause set anywhere else.
 */
class PauseTile : TileService() {
    private var job: Job? = null

    override fun onStartListening() {
        super.onStartListening()
        val graph = applicationContext.graph
        job?.cancel()
        // A pause that ended while the phone slept shows as ended: the clock is read again when the tile appears and at each render.
        graph.pause.refresh()
        job = graph.scope.launch {
            combine(graph.pause.view, graph.database.devices().observeAll()) { _, devices -> devices.firstOrNull() }.collect { pc ->
                withContext(Dispatchers.Main) { render(graph.pause.viewNow(), paired = pc != null) }
            }
        }
    }

    override fun onStopListening() {
        job?.cancel()
        job = null
        super.onStopListening()
    }

    private fun render(view: PauseView, paired: Boolean) {
        val tile = qsTile ?: return
        tile.icon = Icon.createWithResource(this, R.drawable.ic_stat_kin)
        tile.label = getString(R.string.pause_tile_label)
        when {
            !paired -> {
                tile.state = Tile.STATE_UNAVAILABLE
                tile.subtitle = "Not paired"
            }
            view.paused -> {
                tile.state = Tile.STATE_ACTIVE
                tile.subtitle = PauseSupport.sentence(this, view.state)
            }
            else -> {
                tile.state = Tile.STATE_INACTIVE
                tile.subtitle = "Off"
            }
        }
        tile.updateTile()
    }

    override fun onClick() {
        super.onClick()
        // Pausing only makes Kin quieter, so it works on a locked phone. Resuming turns auto-send, Do Not Disturb following and alerts back on, so it asks for the unlock first (`unlockAndRun` runs at
        // once on an unlocked phone). The notification's button follows the same rule (2B.5 review).
        if (applicationContext.graph.pause.paused()) unlockAndRun { toggle() } else toggle()
    }

    private fun toggle() {
        val graph = applicationContext.graph
        graph.scope.launch {
            if (graph.pairedDevice() == null) return@launch
            if (graph.pause.paused()) graph.pause.resume() else graph.pause.set(PauseChoice.HOUR)
        }
    }
}
