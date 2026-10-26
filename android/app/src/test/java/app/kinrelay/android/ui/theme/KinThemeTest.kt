package app.kinrelay.android.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import app.kinrelay.android.data.KinSkin
import app.kinrelay.android.data.KinThemeMode
import kotlin.math.pow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The design's tokens in Kotlin: every skin and mode has its colours, they reach Material's colour scheme, and the skins differ only where the design says. */
class KinThemeTest {
    private val combos = listOf(
        Triple(KinSkin.WARM, false, KinTokens.warmLight),
        Triple(KinSkin.WARM, true, KinTokens.warmDark),
        Triple(KinSkin.SIMPLE, false, KinTokens.simpleLight),
        Triple(KinSkin.SIMPLE, true, KinTokens.simpleDark),
    )

    @Test
    fun eachSkinAndModeHasItsOwnTokenSet() {
        for ((skin, dark, expected) in combos) assertTrue("$skin dark=$dark", KinPalette.colors(skin, dark) === expected)
        assertEquals(4, combos.map { it.third }.toSet().size)
    }

    @Test
    fun theBrandColoursAreTheDesigns() {
        assertEquals(Color(0xFFF2804A), KinTokens.warmLight.primary)
        assertEquals(Color(0xFFD9622C), KinTokens.warmLight.primaryEdge)
        assertEquals(Color(0xFFFF9A66), KinTokens.warmDark.primary)
        assertEquals(Color(0xFF0A7C70), KinTokens.simpleLight.primary)
        assertEquals(Color(0xFF4FD8C6), KinTokens.simpleDark.primary)
        assertEquals(Color(0xFFFFF8F1), KinTokens.warmLight.background)
        assertEquals(Color(0xFF1C1512), KinTokens.warmDark.background)
        assertEquals(Color(0xFFF3F6F5), KinTokens.simpleLight.background)
        assertEquals(Color(0xFF0F1514), KinTokens.simpleDark.background)
    }

    /** The token JSON writes a translucent colour as #RRGGBBAA (CSS order); Compose wants the alpha first. */
    @Test
    fun aTranslucentTokenKeepsItsAlpha() {
        assertEquals(Color(0x613A2620), KinTokens.warmLight.scrim)
        assertEquals(Color(0x8C000000), KinTokens.warmDark.scrim)
        assertEquals(Color(0x242B1208), KinTokens.warmDark.primaryBubble)
        assertEquals(0f, KinTokens.warmLight.artSlot.alpha, 0f)
        assertEquals(1f, KinTokens.simpleLight.artSlot.alpha, 0f)
    }

    @Test
    fun theMaterialSchemeCarriesTheTokensInAllFourCombinations() {
        for ((skin, dark, c) in combos) {
            val scheme = KinPalette.colorScheme(KinPalette.colors(skin, dark), dark)
            val where = "$skin dark=$dark"
            assertEquals(where, c.primary, scheme.primary)
            assertEquals(where, c.onPrimary, scheme.onPrimary)
            assertEquals(where, c.primaryContainer, scheme.primaryContainer)
            assertEquals(where, c.onPrimaryContainer, scheme.onPrimaryContainer)
            assertEquals(where, c.background, scheme.background)
            assertEquals(where, c.surface, scheme.surface)
            assertEquals(where, c.onSurface, scheme.onSurface)
            assertEquals(where, c.onSurfaceVariant, scheme.onSurfaceVariant)
            assertEquals(where, c.error, scheme.error)
            assertEquals(where, c.errorContainer, scheme.errorContainer)
            assertEquals(where, c.outline, scheme.outline)
            assertEquals(where, c.outlineVariant, scheme.outlineVariant)
            assertEquals(where, c.scrim, scheme.scrim)
            // The off track of a switch and the fill of a text field are the design's "surface container high"; dialogs and sheets are plain surface.
            assertEquals(where, c.surfaceContainerHigh, scheme.surfaceContainerHighest)
            assertEquals(where, c.surface, scheme.surfaceContainerHigh)
            assertEquals(where, c.surfaceContainer, scheme.surfaceContainer)
        }
    }

    @Test
    fun systemFollowsThePhoneAndLightAndDarkDoNot() {
        for (systemDark in listOf(false, true)) {
            assertEquals(systemDark, KinPalette.isDark(KinThemeMode.SYSTEM, systemDark))
            assertEquals(false, KinPalette.isDark(KinThemeMode.LIGHT, systemDark))
            assertEquals(true, KinPalette.isDark(KinThemeMode.DARK, systemDark))
        }
    }

