package app.kinrelay.android.ui.theme

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.min

/** How the link looks: the faces of the illustration and the colour of the dot follow it. "Paused" is the user's own Pause, not a link state. */
enum class LinkLook { CONNECTED, CONNECTING, PAUSED }

// The design's illustrations, redrawn on a Canvas. Coordinates are the mock's own (a 146 x 104 box for the PC and phone, 170 x 128 for the box),
// so a path copied from the design draws the same here. Warm shows these; Simple shows a plain outline in the same box.

private fun parse(data: String): Path = PathParser().parsePathString(data).toPath()

private val Heart = Color(0xFFE8556F)

private val PcBase = parse("M3 80a4 4 0 0 1 4-4h80a4 4 0 0 1 4 4v3a6 6 0 0 1-6 6H9a6 6 0 0 1-6-6v-3z")
private val PcTrail = parse("M72 22q22-22 44 14")
private val PcHeart = parse("M93 21.5s-7-4.1-7-9.4A4.1 4.1 0 0 1 93 9.4a4.1 4.1 0 0 1 7 2.7c0 5.3-7 9.4-7 9.4z")
private val PcSpark = parse("M106 6v5M103.5 8.5h5")
private val PcHappyMouth = parse("M41 56q6 5 12 0")
private val PcLookMouth = parse("M43 57h8")
private val PcSleepEyes = parse("M33.5 47.5q3.5 2.5 7 0M53.5 47.5q3.5 2.5 7 0")
private val PcSleepMouth = parse("M42 56q5 3.5 10 0")
private val SleepZBig = parse("M87 9h6l-6 8h6")
private val SleepZSmall = parse("M97 3h4.5l-4.5 6h4.5")
private val PhoneHome = parse("M115 91h10")
private val PhoneHappyMouth = parse("M116 70q4 4 8 0")
private val PhoneLookMouth = parse("M115 71h6")
private val PhoneSleepEyes = parse("M111 62.5q3 2 6 0M123 62.5q3 2 6 0")
private val PhoneSleepMouth = parse("M117 70q3 2.5 6 0")

private val BoxLidLeft = parse("M48 62 30 40h48l12 18z")
private val BoxLidRight = parse("M122 62l18-22H92l-12 18z")
private val BoxMouth = parse("M77 89q8 6.5 16 0")
private val BoxPaper = parse("M70 8h17l8 8v22a3 3 0 0 1-3 3H70a3 3 0 0 1-3-3V11a3 3 0 0 1 3-3z")
private val BoxPaperLines = parse("M73 22h13M73 28h9")
private val BoxHeart = parse("M128 30s-6-3.5-6-8a3.5 3.5 0 0 1 6-2.3 3.5 3.5 0 0 1 6 2.3c0 4.5-6 8-6 8z")
private val BoxSparks = parse("M38 22v6M35 25h6M146 52v5M143.5 54.5h5")

private val MarkMouth = parse("M23 21.2q2.5 2.2 5 0")

/** The ink of the line work: round caps and joins, [width] in the design's units. */
private fun inkStroke(width: Float) = Stroke(width = width, cap = StrokeCap.Round, join = StrokeJoin.Round)

/** Fits a design box of [boxWidth] x [boxHeight] into the canvas, centred, and draws [block] in the design's coordinates. */
private fun DrawScope.inBox(boxWidth: Float, boxHeight: Float, block: DrawScope.(scale: Float) -> Unit) {
    val scale = min(size.width / boxWidth, size.height / boxHeight)
    val dx = (size.width - boxWidth * scale) / 2f
    val dy = (size.height - boxHeight * scale) / 2f
    withTransform({
        translate(dx, dy)
        scale(scale, scale, pivot = Offset.Zero)
    }) { block(scale) }
}

private fun DrawScope.line(path: Path, color: Color, width: Float = 2.6f, alpha: Float = 1f) =
    drawPath(path, color, alpha = alpha, style = inkStroke(width))

private fun DrawScope.dot(x: Float, y: Float, r: Float, color: Color, alpha: Float = 1f) = drawCircle(color, r, Offset(x, y), alpha = alpha)

private fun DrawScope.roundBox(x: Float, y: Float, w: Float, h: Float, r: Float, fill: Color, ink: Color, width: Float = 2.6f) {
    drawRoundRect(fill, Offset(x, y), Size(w, h), CornerRadius(r), style = Fill)
    drawRoundRect(ink, Offset(x, y), Size(w, h), CornerRadius(r), style = inkStroke(width))
}

