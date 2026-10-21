package app.kinrelay.android.features

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The per-feature settings resolver (design section 4, 10.1): an override per peer beats the default, the store is empty and survives a round trip, unlinking a peer deletes its overrides, and
 * grep-style tests that **every new switch is read through the resolver** (no feature code reads its flat key directly), that no code of 2B.4 logs a level or a path, and that the photo
 * permission is declared and asked for only as the design says. (The shared `features[]` vectors are in `PresenceVectorsTest`.)
 */
class FeaturesTest {
    @Test
    fun anOverrideForThePeerBeatsTheDefaultAndNothingElseDoes() {
        val o = FeatureOverrides().with(FeatureIds.PRESENCE_SHARE, "dev-a", false)
        assertFalse(FeatureResolver.enabled(o, FeatureIds.PRESENCE_SHARE, "dev-a", default = true))
        assertTrue("another peer", FeatureResolver.enabled(o, FeatureIds.PRESENCE_SHARE, "dev-b", default = true))
        assertTrue("no peer in particular", FeatureResolver.enabled(o, FeatureIds.PRESENCE_SHARE, null, default = true))
        assertTrue("another feature", FeatureResolver.enabled(o, FeatureIds.RING_ALLOW, "dev-a", default = true))
        assertTrue(FeatureResolver.enabled(FeatureOverrides().with(FeatureIds.SHOTS_CAMERA, "dev-a", true), FeatureIds.SHOTS_CAMERA, "dev-a", default = false))
    }

    @Test
    fun theStoreIsEmptyByDefaultAndSurvivesAJsonRoundTrip() {
        assertEquals(emptyMap<String, Map<String, Boolean>>(), FeatureOverrides.parse(null).values)
        assertEquals(emptyMap<String, Map<String, Boolean>>(), FeatureOverrides.parse("").values)
        assertEquals(emptyMap<String, Map<String, Boolean>>(), FeatureOverrides.parse("not json").values)
        val o = FeatureOverrides().with(FeatureIds.MEDIA_SHOW, "dev-a", true).with(FeatureIds.RING_ALLOW, "dev-a", false).with(FeatureIds.MEDIA_SHOW, "dev-b", false)
        assertEquals(o, FeatureOverrides.parse(o.toJson()))
        assertEquals("""{"dev-a":{"media.show":true,"ring.allow":false},"dev-b":{"media.show":false}}""", o.toJson())
        assertEquals("{}", FeatureOverrides().toJson())
    }

    @Test
    fun unlinkingAPeerDeletesItsOverridesAndOnlyIts() {
        val o = FeatureOverrides().with(FeatureIds.MEDIA_SHOW, "dev-a", true).with(FeatureIds.MEDIA_SHOW, "dev-b", false)
        val after = o.withoutPeer("dev-a")
        assertNull(after.forPeer(FeatureIds.MEDIA_SHOW, "dev-a"))
        assertEquals(false, after.forPeer(FeatureIds.MEDIA_SHOW, "dev-b"))
        assertEquals(FeatureOverrides(), after.withoutPeer("dev-b"))
    }

    @Test
    fun aValueThatIsNotABooleanIsIgnored() {
        val o = FeatureOverrides.parse("""{"dev-a":{"notif.show":"no","media.show":1,"ring.allow":false}}""")
        assertNull(o.forPeer("notif.show", "dev-a"))
        assertNull(o.forPeer("media.show", "dev-a"))
        assertEquals(false, o.forPeer("ring.allow", "dev-a"))
    }

    // ------------------------------------------------------------------ grep-style tests over the source

    private val main = File("src/main/java/app/kinrelay/android")

    private fun sources(): List<File> = main.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()

    @Test
    fun theFlatKeysOfTheNewSwitchesAreNamedOnlyByTheirStores() {
        assertTrue("the source tree is found from ${File(".").absolutePath}", sources().size > 100)
        val stores = setOf("PresenceStore.kt", "ShotsStore.kt", "DndStore.kt")
        val keys = listOf(
            "presence_share_battery", "presence_show_pc", "ring_allow", "ring_loud", "ring_saved_alarm_index", "shots_screenshots", "shots_camera", "shots_baseline",
            // 2B.5
            "dnd_follow", "dnd_share",
        )
        for (file in sources()) {
            if (file.name in stores) continue
            for (key in keys) assertFalse("${file.name} names the flat key $key", file.readText().contains("\"$key\""))
        }
        for (store in stores) {
            val text = sources().first { it.name == store }.readText()
            assertTrue(store, keys.any { text.contains("\"$it\"") })
        }
    }

