package app.kinrelay.android.ui

import app.kinrelay.android.link.LinkState
import app.kinrelay.android.link.PingResult
import java.util.Locale

object UiText {
    fun status(state: LinkState): String = when (state) {
        is LinkState.Connected -> "Connected"
        is LinkState.Connecting ->
            if (state.retryInMillis != null) "Looking for the PC (next try in ${state.retryInMillis / 1000} s)" else "Looking for the PC…"
        LinkState.NotOnWifi -> "Not on Wi-Fi"
        LinkState.Stopped -> "Not running"
        LinkState.Replaced -> "The PC is connected from elsewhere. Tap Reconnect."
        LinkState.UpdateApp -> "Update the app on your phone or PC: the versions don't match."
        LinkState.PairAgain -> "The PC no longer trusts this phone. Remove it and pair again."
    }

    fun ping(result: PingResult?): String = when (result) {
        null -> ""
        is PingResult.Rtt -> "Ping: ${formatMillis(result.millis)}"
        PingResult.NoReply -> "No reply"
        PingResult.NoConnection -> "Not connected to the PC"
        is PingResult.Failed -> "The PC answered with an error (${result.code})"
    }

    fun formatMillis(millis: Double): String =
        if (millis < 10) String.format(Locale.US, "%.1f ms", millis) else String.format(Locale.US, "%d ms", Math.round(millis))
}
