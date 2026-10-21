package app.kinrelay.android.presence

/**
 * The PC's low battery on the phone (2B.5, design 4.4; decision 10: the phone decides, from the PC's `presence.battery`, with its own pause). It runs 2B.4's [LowBatteryNudge] per peer, **in memory
 * until Kin quits** (not per connection): at 15 % or less and not charging one nudge per discharge episode, re-armed at 20 % or by charging. Nothing is stored and no message is sent for it.
 *
 * The state is tracked always, but a nudge is **suppressed** (and does not disarm) while the PC's battery is not shown (`presence.show` off) or this phone is paused, so a nudge that was not wanted is
 * not lost for good: it comes at the next snapshot once it is wanted again, if the battery is still low and nothing re-armed it.
 */
class PcBatteryNudge {
    private val machines = HashMap<String, LowBatteryNudge>()

    /**
     * Feeds the battery of one snapshot of the PC [peerDeviceId] (null: the PC says nothing about a battery, a desktop). Returns the level to tell the user about now, or null.
     */
    @Synchronized
    fun onPcBattery(peerDeviceId: String, battery: Battery?, showOn: Boolean, paused: Boolean): Int? {
        if (battery == null) return null
        val machine = machines.getOrPut(peerDeviceId) { LowBatteryNudge() }
        return battery.level.takeIf { machine.onBattery(battery.level, battery.charging, suppressed = !showOn || paused) }
    }

    /** Unlinking forgets the peer. */
    @Synchronized
    fun forget(peerDeviceId: String) {
        machines.remove(peerDeviceId)
    }

    companion object {
        /** "Desk PC battery is 14%" (the notification's title; the text under it says what to do). */
        fun title(pcName: String, level: Int): String = "$pcName battery is $level%"

        const val TEXT = "It isn't charging."
    }
}
