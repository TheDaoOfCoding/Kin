package app.kinrelay.android.shots

/**
 * The words of the 5-second hold notification (2B.5 review round 2, B2). The row can show collapsed in the shade (a truncated title and a chevron), so the **whole notification is the cancel
 * button**: a tap on it cancels, and the line under the title says so. The Cancel action stays for the expanded row.
 */
object ShotHoldText {
    const val TAP_TO_CANCEL = "Tap to cancel"
    const val CANCEL_ACTION = "Cancel"

    fun title(kind: ShotKind, pc: String, seconds: Long): String {
        val what = if (kind == ShotKind.CAMERA) "photo" else "screenshot"
        return "Sending $what to $pc in $seconds s"
    }
}