    @Test
    fun everyNewSwitchIsReadThroughTheResolver() {
        // The flow of a switch's default is read directly only to wire the resolver's defaults (AppGraph) and by the stores themselves.
        val direct = Regex("""presenceStore\.(share|show|ringAllow)\b|shotsStore\.(screenshots|camera)\b|dndStore\.(follow|share)\b""")
        val offenders = sources().filter { it.name != "AppGraph.kt" && direct.containsMatchIn(it.readText()) }.map { it.name }
        assertEquals("these read a new switch's default directly instead of through Features: $offenders", emptyList<String>(), offenders)
        // And the consumers do go through it.
        val text = sources().associate { it.name to it.readText() }
        assertTrue(text.getValue("PresenceHub.kt").contains("features.flow(FeatureIds.PRESENCE_SHARE)"))
        assertTrue(text.getValue("PresenceHub.kt").contains("features.flow(FeatureIds.PRESENCE_SHOW)"))
        assertTrue(text.getValue("RingController.kt").contains("FeatureIds.RING_ALLOW"))
        assertTrue(text.getValue("ShotController.kt").contains("features.flow(FeatureIds.SHOTS_SCREENSHOTS)"))
        assertTrue(text.getValue("ShotController.kt").contains("features.flow(FeatureIds.SHOTS_CAMERA)"))
        assertTrue(text.getValue("PresenceScreens.kt").contains("features.flow(FeatureIds.RING_ALLOW)"))
        // 2B.5: both Do Not Disturb switches go through it too.
        assertTrue(text.getValue("DndController.kt").contains("features.flow(FeatureIds.DND_FOLLOW)"))
        assertTrue(text.getValue("DndController.kt").contains("features.flow(FeatureIds.DND_SHARE)"))
        assertTrue(text.getValue("DndController.kt").contains("features.enabled(FeatureIds.DND_FOLLOW)"))
    }

    @Test
    fun noCodeOf2B5LogsAnAlertTextATitleOrAFocusOrDndHistory() {
        // Design 8, T1: counts, lengths and enums only. A log line may name the length of an alert's text or title, never the text or title themselves.
        val risky = Regex("""\blog\.[iwe]\(.*(alert\.text(?!\.length)|alert\.title(?!\?\.length)|\.text\b(?!\.length)|Focus history|dndHistory|focusHistory)""")
        val folders = setOf("dnd", "alerts")
        val offenders = sources().filter { it.parentFile?.name in folders }.flatMap { file ->
            file.readLines().withIndex().filter { risky.containsMatchIn(it.value) }.map { "${file.name}:${it.index + 1}: ${it.value.trim()}" }
        }
        assertEquals("a log line names something it must not: $offenders", emptyList<String>(), offenders)
        // The 2B.5 stores keep no history: the only keys are the switches, the "How much" setting and the hold record.
        val store = sources().first { it.name == "DndStore.kt" }.readText()
        val keys = Regex("""PreferencesKey\("([a-z_]+)"\)""").findAll(store).map { it.groupValues[1] }.toSet()
        assertEquals(setOf("dnd_follow", "dnd_share", "dnd_follow_filter", "we_hold", "held_filter", "held_route", "release_owed"), keys)
    }

    @Test
    fun theDoNotDisturbAccessIsDeclaredAsAPermissionThatOnlyNamesTheSpecialAccessAndNeverAsksAtStart() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        assertTrue(manifest.contains("android.permission.ACCESS_NOTIFICATION_POLICY"))
        // The access screen is opened only by the Settings card that turns the feature on, never at start-up and never by the setup flow.
        val openers = sources().filter { it.readText().contains("ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS") }.map { it.name }.toSet()
        assertEquals(setOf("DndScreens.kt"), openers)
        // The narrowest access: nothing in 2B.5 asks for notification access (the listener's access screen).
        assertFalse(sources().filter { it.parentFile?.name in setOf("dnd", "alerts") }.any { it.readText().contains("ACTION_NOTIFICATION_LISTENER_SETTINGS") })
    }

    @Test
    fun noCodeOf2B4LogsALevelAPathOrAFileName() {
        val risky = Regex("""\blog\.[iwe]\(.*(\.level|battery\b|relativePath|displayName|\.path\b|pausedUntil|untilMillis)""")
        val folders = setOf("presence", "ring", "shots", "pause", "features")
        val offenders = sources().filter { it.parentFile?.name in folders }.flatMap { file ->
            file.readLines().withIndex().filter { risky.containsMatchIn(it.value) }.map { "${file.name}:${it.index + 1}" }
        }
        assertEquals("a log line names something it must not: $offenders", emptyList<String>(), offenders)
    }

    @Test
    fun thePhotoPermissionIsDeclaredAsTheDesignSaysAndAskedForOnlyWhenTheSwitchIsTurnedOn() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        assertTrue(manifest.contains("android.permission.READ_MEDIA_IMAGES"))
        assertTrue(manifest.contains("""android:name="android.permission.READ_EXTERNAL_STORAGE" android:maxSdkVersion="32""""))
        assertTrue(manifest.contains("android.permission.USE_FULL_SCREEN_INTENT"))
        assertTrue(manifest.contains("android.permission.VIBRATE"))
        for (never in listOf("READ_MEDIA_VISUAL_USER_SELECTED", "ACCESS_MEDIA_LOCATION", "READ_MEDIA_VIDEO", "MANAGE_EXTERNAL_STORAGE", "FOREGROUND_SERVICE_MEDIA_PLAYBACK")) {
            assertFalse("the design says never $never", Regex("""<uses-permission[^>]*$never""").containsMatchIn(manifest))
        }
        // Never at start-up: the only request for the photo permission is the Settings card that turns the switch on.
        val askers = sources().filter { it.readText().contains("permissionToRequest()") }.map { it.name }.toSet()
        assertEquals(setOf("PresenceScreens.kt", "ShotController.kt"), askers)
        assertFalse(File("src/main/java/app/kinrelay/android/MainActivity.kt").readText().contains("READ_MEDIA_IMAGES"))
        val permissions = File("src/main/java/app/kinrelay/android/Permissions.kt").readText()
        assertFalse("not in the list of what is asked for at start", permissions.substringAfter("fun missing").substringBefore("/** What the setup screen shows").contains("photos"))
    }
}
