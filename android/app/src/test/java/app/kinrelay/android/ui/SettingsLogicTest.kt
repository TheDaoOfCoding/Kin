package app.kinrelay.android.ui

import app.kinrelay.android.pause.PauseWords
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pure parts of the tidied Home and Settings: the Save button, the Pause row's words, the mirror note, and that no setting reads "On: ... Off: ...". */
class SettingsLogicTest {
    @Test
    fun theSaveButtonIsHiddenUntilTheNameDiffersThenSavedUntilTheNextEdit() {
        assertEquals(NameSaveState.HIDDEN, nameSaveState("Pixel", "Pixel", justSaved = false))
        assertEquals(NameSaveState.SAVE, nameSaveState("Pixel 9", "Pixel", justSaved = false))
        // The text is trimmed before it is compared, as the save does.
        assertEquals(NameSaveState.HIDDEN, nameSaveState("  Pixel  ", "Pixel", justSaved = false))
        assertEquals(NameSaveState.SAVED, nameSaveState("Pixel 9", "Pixel 9", justSaved = true))
        assertEquals(NameSaveState.SAVE, nameSaveState("Pixel 9b", "Pixel 9", justSaved = false))
        // Editing and then putting the old name back is not a save: nothing to show.
        assertEquals(NameSaveState.HIDDEN, nameSaveState("Pixel 9", "Pixel 9", justSaved = false))
        assertEquals("Save", NameSaveState.SAVE.label)
        assertEquals("Saved", NameSaveState.SAVED.label)
        assertNull(NameSaveState.HIDDEN.label)
    }

    @Test
    fun thePausedRowReadsPausedUntil() {
        assertEquals("Paused until 18:30", PauseWords.pausedLine("Until 18:30"))
        assertEquals("Paused until tomorrow 08:00", PauseWords.pausedLine("Until tomorrow 08:00"))
        assertEquals("Paused until resumed", PauseWords.pausedLine("Until resumed"))
    }

    @Test
    fun theMirrorSwitchSaysSomethingOnlyForAProblemOrAWait() {
        val on = app.kinrelay.android.notifmirror.MirrorPolicy(enabled = true, allowed = setOf("com.whatsapp"))
        assertNull(mirrorNote(app.kinrelay.android.notifmirror.MirrorPolicy(), access = true, pcAccepts = true))
        assertNull(mirrorNote(on, access = true, pcAccepts = true))
        assertEquals("Paused: Kin doesn't have notification access", mirrorNote(on, access = false, pcAccepts = true))
        assertEquals("On, but no app is ticked", mirrorNote(on.copy(allowed = emptySet()), access = true, pcAccepts = true))
        assertTrue(mirrorNote(on, access = true, pcAccepts = false)!!.contains("0.2.0"))
    }

    @Test
    fun noSettingExplainsItselfAsOnColonOffColon() {
        val dir = java.io.File("src/main/java/app/kinrelay/android")
        val files = listOf("ui/SettingsScreen.kt", "ui/MediaScreens.kt", "ui/MirrorScreens.kt", "ui/PresenceScreens.kt", "ui/DndScreens.kt", "dnd/DndText.kt", "media/MediaUiText.kt")
        for (name in files) {
            for (line in java.io.File(dir, name).readLines()) {
                assertFalse("$name: $line", Regex("\"\\s*(On|Off): ").containsMatchIn(line))
                // A dash in a comment is fine; one in a string literal reaches the user.
                if (!line.trim().startsWith("//") && !line.trim().startsWith("*") && !line.trim().startsWith("/**")) {
                    assertFalse("$name: $line", Regex("\"[^\"]*\u2014[^\"]*\"").containsMatchIn(line))
                }
            }
        }
    }
}
