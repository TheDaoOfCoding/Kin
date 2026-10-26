package app.kinrelay.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.kinrelay.android.ui.theme.KinIcons
import app.kinrelay.android.ui.theme.KinTheme
import app.kinrelay.android.ui.theme.KinTint
import app.kinrelay.android.ui.theme.lipBehind
import app.kinrelay.android.ui.theme.tint

// The parts the screens are built from. They read the theme's tokens and nothing else, so a skin changes them without a change of layout.

/**
 * A card: white in light mode (the dark surface in dark mode), the skin's corner radius, a soft shadow in light mode and a hairline in dark
 * mode. With a [tint] it takes that tint's container colour, with [container] any colour (a notice). With [onClick] the whole card is one tap target.
 */
@Composable
fun KinCard(
    modifier: Modifier = Modifier,
    tint: KinTint? = null,
    shape: Shape = KinTheme.style.shapes.medium,
    container: Color? = null,
    onClick: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val style = KinTheme.style
    val colors = KinTheme.colors
    val fill = container ?: tint?.let { colors.tint(it).container } ?: colors.surface
    val base = modifier
        .shadow(style.cardElevation, shape, clip = false, ambientColor = style.shadowColor, spotColor = style.shadowColor)
        .let { if (style.hairline != Color.Transparent) it.border(BorderStroke(1.dp, style.hairline), shape) else it }
        .clip(shape)
        .background(fill)
    CompositionLocalProvider(LocalContentColor provides colors.onSurface) {
        Column(if (onClick != null) base.clickable(role = Role.Button, onClick = onClick) else base, content = content)
    }
}

/**
 * A feature card on Home: a tinted card with the feature's icon and name on top, and [content] under them at the full width of the card. The
 * content gets the colour for small print on this tint.
 */
@Composable
fun FeatureCard(
    tint: KinTint,
    icon: ImageVector,
    title: String,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.(small: Color) -> Unit,
) {
    val on = KinTheme.colors.tint(tint).onContainer
    KinCard(modifier.fillMaxWidth(), tint = tint) {
        Column(Modifier.padding(start = 14.dp, top = 12.dp, end = 14.dp, bottom = 14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                TintSlot(icon, tint, onCard = true)
                Text(title, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold), color = KinTheme.colors.onSurface)
            }
            content(on)
        }
    }
}

/** The rounded square behind an icon: [background] under a [icon] of [iconColor]. */
@Composable
fun IconSlot(icon: ImageVector, iconColor: Color, background: Color, modifier: Modifier = Modifier, size: Dp = 38.dp, iconSize: Dp = 21.dp) {
    Box(modifier.size(size).clip(KinTheme.style.iconShape).background(background), contentAlignment = Alignment.Center) {
        Icon(icon, contentDescription = null, tint = iconColor, modifier = Modifier.size(iconSize))
    }
}

/** The icon slot of a tinted card (white in Warm light, a teal wash in Simple) or of a settings row (the tint's own container). */
@Composable
fun TintSlot(icon: ImageVector, tint: KinTint, onCard: Boolean, modifier: Modifier = Modifier, size: Dp = 38.dp) {
    val colors = KinTheme.colors
    val t = colors.tint(tint)
    IconSlot(icon, t.icon, if (onCard) colors.iconSlot else t.container, modifier, size)
}

/** The round picture of a history row: the tint's container with its icon, or the error colours. */
@Composable
fun KinAvatar(icon: ImageVector, tint: KinTint?, modifier: Modifier = Modifier, error: Boolean = false) {
    val colors = KinTheme.colors
    val t = tint?.let { colors.tint(it) }
    val background = if (error) colors.errorContainer else t?.container ?: colors.surfaceContainer
    val iconColor = if (error) colors.onErrorContainer else t?.icon ?: colors.onSurfaceVariant
    Box(modifier.size(42.dp).clip(KinTheme.style.avatarShape).background(background), contentAlignment = Alignment.Center) {
        Icon(icon, contentDescription = null, tint = iconColor, modifier = Modifier.size(20.dp))
    }
}

enum class KinButtonStyle { PRIMARY, SOFT, PLAIN, WARN }

/**
 * The design's button: a pill with a solid lip under it (Warm) that goes away while it is pressed, so it looks pressed in. [block] is the
 * full-width, 54 dp tall one. The tap area is at least 48 dp even where the button looks smaller.
 */
