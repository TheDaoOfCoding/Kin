package app.kinrelay.android.pairing

import app.kinrelay.android.identity.DeviceIds

/** Where pairing text came from. This decides whether the user must confirm before anything happens. */
enum class PairingSource {
    /** The user tapped Scan or Pair inside Kin just now. That tap is the confirmation. */
    IN_APP,

    /** An `ACTION_VIEW` intent: any web page, message or app can fire one. It never pairs by itself. */
    EXTERNAL_LINK,
}

/** What the user is asked to confirm. [pcName] and [shortId] come from the link, so they only tell the user what to compare. */
data class PairingConfirmation(
    val target: PairingTarget,
    val pcName: String,
    /** `XXXX-XXXX`, computed from the `id` in the link (the same ID the PC shows). */
    val shortId: String,
    /** The name of a different PC that is paired now and would be replaced, or null. */
    val replacing: String?,
    /** The PC in the link is the one that is paired already: pairing again only refreshes it. */
    val alreadyPaired: Boolean,
)

/** What to do with a piece of pairing text. */
sealed interface PairingIntake {
    /** Not a usable link. [message] is for the user. */
    data class Reject(val message: String, val logLine: String) : PairingIntake

    /** Show the confirmation screen first. Nothing is contacted and nothing is changed until the user confirms. */
    data class AskUser(val confirmation: PairingConfirmation) : PairingIntake

    /** Start now. */
    data class PairNow(val target: PairingTarget) : PairingIntake

    companion object {
        const val UNKNOWN_PC_NAME = "Unknown PC"

        /**
         * The rule: a link that arrives by intent from outside the app opens the confirmation screen, whatever
         * it contains. Text the user scanned or pasted in the app starts pairing directly ([PairingSource.IN_APP]).
         * [autoConfirm] is the debug build's hook for the integration tests; release builds always pass false.
         * [current] is the PC that is paired now: its device ID and name.
         */
        fun decide(
            parsed: PairingParse,
            source: PairingSource,
            current: Pair<String, String>?,
            autoConfirm: Boolean = false,
        ): PairingIntake = when (parsed) {
            is PairingParse.Error -> Reject(
                PairingMessages.forParseError(parsed), "link rejected: ${parsed.category} ${parsed.param.orEmpty()}",
            )
            is PairingParse.Ok -> {
                val target = parsed.target
                if (source == PairingSource.IN_APP || autoConfirm) {
                    PairNow(target)
                } else {
                    val same = current?.first == target.id
                    AskUser(
                        PairingConfirmation(
                            target = target,
                            pcName = target.name ?: UNKNOWN_PC_NAME,
                            shortId = DeviceIds.shortIdOfDeviceId(target.id)?.let(DeviceIds::display) ?: "?",
                            replacing = if (current != null && !same) current.second else null,
                            alreadyPaired = same,
                        )
                    )
                }
            }
        }
    }
}
