package app.kinrelay.android.setup

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.os.PowerManager
import android.provider.Settings
import androidx.core.net.toUri

/**
 * The settings screens the setup (onboarding) opens.
 *
 * **Battery exemption: the compliant route.** Google Play restricts `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` (the permission behind the
 * one-tap "Let Kin always run in the background?" dialog) to apps whose core function is hurt without it. A companion app that keeps
 * a `connectedDevice` connection may qualify, but it has to be declared in the Play Console and the review decides. The route that needs
 * no permission and no declaration is the system's own list ([Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS]), where the user sets
 * Kin to "Not optimised" with two taps. Kin uses that, and so never declares the permission (docs/android.md, "Battery and OEM killers").
 * Whether it is already done is read with [PowerManager.isIgnoringBatteryOptimizations], which needs no permission either.
 */
object SetupActions {
    fun batteryOptimisationIgnored(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) == true

    /** The system's battery optimisation list. Falls back to Kin's own settings page. */
    fun openBatteryOptimisationList(context: Context): Boolean =
        start(context, Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) || openAppDetails(context)

    fun openAppDetails(context: Context): Boolean =
        start(context, Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, "package:${context.packageName}".toUri()))

    fun openNotificationSettings(context: Context): Boolean =
        start(context, Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)) || openAppDetails(context)

    fun openChannelSettings(context: Context, channelId: String): Boolean =
        start(
            context,
            Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                .putExtra(Settings.EXTRA_CHANNEL_ID, channelId),
        ) || openNotificationSettings(context)

    /** What [openOem] did: which of the guide's screens opened, or that none did and the app's own settings page did. */
    sealed interface OemOpen {
        data class Opened(val label: String) : OemOpen
        data object FellBack : OemOpen
    }

    /** Opens the first screen of [guide] that exists on this phone; if none does, Kin's own settings page (with the steps still on screen). */
    fun openOem(context: Context, guide: OemGuide): OemOpen {
        for (target in guide.targets) {
            if (start(context, intentFor(target, context.packageName))) return OemOpen.Opened(target.label)
        }
        openAppDetails(context)
        return OemOpen.FellBack
    }

    internal fun intentFor(target: SettingsTarget, packageName: String): Intent {
        val intent = Intent()
        if (target.packageName != null && target.className != null) intent.setClassName(target.packageName, target.className)
        target.action?.let { intent.action = it }
        target.extras.forEach { (key, value) -> intent.putExtra(key, if (value == "%PACKAGE%") packageName else value) }
        return intent
    }

    /** Starts the activity; false when the phone has no such screen (or won't let Kin open it). */
    private fun start(context: Context, intent: Intent): Boolean = try {
        context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        true
    } catch (_: ActivityNotFoundException) {
        false
    } catch (_: SecurityException) {
        false
    }
}
