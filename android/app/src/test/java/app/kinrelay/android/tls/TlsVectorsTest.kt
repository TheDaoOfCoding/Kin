package app.kinrelay.android.tls

import app.kinrelay.android.identity.CertificateBuilder
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import java.security.KeyPairGenerator
import java.security.spec.ECGenParameterSpec
import java.time.Instant
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Request
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The phone-side `tls[]` cases of `protocol/http.json`, with real TLS 1.3 against [FakePc]. */
class TlsVectorsTest {
    private val phone = Identity.generate()
    private val pcs = mutableListOf<FakePc>()

    @After
    fun tearDown() = pcs.forEach { it.close() }

    private fun pc(identity: Identity = Identity.generate()) = FakePc(identity).start().also { pcs += it }

    private fun reachable(pc: FakePc, pinnedId: String): Boolean = runBlocking {
        AddressRacer(KinTls(phone, pinnedId)).race(listOf(PairingAddress("127.0.0.1", pc.port)), 3_000) is RaceResult.Winner
    }

    private fun httpGet(pc: FakePc, pinnedId: String): Result<Int> = runCatching {
        KinTls(phone, pinnedId).httpClient().newCall(Request.Builder().url("https://127.0.0.1:${pc.port}/anything").build()).execute().use { it.code }
    }

    private fun expiredIdentityWithWrongHostname(): Identity {
        val keys = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
        val der = CertificateBuilder.build(keys.public, keys.private, Instant.parse("2001-01-01T00:00:00Z"), Instant.parse("2002-01-01T00:00:00Z"))
        return Identity(keys.private, der)
    }

    @Test
    fun theTlsVectorsOfThePhoneAreCovered() {
        val phoneCases = Vectors.load("http.json").cases("tls").filter { it.getValue("verifier").str() == "phone" }
        assertEquals(
            setOf("phone-accepts-the-pinned-self-signed-server", "phone-ignores-hostname-and-expiry", "phone-rejects-a-different-key", "phone-rejects-an-invalid-certificateverify-signature"),
            phoneCases.map { it.getValue("name").str() }.toSet(),
        )
        assertTrue(phoneCases.single { it.getValue("name").str().endsWith("invalid-certificateverify-signature") }.getValue("optional").jsonPrimitive.boolean)
    }

    @Test
    fun phoneAcceptsThePinnedSelfSignedServer() {
        val pc = pc()
        assertTrue(reachable(pc, pc.identity.deviceId))
        assertEquals(404, httpGet(pc, pc.identity.deviceId).getOrThrow())
    }

    @Test
    fun phoneIgnoresHostnameAndExpiry() {
        val expired = expiredIdentityWithWrongHostname()
        val pc = pc(expired)
        assertTrue("an expired certificate for no host name, reached by IP, is accepted", reachable(pc, expired.deviceId))
        assertEquals(404, httpGet(pc, expired.deviceId).getOrThrow())
    }

    @Test
    fun phoneRejectsADifferentKey() {
        val pc = pc()
        val other = Identity.generate().deviceId
        assertFalse(reachable(pc, other))
        val failure = httpGet(pc, other).exceptionOrNull()
        assertTrue("got $failure", failure != null && failure.hasCause<PinMismatchException>())
        assertTrue("the request never left the phone", pc.requests.isEmpty())
    }

    @Test
    fun noFallbackToTheSystemTrustStore() {
        val trust = PinnedTrustManager(Identity.generate().deviceId)
        assertEquals(0, trust.acceptedIssuers.size)
        val result = runCatching { trust.checkServerTrusted(emptyArray(), "ECDSA") }
        assertTrue(result.isFailure)
        assertTrue(runCatching { trust.checkClientTrusted(arrayOf(phone.certificate), "ECDSA") }.isFailure)
    }

    @Test
    fun theClientCertificateTheServerSeesIsOurs() {
        val pc = pc()
        httpGet(pc, pc.identity.deviceId).getOrThrow()
        assertEquals(phone.deviceId, pc.requests.single().clientDeviceId)
    }
}
