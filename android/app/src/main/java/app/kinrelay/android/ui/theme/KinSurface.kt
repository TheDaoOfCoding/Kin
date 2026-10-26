package app.kinrelay.android.ui.theme

import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** The four soft colours the design uses to tell features apart (the media card is not one of them: it is plain). */
enum class KinTint { LILAC, HONEY, SAGE, PEACH }

/** A tint's three colours: the container, the text on it and the icon on it. */
class TintColors(val container: Color, val onContainer: Color, val icon: Color)

fun KinColors.tint(tint: KinTint): TintColors = when (tint) {
    KinTint.LILAC -> TintColors(tintLilac, onTintLilac, tintLilacIcon)
    KinTint.HONEY -> TintColors(tintHoney, onTintHoney, tintHoneyIcon)
    KinTint.SAGE -> TintColors(tintSage, onTintSage, tintSageIcon)
    KinTint.PEACH -> TintColors(tintPeach, onTintPeach, tintPeachIcon)
}

/**
 * The "press lip": a solid band of [color] under the shape, [lip] tall, that makes a button look like a key. Drawn outside the bounds (like the
 * design's `box-shadow: 0 3px 0`), so it needs a few dp of room below. Nothing is drawn for a lip of zero (Simple).
 */
fun Modifier.lipBehind(shape: Shape, color: Color, lip: Dp): Modifier =
    if (lip <= 0.dp) this else drawBehind {
        val outline = shape.createOutline(size, layoutDirection, this)
        translate(top = lip.toPx()) { fill(outline, color) }
    }

private fun DrawScope.fill(outline: Outline, color: Color) = when (outline) {
    is Outline.Rectangle -> drawRect(color, outline.rect.topLeft, outline.rect.size)
    is Outline.Rounded -> drawPath(Path().apply { addRoundRect(outline.roundRect) }, color)
    is Outline.Generic -> drawPath(outline.path, color)
}
