package app.kinrelay.android.pairing

import app.kinrelay.android.identity.Identity
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.protocol.Fields
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.PROTOCOL_VERSION
import app.kinrelay.android.protocol.StrictJson
import app.kinrelay.android.tls.AddressRacer
import app.kinrelay.android.tls.KinTls
import app.kinrelay.android.tls.PinMismatchException
import app.kinrelay.android.tls.RaceResult
import app.kinrelay.android.tls.hasCause
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.IOException
import javax.net.SocketFactory
import javax.net.ssl.SSLException

/** What this phone tells the PC about itself in `POST /v1/pair`. */
data class DeviceInfo(val name: String, val model: String, val appVersion: String)

enum class PairFailure {
    BAD_TOKEN, TOKEN_EXPIRED, TOKEN_USED, UNSUPPORTED_VERSION, BAD_REQUEST, TOO_LARGE,
    SERVER_ERROR, UNREACHABLE, PIN_MISMATCH, TLS_REFUSED,
}

sealed interface PairResult {
    data class Success(val pcName: String?, val address: PairingAddress) : PairResult
    data class Failure(val kind: PairFailure, val detail: String) : PairResult
}

/** Steps 1, 2 and 5 of protocol.md, "Pairing > Steps": race the addresses, pin the PC, `POST /v1/pair`, check the answer. */
class PairingClient(
    private val identity: Identity,
    private val device: DeviceInfo,
    private val log: KinLog,
    private val socketFactory: SocketFactory = SocketFactory.getDefault(),
    private val raceTimeoutMillis: Long = 4_000,
) {
    suspend fun pair(target: PairingTarget): PairResult {
        val tls = KinTls(identity, target.id)
        log.i("pairing", "racing ${target.addresses.size} address(es) for PC ${target.id.take(8)}")
        val address = when (val race = AddressRacer(tls, socketFactory, log).race(target.addresses, raceTimeoutMillis)) {
            is RaceResult.Winner -> race.address
            is RaceResult.Failed -> {
                val detail = race.failures.entries.joinToString("; ") { "${it.key}: ${it.value.javaClass.simpleName}" }
                log.w("pairing", "no address worked: $detail")
                return if (race.pinMismatch) {
                    PairResult.Failure(PairFailure.PIN_MISMATCH, "the PC at the address is not the one in the code")
                } else {
                    PairResult.Failure(PairFailure.UNREACHABLE, detail)
                }
            }
        }
        return withContext(Dispatchers.IO) { post(tls, target, address) }
    }

    private fun post(tls: KinTls, target: PairingTarget, address: PairingAddress): PairResult {
        val body = buildJsonObject {
            put("token", target.token)
            put("name", cutName(Frames.sanitize(device.name)))
            put("platform", "android")
            put("model", Frames.sanitize(device.model))
            put("appVersion", device.appVersion)
            put("protocol", PROTOCOL_VERSION)
        }.toString().toRequestBody(JSON)
        val request = Request.Builder().url("https://${address.host}:${address.port}/v1/pair").post(body).build()
        return try {
            tls.httpClient(socketFactory).newCall(request).execute().use { response ->
                val text = response.body.string()
                if (response.code == 200) checkSuccess(target, address, text) else failureFor(response.code, text)
            }
        } catch (e: IOException) {
            // With TLS 1.3 a refused client certificate only shows up here, on the first read.
            log.w("pairing", "request failed: ${e.javaClass.simpleName}: ${e.message}")
            when {
                e.hasCause<PinMismatchException>() -> PairResult.Failure(PairFailure.PIN_MISMATCH, "wrong server key")
                e is SSLException -> PairResult.Failure(PairFailure.TLS_REFUSED, e.message ?: "TLS error")
                else -> PairResult.Failure(PairFailure.UNREACHABLE, e.message ?: e.javaClass.simpleName)
            }
        }
    }

    private fun checkSuccess(target: PairingTarget, address: PairingAddress, text: String): PairResult {
        val json = parseObject(text)
        val deviceId = Fields.string(json?.get("deviceId"))
        val protocol = Fields.integer(json?.get("protocol"))
        if (json == null || deviceId != target.id || protocol != PROTOCOL_VERSION.toLong()) {
            // protocol.md: any other answer is treated as unsupported_version and nothing is saved.
            log.w("pairing", "unexpected pair answer (deviceId matches=${deviceId == target.id}, protocol=$protocol)")
            return PairResult.Failure(PairFailure.UNSUPPORTED_VERSION, "unexpected answer from the PC")
        }
        return PairResult.Success(Fields.nonEmptyString(json["name"]), address)
    }

    private fun failureFor(status: Int, text: String): PairResult {
        val code = Fields.string(parseObject(text)?.get("error"))
        val kind = when (code) {
            "bad_token" -> PairFailure.BAD_TOKEN
            "token_expired" -> PairFailure.TOKEN_EXPIRED
            "token_used" -> PairFailure.TOKEN_USED
            "unsupported_version" -> PairFailure.UNSUPPORTED_VERSION
            "bad_request" -> PairFailure.BAD_REQUEST
            "too_large" -> PairFailure.TOO_LARGE
            else -> if (status == 426) PairFailure.UNSUPPORTED_VERSION else PairFailure.SERVER_ERROR
        }
        log.w("pairing", "pair refused: HTTP $status ${code ?: "(no error code)"}")
        return PairResult.Failure(kind, "HTTP $status ${code.orEmpty()}".trim())
    }

    private fun parseObject(text: String): JsonObject? =
        try { StrictJson.parse(text) as? JsonObject } catch (_: StrictJson.InvalidJson) { null }

    private fun cutName(name: String): String {
        val trimmed = name.ifEmpty { "Android" }
        return if (trimmed.codePointCount(0, trimmed.length) <= 64) trimmed else trimmed.substring(0, trimmed.offsetByCodePoints(0, 64))
    }

    private companion object {
        val JSON = "application/json; charset=utf-8".toMediaType()
    }
}
