package app.kinrelay.android.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/**
 * The design's icon set: a 24 dp grid, 2 dp stroke, round caps and joins, the same in both skins. Drawn in black and tinted by `Icon`. The path
 * data is the design's own (its rects and circles written out as paths). Not Material's icons: that library is large and has another look.
 */
object KinIcons {
    val Send: ImageVector by lazy { icon("Send", line("M20.5 3.5 10.2 13.8"), line("M20.5 3.5 14 20.5l-3.8-6.7-6.7-3.8 17-6.5z")) }
    val Laptop: ImageVector by lazy { icon("Laptop", line("M7 4.5H17A2.5 2.5 0 0 1 19.5 7V12.5A2.5 2.5 0 0 1 17 15H7A2.5 2.5 0 0 1 4.5 12.5V7A2.5 2.5 0 0 1 7 4.5Z"), line("M2.5 18.5h19")) }
    val Phone: ImageVector by lazy { icon("Phone", line("M9.5 2.5H14.5A3 3 0 0 1 17.5 5.5V18.5A3 3 0 0 1 14.5 21.5H9.5A3 3 0 0 1 6.5 18.5V5.5A3 3 0 0 1 9.5 2.5Z"), line("M10.5 18.5h3")) }
    val Bell: ImageVector by lazy { icon("Bell", line("M6 10.5a6 6 0 0 1 12 0v4.3l1.5 2.5a.6.6 0 0 1-.5.9H5a.6.6 0 0 1-.5-.9L6 14.8v-4.3z"), line("M10 21h4")) }
    val Pausec: ImageVector by lazy { icon("Pausec", line("M3.5 12A8.5 8.5 0 1 0 20.5 12A8.5 8.5 0 1 0 3.5 12Z"), line("M10 9v6M14 9v6")) }
    val Pause: ImageVector by lazy { icon("Pause", line("M9 6v12M15 6v12")) }
    val Play: ImageVector by lazy { icon("Play", filled("M8 5.8v12.4a.6.6 0 0 0 .9.5l9.6-6.2a.6.6 0 0 0 0-1L8.9 5.3a.6.6 0 0 0-.9.5z")) }
    val Prev: ImageVector by lazy { icon("Prev", filled("M18 6.6v10.8a.6.6 0 0 1-.9.5L9.4 12.5a.6.6 0 0 1 0-1l7.7-5.4a.6.6 0 0 1 .9.5z"), filled("M6 6v12")) }
    val Next: ImageVector by lazy { icon("Next", filled("M6 6.6v10.8a.6.6 0 0 0 .9.5l7.7-5.4a.6.6 0 0 0 0-1L6.9 6.1a.6.6 0 0 0-.9.5z"), filled("M18 6v12")) }
    val Music: ImageVector by lazy { icon("Music", line("M9 17.5V5.8l10.5-2.1v11.8"), line("M4 17.5A2.5 2.5 0 1 0 9 17.5A2.5 2.5 0 1 0 4 17.5Z"), line("M14.5 15.5A2.5 2.5 0 1 0 19.5 15.5A2.5 2.5 0 1 0 14.5 15.5Z")) }
    val Chat: ImageVector by lazy { icon("Chat", line("M5.5 4.5h13a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1-2.5 2.5H11l-4.5 3.5v-3.5h-1A2.5 2.5 0 0 1 3 15V7a2.5 2.5 0 0 1 2.5-2.5z"), line("M8 9.5h8M8 13h5")) }
    val Clip: ImageVector by lazy { icon("Clip", line("M10 2.5H14A1.5 1.5 0 0 1 15.5 4V5A1.5 1.5 0 0 1 14 6.5H10A1.5 1.5 0 0 1 8.5 5V4A1.5 1.5 0 0 1 10 2.5Z"), line("M15.5 4.5H17a2 2 0 0 1 2 2V19a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6.5a2 2 0 0 1 2-2h1.5"), line("M9 12h6M9 16h4")) }
    val File: ImageVector by lazy { icon("File", line("M13.5 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5L13.5 3z"), line("M13.5 3v5.5H19")) }
    val Image: ImageVector by lazy { icon("Image", line("M6.5 4H17.5A3.5 3.5 0 0 1 21 7.5V16.5A3.5 3.5 0 0 1 17.5 20H6.5A3.5 3.5 0 0 1 3 16.5V7.5A3.5 3.5 0 0 1 6.5 4Z"), line("M7.2 9.5A1.8 1.8 0 1 0 10.8 9.5A1.8 1.8 0 1 0 7.2 9.5Z"), line("m20.5 15.5-4.5-4.5L5 20")) }
    val Folder: ImageVector by lazy { icon("Folder", line("M3 7.5A2.5 2.5 0 0 1 5.5 5H9l2 2.5h7.5A2.5 2.5 0 0 1 21 10v7.5a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5v-10z")) }
    val Check: ImageVector by lazy { icon("Check", line("m5 12.5 4.5 4.5L19 7.5")) }
    val Alert: ImageVector by lazy { icon("Alert", line("M3.5 12A8.5 8.5 0 1 0 20.5 12A8.5 8.5 0 1 0 3.5 12Z"), line("M12 7.8v4.9"), line("M12 16.2h.01")) }
    val Help: ImageVector by lazy { icon("Help", line("M3.5 12A8.5 8.5 0 1 0 20.5 12A8.5 8.5 0 1 0 3.5 12Z"), line("M9.6 9.7a2.4 2.4 0 1 1 3.5 2.1c-.7.4-1.1.9-1.1 1.7"), line("M12 16.8h.01")) }
    val Info: ImageVector by lazy { icon("Info", line("M3.5 12A8.5 8.5 0 1 0 20.5 12A8.5 8.5 0 1 0 3.5 12Z"), line("M12 11v5.2"), line("M12 7.8h.01")) }
    val Retry: ImageVector by lazy { icon("Retry", line("M20.5 12a8.5 8.5 0 1 1-2.6-6.1l2.6 2.6"), line("M20.5 4v4.5H16")) }
    val Home: ImageVector by lazy { icon("Home", line("M3.5 10.2 12 3.5l8.5 6.7V19a1.5 1.5 0 0 1-1.5 1.5h-4v-6H9v6H5A1.5 1.5 0 0 1 3.5 19v-8.8z")) }
    val History: ImageVector by lazy { icon("History", line("M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.5"), line("M3.5 4v4.5H8"), line("M12 7.5V12l3 2")) }
    val Settings: ImageVector by lazy { icon("Settings", line("M4 7h9M17 7h3M4 17h3M11 17h9"), line("M13 7A2 2 0 1 0 17 7A2 2 0 1 0 13 7Z"), line("M7 17A2 2 0 1 0 11 17A2 2 0 1 0 7 17Z")) }
    val Wifi: ImageVector by lazy { icon("Wifi", line("M2.5 9.2a13.5 13.5 0 0 1 19 0"), line("M5.8 12.7a8.8 8.8 0 0 1 12.4 0"), line("M9 16.1a4.3 4.3 0 0 1 6 0"), line("M12 19.5h.01")) }
    val ChevR: ImageVector by lazy { icon("ChevR", line("m9.5 6 6 6-6 6")) }
    val ChevD: ImageVector by lazy { icon("ChevD", line("m6 9.5 6 6 6-6")) }
    val Link: ImageVector by lazy { icon("Link", line("M10 13.5a4 4 0 0 0 5.7.3l3-3a4 4 0 0 0-5.7-5.7l-1.2 1.2"), line("M14 10.5a4 4 0 0 0-5.7-.3l-3 3a4 4 0 0 0 5.7 5.7l1.2-1.2")) }
    val Unlink: ImageVector by lazy { icon("Unlink", line("M14.5 9.5 18.7 5.3M9.5 14.5l-4.2 4.2"), line("M13 6.2l.8-.8a4 4 0 0 1 5.7 5.7l-.8.8M11 17.8l-.8.8a4 4 0 0 1-5.7-5.7l.8-.8"), line("M4 4l2 2M18 18l2 2")) }
    val Up: ImageVector by lazy { icon("Up", line("M12 19V5"), line("m6 11 6-6 6 6")) }
    val Down: ImageVector by lazy { icon("Down", line("M12 5v14"), line("m6 13 6 6 6-6")) }
    val Upload: ImageVector by lazy { icon("Upload", line("M12 15V4"), line("m7.5 8.5 4.5-4.5 4.5 4.5"), line("M4 14.5V18a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3.5")) }
    val Plus: ImageVector by lazy { icon("Plus", line("M12 5v14M5 12h14")) }
    val X: ImageVector by lazy { icon("X", line("m6.5 6.5 11 11M17.5 6.5l-11 11")) }
    val Share: ImageVector by lazy { icon("Share", line("M15 5.5A2.5 2.5 0 1 0 20 5.5A2.5 2.5 0 1 0 15 5.5Z"), line("M4 12A2.5 2.5 0 1 0 9 12A2.5 2.5 0 1 0 4 12Z"), line("M15 18.5A2.5 2.5 0 1 0 20 18.5A2.5 2.5 0 1 0 15 18.5Z"), line("m8.7 10.8 6.6-4M8.7 13.2l6.6 4")) }
    val Copy: ImageVector by lazy { icon("Copy", line("M11 8.5H18A2.5 2.5 0 0 1 20.5 11V18A2.5 2.5 0 0 1 18 20.5H11A2.5 2.5 0 0 1 8.5 18V11A2.5 2.5 0 0 1 11 8.5Z"), line("M15.5 8.5v-3a2 2 0 0 0-2-2h-8a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h3")) }
    val Shield: ImageVector by lazy { icon("Shield", line("M12 3 5 5.8v5.7c0 4.3 2.9 7.8 7 9.5 4.1-1.7 7-5.2 7-9.5V5.8L12 3z"), line("m9 12 2 2 4-4")) }
    val Sun: ImageVector by lazy { icon("Sun", line("M8 12A4 4 0 1 0 16 12A4 4 0 1 0 8 12Z"), line("M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4")) }
    val Moon: ImageVector by lazy { icon("Moon", line("M19.5 14.6A7.8 7.8 0 1 1 9.4 4.5a6.3 6.3 0 0 0 10.1 10.1z")) }
    val Auto: ImageVector by lazy { icon("Auto", line("M3.5 12A8.5 8.5 0 1 0 20.5 12A8.5 8.5 0 1 0 3.5 12Z"), filled("M12 3.5v17a8.5 8.5 0 0 0 0-17z")) }

    private class Part(val data: String, val fill: Boolean)

    private fun line(data: String) = Part(data, fill = false)

    /** A part drawn filled as well as stroked (the media buttons, the half of the "system" theme icon). */
    private fun filled(data: String) = Part(data, fill = true)

    private fun icon(name: String, vararg parts: Part): ImageVector {
        val builder = ImageVector.Builder(name = name, defaultWidth = 24.dp, defaultHeight = 24.dp, viewportWidth = 24f, viewportHeight = 24f)
        for (part in parts) {
            builder.addPath(
                pathData = addPathNodes(part.data),
                fill = if (part.fill) SolidColor(Color.Black) else null,
                stroke = SolidColor(Color.Black),
                strokeLineWidth = 2f,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        return builder.build()
    }
}
