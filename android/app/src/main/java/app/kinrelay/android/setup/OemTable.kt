package app.kinrelay.android.setup

/**
 * Where an OEM keeps the settings that let a background app live (docs/android.md, "Battery and OEM killers"). **This is the one table to
 * update** when a vendor moves a screen: every other place only asks [OemTable.select].
 *
 * Nothing here is a public API. These are the activities that the vendors' own settings apps have, collected from the field (the
 * dontkillmyapp.com guides, and the auto-start helper libraries). An activity that doesn't exist on a phone is simply skipped ([SetupActions]
 * tries the next one and ends at the app's own settings page), so a stale entry costs nothing.
 */
data class SettingsTarget(
    val label: String,
    val packageName: String? = null,
    val className: String? = null,
    val action: String? = null,
    val extras: Map<String, String> = emptyMap(),
)

data class OemGuide(
    val id: String,
    /** What the user sees: "Xiaomi, Redmi and POCO (MIUI, HyperOS)". */
    val title: String,
    /** `Build.MANUFACTURER` values (lower case) this guide is for. */
    val manufacturers: Set<String>,
    /** `Build.BRAND` values (lower case): Redmi and POCO phones say "Xiaomi" as the manufacturer, but some builds only say it in the brand. */
    val brands: Set<String> = emptySet(),
    /** Short steps in the order to do them. */
    val steps: List<String>,
    /** The settings screens to open, the first that exists on the phone wins. */
    val targets: List<SettingsTarget>,
)

object OemTable {
    val guides: List<OemGuide> = listOf(
        OemGuide(
            id = "xiaomi",
            title = "Xiaomi, Redmi and POCO (MIUI, HyperOS)",
            manufacturers = setOf("xiaomi", "redmi", "poco"),
            brands = setOf("xiaomi", "redmi", "poco"),
            steps = listOf(
                "Open Autostart and turn Kin on.",
                "Open Kin's battery saver and choose “No restrictions”.",
                "In the recent apps list, press and hold Kin's card and tap the lock, so it isn't cleared.",
            ),
            targets = listOf(
                SettingsTarget("Autostart", "com.miui.securitycenter", "com.miui.permcenter.autostart.AutoStartManagementActivity"),
                SettingsTarget("Battery saver", "com.miui.powerkeeper", "com.miui.powerkeeper.ui.HiddenAppsConfigActivity", extras = mapOf("package_name" to "%PACKAGE%", "package_label" to "Kin")),
            ),
        ),
        OemGuide(
            id = "samsung",
            title = "Samsung (One UI)",
            manufacturers = setOf("samsung"),
            steps = listOf(
                "Open Settings, then Battery, then Background usage limits.",
                "Open “Never sleeping apps” and add Kin.",
                "Turn off “Put unused apps to sleep”, or make sure Kin isn't in the sleeping apps list.",
            ),
            targets = listOf(
                SettingsTarget("Battery", "com.samsung.android.lool", "com.samsung.android.sm.ui.battery.BatteryActivity"),
                SettingsTarget("Battery", "com.samsung.android.sm_cn", "com.samsung.android.sm.ui.battery.BatteryActivity"),
            ),
        ),
        OemGuide(
            id = "coloros",
            title = "Oppo, Realme and OnePlus (ColorOS, OxygenOS)",
            manufacturers = setOf("oppo", "realme", "oneplus"),
            brands = setOf("oppo", "realme", "oneplus"),
            steps = listOf(
                "Open the startup manager and allow Kin to start automatically.",
                "Open Kin's battery settings and allow background activity (turn off “Optimise battery use”).",
                "In the recent apps list, pull Kin's card down (or tap the lock) so it isn't cleared.",
            ),
            targets = listOf(
                SettingsTarget("Startup manager", "com.coloros.safecenter", "com.coloros.safecenter.permission.startup.StartupAppListActivity"),
                SettingsTarget("Startup manager", "com.coloros.safecenter", "com.coloros.safecenter.startupapp.StartupAppListActivity"),
                SettingsTarget("Startup manager", "com.oppo.safe", "com.oppo.safe.permission.startup.StartupAppListActivity"),
                SettingsTarget("Startup manager", "com.oneplus.security", "com.oneplus.security.chainlaunch.view.ChainLaunchAppListActivity"),
                SettingsTarget("Battery", "com.coloros.oppoguardelf", "com.coloros.powermanager.fuelgaue.PowerUsageModelActivity"),
            ),
        ),
        OemGuide(
            id = "vivo",
            title = "Vivo and iQOO (Funtouch OS, OriginOS)",
            manufacturers = setOf("vivo", "iqoo"),
            brands = setOf("vivo", "iqoo"),
            steps = listOf(
                "Open the autostart list (i Manager, App manager) and turn Kin on.",
                "Open the battery settings, then “Background power consumption”, and allow Kin.",
            ),
            targets = listOf(
                SettingsTarget("Autostart", "com.vivo.permissionmanager", "com.vivo.permissionmanager.activity.BgStartUpManagerActivity"),
                SettingsTarget("Autostart", "com.iqoo.secure", "com.iqoo.secure.ui.phoneoptimize.AddWhiteListActivity"),
                SettingsTarget("Background power", "com.vivo.abe", "com.vivo.applicationbehaviorengine.ui.ExcessivePowerManagerActivity"),
            ),
        ),
        OemGuide(
            id = "huawei",
            title = "Huawei and Honor (EMUI, MagicOS)",
            manufacturers = setOf("huawei", "honor"),
            brands = setOf("huawei", "honor"),
            steps = listOf(
                "Open App launch, find Kin and switch it to “Manage manually”.",
                "Turn on Auto-launch, Secondary launch and Run in background for Kin.",
            ),
            targets = listOf(
                SettingsTarget("App launch", "com.huawei.systemmanager", "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity"),
                SettingsTarget("App launch", "com.huawei.systemmanager", "com.huawei.systemmanager.optimize.process.ProtectActivity"),
                SettingsTarget("App launch", "com.hihonor.systemmanager", "com.hihonor.systemmanager.startupmgr.ui.StartupNormalAppListActivity"),
            ),
        ),
    )

    /** The guide for a phone, or null when the phone's maker isn't in the table (the general battery step is all there is then). */
    fun select(manufacturer: String?, brand: String? = null): OemGuide? {
        val m = manufacturer?.trim()?.lowercase().orEmpty()
        val b = brand?.trim()?.lowercase().orEmpty()
        return guides.firstOrNull { m in it.manufacturers } ?: guides.firstOrNull { b.isNotEmpty() && b in it.brands }
    }

    /** What every phone gets: the battery optimisation list of Android itself. */
    val GENERAL_STEPS = listOf(
        "Open the battery optimisation list and set Kin to “Not optimised” (Unrestricted).",
    )
}
