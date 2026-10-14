package app.kinrelay.android.identity

import java.io.ByteArrayInputStream
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.PrivateKey
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.ECGenParameterSpec
import java.security.spec.PKCS8EncodedKeySpec
import java.time.Duration
import java.time.Instant

/** This install's identity: a P-256 key and its self-signed certificate (D15). */
class Identity(val privateKey: PrivateKey, val certificateDer: ByteArray) {
    val certificate: X509Certificate by lazy {
        CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(certificateDer)) as X509Certificate
    }
    val spki: ByteArray = CertificateInfo.spkiOf(certificateDer)
    val deviceId: String = DeviceIds.deviceId(spki)
    val shortId: String = DeviceIds.shortId(spki)

    companion object {
        fun generate(now: Instant = Instant.now()): Identity {
            val keys = KeyPairGenerator.getInstance("EC").run {
                initialize(ECGenParameterSpec("secp256r1"))
                generateKeyPair()
            }
            check(keys.private.encoded != null) { "the private key must be exportable so it can be stored encrypted" }
            val der = CertificateBuilder.build(
                publicKey = keys.public,
                privateKey = keys.private,
                notBefore = now.minus(Duration.ofDays(1)),
                notAfter = now.plus(Duration.ofDays(365L * 100)),
            )
            return Identity(keys.private, der)
        }

        fun privateKeyFromPkcs8(pkcs8: ByteArray): PrivateKey =
            KeyFactory.getInstance("EC").generatePrivate(PKCS8EncodedKeySpec(pkcs8))
    }
}
