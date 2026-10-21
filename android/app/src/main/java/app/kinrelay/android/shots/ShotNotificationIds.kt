package app.kinrelay.android.shots

/**
 * The notification slot and `PendingIntent` request code of one screenshot's hold, sending and result notifications. The slot is the screenshot's own id (folded to 32 bits), so two screenshots
 * can never share one; it used to be `5000 + id % 1000`, which two ids 1000 apart inside the 5-second hold would have shared (2B.5 review round 3, fixed in 2B.6). The notifications also carry the
 * `kin-shot` tag, which already keeps them apart from every other id the app posts, so no offset is needed.
 */
object ShotNotificationIds {
    fun of(id: Long): Int = (id xor (id ushr 32)).toInt()
}
