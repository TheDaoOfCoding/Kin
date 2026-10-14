package app.kinrelay.android.pairing

import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.StrictJson
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.str
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import mockwebserver3.MockResponse
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingClientTest {
    private val phone = Identity.generate()
    private val device = DeviceInfo("Pixel 8", "Google Pixel 8", "0.1.0")
    private val pcs = mutableListOf<FakePc>()

    @After
    fun tearDown() = pcs.forEach { it.close() }

    private fun pc(block: FakePc.() -> Unit = {}) = FakePc().apply(block).start().also { pcs += it }

    private fun target(pc: FakePc, id: String = pc.identity.deviceId) = PairingTarget(
        version = 1,
        addresses = listOf(PairingAddress("127.0.0.1", pc.port)),
        id = id,
        token = "AAECAwQFBgcICQoLDA0ODw",
        name = "Desk PC",
    )

    private fun pair(target: PairingTarget) = runBlocking { PairingClient(phone, device, KinLog.None, raceTimeoutMillis = 3_000).pair(target) }

    @Test
    fun pairsAndSendsTheRequiredFieldsWithTheRightContentType() {
        val pc = pc()
        val result = pair(target(pc))
        assertEquals(PairResult.Success("Desk PC", PairingAddress("127.0.0.1", pc.port)), result)

        val request = pc.requests.single()
        assertEquals("POST", request.method)
        assertEquals("/v1/pair", request.path)
        assertEquals("application/json; charset=utf-8", request.contentType)
        assertEquals("the PC learns the phone's ID from the TLS certificate", phone.deviceId, request.clientDeviceId)
        val body = StrictJson.parse(request.body).jsonObject
        assertEquals("AAECAwQFBgcICQoLDA0ODw", body.getValue("token").str())
        assertEquals("Pixel 8", body.getValue("name").str())
        assertEquals("android", body.getValue("platform").str())
        assertEquals("Google Pixel 8", body.getValue("model").str())
        assertEquals("0.1.0", body.getValue("appVersion").str())
        assertEquals(1, body.getValue("protocol").jsonPrimitive.int)
        assertTrue("no device ID in the body", "deviceId" !in body)
    }

    @Test
    fun aLongDeviceNameIsCutTo64CodePointsWithoutSplittingASurrogatePair() {
        val pc = pc()
        val name = "a".repeat(63) + "😀😀"
        runBlocking { PairingClient(phone, DeviceInfo(name, "m", "1"), KinLog.None).pair(target(pc)) }
        val sent = StrictJson.parse(pc.requests.single().body).jsonObject.getValue("name").str()
        assertEquals("a".repeat(63) + "😀", sent)
    }

    @Test
    fun abortsWhenTheServerKeyIsNotThePinnedId() {
        val pc = pc()
        val other = Identity.generate().deviceId
        val result = pair(target(pc, id = other))
        assertEquals(PairFailure.PIN_MISMATCH, (result as PairResult.Failure).kind)
        assertTrue("nothing, not even the token, reached the server", pc.requests.isEmpty())
    }

    @Test
    fun reportsAnUnreachablePc() {
        val closed = pc().also { it.close() }
        val result = pair(target(closed))
        assertEquals(PairFailure.UNREACHABLE, (result as PairResult.Failure).kind)
    }

    @Test
    fun raceTakesTheAddressThatWorksWhenTheOthersAreDead() {
        val pc = pc()
        val dead = pc().also { it.close() }
        val mixed = target(pc).copy(addresses = listOf(PairingAddress("127.0.0.1", dead.port), PairingAddress("127.0.0.1", pc.port)))
        val result = pair(mixed)
        assertEquals(PairingAddress("127.0.0.1", pc.port), (result as PairResult.Success).address)
    }

    @Test
    fun aServerThatSpeaksOnlyTls12IsRefused() {
        val pc = FakePc(tlsProtocol = "TLSv1.2").start().also { pcs += it }
        val result = pair(target(pc))
        assertTrue(result is PairResult.Failure)
        assertTrue(pc.requests.isEmpty())
    }

    @Test
    fun aRefusedClientCertificateIsAFailedPairingNeverSuccess() {
        // With TLS 1.3 the phone's handshake call can return before the server has checked the
        // certificate (Conscrypt), so the refusal may only show on the first read. Either way it fails.
        val pc = FakePc(rejectClientCertificates = true).start().also { pcs += it }
        val result = pair(target(pc))
        assertTrue("got $result", result is PairResult.Failure)
        assertTrue(pc.requests.isEmpty())
    }

    @Test
    fun anUnexpectedDeviceIdOrProtocolInTheAnswerSavesNothing() {
        val wrongId = pc { pairResponse = { jsonResponse(200, """{"name":"x","deviceId":"${DeviceIds.deviceIdOfHash(ByteArray(32))}","protocol":1}""") } }
        assertEquals(PairFailure.UNSUPPORTED_VERSION, (pair(target(wrongId)) as PairResult.Failure).kind)
        val wrongProtocol = pc { pairResponse = { jsonResponse(200, """{"name":"x","deviceId":"${identity.deviceId}","protocol":2}""") } }
        assertEquals(PairFailure.UNSUPPORTED_VERSION, (pair(target(wrongProtocol)) as PairResult.Failure).kind)
        val garbage = pc { pairResponse = { jsonResponse(200, "yes") } }
        assertEquals(PairFailure.UNSUPPORTED_VERSION, (pair(target(garbage)) as PairResult.Failure).kind)
    }

    @Test
    fun everyErrorOfTheErrorTableIsMappedToAFailure() {
        val expected = mapOf(
            "bad_request" to PairFailure.BAD_REQUEST, "bad_token" to PairFailure.BAD_TOKEN,
            "token_expired" to PairFailure.TOKEN_EXPIRED, "token_used" to PairFailure.TOKEN_USED,
            "unsupported_version" to PairFailure.UNSUPPORTED_VERSION, "too_large" to PairFailure.TOO_LARGE,
        )
        var checked = 0
        for (case in Vectors.load("http.json").cases("errors")) {
            val status = case["status"]?.takeIf { it !is kotlinx.serialization.json.JsonNull }?.jsonPrimitive?.int ?: continue
            val code = case.getValue("code").str()
            val body = case.getValue("body").toString()
            val pc = pc { pairResponse = { jsonResponse(status, body) } }
            val failure = (pair(target(pc)) as PairResult.Failure).kind
            val want = expected[code] ?: PairFailure.SERVER_ERROR
            assertEquals("HTTP $status $code", want, failure)
            checked++
        }
        assertTrue("checked $checked", checked >= 12)
    }

    private fun jsonResponse(status: Int, body: String) = MockResponse.Builder()
        .code(status).addHeader("Content-Type", "application/json; charset=utf-8").body(body).build()
}
