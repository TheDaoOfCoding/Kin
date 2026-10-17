package app.kinrelay.android.notifmirror

import android.app.AppOpsManager
import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Process
import android.provider.Settings

/**
 * "Restricted setting" (design 4.2). Android 13 added it: an app installed **outside an app store** can't be given the notification-listener
 * special access until the user allows it in App info; Android 15 widened it. Android decides by the **installer**: an app installed through the
 * session-based installer API (what stores use) isn't restricted, one installed by a browser, a file manager or a messaging app is.
 *
 * There is **no documented API** that says "this app is restricted", so the detection is a chain, and only the first link ever turns mirroring on:
 * 1. **Granted** (`isNotificationListenerAccessGranted`): the only thing the code relies on.
 * 2. **Likely restricted**: a hint from the install source, shown as a heads-up ("Android may ask for one extra step"). Never a gate.
 * 3. **An app-ops probe** of `android:access_restricted_settings`, wrapped so that any failure is "unknown".
 * 4. **After the user comes back and access is still not granted**: the steps card, whatever 2 and 3 said (Android gives no direct signal).
 */
object RestrictedSettings {
    /** Stores whose installs are never restricted (the installer is a store). */
    private val STORES = setOf(
        "com.android.vending", "com.sec.android.app.samsungapps", "com.huawei.appmarket", "com.xiaomi.mipicks", "com.amazon.venezia",
        "com.heytap.market", "com.vivo.appstore", "com.oppo.market", "org.fdroid.fdroid", "com.aurora.store",
    )

    /** Installers that put an APK on the phone from a file: the usual cause of "Restricted setting". */
    private val SIDELOADERS = setOf(
        "com.google.android.packageinstaller", "com.android.packageinstaller", "com.samsung.android.packageinstaller",
        "com.google.android.documentsui", "com.android.documentsui", "com.android.shell", "com.android.chrome", "org.mozilla.firefox",
        "com.whatsapp", "org.telegram.messenger",
    )

    /** Restricted settings exist from Android 13 (API 33). */
    const val FIRST_SDK = 33

    /** A hint, not a fact: this install probably went through a file or a browser, not a store. [installer] is null when Android doesn't say. */
    fun likelyRestricted(sdkInt: Int, installer: String?): Boolean =
        sdkInt >= FIRST_SDK && installer !in STORES && (installer == null || installer in SIDELOADERS)

    enum class Probe { ALLOWED, RESTRICTED, UNKNOWN }

    /** App-ops mode to [Probe]: `MODE_ALLOWED` is allowed, `MODE_IGNORED` and `MODE_ERRORED` are restricted, anything else is unknown. */
    fun probeOf(mode: Int?): Probe = when (mode) {
        null -> Probe.UNKNOWN
        AppOpsManager.MODE_ALLOWED -> Probe.ALLOWED
        AppOpsManager.MODE_IGNORED, AppOpsManager.MODE_ERRORED -> Probe.RESTRICTED
        else -> Probe.UNKNOWN
    }

    /** The numbered steps the card shows (design 4.2). The wording says "the menu is on the app's info page" because phones rename these screens. */
    val STEPS: List<String> = listOf(
        "In Notification access, tap Kin (or its switch). If Android says \"Restricted setting\", tap OK. This step matters: the menu in step 3 usually isn't there until Android has refused once.",
        "Open Settings, Apps, Kin (or press and hold the Kin icon and choose App info).",
        "Tap the three dots at the top right, then \"Allow restricted settings\", and confirm with your PIN or fingerprint. (On some phones the menu is on the app's info page under another name.)",
        "Go back to Notification access and turn Kin on, then confirm Android's own question.",
    )

    fun probe(context: Context): Probe = try {
        val ops = context.getSystemService(AppOpsManager::class.java)
        val mode = ops?.unsafeCheckOpNoThrow(OP_ACCESS_RESTRICTED_SETTINGS, Process.myUid(), context.packageName)
        probeOf(mode)
    } catch (_: Exception) {
        Probe.UNKNOWN
    }

    /** The installer of Kin, or null when Android doesn't say (API 33+; the hint isn't used below that). */
    fun installer(context: Context): String? = try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val info = context.packageManager.getInstallSourceInfo(context.packageName)
            if (Build.VERSION.SDK_INT >= 34) info.installingPackageName ?: info.initiatingPackageName else info.installingPackageName
        } else {
            null
        }
    } catch (_: Exception) {
        null
    }

    fun hint(context: Context): Boolean = likelyRestricted(Build.VERSION.SDK_INT, installer(context))

    /**
     * The intent of "Open notification access": the detail page of Kin's own service on API 30+ (the system keeps its component name), the list on API 29
     * and as the fallback when the detail page can't be resolved. Returns true if a screen opened.
     */
    fun openNotificationAccess(context: Context): Boolean {
        val component = ComponentName(context, MirrorListenerService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val detail = Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, component.flattenToString())
            if (start(context, detail)) return true
        }
        return start(context, Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
    }

    private fun start(context: Context, intent: Intent): Boolean = try {
        context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        true
    } catch (_: ActivityNotFoundException) {
        false
    } catch (_: SecurityException) {
        false
    }

    /** The op name is from memory (the string behind `AppOpsManager.OPSTR_ACCESS_RESTRICTED_SETTINGS`); a wrong name only makes the probe "unknown". */
    private const val OP_ACCESS_RESTRICTED_SETTINGS = "android:access_restricted_settings"
}
