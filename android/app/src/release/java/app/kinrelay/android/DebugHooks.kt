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
}
