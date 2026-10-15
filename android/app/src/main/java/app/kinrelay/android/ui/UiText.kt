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
        LinkState.PermissionNeeded -> "Nearby devices permission needed"
    }

    /** Why a send found no connection, for the "PC not reachable" notification. */
    fun unreachable(state: LinkState): String = when (state) {
        LinkState.NotOnWifi -> "This phone isn't on Wi-Fi."
        LinkState.PermissionNeeded -> "Nearby devices permission needed. Open Kin to allow it."
        LinkState.PairAgain -> "The PC no longer trusts this phone. Open Kin and pair again."
        LinkState.UpdateApp -> "Update the app on your phone or PC: the versions don't match."
        LinkState.Replaced -> "The PC is connected from elsewhere. Open Kin to reconnect."
        is LinkState.Connected, is LinkState.Connecting, LinkState.Stopped -> "Check that both are on the same Wi-Fi and that Kin is running on the PC."
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

/** Words and numbers for the history and debug screens. Pure, so the tests can check them. */
object HistoryText {
    /** "512 B", "1.5 KB", "3.4 MB", "2.00 GB" (powers of 1000, like the system's storage screens). */
    fun size(bytes: Long): String = when {
        bytes < 1_000 -> "$bytes B"
        bytes < 1_000_000 -> String.format(Locale.US, "%.1f KB", bytes / 1_000.0)
        bytes < 1_000_000_000 -> String.format(Locale.US, "%.1f MB", bytes / 1_000_000.0)
        else -> String.format(Locale.US, "%.2f GB", bytes / 1_000_000_000.0)
    }

    /** "12:03" for today, "14 Oct 12:03" for another day. */
    fun time(millis: Long, nowMillis: Long, zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): String {
        val at = java.time.Instant.ofEpochMilli(millis).atZone(zone)
        val now = java.time.Instant.ofEpochMilli(nowMillis).atZone(zone)
        val pattern = if (at.toLocalDate() == now.toLocalDate()) "HH:mm" else "d MMM HH:mm"
        return java.time.format.DateTimeFormatter.ofPattern(pattern, Locale.getDefault()).format(at)
    }

    fun status(item: app.kinrelay.android.history.HistoryItem): String = when (item.status) {
        app.kinrelay.android.history.HistoryStatus.QUEUED -> "Waiting to send"
        app.kinrelay.android.history.HistoryStatus.ACTIVE -> if (item.outgoing) "Sending…" else "Receiving…"
        app.kinrelay.android.history.HistoryStatus.DONE -> if (item.outgoing) "Sent" else "Received"
        app.kinrelay.android.history.HistoryStatus.FAILED -> "Failed"
        app.kinrelay.android.history.HistoryStatus.CANCELED -> "Canceled"
        app.kinrelay.android.history.HistoryStatus.EXPIRED -> "Expired"
    }

    /** "To Desk PC" or "From Desk PC". */
    fun direction(item: app.kinrelay.android.history.HistoryItem): String = if (item.outgoing) "To ${item.peerName}" else "From ${item.peerName}"
}
