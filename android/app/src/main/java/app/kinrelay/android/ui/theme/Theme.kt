package app.kinrelay.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.kinrelay.android.data.KinSkin
import app.kinrelay.android.data.KinThemeMode

/**
 * What a skin changes besides colour: the corner radii, the press lip, whether the art slots show an illustration (Warm) or a plain icon
 * (Simple), and how much shadow a card has. Layout is never part of it.
 */
@Immutable
class KinStyle(
    val skin: KinSkin,
    val dark: Boolean,
    val showIllustrations: Boolean,
    /** Material's five sizes: xs, small, medium and large from the design, and extra large (dialogs) = large. */
    val shapes: Shapes,
    /** The square tile behind an icon (40% of its side in Warm, 30% in Simple). */
    val iconShape: Shape,
    /** The round picture on a list row (a circle in Warm, the same soft square as the icon tile in Simple). */
    val avatarShape: Shape,
    val full: Shape,
    /** The solid band under a pressed-looking button and the selected nav pill. Zero in Simple, which is flat. */
    val pressLip: Dp,
    val pressLipSmall: Dp,
    /** The soft ring around a focused text field. Zero in Simple. */
    val focusRing: Dp,
    val cardElevation: Dp,
    val raisedElevation: Dp,
    /** The tint of the shadows (a warm brown in Warm, a deep teal in Simple). Dark mode has no shadows, only a hairline. */
    val shadowColor: Color,
    /** The faint 1 dp edge that stands in for a shadow in dark mode. Transparent in light mode. */
    val hairline: Color,
)

/** Skin and mode to colours and shapes. Pure, so a unit test can check all four combinations. */
object KinPalette {
    /** "System" follows the phone; Light and Dark are the user's own choice. */
    fun isDark(mode: KinThemeMode, systemDark: Boolean): Boolean = when (mode) {
        KinThemeMode.SYSTEM -> systemDark
        KinThemeMode.LIGHT -> false
        KinThemeMode.DARK -> true
    }

    fun colors(skin: KinSkin, dark: Boolean): KinColors = when (skin) {
        KinSkin.WARM -> if (dark) KinTokens.warmDark else KinTokens.warmLight
        KinSkin.SIMPLE -> if (dark) KinTokens.simpleDark else KinTokens.simpleLight
    }

    fun style(skin: KinSkin, dark: Boolean): KinStyle {
        val warm = skin == KinSkin.WARM
        val xs = if (warm) 8.dp else 6.dp
        val small = if (warm) 14.dp else 12.dp
        val medium = if (warm) 24.dp else 20.dp
        val large = if (warm) 32.dp else 28.dp
        val icon = RoundedCornerShape(percent = if (warm) 40 else 30)
        return KinStyle(
            skin = skin,
            dark = dark,
            showIllustrations = warm,
            shapes = Shapes(
                extraSmall = RoundedCornerShape(xs), small = RoundedCornerShape(small), medium = RoundedCornerShape(medium),
                large = RoundedCornerShape(large), extraLarge = RoundedCornerShape(large),
            ),
            iconShape = icon,
            avatarShape = if (warm) CircleShape else icon,
            full = CircleShape,
            pressLip = if (warm) 3.dp else 0.dp,
            pressLipSmall = if (warm) 2.dp else 0.dp,
            focusRing = if (warm) 5.dp else 0.dp,
            cardElevation = if (dark) 0.dp else 2.dp,
            raisedElevation = if (dark) 0.dp else 14.dp,
            shadowColor = if (warm) Color(0xFF8C4620) else Color(0xFF102824),
            hairline = if (dark) Color.White.copy(alpha = 0.05f) else Color.Transparent,
        )
    }

