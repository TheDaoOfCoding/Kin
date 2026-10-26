package app.kinrelay.android.notifmirror

import android.app.AppOpsManager
import app.kinrelay.android.links.LinkRules
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The picker's classification (design 4.3), the Android 15+ redaction matcher (4.9) and the restricted-settings logic (4.2). */
class CatalogAndRedactionTest {
    @Test
    fun messagingAppsAreMessaging() {
        for (pkg in listOf("com.whatsapp", "com.whatsapp.w4b", "org.telegram.messenger", "org.thoughtcrime.securesms", "com.facebook.orca", "com.google.android.apps.messaging", "com.discord", "com.Slack")) {
            assertEquals(pkg, AppClass.MESSAGING, AppCatalog.classify(pkg, null))
        }
    }

    @Test
    fun theSocialAppsAreMessagingToo() {
        for (pkg in listOf("com.instagram.android", "com.snapchat.android", "com.twitter.android")) {
            assertTrue(pkg, pkg in AppCatalog.MESSAGING)
            assertEquals(pkg, AppClass.MESSAGING, AppCatalog.classify(pkg, "Whatever"))
            assertFalse(pkg, AppCatalog.isFlagged(pkg, null))
        }
    }

    @Test
    fun theFlaggedTableAndTheWordsFlagAnApp() {
        for (pkg in listOf("com.google.android.apps.authenticator2", "com.azure.authenticator", "com.bitwarden.authenticator", "com.agilebits.onepassword", "proton.android.pass", "com.paypal.android.p2pmobile", "com.coinbase.android")) {
            assertEquals(pkg, AppClass.FLAGGED, AppCatalog.classify(pkg, "Whatever"))
        }
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.example.app", "My Bank"))
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.example.app", "Banco del Sur"))
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.example.app", "Crypto Wallet"))
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.example.app", "2FA Codes"))
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.mybank.mobile", "Banking"))
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.example.pay", null))
        assertEquals(AppClass.FLAGGED, AppCatalog.classify("com.example.password_vault", null))
    }

    @Test
    fun wholeWordsOnlyAndAMessagingAppIsNeverFlagged() {
        assertEquals("pay is not in payroll or paypal", AppClass.OTHER, AppCatalog.classify("com.example.app", "Payroll"))
        assertEquals(AppClass.OTHER, AppCatalog.classify("com.example.app", "Spaceship"))
        assertEquals(AppClass.OTHER, AppCatalog.classify("com.example.camera", "Camera"))
        assertEquals("a messaging app whose name says wallet", AppClass.MESSAGING, AppCatalog.classify("com.whatsapp", "WhatsApp Wallet Chat"))
    }

    @Test
    fun theFirstRunTicksOnlyInstalledMessagingAppsAndEverythingElseIsOff() {
        val candidates = listOf(
            PickerModel.Candidate("com.example.game", "Game"), PickerModel.Candidate("com.whatsapp", "WhatsApp"),
            PickerModel.Candidate("com.mybank.app", "My Bank"), PickerModel.Candidate("org.telegram.messenger", "Telegram"),
        )
        val rows = PickerModel.build(candidates, emptySet(), MirrorPolicy(), firstRun = true)
        assertEquals(listOf("Telegram", "WhatsApp", "Game", "My Bank"), rows.map { it.label })
        assertEquals(setOf("com.whatsapp", "org.telegram.messenger"), rows.filter { it.allowed }.map { it.packageName }.toSet())
        assertEquals(AppClass.FLAGGED, rows.first { it.packageName == "com.mybank.app" }.appClass)
    }

    @Test
    fun afterTheFirstRunOnlyWhatTheUserTickedIsTickedAndTickedAppsWithoutALauncherAreListed() {
        val policy = MirrorPolicy(allowed = setOf("com.android.shell", "com.example.game"), hiddenAllowed = setOf("com.example.game"))
        val rows = PickerModel.build(listOf(PickerModel.Candidate("com.example.game", "Game"), PickerModel.Candidate("com.whatsapp", "WhatsApp")), emptySet(), policy, firstRun = false)
        assertEquals(setOf("com.android.shell", "com.example.game"), rows.filter { it.allowed }.map { it.packageName }.toSet())
        assertEquals("the label falls back to the package name", "com.android.shell", rows.first { it.packageName == "com.android.shell" }.label)
        assertTrue(rows.first { it.packageName == "com.example.game" }.hiddenAllowed)
        assertFalse(rows.first { it.packageName == "com.whatsapp" }.allowed)
    }

