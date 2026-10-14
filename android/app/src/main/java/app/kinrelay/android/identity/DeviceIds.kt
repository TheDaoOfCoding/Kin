package app.kinrelay.android.identity

import java.security.MessageDigest
import java.util.Base64

/** Device IDs, short IDs and the base64url rules of protocol.md, "Encodings". */
object DeviceIds {
    const val DEVICE_ID_LENGTH = 43
    const val TOKEN_LENGTH = 22
    private const val BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"

    fun base64Url(bytes: ByteArray): String = Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)

    /** Length and alphabet only, as the spec's validity check. */
    fun isBase64Url(value: String, length: Int): Boolean =
        value.length == length && value.all { it in 'A'..'Z' || it in 'a'..'z' || it in '0'..'9' || it == '-' || it == '_' }

    fun isDeviceId(value: String) = isBase64Url(value, DEVICE_ID_LENGTH)

    fun isToken(value: String) = isBase64Url(value, TOKEN_LENGTH)

    private fun sha256(bytes: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(bytes)

    /** base64url(SHA-256(SPKI)). */
    fun deviceId(spki: ByteArray): String = deviceIdOfHash(sha256(spki))

    fun deviceIdOfHash(hash: ByteArray): String = base64Url(hash)

    /** First 8 characters of the RFC 4648 base32 of the SHA-256 of the SPKI. */
    fun shortId(spki: ByteArray): String = shortIdOfHash(sha256(spki))

    fun shortIdOfHash(hash: ByteArray): String {
        var bits = 0L
        for (i in 0 until 5) bits = (bits shl 8) or (hash[i].toLong() and 0xFF)
        return buildString {
            for (i in 7 downTo 0) append(BASE32_ALPHABET[((bits shr (i * 5)) and 0x1F).toInt()])
        }
    }

    /** The short ID of a device ID, which is the base64url of the same hash. Null if it isn't a valid device ID. */
    fun shortIdOfDeviceId(deviceId: String): String? =
        if (isDeviceId(deviceId)) shortIdOfHash(Base64.getUrlDecoder().decode(deviceId)) else null

    /** `XXXX-XXXX`, for display only. */
    fun display(shortId: String): String = shortId.chunked(4).joinToString("-")

    fun deviceIdOfCertificate(certificateDer: ByteArray): String = deviceId(CertificateInfo.spkiOf(certificateDer))
}
