package app.kinrelay.android.ui.theme

import android.graphics.Color as AndroidColor
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import app.kinrelay.android.data.KinSettings

/**
 * [KinTheme] with the skin and theme the user chose in Settings, read live from [settings]: a change shows at once, in every screen of the app.
 * Nothing is drawn until the choice is read (a few milliseconds at start-up), so a Simple user never sees a frame of Warm.
 *
 * Pass the [bars] activity when it draws behind the system bars: they then get light or dark icons to match the theme the user picked (which may
 * not be the phone's own), the way `enableEdgeToEdge()` alone only follows the phone.
 */
@Composable
fun KinAppTheme(settings: KinSettings, bars: ComponentActivity? = null, content: @Composable () -> Unit) {
    val appearance by settings.appearance.collectAsState(initial = null)
    val chosen = appearance ?: return
    if (bars != null) {
        val dark = KinPalette.isDark(chosen.mode, isSystemInDarkTheme())
        DisposableEffect(bars, dark) {
            bars.enableEdgeToEdge(
                statusBarStyle = SystemBarStyle.auto(AndroidColor.TRANSPARENT, AndroidColor.TRANSPARENT) { dark },
                navigationBarStyle = SystemBarStyle.auto(LIGHT_SCRIM, DARK_SCRIM) { dark },
            )
            onDispose { }
        }
    }
    KinTheme(chosen.skin, chosen.mode, content)
}

// The scrims `enableEdgeToEdge()` uses behind a 3-button navigation bar.
private val LIGHT_SCRIM = AndroidColor.argb(0xe6, 0xFF, 0xFF, 0xFF)
private val DARK_SCRIM = AndroidColor.argb(0x80, 0x1b, 0x1b, 0x1b)