    @Test
    fun theSkinsDifferInRadiusAndLipOnly() {
        val warm = KinPalette.style(KinSkin.WARM, dark = false)
        val simple = KinPalette.style(KinSkin.SIMPLE, dark = false)
        assertEquals(RoundedCornerShape(8.dp), warm.shapes.extraSmall)
        assertEquals(RoundedCornerShape(14.dp), warm.shapes.small)
        assertEquals(RoundedCornerShape(24.dp), warm.shapes.medium)
        assertEquals(RoundedCornerShape(32.dp), warm.shapes.large)
        assertEquals(RoundedCornerShape(6.dp), simple.shapes.extraSmall)
        assertEquals(RoundedCornerShape(12.dp), simple.shapes.small)
        assertEquals(RoundedCornerShape(20.dp), simple.shapes.medium)
        assertEquals(RoundedCornerShape(28.dp), simple.shapes.large)
        assertEquals(RoundedCornerShape(percent = 40), warm.iconShape)
        assertEquals(RoundedCornerShape(percent = 30), simple.iconShape)
        assertNotEquals(warm.avatarShape, simple.avatarShape)
        assertEquals(3.dp, warm.pressLip)
        assertEquals(2.dp, warm.pressLipSmall)
        assertEquals(5.dp, warm.focusRing)
        assertEquals(0.dp, simple.pressLip)
        assertEquals(0.dp, simple.pressLipSmall)
        assertEquals(0.dp, simple.focusRing)
        assertTrue(warm.showIllustrations)
        assertEquals(false, simple.showIllustrations)
    }

    @Test
    fun darkModeHasAHairlineInsteadOfAShadow() {
        for (skin in KinSkin.entries) {
            val light = KinPalette.style(skin, dark = false)
            val dark = KinPalette.style(skin, dark = true)
            assertTrue(light.cardElevation > 0.dp && light.hairline.alpha == 0f)
            assertTrue(dark.cardElevation == 0.dp && dark.hairline.alpha > 0f)
        }
    }

    /** Text must be readable in every skin and mode: 4.5:1 for the pairs the screens use for body text and buttons. */
    @Test
    fun textPairsAreReadable() {
        for ((skin, dark, c) in combos) {
            val where = "$skin dark=$dark"
            assertContrast("$where onSurface on background", c.onSurface, c.background)
            assertContrast("$where onSurface on surface", c.onSurface, c.surface)
            assertContrast("$where onSurfaceVariant on surface", c.onSurfaceVariant, c.surface)
            assertContrast("$where onPrimary on primary", c.onPrimary, c.primary)
            assertContrast("$where onPrimaryContainer on primaryContainer", c.onPrimaryContainer, c.primaryContainer)
            assertContrast("$where onWarning on warning", c.onWarning, c.warning)
            assertContrast("$where onWarningContainer on warningContainer", c.onWarningContainer, c.warningContainer)
            assertContrast("$where onErrorContainer on errorContainer", c.onErrorContainer, c.errorContainer)
            assertContrast("$where onSuccessContainer on successContainer", c.onSuccessContainer, c.successContainer)
            assertContrast("$where primaryText on surface", c.primaryText, c.surface)
            assertContrast("$where onNavIndicator on navIndicator", c.onNavIndicator, c.navIndicator)
            for (tint in KinTint.entries) {
                val t = c.tint(tint)
                assertContrast("$where $tint text on tint", t.onContainer, t.container)
                assertContrast("$where $tint icon on tint", t.icon, t.container)
            }
        }
    }

    @Test
    fun everyIconBuilds() {
        val icons = KinIcons::class.java.declaredMethods.filter { it.name.startsWith("get") && it.returnType == ImageVector::class.java }
        assertTrue("found ${icons.size} icons", icons.size >= 35)
        for (getter in icons) {
            val icon = getter.invoke(KinIcons) as ImageVector
            assertEquals(getter.name, 24f, icon.viewportWidth, 0f)
            assertTrue(getter.name, icon.root.size > 0)
        }
    }

    private fun assertContrast(what: String, a: Color, b: Color) {
        val ratio = contrast(a, b)
        assertTrue("$what: contrast %.2f is below 4.5".format(ratio), ratio >= 4.5)
    }

    private fun contrast(a: Color, b: Color): Double {
        val x = luminance(a)
        val y = luminance(b)
        return (maxOf(x, y) + 0.05) / (minOf(x, y) + 0.05)
    }

    private fun luminance(c: Color): Double {
        fun channel(v: Float): Double = if (v <= 0.03928f) v / 12.92 else ((v + 0.055) / 1.055).pow(2.4)
        return 0.2126 * channel(c.red) + 0.7152 * channel(c.green) + 0.0722 * channel(c.blue)
    }
}
