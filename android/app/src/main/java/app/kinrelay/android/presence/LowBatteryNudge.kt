package app.kinrelay.android.presence

/**
 * The low-battery state machine of design 3.6, a pure class the receiver of a `presence` runs per peer, in memory until Kin quits (not per connection):
 *
 * ```
 * armed starts true
 * if charging or level >= 20      -> armed = true
 * else if level <= 15 and armed   -> NUDGE, armed = false
 * ```
 *
 * One nudge per discharge episode, even across reconnects; re-armed by charging or by reaching 20. 2B.4 builds and tests it; the phone shows a nudge for a laptop in 2B.5.
 */
class LowBatteryNudge {
    private var armed = true

    /** Feeds the battery of one snapshot. True when the user should be told now. [suppressed] (the receiver is paused) still tracks the state but never nudges and does not disarm. */
    fun onBattery(level: Int, charging: Boolean, suppressed: Boolean = false): Boolean {
        if (charging || level >= REARM_LEVEL) {
            armed = true
            return false
        }
        if (level <= NUDGE_LEVEL && armed) {
            if (suppressed) return false
            armed = false
            return true
        }
        return false
    }

    companion object {
        const val NUDGE_LEVEL = 15
        const val REARM_LEVEL = 20
    }
}

/**
 * Android's `ACTION_BATTERY_CHANGED` extras as a wire `battery` (vectors `batteryMap[]`): no battery for a scale of 0 or less or a level below 0, level = round(level * 100 / scale)
 * cut to 0..100, `charging` = plugged in (any `EXTRA_PLUGGED`, which also covers "full while plugged").
 */
object BatteryMap {
    fun fromExtras(level: Int, scale: Int, plugged: Int, saver: Boolean?): Battery? {
        if (scale <= 0 || level < 0) return null
        val percent = Math.round(level * 100.0 / scale).toInt().coerceIn(0, 100)
        return Battery(percent, plugged != 0, saver)
    }
}
