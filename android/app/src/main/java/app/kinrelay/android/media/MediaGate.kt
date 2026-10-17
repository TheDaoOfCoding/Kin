package app.kinrelay.android.media

/** What the media sender needs of a connection: ready, the peer's latest `hello` lists `media`, and a frame can be put on the wire (best effort, no ack). */
interface MediaLink {
    val isReady: Boolean
    val peerUnderstandsMedia: Boolean

    /** Puts the frame on the socket. False when it could not be queued (the socket is closed). Nothing is kept or re-sent. */
    fun sendEphemeral(frame: String): Boolean
}

/** What a source pushes (`media.state`, `media.image`, `media.volume`, `media.clear`). */
enum class PushKind { STATE, IMAGE, VOLUME, CLEAR }

/** Who may send what (design 3.1, vectors `sendRule[]` and `viewerRule[]`). All pure: [peerListsMedia] is "the peer's latest `hello` lists `media`", [peerStatus] the peer's latest `media.status` (null before any). */
object MediaGate {
    /**
     * A source sends `media.state`, `media.image`, `media.volume` and `media.clear` only if its own `share` is on, the peer lists `media` **and the peer's latest status says `show: true`**
     * (before any status arrives it sends nothing). The one exception is the single `media.clear` after `share` was just turned off: it still needs the viewer to show and a status.
     */
    fun mayPush(kind: PushKind, ownShare: Boolean, peerListsMedia: Boolean, peerStatus: MediaStatus?, shareJustTurnedOff: Boolean = false): Boolean {
        if (!peerListsMedia || peerStatus?.show != true) return false
        return ownShare || (kind == PushKind.CLEAR && shareJustTurnedOff)
    }

    /** A viewer sends `media.cmd` only to a peer that lists `media` and whose latest status says `share: true`. */
    fun mayCommand(peerListsMedia: Boolean, peerStatus: MediaStatus?): Boolean = peerListsMedia && peerStatus?.share == true

    /** `media.status` goes to any peer that lists `media`, even before its own status. */
    fun mayStatus(peerListsMedia: Boolean): Boolean = peerListsMedia

    /** A viewer whose `show` is off drops `media.state`, `media.image`, `media.clear` and `media.volume` silently (counted), whatever the sender did. Status and commands aren't the viewer's. */
    fun viewerAccepts(type: String, ownShow: Boolean): Boolean = when (type) {
        MediaWire.STATE, MediaWire.IMAGE, MediaWire.CLEAR, MediaWire.VOLUME -> ownShow
        else -> true
    }
}
