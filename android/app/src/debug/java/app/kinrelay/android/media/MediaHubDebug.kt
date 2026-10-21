package app.kinrelay.android.media

/**
 * Debug builds only (design 4.9): helpers for the `MEDIA_*` debug commands. They are extensions of [MediaHub] in the debug source set, so none of them is in the release APK. They read
 * and drive the hub through its `internal` members; every line they print is counts and settings, never a title, artist, album or art.
 */
fun MediaHub.pcSessionShown(): Boolean = viewer.isShown

fun MediaHub.debugState(): String {
    val peer = sender.peerStatus.value
    val policy = currentPolicy
    return "share=${policy.share} show=${policy.show} access=${accessGranted()} listener=${source.isRunning} online=${sender.isOnline()} open=${sender.isOpen()} " +
        "peerShow=${peer?.show} peerShare=${peer?.share} sentStates=${stats.statesSent.get()} images=${stats.imagesSent.get()} volumes=${stats.volumesSent.get()} clears=${stats.clearsSent.get()} " +
        "statuses=${stats.statusesSent.get()} cmdsSent=${stats.cmdsSent.get()} shown=${stats.statesShown.get()} dropped={${stats.dropsLine()}} cmdsIn={${stats.cmdsInLine()}} cmdErrors={${stats.cmdErrorsLine()}} " +
        "lastCmdIn=${stats.lastCmdIn} pcSession=${pcSessionShown()} pcState=${viewer.current?.state?.wire} readAll=$readAllSessions notificationsAllowed=${viewer.notificationsAllowed}"
}

/** Calls the callback of Kin's own PC-media session through its own controller: exactly what a carousel button does, with no input injection. */
fun MediaHub.debugPcCommand(cmd: String, positionMs: Long, level: Int, delta: Int): String {
    val controller = (if (pcSessionLazy.isInitialized()) pcSessionLazy.value.controller() else null) ?: return "no session"
    val controls = controller.transportControls
    when (cmd) {
        "play" -> controls.play()
        "pause" -> controls.pause()
        "next" -> controls.skipToNext()
        "prev" -> controls.skipToPrevious()
        "seek" -> controls.seekTo(positionMs)
        "volume" -> {
            if (delta != 0) controller.adjustVolume(if (delta > 0) 1 else -1, 0) else controller.setVolumeTo(level, 0)
        }
        else -> return "unknown"
    }
    return "sent"
}

/** Feeds a state as if the PC had sent it (the switches still apply except `show`, which the caller of the debug command sets). */
fun MediaHub.debugShowPc(state: MediaState, artBytes: ByteArray?, volumeKeys: Boolean = false) {
    val base = currentPolicy
    val policy = base.copy(show = true, useVolumeKeys = volumeKeys || base.useVolumeKeys)
    viewer.forgetDismissal()
    if (artBytes != null) {
        val hash = MediaWire.imageHash(artBytes)
        viewer.onImage(MediaImage(hash, MediaWire.KIND_ART, artBytes), policy)
        viewer.onState(state.copy(artHash = hash), policy)
    } else {
        viewer.onState(state, policy)
    }
}

fun MediaHub.debugClearPc() = viewer.forgetAll()

fun MediaHub.debugReadAll(on: Boolean) {
    readAllSessions = on
    source.refresh()
}

fun MediaHub.debugReadAllOn(): Boolean = readAllSessions
