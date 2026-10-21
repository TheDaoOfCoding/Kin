package app.kinrelay.android.link

/**
 * Which Wi-Fi networks are up, and which one the app talks over. With two Wi-Fi networks (for example the
 * phone's own and a second one that came up) losing one must not say "Not on Wi-Fi" while the other is
 * still there. The newest network to appear is used; when it goes, the next newest takes over.
 */
class WifiTracker<N : Any> {
    private val up = LinkedHashSet<N>()

    /** The network in use, or null when no Wi-Fi is up. */
    var current: N? = null
        private set

    /** A Wi-Fi network came up and becomes the current one. */
    fun available(network: N) {
        up.remove(network)
        up.add(network)
        current = network
    }

    /** What changed when a network went away. */
    class Lost<N>(
        /** The lost network was the one in use, so the caller has to switch. */
        val wasCurrent: Boolean,
        /** The network to use now: another Wi-Fi network that is still up, or null if there is none. */
        val next: N?,
    )

    fun lost(network: N): Lost<N> {
        up.remove(network)
        if (network != current) return Lost(wasCurrent = false, next = current)
        current = up.lastOrNull()
        return Lost(wasCurrent = true, next = current)
    }

    val isEmpty: Boolean get() = up.isEmpty()
}
