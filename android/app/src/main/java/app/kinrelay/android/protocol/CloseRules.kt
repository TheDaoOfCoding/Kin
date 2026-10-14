package app.kinrelay.android.protocol

/** What a client does after its WebSocket ended (protocol.md, "Close codes"). */
enum class CloseAction { RECONNECT_WITH_BACKOFF, RECONNECT_ON_USER_ACTION, STOP_UPDATE_APP, STOP_PAIR_AGAIN }

object CloseRules {
    const val HELLO_TIMEOUT_MILLIS = 10_000L
    const val STABLE_MILLIS = 60_000L

    /**
     * @param closeCode the close code, or null for a network error / no code
     * @param upgradeStatus the HTTP status that refused the upgrade, if it was refused
     */
    fun action(closeCode: Int?, upgradeStatus: Int? = null): CloseAction = when {
        upgradeStatus == 401 -> CloseAction.STOP_PAIR_AGAIN
        closeCode == 4001 -> CloseAction.RECONNECT_ON_USER_ACTION
        closeCode == 4002 -> CloseAction.STOP_UPDATE_APP
        closeCode == 4003 -> CloseAction.STOP_PAIR_AGAIN
        else -> CloseAction.RECONNECT_WITH_BACKOFF
    }

    /** Only a connection that kept an accepted hello for at least 60 s resets the backoff. */
    fun resetsBackoff(helloAcceptedForMillis: Long?): Boolean =
        helloAcceptedForMillis != null && helloAcceptedForMillis >= STABLE_MILLIS

    /** The close code to use if no hello was accepted within 10 s of the socket opening. */
    fun helloTimeoutClose(helloAccepted: Boolean, elapsedSinceOpenMillis: Long, limitMillis: Long = HELLO_TIMEOUT_MILLIS): Int? =
        if (!helloAccepted && elapsedSinceOpenMillis > limitMillis) 1002 else null
}
