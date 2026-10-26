package app.kinrelay.android.ui

/** The Save button under Settings > Device name: not there, a live "Save", or a disabled "Saved" until the name is edited again. */
enum class NameSaveState(val label: String?) { HIDDEN(null), SAVE("Save"), SAVED("Saved") }

/**
 * [typed] is the text in the field, [saved] the name that is stored, [justSaved] whether the button was tapped and the field not edited since.
 * Pure, so the three states are tested on the JVM.
 */
fun nameSaveState(typed: String, saved: String, justSaved: Boolean): NameSaveState = when {
    typed.trim() != saved -> NameSaveState.SAVE
    justSaved -> NameSaveState.SAVED
    else -> NameSaveState.HIDDEN
}
