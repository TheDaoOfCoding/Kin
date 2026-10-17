package app.kinrelay.android.presence

/** What this phone knows about the PC's pause, in words' terms: [untilClock] is the wall-clock time of its end as the phone shows it ("14:30"), null when it has no end or the end is unknown. */
data class PeerPauseText(val untilResumed: Boolean, val untilClock: String?)

/** The words of the persistent notification and Home (design 5.2): `Connected to Desk PC · 82% · charging · Kin paused on the PC until 14:30`. Nothing is stored. */
object PresenceText {
    fun battery(battery: Battery): String = if (battery.charging) "${battery.level}% · charging" else "${battery.level}%"

    fun pause(pause: PeerPauseText): String =
        if (pause.untilResumed || pause.untilClock == null) "paused on the PC" else "Kin paused on the PC until ${pause.untilClock}"

    /** [base] is the usual line ("Connected to Desk PC"); the battery is added only when the PC sent one and "Show the PC's status" is on ([show]). */
    fun connected(base: String, battery: Battery?, show: Boolean, pause: PeerPauseText?): String {
        val parts = ArrayList<String>(3)
        parts += base
        if (show && battery != null) parts += battery(battery)
        if (show && pause != null) parts += pause(pause)
        return parts.joinToString(" · ")
    }
}
