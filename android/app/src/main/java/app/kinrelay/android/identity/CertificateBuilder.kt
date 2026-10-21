package app.kinrelay.android.identity

import java.security.PrivateKey
import java.security.PublicKey
import java.security.SecureRandom
import java.security.Signature
import java.time.Instant
import java.time.ZoneOffset
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter

/**
 * Writes the self-signed certificate of protocol.md, "Certificate profile", by hand:
 * X.509 v3, ECDSA P-256, `ecdsa-with-SHA256` with no parameters, a positive serial of at most
 * 20 bytes, RFC 5280 times and no extensions. The layout is the `minimal` certificate of
 * `protocol/device-id.json`.
 */
object CertificateBuilder {
    private const val SUBJECT_COMMON_NAME = "Kin"
    private const val OID_COMMON_NAME = "2.5.4.3"
    private val UTC_TIME_FORMAT = DateTimeFormatter.ofPattern("yyMMddHHmmss'Z'")
    private val GENERALIZED_TIME_FORMAT = DateTimeFormatter.ofPattern("yyyyMMddHHmmss'Z'")
    private val GENERALIZED_TIME_FROM = ZonedDateTime.of(2050, 1, 1, 0, 0, 0, 0, ZoneOffset.UTC).toInstant()

    fun build(
        publicKey: PublicKey,
        privateKey: PrivateKey,
        notBefore: Instant,
        notAfter: Instant,
        random: SecureRandom = SecureRandom(),
    ): ByteArray {
        val spki = publicKey.encoded
        require(isP256Spki(spki)) { "not a P-256 SubjectPublicKeyInfo" }

        val algorithm = Der.sequence(Der.oid(CertificateInfo.ECDSA_WITH_SHA256))
        val name = Der.sequence(Der.set(Der.sequence(Der.oid(OID_COMMON_NAME), Der.utf8String(SUBJECT_COMMON_NAME))))
        val tbs = Der.sequence(
            Der.explicit0(Der.smallInteger(2)),
            Der.positiveInteger(newSerial(random)),
            algorithm,
            name,
            Der.sequence(time(notBefore), time(notAfter)),
            name,
            spki,
        )
        val signature = Signature.getInstance("SHA256withECDSA").run {
            initSign(privateKey)
            update(tbs)
            sign()
        }
        return Der.sequence(tbs, algorithm, Der.bitString(signature))
    }

    /** 20 random bytes whose first byte is 1..127, so the INTEGER is positive and stays 20 bytes. */
    fun newSerial(random: SecureRandom): ByteArray {
        val serial = ByteArray(20)
        random.nextBytes(serial)
        serial[0] = ((serial[0].toInt() and 0x7F).takeIf { it != 0 } ?: 1).toByte()
        return serial
    }

    /** UTCTime up to 2049, GeneralizedTime from 2050 (RFC 5280, 4.1.2.5). */
    fun time(instant: Instant): ByteArray {
        val utc = instant.atZone(ZoneOffset.UTC)
        return if (instant < GENERALIZED_TIME_FROM) {
            Der.tlv(Der.TAG_UTC_TIME, UTC_TIME_FORMAT.format(utc).toByteArray(Charsets.US_ASCII))
        } else {
            Der.tlv(Der.TAG_GENERALIZED_TIME, GENERALIZED_TIME_FORMAT.format(utc).toByteArray(Charsets.US_ASCII))
        }
    }

    private fun isP256Spki(spki: ByteArray): Boolean {
        if (spki.size != 91) return false
        val info = runCatching { CertificateInfo.parseSpki(spki) }.getOrNull() ?: return false
        return info.first == CertificateInfo.EC_PUBLIC_KEY && info.second == CertificateInfo.PRIME256V1
    }
}
