package app.kinrelay.android.pairing

/** What the user is told when a pairing link or the pairing itself fails. */
object PairingMessages {
    const val LOCAL_NETWORK_DENIED =
        "Kin isn't allowed to reach devices on your network. Allow it in the app's permissions (Nearby devices), then try again."

    fun forParseError(error: PairingParse.Error): String = when (error.category) {
        PairingErrorCategory.NOT_PAIRING_URL -> "That's not a Kin Relay code."
        PairingErrorCategory.UNSUPPORTED_VERSION -> "Update the app: this code is from a newer version."
        PairingErrorCategory.MISSING_PARAM,
        PairingErrorCategory.INVALID_PARAM,
        PairingErrorCategory.DUPLICATE_PARAM -> "That code is damaged. Show a new one on your PC."
    }

    fun forFailure(failure: PairFailure): String = when (failure) {
        PairFailure.BAD_TOKEN -> "That code isn't valid. Show a new one on your PC."
        PairFailure.TOKEN_EXPIRED -> "That code has expired. Show a new one on your PC."
        PairFailure.TOKEN_USED -> "That code was already used. Show a new one on your PC."
        PairFailure.UNSUPPORTED_VERSION -> "Update the app on your phone or PC: the versions don't match."
        PairFailure.BAD_REQUEST, PairFailure.TOO_LARGE, PairFailure.SERVER_ERROR ->
            "The PC refused the pairing. Try again with a new code."
        PairFailure.UNREACHABLE -> "Couldn't reach the PC. Are both on the same Wi-Fi?"
        PairFailure.PIN_MISMATCH -> "The device at that address isn't the PC in the code. Nothing was paired."
        PairFailure.TLS_REFUSED -> "The PC refused the secure connection."
    }
}
