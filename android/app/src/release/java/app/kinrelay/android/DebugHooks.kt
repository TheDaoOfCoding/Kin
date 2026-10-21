package app.kinrelay.android

import android.content.Intent

/** The release version of the debug hooks: there are none. A pairing link from outside the app always has to be confirmed. */
object DebugHooks {
    @Suppress("UNUSED_PARAMETER")
    fun autoConfirmPairing(intent: Intent): Boolean = false

    /** Always true: a share hands its URIs to the service with a read grant. */
    @Suppress("UNUSED_PARAMETER")
    fun grantUrisToService(context: android.content.Context): Boolean = true

    /** Always false: a new file name is never refused on purpose. */
    @Suppress("UNUSED_PARAMETER")
    fun refuseNewNames(context: android.content.Context): Boolean = false

    /** The debug-only "Mirror test" section: nothing in a release build. */
    @androidx.compose.runtime.Composable
    @Suppress("UNUSED_PARAMETER")
    fun MirrorTest(graph: AppGraph, context: android.content.Context) = Unit

    /** Always false: a ring always plays the real alarm and touches the real alarm volume. */
    @Suppress("UNUSED_PARAMETER")
    fun ringSilent(context: android.content.Context): Boolean = false

    /** Always 0: the screenshot picker judges "taken a moment ago" by the real clock. */
    @Suppress("UNUSED_PARAMETER")
    fun shotsClockOffsetSeconds(context: android.content.Context): Long = 0L

    /** Always the phone's real maker. */
    @Suppress("UNUSED_PARAMETER")
    fun manufacturer(context: android.content.Context): String = android.os.Build.MANUFACTURER
}
