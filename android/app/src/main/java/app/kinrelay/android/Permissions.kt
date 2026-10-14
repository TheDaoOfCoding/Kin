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

    private fun granted(context: Context, permission: String) =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
}