@Composable
fun KinButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    style: KinButtonStyle = KinButtonStyle.PRIMARY,
    icon: ImageVector? = null,
    small: Boolean = false,
    block: Boolean = false,
    enabled: Boolean = true,
) {
    val colors = KinTheme.colors
    val look = KinTheme.style
    val shape = look.full
    // A disabled button fades its colours; a fading layer (Modifier.alpha) would clip the lip, which is drawn outside the button's bounds.
    val fade = if (enabled) 1f else 0.45f
    val container = when (style) {
        KinButtonStyle.PRIMARY -> colors.primary
        KinButtonStyle.SOFT -> colors.primaryContainer
        KinButtonStyle.PLAIN -> colors.surface
        KinButtonStyle.WARN -> colors.warning
    }.let { it.copy(alpha = it.alpha * fade) }
    val content = when (style) {
        KinButtonStyle.PRIMARY -> colors.onPrimary
        KinButtonStyle.SOFT -> colors.onPrimaryContainer
        KinButtonStyle.PLAIN -> colors.onSurface
        KinButtonStyle.WARN -> colors.onWarning
    }.let { it.copy(alpha = it.alpha * fade) }
    val edge = when (style) {
        KinButtonStyle.PRIMARY -> colors.primaryEdge
        KinButtonStyle.SOFT, KinButtonStyle.PLAIN -> colors.outline
        KinButtonStyle.WARN -> colors.warningEdge
    }.let { it.copy(alpha = it.alpha * fade) }
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val lip = look.pressLip
    val down = pressed && enabled
    Box(
        modifier
            .let { if (block) it.fillMaxWidth() else it }
            .minimumInteractiveComponentSize()
            .offset { IntOffset(0, if (down) lip.roundToPx() else 0) }
            .lipBehind(shape, edge, if (down) 0.dp else lip)
            .clip(shape)
            .background(container)
            .let { if (style == KinButtonStyle.PLAIN) it.border(BorderStroke(1.5.dp, colors.outline), shape) else it }
            .clickable(interactionSource = interaction, indication = ripple(), enabled = enabled, role = Role.Button, onClick = onClick)
            .heightIn(min = if (block) 54.dp else if (small) 34.dp else 42.dp)
            .padding(horizontal = if (small) 14.dp else 18.dp),
        contentAlignment = Alignment.Center,
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            if (icon != null) Icon(icon, contentDescription = null, tint = content, modifier = Modifier.size(18.dp))
            Text(
                label, color = content, maxLines = 1, overflow = TextOverflow.Ellipsis,
                style = if (block) MaterialTheme.typography.titleSmall.copy(fontSize = 16.sp, fontWeight = FontWeight.SemiBold) else if (small) MaterialTheme.typography.labelMedium.copy(fontSize = 13.sp) else MaterialTheme.typography.labelLarge,
            )
        }
    }
}