/** The PC, with a face that follows the link: happy, looking around, or asleep. */
private fun DrawScope.pc(c: KinColors, look: LinkLook) {
    val ink = c.illustrationInk
    drawOval(ink, Offset(66f - 62f, 98f - 5f), Size(124f, 10f), alpha = 0.08f)
    roundBox(10f, 26f, 74f, 50f, 11f, c.illustrationFill, ink)
    drawPath(PcBase, c.illustrationFill)
    drawPath(PcBase, ink, style = inkStroke(2.6f))
    dot(29f, 55f, 4.5f, c.illustrationCheek, 0.7f)
    dot(65f, 55f, 4.5f, c.illustrationCheek, 0.7f)
    drawPath(PcTrail, ink, alpha = 0.55f, style = Stroke(width = 2.2f, cap = StrokeCap.Round, pathEffect = PathEffect.dashPathEffect(floatArrayOf(1f, 6f))))
    when (look) {
        LinkLook.CONNECTED -> {
            dot(37f, 47f, 3.2f, ink)
            dot(57f, 47f, 3.2f, ink)
            line(PcHappyMouth, ink)
            drawPath(PcHeart, Heart)
            line(PcSpark, c.warning)
        }
        LinkLook.CONNECTING -> {
            dot(39f, 46f, 3.2f, ink)
            dot(59f, 46f, 3.2f, ink)
            line(PcLookMouth, ink)
            dot(86f, 13f, 2.4f, ink)
            dot(94f, 10f, 2.4f, ink, 0.6f)
            dot(102f, 11f, 2.4f, ink, 0.3f)
        }
        LinkLook.PAUSED -> {
            line(PcSleepEyes, ink)
            line(PcSleepMouth, ink)
            line(SleepZBig, ink, 1.8f)
            line(SleepZSmall, ink, 1.6f)
        }
    }
}

/** The phone, apricot, with the same face. */
private fun DrawScope.phone(c: KinColors, look: LinkLook) {
    val ink = c.illustrationInk
    roundBox(102f, 40f, 36f, 58f, 10f, c.primary, ink)
    line(PhoneHome, ink)
    when (look) {
        LinkLook.CONNECTED -> {
            dot(114f, 62f, 2.8f, ink)
            dot(126f, 62f, 2.8f, ink)
            line(PhoneHappyMouth, ink)
        }
        LinkLook.CONNECTING -> {
            dot(112f, 61f, 2.8f, ink)
            dot(124f, 61f, 2.8f, ink)
            line(PhoneLookMouth, ink)
        }
        LinkLook.PAUSED -> {
            line(PhoneSleepEyes, ink)
            line(PhoneSleepMouth, ink)
        }
    }
}

/** Simple's version of the same box: the PC and the phone as outlines, the dashes between them, and the state dot on the PC. */
private fun DrawScope.plainPair(c: KinColors, look: LinkLook?, strokePx: Float) {
    inBox(146f, 104f) { scale ->
        val w = strokePx / scale
        val ink = c.illustrationInk
        drawRoundRect(ink, Offset(24f, 28f), Size(56f, 38f), CornerRadius(6f), style = inkStroke(w))
        line(parse("M16 76h72"), ink, w)
        drawRoundRect(ink, Offset(98f, 32f), Size(28f, 46f), CornerRadius(6f), style = inkStroke(w))
        line(parse("M108 70h8"), ink, w)
        drawPath(parse("M84 50h10"), ink, style = Stroke(width = w, cap = StrokeCap.Round, pathEffect = PathEffect.dashPathEffect(floatArrayOf(2f, 4f))))
        if (look != null) {
            val dotColor = when (look) {
                LinkLook.CONNECTED -> c.success
                LinkLook.CONNECTING -> c.onSurfaceSubtle
                LinkLook.PAUSED -> c.warning
            }
            dot(80f, 28f, 12f, c.artSlot)
            dot(80f, 28f, 8f, dotColor)
        }
    }
}