    /**
     * The Material part of the design. Where the design has no Material slot the colour stays in [KinColors]. Two choices worth knowing:
     * `surfaceContainerHighest` is the design's `surfaceContainerHigh` (the off track of a switch, the fill of a text field), and the
     * dialog and sheet containers are plain `surface`, so a dialog is white (or the dark card colour) and not the peach of a pressed chip.
     */
    fun colorScheme(c: KinColors, dark: Boolean): ColorScheme =
        if (dark) {
            darkColorScheme(
                primary = c.primary, onPrimary = c.onPrimary, primaryContainer = c.primaryContainer, onPrimaryContainer = c.onPrimaryContainer,
                inversePrimary = c.primaryContainer,
                secondary = c.primary, onSecondary = c.onPrimary, secondaryContainer = c.primaryContainer, onSecondaryContainer = c.onPrimaryContainer,
                tertiary = c.tintLilacIcon, onTertiary = c.background, tertiaryContainer = c.tintLilac, onTertiaryContainer = c.onTintLilac,
                background = c.background, onBackground = c.onSurface,
                surface = c.surface, onSurface = c.onSurface, surfaceVariant = c.surfaceContainer, onSurfaceVariant = c.onSurfaceVariant,
                surfaceTint = c.primary, inverseSurface = c.onSurface, inverseOnSurface = c.background,
                error = c.error, onError = c.surface, errorContainer = c.errorContainer, onErrorContainer = c.onErrorContainer,
                outline = c.outline, outlineVariant = c.outlineVariant, scrim = c.scrim,
                surfaceBright = c.surface, surfaceDim = c.background,
                surfaceContainerLowest = c.surface, surfaceContainerLow = c.surface, surfaceContainer = c.surfaceContainer,
                surfaceContainerHigh = c.surface, surfaceContainerHighest = c.surfaceContainerHigh,
            )
        } else {
            lightColorScheme(
                primary = c.primary, onPrimary = c.onPrimary, primaryContainer = c.primaryContainer, onPrimaryContainer = c.onPrimaryContainer,
                inversePrimary = c.primaryContainer,
                secondary = c.primary, onSecondary = c.onPrimary, secondaryContainer = c.primaryContainer, onSecondaryContainer = c.onPrimaryContainer,
                tertiary = c.tintLilacIcon, onTertiary = c.background, tertiaryContainer = c.tintLilac, onTertiaryContainer = c.onTintLilac,
                background = c.background, onBackground = c.onSurface,
                surface = c.surface, onSurface = c.onSurface, surfaceVariant = c.surfaceContainer, onSurfaceVariant = c.onSurfaceVariant,
                surfaceTint = c.primary, inverseSurface = c.onSurface, inverseOnSurface = c.background,
                error = c.error, onError = c.surface, errorContainer = c.errorContainer, onErrorContainer = c.onErrorContainer,
                outline = c.outline, outlineVariant = c.outlineVariant, scrim = c.scrim,
                surfaceBright = c.surface, surfaceDim = c.background,
                surfaceContainerLowest = c.surface, surfaceContainerLow = c.surface, surfaceContainer = c.surfaceContainer,
                surfaceContainerHigh = c.surface, surfaceContainerHighest = c.surfaceContainerHigh,
            )
        }
}

private val LocalKinColors = staticCompositionLocalOf { KinTokens.warmLight }
private val LocalKinStyle = staticCompositionLocalOf { KinPalette.style(KinSkin.WARM, dark = false) }

/** The app's theme: one skin (Warm or Simple) in light or dark, no dynamic colour. [mode] "System" follows the phone and changes live with it. */
@Composable
fun KinTheme(
    skin: KinSkin = KinSkin.WARM,
    mode: KinThemeMode = KinThemeMode.SYSTEM,
    content: @Composable () -> Unit,
) {
    val dark = KinPalette.isDark(mode, isSystemInDarkTheme())
    val colors = KinPalette.colors(skin, dark)
    val style = KinPalette.style(skin, dark)
    CompositionLocalProvider(LocalKinColors provides colors, LocalKinStyle provides style) {
        MaterialTheme(
            colorScheme = KinPalette.colorScheme(colors, dark),
            typography = KinTypography,
            shapes = style.shapes,
            content = content,
        )
    }
}

/** The design's own tokens, next to `MaterialTheme`: `KinTheme.colors.primaryEdge`, `KinTheme.style.pressLip`. */
object KinTheme {
    val colors: KinColors
        @Composable @ReadOnlyComposable get() = LocalKinColors.current

    val style: KinStyle
        @Composable @ReadOnlyComposable get() = LocalKinStyle.current
}
