package app.kinrelay.android.identity

import app.kinrelay.android.testing.CertificateProfile
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.str
import java.io.ByteArrayInputStream
import java.io.File
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.time.Instant
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CertificateBuilderTest {
    private val now = Instant.parse("2026-10-14T12:00:00Z")

    @Test
    fun generatedCertificateFollowsTheProfile() {
        val identity = Identity.generate(now)
        assertEquals(emptyList<String>(), CertificateProfile.violations(identity.certificateDer))
        val info = CertificateInfo.parse(identity.certificateDer)
        assertEquals("v3", 2, info.version)
        assertEquals(CertificateInfo.EC_PUBLIC_KEY, info.keyAlgorithmOid)
        assertEquals(CertificateInfo.PRIME256V1, info.keyCurveOid)
        assertEquals(91, info.spki.size)
        assertEquals(CertificateInfo.ECDSA_WITH_SHA256, info.innerSignatureAlgorithm.oid)
        assertTrue("no parameters inside the TBS", !info.innerSignatureAlgorithm.hasParameters)
        assertTrue("no parameters outside the TBS", !info.outerSignatureAlgorithm.hasParameters)
        assertTrue("serial is at most 20 bytes", info.serial.size in 1..20)
        assertTrue("serial is positive", info.serial[0] > 0)
        assertTrue("no extensions at all", info.extensions.isEmpty())
    }

    @Test
    fun javaParsesItAndTheSignatureVerifies() {
        val identity = Identity.generate(now)
        val cert = CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(identity.certificateDer)) as X509Certificate
        assertEquals(3, cert.version)
        assertEquals("1.2.840.10045.4.3.2", cert.sigAlgOID)
        cert.verify(cert.publicKey)
        assertEquals("EC", cert.publicKey.algorithm)
        assertTrue(cert.criticalExtensionOIDs.isNullOrEmpty())
    }

    @Test
    fun deviceIdIsTheHashOfTheSpki() {
        val identity = Identity.generate(now)
        val expected = MessageDigest.getInstance("SHA-256").digest(identity.spki)
        assertEquals(DeviceIds.base64Url(expected), identity.deviceId)
        assertEquals(43, identity.deviceId.length)
        assertEquals(8, identity.shortId.length)
        assertEquals(identity.deviceId, DeviceIds.deviceIdOfCertificate(identity.certificateDer))
    }

    @Test
    fun serialsArePositiveTwentyBytesAndDiffer() {
        val random = SecureRandom()
        val serials = (1..200).map { CertificateBuilder.newSerial(random) }
        serials.forEach {
            assertEquals(20, it.size)
            assertTrue(it[0] in 1..127)
        }
        assertEquals(200, serials.map { it.toList() }.toSet().size)
    }

    @Test
    fun timesSwitchFromUtcTimeToGeneralizedTimeIn2050() {
        fun text(instant: String) = CertificateBuilder.time(Instant.parse(instant)).let { it[0].toInt() to String(it, 2, it.size - 2) }
        assertEquals(0x17 to "261014120000Z", text("2026-10-14T12:00:00Z"))
        assertEquals(0x17 to "491231235959Z", text("2049-12-31T23:59:59Z"))
        assertEquals(0x18 to "20500101000000Z", text("2050-01-01T00:00:00Z"))
        assertEquals(0x18 to "21261014120000Z", text("2126-10-14T12:00:00Z"))
    }

    @Test
    fun derHelpersEncodeLikeX690() {
        assertArrayEquals(byteArrayOf(0x02, 0x01, 0x05), Der.smallInteger(5))
        assertArrayEquals(byteArrayOf(0x02, 0x02, 0x00, 0x80.toByte()), Der.positiveInteger(byteArrayOf(0x80.toByte())))
        assertArrayEquals(byteArrayOf(0x02, 0x01, 0x7F), Der.positiveInteger(byteArrayOf(0, 0, 0x7F)))
        assertArrayEquals(Vectors.hex("06082a8648ce3d040302"), Der.oid("1.2.840.10045.4.3.2"))
        assertArrayEquals(Vectors.hex("06072a8648ce3d0201"), Der.oid("1.2.840.10045.2.1"))
        assertArrayEquals(Vectors.hex("0603550403"), Der.oid("2.5.4.3"))
        assertEquals(0x81, Der.tlv(0x04, ByteArray(200))[1].toInt() and 0xFF)
    }

    /** Run once with KIN_EXPORT_FIXTURE=1 to (re)write the fixture the desktop verifier tests use. */
    @Test
    fun exportFixtureWhenAsked() {
        if (System.getenv("KIN_EXPORT_FIXTURE") != "1") return
        val identity = Identity.generate()
        val dir = File("src/test/resources/fixtures").also { it.mkdirs() }
        File(dir, "android-generated-cert.der").writeBytes(identity.certificateDer)
        val pem = "-----BEGIN CERTIFICATE-----\n" +
            java.util.Base64.getMimeEncoder(64, "\n".toByteArray()).encodeToString(identity.certificateDer) +
            "\n-----END CERTIFICATE-----\n"
        File(dir, "android-generated-cert.pem").writeText(pem)
        File(dir, "android-generated-cert.json").writeText(
            """
            {
              "description": "A certificate generated by the Android app's own CertificateBuilder (hand-rolled DER). The desktop verifier tests must accept it as a client certificate. deviceId = base64url(SHA-256(SPKI)).",
              "file": "android-generated-cert.der",
              "deviceId": "${identity.deviceId}",
              "shortId": "${identity.shortId}",
              "spkiHex": "${identity.spki.joinToString("") { "%02x".format(it) }}",
              "derBytes": ${identity.certificateDer.size}
            }
            """.trimIndent() + "\n"
        )
    }

    @Test
    fun committedFixtureIsAValidProfileCertificateWithItsExpectedId() {
        val der = javaClass.classLoader!!.getResource("fixtures/android-generated-cert.der")!!.readBytes()
        val expected = Json.parseToJsonElement(
            javaClass.classLoader!!.getResource("fixtures/android-generated-cert.json")!!.readText()
        ).jsonObject
        assertEquals(emptyList<String>(), CertificateProfile.violations(der))
        assertEquals(expected.getValue("deviceId").str(), DeviceIds.deviceIdOfCertificate(der))
        assertEquals(expected.getValue("shortId").str(), DeviceIds.shortIdOfDeviceId(expected.getValue("deviceId").str()))
        val cert = CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(der)) as X509Certificate
        cert.verify(cert.publicKey)
    }
}
