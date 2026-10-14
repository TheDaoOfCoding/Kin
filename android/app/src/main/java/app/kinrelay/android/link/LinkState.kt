package app.kinrelay.android.link

import app.kinrelay.android.pairing.PairingAddress

/** What the link to the paired PC is doing right now. */
sealed interface LinkState {
    /** No service is running. */
    data object Stopped : LinkState

    data object NotOnWifi : LinkState

    /** Looking for the PC or waiting before the next attempt. */
    data class Connecting(val retryInMillis: Long? = null) : LinkState

    data class Connected(val peerName: String, val address: PairingAddress) : LinkState

    /** Another connection of this device took over (4001). Waits for a user action or a network change. */
    data object Replaced : LinkState

    /** The PC speaks another protocol version (4002). */
    data object UpdateApp : LinkState

    /** The PC no longer trusts this phone (4003 or 401). */
    data object PairAgain : LinkState
}

sealed interface PingResult {
    data class Rtt(val millis: Double) : PingResult
    data object NoConnection : PingResult
    data object NoReply : PingResult
    data class Failed(val code: String, val message: String) : PingResult
}
