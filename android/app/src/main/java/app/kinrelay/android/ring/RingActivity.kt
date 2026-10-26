package app.kinrelay.android.ring

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.graph
import app.kinrelay.android.ui.theme.KinAppTheme
import kotlinx.coroutines.delay
import java.lang.ref.WeakReference

/**
 * The Stop screen of "Ring my phone" (design 5.4): shown over the lock screen when Android allows the full-screen intent, otherwise reached from the notification. One huge **Stop** button,
 * the PC's name and a countdown; it keeps the screen on while it rings and finishes itself when the ring ends. Not exported, no recents entry.
 */
class RingActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        current = WeakReference(this)
        // Over the lock screen and turning the screen on (the manifest says it too; minSdk 29 always has these two calls).
        setShowWhenLocked(true)
        setTurnScreenOn(true)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val ring = applicationContext.graph.ring
        setContent {
            KinAppTheme(applicationContext.graph.settings) {
                val ui by ring.ui.collectAsState()
                var secondsLeft by remember { mutableLongStateOf(0L) }
                LaunchedEffect(ui.ringing, ui.endsAtElapsed) {
                    while (ui.ringing) {
                        secondsLeft = ((ui.endsAtElapsed - SystemClock.elapsedRealtime()) / 1000).coerceAtLeast(0)
                        delay(500)
                    }
                }
                LaunchedEffect(ui.ringing) { if (!ui.ringing) finish() }
                Column(
                    modifier = Modifier.fillMaxSize().padding(24.dp),
                    verticalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterVertically),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("${ui.pcName.ifBlank { "Your PC" }} is ringing your phone", style = MaterialTheme.typography.headlineMedium, textAlign = TextAlign.Center)
                    if (!ui.audible) Text("Do Not Disturb is on total silence, so this phone makes no sound.", textAlign = TextAlign.Center)
                    Text("Stops by itself in ${secondsLeft}s", style = MaterialTheme.typography.bodyLarge)
                    Button(onClick = { ring.stopByUser() }, modifier = Modifier.fillMaxWidth().height(160.dp)) {
                        Text("Stop", fontSize = 48.sp)
                    }
                }
            }
        }
    }

    override fun onDestroy() {
        if (current?.get() === this) current = null
        super.onDestroy()
    }

    companion object {
        @Volatile private var current: WeakReference<RingActivity>? = null

        fun intent(context: Context): Intent =
            Intent(context, RingActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_USER_ACTION)

        /** The ring ended: the screen goes (the countdown also notices, this is for a screen that is not composing). */
        fun finishIfOpen() {
            current?.get()?.runOnUiThread { current?.get()?.finish() }
        }
    }
}