/** A gentle up and down, 3 units over 3 seconds. The phone bobs; with the animation scale at 0 (the "remove animations" setting) it holds still. */
@Composable
private fun rememberBob(): State<Float> =
    rememberInfiniteTransition(label = "bob").animateFloat(
        initialValue = 0f, targetValue = -3f,
        animationSpec = infiniteRepeatable(tween(durationMillis = 1500, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "bob",
    )

/**
 * The PC and the phone in the status card and on the welcome screen. Warm: the two with faces that follow [look]. Simple: a plain outline of the
 * same two in the same box (with the state dot on the PC when [look] is given). [plainStroke] is the line's width in dp in Simple.
 */
@Composable
fun PairArt(look: LinkLook?, modifier: Modifier = Modifier, plainStroke: Dp = 2.4.dp) {
    val c = KinTheme.colors
    if (KinTheme.style.showIllustrations) {
        val bob = rememberBob()
        Canvas(modifier) {
            inBox(146f, 104f) {
                val face = look ?: LinkLook.CONNECTED
                pc(c, face)
                translate(top = bob.value) { phone(c, face) }
            }
        }
    } else {
        Canvas(modifier) { plainPair(c, look, plainStroke.toPx()) }
    }
}

/** An art slot: the box the illustration or icon sits in, with the skin's art-slot colour behind it (clear in Warm, a teal wash in Simple). */
@Composable
fun ArtSlot(width: Dp, height: Dp, modifier: Modifier = Modifier, shape: Shape = KinTheme.style.shapes.medium, content: @Composable () -> Unit) {
    Box(modifier.size(width, height).clip(shape).background(KinTheme.colors.artSlot), contentAlignment = Alignment.Center) { content() }
}

/** The happy box of the empty states (Warm), or a plain history icon (Simple). */
@Composable
fun EmptyArt(modifier: Modifier = Modifier) {
    val c = KinTheme.colors
    if (KinTheme.style.showIllustrations) {
        val bob = rememberBob()
        Canvas(modifier) {
            inBox(170f, 128f) {
                val ink = c.illustrationInk
                drawOval(ink, Offset(85f - 54f, 120f - 5.5f), Size(108f, 11f), alpha = 0.08f)
                drawPath(BoxLidLeft, c.tintHoney)
                line(BoxLidLeft, ink)
                drawPath(BoxLidRight, c.tintHoney)
                line(BoxLidRight, ink)
                roundBox(44f, 56f, 82f, 60f, 14f, c.illustrationFill, ink)
                dot(72f, 80f, 3.4f, ink)
                dot(98f, 80f, 3.4f, ink)
                dot(63f, 89f, 5f, c.illustrationCheek, 0.7f)
                dot(107f, 89f, 5f, c.illustrationCheek, 0.7f)
                line(BoxMouth, ink)
                translate(top = bob.value) {
                    rotate(-10f, Offset(81f, 24f)) {
                        drawPath(BoxPaper, c.illustrationFill)
                        line(BoxPaper, ink)
                        line(BoxPaperLines, ink)
                    }
                }
                drawPath(BoxHeart, Heart)
                line(BoxSparks, c.warning)
            }
        }
    } else {
        Box(modifier, contentAlignment = Alignment.Center) {
            Icon(KinIcons.History, contentDescription = null, tint = c.illustrationInk, modifier = Modifier.size(56.dp))
        }
    }
}

/** The record in the media card (Warm), or a music note (Simple). Still: nothing here animates but the faces. */
@Composable
fun MediaArt(modifier: Modifier = Modifier) {
    val c = KinTheme.colors
    if (KinTheme.style.showIllustrations) {
        Canvas(modifier) {
            inBox(50f, 50f) {
                val groove = Color(0xFF3A261E)
                dot(25f, 25f, 24f, Color(0xFF2B1A14))
                drawCircle(groove, 20f, Offset(25f, 25f), style = Stroke(2f))
                drawCircle(groove, 15.5f, Offset(25f, 25f), style = Stroke(2f))
                dot(25f, 25f, 10f, Color(0xFFF2804A))
                dot(25f, 25f, 2.5f, Color(0xFF2B1A14))
            }
        }
    } else {
        Box(modifier, contentAlignment = Alignment.Center) {
            Icon(KinIcons.Music, contentDescription = null, tint = c.illustrationInk, modifier = Modifier.size(24.dp))
        }
    }
}

/**
 * The Kin mark: two friendly circles leaning together. **The same in every skin and mode** (it is the brand, and the app icon), so its colours
 * are fixed here and not read from the theme.
 */
@Composable
fun KinMark(modifier: Modifier = Modifier) {
    Canvas(modifier) { drawKinMark() }
}

private val MarkInk = Color(0xFF3A1A0E)
private val MarkBack = Color(0xFFFFF6EF)
private val MarkFront = Color(0xFFFFD3BA)

private fun DrawScope.drawKinMark() = inBox(40f, 40f) {
    drawCircle(MarkBack, 9.5f, Offset(15f, 21f))
    drawCircle(MarkInk, 9.5f, Offset(15f, 21f), style = inkStroke(2.6f))
    drawCircle(MarkFront, 9.5f, Offset(25.5f, 18f))
    drawCircle(MarkInk, 9.5f, Offset(25.5f, 18f), style = inkStroke(2.6f))
    dot(22.5f, 17f, 1.6f, MarkInk)
    dot(28.5f, 17f, 1.6f, MarkInk)
    line(MarkMouth, MarkInk, 2.2f)
    dot(11.5f, 21f, 1.5f, MarkInk)
}

/** The app's logo: the mark on an apricot tile with its own little lip. Always apricot, in every skin and mode. */
@Composable
fun KinLogo(size: Dp = 36.dp, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(percent = 28)
    val lip = size * (2f / 36f)
    Box(
        modifier
            .size(size)
            .lipBehind(shape, Color(0xFFD9622C), lip)
            .clip(shape)
            .background(Color(0xFFF2804A)),
        contentAlignment = Alignment.Center,
    ) {
        KinMark(Modifier.size(size * (26f / 36f)))
    }
}