    // --- redaction ---

    @Test
    fun theSystemPlaceholderInAnyLocaleIsRedactedOnAndroid15Plus() {
        for (text in listOf("Sensitive notification content hidden", "  sensitive NOTIFICATION content hidden ", RedactedStrings.ALL.first(), RedactedStrings.ALL.last())) {
            assertTrue(text, RedactionMatcher.isRedacted(35, listOf(null, text)))
            assertTrue(text, RedactionMatcher.isRedacted(37, listOf(text)))
        }
        assertTrue("many locales are in the table", RedactedStrings.ALL.size >= 80)
        for (text in RedactedStrings.ALL) assertTrue(text, RedactionMatcher.isPlaceholder(text))
    }

    @Test
    fun beforeAndroid15NothingIsRedacted() {
        assertFalse(RedactionMatcher.isRedacted(34, listOf("Sensitive notification content hidden")))
        assertFalse(RedactionMatcher.codesHidden(34))
        assertTrue(RedactionMatcher.codesHidden(35))
    }

    @Test
    fun lookAlikesAreNotRedacted() {
        for (text in listOf("Sensitive notification content", "Sensitive notification content hidden by me", "Your code is 482913", "", "hidden", "Sensitive content hidden")) {
            assertFalse(text, RedactionMatcher.isRedacted(37, listOf(text, null)))
        }
    }

    // --- restricted settings (design 4.2) ---

    @Test
    fun theInstallSourceGivesAHintOnlyOnAndroid13Plus() {
        assertTrue(RestrictedSettings.likelyRestricted(33, null))
        assertTrue(RestrictedSettings.likelyRestricted(34, "com.google.android.packageinstaller"))
        assertTrue(RestrictedSettings.likelyRestricted(37, "com.android.chrome"))
        assertFalse("a store", RestrictedSettings.likelyRestricted(34, "com.android.vending"))
        assertFalse("older Android", RestrictedSettings.likelyRestricted(32, null))
        assertFalse("an installer nobody knows is not assumed to restrict", RestrictedSettings.likelyRestricted(34, "org.example.installer"))
    }

    @Test
    fun theAppOpsProbeMapsItsModesAndAnythingElseIsUnknown() {
        assertEquals(RestrictedSettings.Probe.ALLOWED, RestrictedSettings.probeOf(AppOpsManager.MODE_ALLOWED))
        assertEquals(RestrictedSettings.Probe.RESTRICTED, RestrictedSettings.probeOf(AppOpsManager.MODE_IGNORED))
        assertEquals(RestrictedSettings.Probe.RESTRICTED, RestrictedSettings.probeOf(AppOpsManager.MODE_ERRORED))
        assertEquals(RestrictedSettings.Probe.UNKNOWN, RestrictedSettings.probeOf(AppOpsManager.MODE_DEFAULT))
        assertEquals(RestrictedSettings.Probe.UNKNOWN, RestrictedSettings.probeOf(null))
    }

    @Test
    fun theStepsCardHasFourSteps() {
        assertEquals(4, RestrictedSettings.STEPS.size)
        assertTrue(RestrictedSettings.STEPS[0].contains("Restricted setting"))
        assertTrue(RestrictedSettings.STEPS[2].contains("Allow restricted settings"))
    }

    // --- the 2B.1 review cleanups ---

    @Test
    fun anUpperCaseSchemeIsLowerCasedInTheIntentOnly() {
        assertEquals("https://Example.com/Path?Q=1#F", LinkRules.intentUrl("HTTPS://Example.com/Path?Q=1#F"))
        assertEquals("http://EXAMPLE.com/", LinkRules.intentUrl("Http://EXAMPLE.com/"))
        assertEquals("https://example.com/", LinkRules.intentUrl("https://example.com/"))
        assertEquals("the URL checked and shown is untouched", "HTTPS://example.com/", LinkRules.acceptUrl("HTTPS://example.com/"))
        assertEquals("only the scheme: a path with :// stays", "https://example.com/redirect?to=HTTP://x.example", LinkRules.intentUrl("HTTPS://example.com/redirect?to=HTTP://x.example"))
    }
}
