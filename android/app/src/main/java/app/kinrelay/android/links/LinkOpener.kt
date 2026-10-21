package app.kinrelay.android.links

import android.annotation.SuppressLint
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.os.Build
import androidx.core.net.toUri

/** An app (not a browser) that handles a link, as the open plan sees it. [isDefault]: it is the one the system would pick for the URL. */
data class HandlerApp(val packageName: String, val label: String, val isDefault: Boolean = false)

/** One way of starting a link, tried in order until one works. */
sealed interface OpenAttempt {
    /** `ACTION_VIEW` with `FLAG_ACTIVITY_REQUIRE_NON_BROWSER` (Android 11+): the system starts a non-browser handler or throws. */
    data object RequireNonBrowser : OpenAttempt

    /** `ACTION_VIEW` limited to one package: the Android 10 way to open "the app", found by [LinkHandlers]. */
    data class InPackage(val packageName: String) : OpenAttempt

    /** Plain `ACTION_VIEW`: whatever the user has as the default (a browser, if no app owns the link). */
    data object Default : OpenAttempt
}

/** Which apps besides browsers can open a URL. The real one asks the package manager; the tests give a list. */
fun interface LinkHandlers {
    /** Non-browser apps that handle [url], the system's pick first, then by priority. Empty when only browsers (or nothing) do. */
    fun nonBrowserApps(url: String): List<HandlerApp>
}

/**
 * What a tap on a link notification does (milestone 2B.1), as pure rules:
 * - **Android 11 and later:** try `ACTION_VIEW` with `FLAG_ACTIVITY_REQUIRE_NON_BROWSER` first (the system picks an app that owns the link,
 *   YouTube for a YouTube link) and fall back to the default browser when none does. No app has to be found by us.
 * - **Android 10:** that flag doesn't exist, so a resolve check finds the non-browser apps ([LinkHandlers]); the first is opened by package,
 *   and with none the default browser gets it.
 * The label "Open in <App>" is the same on every version: the app of the first non-browser handler **that Kin can see** (the manifest's `<queries>` list
 * the browsers and the well-known sites: Android hides an app with a host in its filter from a query that names no host). With none it is the neutral
 * "Open link", because a tap may still open an app the system knows and Kin can't see; it never claims "browser".
 */
object LinkOpenPlan {
    fun attempts(sdkInt: Int, apps: List<HandlerApp>): List<OpenAttempt> = when {
        sdkInt >= Build.VERSION_CODES.R -> listOf(OpenAttempt.RequireNonBrowser, OpenAttempt.Default)
        apps.isNotEmpty() -> listOf(OpenAttempt.InPackage(apps.first().packageName), OpenAttempt.Default)
        else -> listOf(OpenAttempt.Default)
    }

    /** The app a tap opens, or null when it goes to the browser. */
    fun app(apps: List<HandlerApp>): HandlerApp? = apps.firstOrNull()

    /** The text that says what a tap does: "Open in YouTube" when a non-browser app handles the link, else the neutral "Open link". */
    fun label(apps: List<HandlerApp>): String = app(apps)?.let { "Open in ${it.label}" } ?: OPEN_LINK

    const val OPEN_LINK = "Open link"
}

/** Starts one attempt. True if the system accepted it, false if nothing could handle it. */
fun interface LinkLauncher {
    fun launch(url: String, attempt: OpenAttempt): Boolean
}

/** Opens a link by the plan: tries each attempt in order and stops at the first that starts. */
class LinkOpener(private val handlers: LinkHandlers, private val launcher: LinkLauncher, private val sdkInt: Int = Build.VERSION.SDK_INT) {
    /** True if some attempt started. A URL that isn't `http` or `https` is never started. */
    fun open(url: String): Boolean {
        val safe = LinkRules.acceptUrl(url) ?: return false
        val apps = if (sdkInt >= Build.VERSION_CODES.R) emptyList() else handlers.nonBrowserApps(safe)
        return LinkOpenPlan.attempts(sdkInt, apps).any { launcher.launch(safe, it) }
    }
}

/** The package manager's side of [LinkHandlers] and [LinkLauncher]. Needs `<queries>` for `ACTION_VIEW` of `https` (and `http`) in the manifest. */
class AndroidLinks(context: Context) : LinkHandlers, LinkLauncher {
    private val context = context.applicationContext
    private val packages: PackageManager = this.context.packageManager

    override fun nonBrowserApps(url: String): List<HandlerApp> {
        val handlers = resolve(view(url))
        if (handlers.isEmpty()) return emptyList()
        val browsers = resolve(view(BROWSER_PROBE)).map { it.activityInfo.packageName }.toSet()
        val chosen = runCatching { packages.resolveActivity(view(url), PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo?.packageName }.getOrNull()
        return handlers
            .filter { it.activityInfo.packageName !in browsers && it.activityInfo.packageName != context.packageName }
            .distinctBy { it.activityInfo.packageName }
            .map { HandlerApp(it.activityInfo.packageName, it.loadLabel(packages).toString(), isDefault = it.activityInfo.packageName == chosen) }
            .sortedByDescending { it.isDefault }
    }

    @SuppressLint("InlinedApi") // the flag is only ever added on Android 11+ (LinkOpenPlan.attempts)
    override fun launch(url: String, attempt: OpenAttempt): Boolean {
        val intent = view(url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        when (attempt) {
            OpenAttempt.RequireNonBrowser -> intent.addFlags(Intent.FLAG_ACTIVITY_REQUIRE_NON_BROWSER)
            is OpenAttempt.InPackage -> intent.setPackage(attempt.packageName)
            OpenAttempt.Default -> Unit
        }
        return try {
            context.startActivity(intent)
            true
        } catch (_: ActivityNotFoundException) {
            false
        } catch (_: SecurityException) {
            false
        }
    }

    private fun view(url: String) = Intent(Intent.ACTION_VIEW, LinkRules.intentUrl(url).toUri()).addCategory(Intent.CATEGORY_BROWSABLE)

    @Suppress("DEPRECATION")
    private fun resolve(intent: Intent): List<ResolveInfo> = runCatching {
        if (Build.VERSION.SDK_INT >= 33) {
            packages.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(PackageManager.MATCH_ALL.toLong()))
        } else {
            packages.queryIntentActivities(intent, PackageManager.MATCH_ALL)
        }
    }.getOrDefault(emptyList())

    companion object {
        /** An address with no host: only apps that take any web address (browsers) match it. */
        private const val BROWSER_PROBE = "http://"
    }
}
