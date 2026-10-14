package app.kinrelay.android

import android.content.Intent

/** The release version of the debug hooks: there are none. A pairing link from outside the app always has to be confirmed. */
object DebugHooks {
    @Suppress("UNUSED_PARAMETER")
    fun autoConfirmPairing(intent: Intent): Boolean = false
}
