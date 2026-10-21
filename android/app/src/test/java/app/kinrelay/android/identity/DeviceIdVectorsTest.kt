package app.kinrelay.android.identity

import app.kinrelay.android.testing.CertificateProfile
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import java.io.ByteArrayInputStream
import java.security.MessageDigest
import java.security.Signature
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.util.Base64
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceIdVectorsTest {
    private val vectors = Vectors.load("device-id.json")

    private fun pemBody(pem: String) = Base64.getMimeDecoder().decode(pem.lines().filterNot { it.startsWith("-----") }.joinToString(""))

    @Test
    fun keys_giveTheExpectedIdsFromTheSpkiAndTheCertificate() {
        for (key in vectors.cases("keys")) {
            val name = key.getValue("name").str()
            val spki = Vectors.hex(key.getValue("spkiDerHex").str())
            val certDer = pemBody(key.getValue("certificatePem").str())
            assertEquals("$name: spki length", 91, spki.size)
            assertEquals("$name: sha256", key.getValue("sha256Hex").str(), MessageDigest.getInstance("SHA-256").digest(spki).joinToString("") { "%02x".format(it) })
            assertEquals("$name: deviceId", key.getValue("deviceId").str(), DeviceIds.deviceId(spki))
            assertEquals("$name: shortId", key.getValue("shortId").str(), DeviceIds.shortId(spki))
            assertTrue("$name: SPKI read from the certificate is the same", spki.contentEquals(CertificateInfo.spkiOf(certDer)))
            assertEquals("$name: deviceId from the certificate", key.getValue("deviceId").str(), DeviceIds.deviceIdOfCertificate(certDer))
            assertEquals("$name: short ID of the device ID", key.getValue("shortId").str(), DeviceIds.shortIdOfDeviceId(key.getValue("deviceId").str()))
        }
    }

    @Test
    fun keys_privateKeyBelongsToTheCertificate() {
        for (key in vectors.cases("keys")) {
            val name = key.getValue("name").str()
            val privateKey = Identity.privateKeyFromPkcs8(pemBody(key.getValue("privateKeyPkcs8Pem").str()))
            val cert = CertificateFactory.getInstance("X.509")
                .generateCertificate(ByteArrayInputStream(pemBody(key.getValue("certificatePem").str()))) as X509Certificate
            val signature = Signature.getInstance("SHA256withECDSA").run { initSign(privateKey); update(byteArrayOf(1, 2, 3)); sign() }
            val ok = Signature.getInstance("SHA256withECDSA").run { initVerify(cert.publicKey); update(byteArrayOf(1, 2, 3)); verify(signature) }
            assertTrue("$name: the private key signs for the certificate's key", ok)
        }
    }

    @Test
    fun keys_certificatesFollowTheProfileAndOurParserSeesTheVariant() {
        for (key in vectors.cases("keys")) {
            val name = key.getValue("name").str()
            val der = pemBody(key.getValue("certificatePem").str())
            assertEquals("$name: profile violations", emptyList<String>(), CertificateProfile.violations(der))
            val extensions = CertificateInfo.parse(der).extensions
            when (key.getValue("certificateVariant").str()) {
                "minimal" -> assertTrue("$name: no extensions", extensions.isEmpty())
                else -> assertTrue("$name: non-critical extension present", extensions.isNotEmpty() && extensions.none { it.critical })
            }
        }
    }

    @Test
    fun hashVectors() {
        for (case in vectors.cases("hashVectors")) {
            val hash = Vectors.hex(case.getValue("sha256Hex").str())
            assertEquals(case.getValue("name").str(), case.getValue("deviceId").str(), DeviceIds.deviceIdOfHash(hash))
            assertEquals(case.getValue("name").str(), case.getValue("shortId").str(), DeviceIds.shortIdOfHash(hash))
        }
    }

    @Test
    fun tokenVectors() {
        for (case in vectors.cases("tokenVectors")) {
            val token = DeviceIds.base64Url(Vectors.hex(case.getValue("bytesHex").str()))
            assertEquals(case.getValue("name").str(), case.getValue("token").str(), token)
            assertTrue(DeviceIds.isToken(token))
        }
    }

    @Test
    fun shapeVectors() {
        for (case in vectors.cases("shapeVectors")) {
            val value = case.getValue("value").str()
            val expected = case.getValue("valid").jsonPrimitive.boolean
            val actual = if (case.getValue("kind").str() == "deviceId") DeviceIds.isDeviceId(value) else DeviceIds.isToken(value)
            assertEquals("${case.getValue("kind").str()} '$value'", expected, actual)
        }
    }

    @Test
    fun shortIdOfInvalidDeviceIdIsNull() {
        assertNull(DeviceIds.shortIdOfDeviceId("short"))
        assertEquals("AAAA-AAAA", DeviceIds.display("AAAAAAAA"))
    }
}
