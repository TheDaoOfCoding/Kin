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
}
