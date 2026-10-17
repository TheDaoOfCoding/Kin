package app.kinrelay.android

import android.content.Intent

/**
 * Debug builds only. The release source set has another `DebugHooks` whose answer is always "no", so the
 * release APK can't skip the pairing confirmation (check: `strings classes.dex | grep AUTO_CONFIRM` finds nothing).
 */
object DebugHooks {
    const val EXTRA_AUTO_CONFIRM = "app.kinrelay.android.AUTO_CONFIRM"

    /**
     * `adb shell am start -a android.intent.action.VIEW -d "<link>" --ez app.kinrelay.android.AUTO_CONFIRM true`
     * pairs without the confirmation screen, so scripts can pair without tapping.
     */
    fun autoConfirmPairing(intent: Intent): Boolean = intent.getBooleanExtra(EXTRA_AUTO_CONFIRM, false)

    /**
     * The R5 control experiment: with the file `files/debug-no-grant` present, a share does NOT hand its URIs to the service with a read
     * grant (`adb shell run-as app.kinrelay.android touch files/debug-no-grant`), so the activity's own temporary access is all there is.
     */
    fun grantUrisToService(context: android.content.Context): Boolean = !java.io.File(context.filesDir, "debug-no-grant").exists()

    /**
     * With the file `files/debug-refuse-names` present, saving a received file always says "that name is taken" (what MediaStore does for a
     * file this app can't see), so the 100-suffix limit can be reached on the emulator
     * (`adb shell run-as app.kinrelay.android touch files/debug-refuse-names`).
     */
    fun refuseNewNames(context: android.content.Context): Boolean = java.io.File(context.filesDir, "debug-refuse-names").exists()

    /** The debug-only "Mirror test" section of the Debug screen (design 4.14): buttons that post test notifications from Kin itself. */
    @androidx.compose.runtime.Composable
    fun MirrorTest(graph: AppGraph, context: android.content.Context) = app.kinrelay.android.debug.MirrorTestSection(graph, context)

    /**
     * The phone maker the setup screen picks its guide for. With the file `files/debug-oem` present, its first line is used instead of
     * `Build.MANUFACTURER`, so the emulator can show the Xiaomi, Samsung ... steps
     * (`adb shell "run-as app.kinrelay.android sh -c 'echo xiaomi > files/debug-oem'"`).
     */
    fun manufacturer(context: android.content.Context): String =
        java.io.File(context.filesDir, "debug-oem").takeIf { it.exists() }?.readLines()?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() } ?: android.os.Build.MANUFACTURER
}
