package app.kinrelay.android.transfer

import android.content.Context
import android.os.PowerManager

/**
 * A partial wake lock that exists only while bytes move (architecture.md, "Battery rules for the phone"). Calls nest, so two
 * holders can't release each other's lock; the lock is let go as soon as the last holder is done. The timeout is a safety net
 * for a bug that forgets to release, not a plan: it is far longer than any single request.
 */
class AndroidWakeLocks(context: Context) : WakeLockPort {
    private val lock = context.applicationContext.getSystemService(PowerManager::class.java)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "kin:transfer")
        .apply { setReferenceCounted(false) }
    private var holders = 0

    @Synchronized
    override fun acquire() {
        if (holders++ == 0) lock.acquire(SAFETY_TIMEOUT_MILLIS)
    }

    @Synchronized
    override fun release() {
        if (holders > 0 && --holders == 0 && lock.isHeld) lock.release()
    }

    /** For the debug state dump. */
    @Synchronized
    fun isHeld(): Boolean = lock.isHeld

    private companion object {
        const val SAFETY_TIMEOUT_MILLIS = 30 * 60_000L
    }
}