/** A small pill with a message: [container] and [content] come from the status colours. */
@Composable
fun KinChip(text: String, container: Color, content: Color, modifier: Modifier = Modifier, leading: @Composable (() -> Unit)? = null) {
    Row(
        modifier.heightIn(min = 28.dp).clip(KinTheme.style.full).background(container).padding(horizontal = 11.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        leading?.invoke()
        Text(text, color = content, style = MaterialTheme.typography.labelMedium, maxLines = 1)
    }
}

/** The round "done" badge at the end of a finished row. */
@Composable
fun DoneBadge(description: String, modifier: Modifier = Modifier) {
    val colors = KinTheme.colors
    Box(modifier.size(28.dp).clip(KinTheme.style.full).background(colors.successContainer), contentAlignment = Alignment.Center) {
        Icon(KinIcons.Check, contentDescription = description, tint = colors.onSuccessContainer, modifier = Modifier.size(16.dp))
    }
}

/** A round icon-only button (cancel, close). 48 dp to touch, the icon 20 dp. */
@Composable
fun KinIconButton(icon: ImageVector, description: String, onClick: () -> Unit, modifier: Modifier = Modifier, tint: Color = KinTheme.colors.onSurfaceVariant) {
    Box(
        modifier.minimumInteractiveComponentSize().size(40.dp).clip(KinTheme.style.full).clickable(role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = description, tint = tint, modifier = Modifier.size(20.dp))
    }
}

/** "Recent" and the like: a bold title, and an action on the right. */
@Composable
fun SectionHead(title: String, modifier: Modifier = Modifier, action: String? = null, onAction: () -> Unit = {}) {
    Row(modifier.fillMaxWidth().padding(start = 6.dp, end = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
        Text(title, style = MaterialTheme.typography.titleMedium, color = KinTheme.colors.onSurface)
        if (action != null) {
            Text(
                action, style = MaterialTheme.typography.labelLarge, color = KinTheme.colors.primaryText,
                modifier = Modifier.minimumInteractiveComponentSize().clickable(role = Role.Button, onClick = onAction).padding(horizontal = 4.dp),
            )
        }
    }
}

/** A quiet label above a group ("Today", "Appearance"). */
@Composable
fun GroupLabel(text: String, modifier: Modifier = Modifier) {
    Text(text, style = MaterialTheme.typography.labelLarge, color = KinTheme.colors.onSurfaceVariant, modifier = modifier.padding(start = 6.dp, top = 4.dp))
}

/** The switch of the design: the darker apricot (or teal) track when on, so the track clears 3:1 on a white card; the knob colour of the skin. */
@Composable
fun kinSwitchColors(): SwitchColors {
    val c = KinTheme.colors
    return SwitchDefaults.colors(
        checkedThumbColor = c.switchKnob, checkedTrackColor = c.primaryEdge, checkedBorderColor = Color.Transparent,
        uncheckedThumbColor = c.switchKnob, uncheckedTrackColor = c.surfaceContainerHigh, uncheckedBorderColor = c.outline,
    )
}

/** One of a few exclusive choices, drawn as a row of pills in a tray (the Theme choice). */
@Composable
fun <T> KinSegmented(options: List<Triple<T, String, ImageVector>>, selected: T, onSelect: (T) -> Unit, modifier: Modifier = Modifier) {
    val colors = KinTheme.colors
    val tray = KinTheme.style.full
    Row(modifier.fillMaxWidth().clip(tray).background(colors.surfaceContainer).padding(3.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        options.forEach { (value, label, icon) ->
            val on = value == selected
            Row(
                Modifier
                    .weight(1f)
                    .heightIn(min = 40.dp)
                    .clip(tray)
                    .let { if (on) it.background(colors.surface).border(BorderStroke(1.dp, colors.outlineVariant), tray) else it }
                    .selectable(selected = on, role = Role.RadioButton, onClick = { onSelect(value) })
                    .padding(horizontal = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(icon, contentDescription = null, tint = if (on) colors.onSurface else colors.onSurfaceVariant, modifier = Modifier.size(16.dp))
                Text(label, style = MaterialTheme.typography.labelLarge, color = if (on) colors.onSurface else colors.onSurfaceVariant, maxLines = 1)
            }
        }
    }
}

/** The tabs of the signed-in app. The order is the tab index used by [KinRoot] and the alerts notification (0 Home, 1 History, 2 Settings). */
private val NavItems = listOf("Home" to KinIcons.Home, "History" to KinIcons.History, "Settings" to KinIcons.Settings)

/** How tall the floating nav and its gap are: the screens leave this much room at the bottom of their list. */
val NavClearance: Dp = 68.dp + 22.dp + 18.dp

/**
 * The floating bottom nav: a rounded bar with Home, History and Settings. The selected one has a pill behind its icon (with the press lip in
 * Warm). The fade above it is [NavFade], drawn by the caller under the bar.
 */
@Composable
fun KinBottomNav(selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    val colors = KinTheme.colors
    val look = KinTheme.style
    val shape = look.shapes.large
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 14.dp)
            .shadow(look.raisedElevation, shape, clip = false, ambientColor = look.shadowColor, spotColor = look.shadowColor)
            .clip(shape)
            .background(colors.surface)
            .border(BorderStroke(1.dp, colors.outlineVariant), shape)
            .heightIn(min = 68.dp),
        horizontalArrangement = Arrangement.SpaceAround, verticalAlignment = Alignment.CenterVertically,
    ) {
        NavItems.forEachIndexed { index, (label, icon) ->
            val on = index == selected
            Column(
                Modifier
                    .width(96.dp)
                    .selectable(selected = on, role = Role.Tab, onClick = { onSelect(index) }),
                horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                val pill = look.full
                Box(
                    Modifier
                        .size(58.dp, 30.dp)
                        .let { if (on) it.lipBehind(pill, colors.primaryEdge, look.pressLipSmall).clip(pill).background(colors.navIndicator) else it },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(icon, contentDescription = null, tint = if (on) colors.onNavIndicator else colors.onSurfaceVariant, modifier = Modifier.size(22.dp))
                }
                Text(label, style = MaterialTheme.typography.labelSmall, color = if (on) colors.onSurface else colors.onSurfaceVariant)
            }
        }
    }
}

/** The fade that lets the list slide under the floating nav: clear at the top, the page colour from about 60% down. */
@Composable
fun NavFade(modifier: Modifier = Modifier) {
    val page = KinTheme.colors.background
    Box(modifier.background(Brush.verticalGradient(0f to page.copy(alpha = 0f), 0.62f to page, 1f to page)))
}

/** A page title: "History", "Settings". */
@Composable
fun PageTitle(text: String, modifier: Modifier = Modifier) {
    Text(text, style = MaterialTheme.typography.headlineMedium, color = KinTheme.colors.onSurface, modifier = modifier.padding(start = 2.dp))
}
