package app.kinrelay.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat

/** The two runtime permissions the app asks for. Both only exist on newer Android versions. */
object Permissions {
    /** Android 17 (API 37) gates all traffic to local network addresses behind this permission. */
    const val LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
    private const val ANDROID_17 = 37

    fun localNetworkGranted(context: Context): Boolean =
        Build.VERSION.SDK_INT < ANDROID_17 || granted(context, LOCAL_NETWORK)

    fun notificationsGranted(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU || granted(context, Manifest.permission.POST_NOTIFICATIONS)

    /** What is still missing and can be asked for on this Android version. */
    fun missing(context: Context): List<String> = buildList {
        if (!localNetworkGranted(context)) add(LOCAL_NETWORK)
        if (!notificationsGranted(context)) add(Manifest.permission.POST_NOTIFICATIONS)
    }

    /** What the setup screen shows. */
    fun snapshot(context: Context) = PermissionSnapshot(
        notifications = notificationsGranted(context),
        notificationsExist = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU,
        localNetwork = localNetworkGranted(context),
        localNetworkExists = Build.VERSION.SDK_INT >= ANDROID_17,
        batteryUnrestricted = app.kinrelay.android.setup.SetupActions.batteryOptimisationIgnored(context),
        notificationAccess = notificationAccessGranted(context),
        photos = photosGranted(context),
    )

    /**
     * Android lets Kin read **all** photos (2B.4 auto-send): `READ_MEDIA_IMAGES` on Android 13+, `READ_EXTERNAL_STORAGE` on 10 to 12. Read each time, **never stored**: on Android 14+ "Select photos" gives
     * a grant for one session only, which does not work for a background watcher, so the answer must be the system's at the moment of use. Kin does not declare `READ_MEDIA_VISUAL_USER_SELECTED`.
     */
    fun photosGranted(context: Context): Boolean =
        granted(context, if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_IMAGES else Manifest.permission.READ_EXTERNAL_STORAGE)

    /** "Notification access" (2B.2): a special access the user gives in the system settings, not a runtime permission. Read again each time the app comes back. */
    fun notificationAccessGranted(context: Context): Boolean = try {
        context.getSystemService(android.app.NotificationManager::class.java)
            .isNotificationListenerAccessGranted(android.content.ComponentName(context, app.kinrelay.android.notifmirror.MirrorListenerService::class.java))
    } catch (_: Exception) {
        false
    }

    private fun granted(context: Context, permission: String) =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
}

/** The state of everything the setup (onboarding) is about. A permission that doesn't exist on this Android version counts as granted and isn't shown. */
data class PermissionSnapshot(
    val notifications: Boolean = true,
    val notificationsExist: Boolean = false,
    val localNetwork: Boolean = true,
    val localNetworkExists: Boolean = false,
    val batteryUnrestricted: Boolean = false,
    /** The user gave Kin notification access (2B.2 mirroring). */
    val notificationAccess: Boolean = false,
    /** Android lets Kin read all photos right now (2B.4 auto-send). Never stored. */
    val photos: Boolean = false,
)
