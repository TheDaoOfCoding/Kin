package app.kinrelay.android.media

/** One session as the picker sees it, in the platform's own priority order. [own] is Kin's own session, [title] whether it has one, [pausedMs] how long it has been paused. */
data class PickSession(val sid: String, val own: Boolean = false, val title: Boolean, val state: PlayState?, val pausedMs: Long = 0L)

/**
 * Which session is "active" (design 3.5, vectors `pick[]`). The source decides, by a fixed rule:
 * 1. candidates: not Kin's own, with a title, playing, paused or buffering (a session paused for 30 minutes is dropped);
 * 2. the current session stays while it is a candidate **and playing** (no flip-flop between two players);
 * 3. otherwise the first candidate that is playing, in the platform's order;
 * 4. otherwise the current one if it is still a candidate, else the first candidate.
 * `null` means no candidate: the caller sends `media.clear`, debounced 1.5 s.
 */
object MediaPick {
    fun choose(sessions: List<PickSession>, current: String?): String? {
        val candidates = sessions.filter(::isCandidate)
        candidates.firstOrNull { it.sid == current && it.state == PlayState.PLAYING }?.let { return it.sid }
        candidates.firstOrNull { it.state == PlayState.PLAYING }?.let { return it.sid }
        candidates.firstOrNull { it.sid == current }?.let { return it.sid }
        return candidates.firstOrNull()?.sid
    }

    fun isCandidate(s: PickSession): Boolean =
        !s.own && s.title && when (s.state) {
            PlayState.PLAYING, PlayState.BUFFERING -> true
            PlayState.PAUSED -> s.pausedMs < MediaLimits.PAUSED_EXPIRY_MILLIS
            else -> false
        }
}
