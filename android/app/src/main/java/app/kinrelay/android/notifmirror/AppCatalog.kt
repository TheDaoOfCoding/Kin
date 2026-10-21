package app.kinrelay.android.notifmirror

import java.util.Locale

/** How the picker treats an app (design 4.3). */
enum class AppClass { MESSAGING, FLAGGED, OTHER }

/**
 * The bundled tables and rules that sort apps for the picker. Pure, so it is unit-tested. `ApplicationInfo.category` has no finance value,
 * so banking can only be flagged by the heuristic below; the real safety is that an unknown app is **off** by default. The heuristic only
 * adds a warning and the forced `sensitive` form.
 */
object AppCatalog {
    /** Messaging apps: pre-selected in the picker when installed. */
    val MESSAGING: Set<String> = setOf(
        "com.whatsapp", "com.whatsapp.w4b",
        "org.telegram.messenger", "org.telegram.messenger.web", "org.thunderdog.challegram",
        "org.thoughtcrime.securesms",
        "com.facebook.orca", "com.facebook.mlite",
        "com.google.android.apps.messaging", "com.samsung.android.messaging",
        "com.viber.voip", "jp.naver.line.android", "com.tencent.mm", "com.discord",
        "com.Slack", "com.microsoft.teams", "com.skype.raider", "com.google.android.apps.dynamite",
    )

    /** Authenticators, password managers, wallets and payments: off, with a warning, and always mirrored without a preview. */
    val FLAGGED: Set<String> = setOf(
        "com.google.android.apps.authenticator2", "com.azure.authenticator", "com.authy.authy", "com.twofasapp",
        "com.beemdevelopment.aegis", "org.fedorahosted.freeotp", "com.duosecurity.duomobile",
        "com.x8bit.bitwarden", "com.bitwarden.authenticator", "com.agilebits.onepassword", "com.onepassword.android",
        "com.lastpass.lpandroid", "com.dashlane",
        "keepass2android.keepass2android", "keepass2android.keepass2android_nonet", "com.kunzisoft.keepass.free",
        "com.kunzisoft.keepass.libre", "org.keepassxc", "proton.android.pass",
        "com.google.android.apps.walletnfcrel", "com.paypal.android.p2pmobile", "com.venmo", "com.coinbase.android",
    )

    /** Whole words of an app label or a package segment that make an app "likely sensitive". */
    val FLAG_WORDS: Set<String> = setOf(
        "bank", "banking", "banco", "banque", "credit", "wallet", "vault", "otp", "authenticator", "2fa", "password", "passwd",
        "keepass", "crypto", "invest", "trading", "finance", "pay",
    )

    fun classify(packageName: String, label: String?): AppClass = when {
        packageName in MESSAGING -> AppClass.MESSAGING
        isFlagged(packageName, label) -> AppClass.FLAGGED
        else -> AppClass.OTHER
    }

    fun isFlagged(packageName: String, label: String?): Boolean {
        if (packageName in FLAGGED) return true
        if (packageName in MESSAGING) return false
        val words = buildList {
            label?.let { addAll(words(it)) }
            packageName.split('.').forEach { addAll(words(it)) }
        }
        return words.any { it in FLAG_WORDS }
    }

    /** Lower-case words: letter and digit runs ("2FA Authenticator" -> "2fa", "authenticator"). */
    fun words(text: String): List<String> =
        Regex("[\\p{L}\\p{N}]+").findAll(text.lowercase(Locale.ROOT)).map { it.value }.toList()
}

/**
 * The picker's rows and which are ticked first. [launchable] are the apps the system shows with a launcher icon (the `<queries>` entry);
 * [posted] are the packages that posted a notification while access was on (their label falls back to the package name); [allowed] are the
 * apps already ticked. Nothing is ticked that the user did not tick, except messaging apps on the **first** run ([firstRun]).
 */
data class PickerApp(val packageName: String, val label: String, val appClass: AppClass, val allowed: Boolean, val hiddenAllowed: Boolean)

object PickerModel {
    data class Candidate(val packageName: String, val label: String?)

    fun build(
        launchable: List<Candidate>,
        posted: Set<String>,
        policy: MirrorPolicy,
        firstRun: Boolean,
    ): List<PickerApp> {
        val byPackage = LinkedHashMap<String, Candidate>()
        launchable.forEach { byPackage.putIfAbsent(it.packageName, it) }
        (posted + policy.allowed).forEach { byPackage.putIfAbsent(it, Candidate(it, null)) }
        return byPackage.values
            .map { c ->
                val label = c.label?.takeIf { it.isNotBlank() } ?: c.packageName
                val appClass = AppCatalog.classify(c.packageName, c.label)
                val allowed = c.packageName in policy.allowed || (firstRun && appClass == AppClass.MESSAGING)
                PickerApp(c.packageName, label, appClass, allowed, c.packageName in policy.hiddenAllowed)
            }
            .sortedWith(compareBy<PickerApp>({ it.appClass != AppClass.MESSAGING }, { it.label.lowercase(Locale.ROOT) }))
    }
}
