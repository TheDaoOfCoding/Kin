package app.kinrelay.android.dnd

/** The words of the Do Not Disturb settings (design 4.2.1, 4.3): plain, and honest about what the access allows. */
object DndText {
    const val FOLLOW_TITLE = "Silence this phone while my PC is in Focus"
    const val FOLLOW_DESCRIPTION =
        "When Windows Focus or Do Not Disturb is on, Kin turns Do Not Disturb on here too, and turns it off again when your PC leaves Focus. " +
            "Kin only adds silence: if you are already in Do Not Disturb it changes nothing, and if you turn it off yourself it stays off until the next time. Off by default."

    const val SHARE_TITLE = "Tell my PC when this phone is on Do Not Disturb"
    const val SHARE_DESCRIPTION =
        "Your PC shows \"Phone: Do Not Disturb\" and holds its own optional pop-ups while it lasts. Only your paired PC sees it; Kin keeps no history. Needs no special access. Off by default."

    const val PRIORITY_LABEL = "Priority only"
    const val PRIORITY_NOTE = "Your own exceptions still ring (calls and messages from starred contacts, alarms)."
    const val ALARMS_LABEL = "Alarms only"
    const val ALARMS_NOTE = "Only alarms make a sound. Kin never uses total silence: that would mute your alarm and \"Ring my phone\"."

    const val ACCESS_TITLE = "Do Not Disturb access"
    const val ACCESS_NEEDED = "Kin needs Do Not Disturb access to do this."
    const val ACCESS_EXPLANATION = "Kin can turn Do Not Disturb on and off. It cannot read your notifications."
    const val ACCESS_BUTTON = "Allow Do Not Disturb access"

    const val UPDATE_PC = "Update Kin on your PC to use this."

    /** Android keeps Kin's silence after a force-stop, and Kin can't run to give it back until it is opened again (2B.5 review round 3). */
    const val FORCE_STOP_NOTE = "If Kin was force-stopped, open Kin to release."

    /** "Silenced by Kin (Priority only) · …" while Kin holds the phone silent; [route] is `rule` on Android 15+ and `global` before. */
    fun holding(route: String?, filter: Int): String {
        val how = if (filter == DndFilter.ALARMS) "Alarms only" else "Priority only"
        val where = if (route == DndApplier.ROUTE_RULE) "Settings > Modes shows it as \"Do Not Disturb (Kin)\"" else "Do Not Disturb is on"
        return "Silenced by Kin ($how). $where."
    }

    /** What the access row says. A [holding] Kin whose access is gone can't release now (the access screen or the next start does). */
    fun access(granted: Boolean, holding: Boolean): String = when {
        granted -> "Do Not Disturb access is on."
        holding -> "Access lost: Kin can't release Do Not Disturb until access is back, or the next time it starts."
        else -> ACCESS_NEEDED
    }
}
