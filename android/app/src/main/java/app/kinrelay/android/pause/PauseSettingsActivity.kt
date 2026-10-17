package app.kinrelay.android.pause

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Scaffold
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.kinrelay.android.graph
import app.kinrelay.android.ui.PauseCard
import app.kinrelay.android.ui.theme.KinTheme

/**
 * The long-press of the "Pause Kin" tile (`ACTION_QS_TILE_PREFERENCES`): the three choices and Resume, in the same card as Home. Exported only for the system's tile settings intent.
 */
class PauseSettingsActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val graph = applicationContext.graph
        setContent {
            KinTheme {
                Scaffold(modifier = Modifier.fillMaxSize()) { padding ->
                    Column(
                        modifier = Modifier.padding(padding).padding(24.dp).verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        PauseCard(graph, this@PauseSettingsActivity)
                    }
                }
            }
        }
    }
}
