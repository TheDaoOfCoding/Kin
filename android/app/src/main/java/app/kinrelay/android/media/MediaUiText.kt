package app.kinrelay.android.media

/** The words of the Media card (design 4.7): pure, so they are tested on the JVM. Nothing here has the title, artist or album of any session. */
object MediaUiText {
    /** Under "Show this phone's media on your PC": only what the user must act on or wait for (null: the switch says it all). */
    fun shareLine(ui: MediaUi, access: Boolean): String? {
        val pc = ui.pcName ?: "your PC"
        return when {
            !ui.policy.share -> null
            !access -> "Paused: Kin doesn't have notification access"
            !ui.connected -> "Waiting for $pc"
            !ui.peerListsMedia -> "$pc needs Kin 0.2.0 to show media"
            ui.peerStatus == null -> null
            !ui.peerStatus.show -> "$pc isn't showing your media: turn it on in Kin on your PC"
            else -> null
        }
    }

    /** Under "Show your PC's media on this phone": the same rule as [shareLine]. */
    fun showLine(ui: MediaUi): String? {
        val pc = ui.pcName ?: "your PC"
        return when {
            !ui.policy.show -> null
            !ui.connected -> "Waiting for $pc"
            !ui.peerListsMedia -> "$pc needs Kin 0.2.0 to share media"
            ui.peerStatus == null -> null
            !ui.peerStatus.share -> "$pc isn't sharing its media: turn it on in Kin on your PC"
            !ui.notificationsAllowed -> "Notifications are off for Kin, so Android can't show it"
            ui.showing -> "Showing $pc's media"
            else -> null
        }
    }

    /** The one line of the Home card, or null when neither direction is on. */
    fun homeLine(ui: MediaUi, access: Boolean): String? {
        val pc = ui.pcName ?: "your PC"
        val share = ui.policy.share
        val show = ui.policy.show
        return when {
            !share && !show -> null
            share && !access -> "Notification access is off: this phone's media isn't shared"
            show && ui.showing -> if (ui.pcPlaying) "$pc is playing" else "$pc has media paused"
            share && ui.sharing -> "Sharing this phone's media with $pc"
            else -> "Media is on. ${if (!ui.connected) "Waiting for $pc" else "Nothing is playing"}"
        }
    }
}
