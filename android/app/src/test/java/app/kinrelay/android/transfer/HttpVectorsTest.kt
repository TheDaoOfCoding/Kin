package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.link.HttpEndpoint
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.Fields
import app.kinrelay.android.testing.FakeClock
import app.kinrelay.android.testing.FakePc
import app.kinrelay.android.testing.TransferRig
import app.kinrelay.android.testing.Vectors
import app.kinrelay.android.testing.cases
import app.kinrelay.android.testing.parse
import app.kinrelay.android.testing.str
import app.kinrelay.android.tls.KinTls
import java.io.ByteArrayInputStream
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import mockwebserver3.MockResponse
import mockwebserver3.RecordedRequest
import mockwebserver3.SocketEffect
import okio.Buffer
import okio.ByteString.Companion.toByteString
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The client side of `protocol/http.json` (protocol/README.md): every exchange about transfers is replayed from the fake PC to the
 * phone's [TransferHttp] and the answer is checked (a `409 offset_mismatch` carries the offset, a `206` has its `Content-Range`, a
 * `204` and a retried `404` on `DELETE` both mean done, every other error carries its code). Where the vector's request is one the
 * phone would send, the request it really sent is compared too. The sequences that follow the phone's own order of calls are run
 * through the real upload and download runners.
 */
class HttpVectorsTest {
    private val vectors = Vectors.load("http.json")
    private val pcs = mutableListOf<FakePc>()
    private val rigs = mutableListOf<TransferRig>()

    @After
    fun tearDown() {
        pcs.forEach { it.close() }
        rigs.forEach { it.close() }
    }

    private val canonicalNumber = Regex("0|[1-9][0-9]{0,15}")

    /** A MockWebServer response made from a vector's `response`. */
    private fun mockOf(response: JsonObject): MockResponse {
        val builder = MockResponse.Builder().code(response.getValue("status").jsonPrimitive.int)
        response["headers"]?.jsonObject?.forEach { (name, value) ->
            if (!name.equals("Content-Length", ignoreCase = true)) builder.addHeader(name, value.str())
        }
        val body = response["body"]
        val bytes = response["bodyBytes"]?.jsonPrimitive?.int
        when {
            body != null && body !is JsonNull -> builder.body(if (body is JsonPrimitive && body.isString) body.content else body.toString())
            bytes != null && bytes > 0 -> builder.body(Buffer().write(ByteArray(bytes).toByteString()))
        }
        return builder.build()
    }

    private fun endpointOf(pc: FakePc, phone: Identity): HttpEndpoint {
        val client = KinTls(phone, pc.identity.deviceId).httpClient().newBuilder().callTimeout(0, TimeUnit.MILLISECONDS).build()
        return HttpEndpoint(client, PairingAddress("127.0.0.1", pc.port))
    }

    private fun header(headers: JsonObject?, name: String): String? = headers?.entries?.firstOrNull { it.key.equals(name, true) }?.value?.str()

    private fun numberHeader(headers: JsonObject?, name: String): Long? = header(headers, name)?.toLongOrNull()

    /** The offset of a vector's PUT path if it is one the phone would write (canonical digits, at most 2^53 - 1), else null. */
    private fun canonicalOffset(path: String): Long? {
        val text = Regex("\\?offset=([0-9]+)$").find(path)?.groupValues?.get(1) ?: return null
        return text.takeIf { canonicalNumber.matches(it) }?.toLongOrNull()?.takeIf { it <= Fields.MAX_SAFE_INTEGER }
    }

    private fun canonicalRange(header: String?): Long? =
        header?.let { Regex("bytes=([0-9]+)-").matchEntire(it)?.groupValues?.get(1) }?.takeIf { canonicalNumber.matches(it) }?.toLongOrNull()?.takeIf { it <= Fields.MAX_SAFE_INTEGER }

    /** The phone's side of `http.json`: a fake PC that answers with a vector's response, and the phone's client talking to it. */
    private inner class ExchangeClient {
        val phone = Identity.generate()
        val pc = FakePc().apply { pairedPhones += phone.deviceId }.start().also { pcs += it }
        val http = TransferHttp(endpointOf(pc, phone))
        val seen = CopyOnWriteArrayList<RecordedRequest>()
    }

    @Test
    fun everyTransferExchangeIsHandledByTheClient() {
        val client = ExchangeClient()
        val exchanges = vectors.cases("exchanges").filter { it.getValue("request").jsonObject.getValue("path").str().startsWith("/v1/transfers") }
        assertTrue("there are ${exchanges.size} transfer exchanges", exchanges.size >= 100)
        var shapeChecks = 0

        for (case in exchanges) {
            if (replayExchange(client, case.getValue("name").str(), case.getValue("request").jsonObject, case.getValue("response").jsonObject)) shapeChecks++
        }
        assertTrue("$shapeChecks request shapes were compared", shapeChecks >= 40)
    }

    /**
     * One exchange: the fake PC answers with [response], the phone's client sends what it would send for [request] and its reading of
     * the answer is checked. Returns true when the vector's request is one the phone sends and its request shape was compared too.
     */
    private fun replayExchange(client: ExchangeClient, name: String, request: JsonObject, response: JsonObject): Boolean {
        val (pc, http, seen) = Triple(client.pc, client.http, client.seen)
        val status = response.getValue("status").jsonPrimitive.int
        val method = request.getValue("method").str()
        val path = request.getValue("path").str()
        val headers = request["headers"] as? JsonObject
        seen.clear()
        pc.canned = { req -> seen += req; mockOf(response) }
        val segments = path.substringBefore('?').trim('/').split('/')

        val outcome: Any = runBlocking {
            when (method) {
                "POST" -> {
                    val body = request["body"] as? JsonObject
                    val files = (body?.get("files") as? JsonArray)?.mapNotNull { it as? JsonObject }?.mapNotNull {
                        val fileId = it["fileId"]?.jsonPrimitive?.content
                        val size = it["size"]?.jsonPrimitive?.content?.toLongOrNull()
                        if (fileId == null || size == null) null else UploadFileSpec(fileId, it["name"]?.jsonPrimitive?.content ?: "x", size, it["mime"]?.jsonPrimitive?.content ?: "x/x")
                    }
                    http.createTransfer(body?.get("id")?.jsonPrimitive?.content ?: "x", files?.takeIf { it.isNotEmpty() } ?: listOf(UploadFileSpec("f1", "x", 1, "x/x")))
                }
                "PUT" -> {
                    val length = request["bodyBytes"]?.jsonPrimitive?.long ?: 0L
                    http.put(segments[2], segments[4], canonicalOffset(path) ?: 0L, SourceRequestBody({ ByteArrayInputStream(ByteArray(length.toInt())) }, length) { })
                }
                "DELETE" -> http.delete(segments[2])
                "GET" -> http.download(segments[2], segments[4], canonicalRange(header(headers, "Range"))) { r ->
                    Triple(r.code, r.header("Content-Range"), r.body.bytes().size.toLong())
                }
                else -> error("unexpected method $method in $name")
            }
        }

        // What the phone made of the answer.
        val responseHeaders = response["headers"] as? JsonObject
        when (method) {
            "GET" -> {
                @Suppress("UNCHECKED_CAST") val (code, contentRange, length) = outcome as Triple<Int, String?, Long>
                assertEquals("$name: status", status, code)
                if (status == 200 || status == 206) {
                    assertEquals("$name: Content-Range", header(responseHeaders, "Content-Range"), contentRange)
                    assertEquals("$name: body", response["bodyBytes"]?.jsonPrimitive?.long ?: 0L, length)
                }
            }
            else -> {
                val success = when (method) { "DELETE" -> status == 204 || status == 404; else -> status == 200 }
                if (success) {
                    assertTrue("$name: $outcome", outcome is ApiResult.Ok<*>)
                    val value = (outcome as ApiResult.Ok<*>).value
                    val body = response["body"] as? JsonObject
                    if (method == "POST") {
                        val expected = body!!.getValue("files").jsonArray.associate { it.jsonObject.getValue("fileId").str() to it.jsonObject.getValue("offset").jsonPrimitive.long }
                        assertEquals("$name: offsets", expected, value)
                    }
                    if (method == "PUT") assertEquals("$name: received", body!!.getValue("received").jsonPrimitive.long, value)
                } else {
                    assertTrue("$name: $outcome", outcome is ApiResult.Error)
                    val error = outcome as ApiResult.Error
                    val body = response["body"] as? JsonObject
                    assertEquals("$name: status", status, error.status)
                    assertEquals("$name: error code", body!!.getValue("error").str(), error.code)
                    assertEquals("$name: offset", body["offset"]?.jsonPrimitive?.long, error.offset)
                }
            }
        }

        // The request the phone sent, where the vector's request is one it would send.
        val sent = seen.firstOrNull() ?: return false
        val body = request["body"] as? JsonObject
        val wellFormed = when (method) {
            "POST" -> body != null && body.keys == setOf("id", "files") && header(headers, "Content-Type") == "application/json; charset=utf-8" &&
                (body.getValue("files") as? JsonArray)?.let { files ->
                    files.isNotEmpty() && files.all { f ->
                        val file = f as? JsonObject
                        file?.keys == setOf("fileId", "name", "size", "mime") &&
                            listOf("fileId", "name", "mime").all { (file[it] as? JsonPrimitive)?.isString == true } &&
                            (file["size"] as? JsonPrimitive)?.let { size -> !size.isString && canonicalNumber.matches(size.content) } == true
                    }
                } == true
            "PUT" -> canonicalOffset(path) != null && numberHeader(headers, "Content-Length") == request["bodyBytes"]?.jsonPrimitive?.long
            "GET" -> header(headers, "Range") == null || canonicalRange(header(headers, "Range")) != null
            else -> true
        }
        if (!wellFormed) return false
        assertEquals("$name: method", method, sent.method)
        assertEquals("$name: path", path, sent.url.encodedPath + (sent.url.encodedQuery?.let { "?$it" } ?: ""))
        when (method) {
            "POST" -> {
                assertEquals("$name: Content-Type", "application/json; charset=utf-8", sent.headers["Content-Type"])
                assertEquals("$name: body", body, Vectors.parse(sent.body!!.utf8()).jsonObject)
            }
            "PUT" -> {
                assertEquals("$name: Content-Length", numberHeader(headers, "Content-Length"), sent.headers["Content-Length"]?.toLongOrNull())
                assertNull("$name: no chunked encoding", sent.headers["Transfer-Encoding"])
            }
            "GET" -> assertEquals("$name: Range", header(headers, "Range"), sent.headers["Range"])
        }
        return true
    }

    /**
     * The `transfer-id-*` sequences (a transfer ID is never reused). The phone's client reads every answer the way the sequence says
     * (a `404` on a `POST` is an error with `not_found`, never a transfer to go on with), and a [FakeClock] follows the `clock` steps:
     * a finished upload's ID answers a `POST` or a `PUT` for 24 hours after its file arrived and is `404` after that, a canceled
     * ID is `404` at once. The phone has no PC clock of its own: the clock only orders the steps and checks the vector against itself.
     */
    @Test
    fun everyTransferIdSequenceIsUnderstoodByTheClient() {
        val sequences = vectors.cases("sequences").filter { it.getValue("name").str().startsWith("transfer-id-") }
        assertEquals(listOf("transfer-id-canceled-is-never-reused", "transfer-id-finished-answers-for-a-day-then-404"), sequences.map { it.getValue("name").str() })
        var clockSteps = 0
        var requests = 0
        for (sequence in sequences) {
            val sequenceName = sequence.getValue("name").str()
            val client = ExchangeClient()
            val clock = FakeClock()
            val finishedAt = HashMap<String, Long>() // transfer ID -> the clock when its last file arrived
            val canceled = HashSet<String>()
            for ((index, step) in sequence.getValue("steps").jsonArray.map { it.jsonObject }.withIndex()) {
                val name = "$sequenceName#$index"
                val advance = step["clock"]?.jsonObject
                if (advance != null) {
                    assertNull("$name: a clock step sends no request", step["request"])
                    val before = clock.nowMillis
                    clock.advanceHours(advance.getValue("advanceHours").jsonPrimitive.long)
                    assertTrue("$name: the clock moved forward", clock.nowMillis > before)
                    clockSteps++
                    continue
                }
                val request = step.getValue("request").jsonObject
                val response = step.getValue("response").jsonObject
                val method = request.getValue("method").str()
                val path = request.getValue("path").str()
                val status = response.getValue("status").jsonPrimitive.int
                val id = (request["body"] as? JsonObject)?.get("id")?.str() ?: path.removePrefix("/v1/transfers/").substringBefore('/')

                // What the vector says must agree with the clock: its own rule, checked on the sequence.
                val finished = finishedAt[id]
                val expected = when {
                    id in canceled -> if (method == "DELETE" || method == "POST" || method == "PUT") 404 else status
                    finished != null && clock.nowMillis - finished >= 24 * FakeClock.HOUR_MILLIS -> 404
                    else -> status
                }
                assertEquals("$name: $method at ${clock.nowMillis / FakeClock.HOUR_MILLIS} h", expected, status)

                replayExchange(client, name, request, response)
                requests++

                if (method == "DELETE" && status == 204) canceled += id
                if (method == "PUT" && status == 200 && (request["bodyBytes"]?.jsonPrimitive?.long ?: 0L) > 0) finishedAt[id] = clock.nowMillis
            }
            val notFound = sequence.getValue("steps").jsonArray.count { (it.jsonObject["response"] as? JsonObject)?.get("status")?.jsonPrimitive?.int == 404 }
            assertTrue("$sequenceName: it has 404 answers", notFound >= 3)
        }
        assertEquals("one sequence moves the clock, twice", 2, clockSteps)
        assertTrue("requests replayed: $requests", requests >= 14)
    }

    @Test
    fun aDeleteThatIsRetriedAfterALost204IsDone() {
        val phone = Identity.generate()
        val pc = FakePc().apply { pairedPhones += phone.deviceId }.start().also { pcs += it }
        val http = TransferHttp(endpointOf(pc, phone))
        val sequence = vectors.cases("sequences").single { it.getValue("name").str() == "delete-retried-after-lost-204-is-done" }
        val queue = LinkedBlockingQueue(sequence.getValue("steps").jsonArray.map { mockOf(it.jsonObject.getValue("response").jsonObject) })
        pc.canned = { queue.poll() }
        val id = "0193a5c2-8000-7a11-8b22-0123456789ab"
        assertTrue(runBlocking { http.delete(id) } is ApiResult.Ok)
        assertTrue("404 on the retried DELETE is done, not an error", runBlocking { http.delete(id) } is ApiResult.Ok)
    }

    // ---- sequences replayed through the real runners

    private class Step(val request: JsonObject, val response: JsonObject?, val interruptedAfterBytes: Long?)

    private fun steps(name: String): List<Step> = vectors.cases("sequences").single { it.getValue("name").str() == name }
        .getValue("steps").jsonArray.map { it.jsonObject }
        .map { Step(it.getValue("request").jsonObject, it["response"]?.takeIf { r -> r !is JsonNull }?.jsonObject, it["interruptedAfterBytes"]?.jsonPrimitive?.long) }

    /**
     * Installs the steps, runs [start] (which makes the phone send), then waits for every step to be served. Each request is checked against
     * the vector (method, path with query, Content-Length, Range, JSON body); [serve] makes the response. Returns what didn't match.
     */
    private fun replay(rig: TransferRig, steps: List<Step>, start: () -> Unit, serve: (Int, Step, RecordedRequest) -> MockResponse): List<String> {
        val problems = CopyOnWriteArrayList<String>()
        val next = AtomicInteger()
        rig.pc.canned = { req ->
            val index = next.getAndIncrement()
            val step = steps.getOrNull(index)
            if (step == null) {
                problems += "unexpected request ${req.method} ${req.url.encodedPath}"
                MockResponse.Builder().code(500).build()
            } else {
                val want = step.request
                val path = req.url.encodedPath + (req.url.encodedQuery?.let { "?$it" } ?: "")
                if (req.method != want.getValue("method").str()) problems += "step $index: method ${req.method}"
                if (path != want.getValue("path").str()) problems += "step $index: path $path, wanted ${want.getValue("path").str()}"
                val headers = want["headers"] as? JsonObject
                numberHeader(headers, "Content-Length")?.let { if (req.headers["Content-Length"]?.toLongOrNull() != it) problems += "step $index: Content-Length ${req.headers["Content-Length"]}, wanted $it" }
                header(headers, "Range")?.let { if (req.headers["Range"] != it) problems += "step $index: Range ${req.headers["Range"]}, wanted $it" }
                want["body"]?.let {
                    val sent = Vectors.parse(req.body!!.utf8()).jsonObject
                    if (sent != it.jsonObject) problems += "step $index: body $sent, wanted $it"
                }
                serve(index, step, req)
            }
        }
        start()
        try {
            rig.await("the sequence to be served", 20_000) { next.get() >= steps.size }
        } catch (e: IllegalStateException) {
            problems += "timed out after ${next.get()} of ${steps.size} steps; requests: ${rig.pc.requests.map { it.method + " " + it.path }}"
        }
        Thread.sleep(200)
        if (next.get() != steps.size) problems += "served ${next.get()} of ${steps.size} steps"
        return problems
    }

    /** The step without a response is the request whose connection broke: the phone sees no answer. */
    private fun brokenOrServed(step: Step): MockResponse = step.response?.let(::mockOf) ?: MockResponse.Builder().onResponseStart(SocketEffect.CloseSocket()).build()

    @Test
    fun uploadResumeAfterBrokenConnection() {
        val id = "0193a5c2-8000-7a11-8b22-0123456789ab"
        val rig = TransferRig(newId = { id }).also { rigs += it; it.connect() }
        val uri = rig.sources.add("IMG_0042.jpg", TransferRig.bytes(3_145_728), "image/jpeg")
        val problems = replay(rig, steps("upload-resume-after-broken-connection"), { rig.blocking { rig.engine.enqueueUpload(listOf(ShareSource(uri))) } }) { _, step, _ ->
            brokenOrServed(step)
        }
        assertEquals(emptyList<String>(), problems.toList())
        rig.awaitStatus(id, TransferStatus.DONE)
    }

    @Test
    fun uploadSourceChangedUsesANewTransferId() {
        val ids = LinkedBlockingQueue(listOf("0193a5c2-8000-7a11-8b22-0123456789ab", "0193a5c2-8000-7a11-8b22-0123456789ac"))
        val rig = TransferRig(newId = { ids.take() }).also { rigs += it; it.connect() }
        val uri = rig.sources.add("IMG_0042.jpg", TransferRig.bytes(3_145_728), "image/jpeg")
        // Step 4 of the vector is a stale PUT to the old ID (the server answers 404): it shows the old ID is gone, but the phone never sends it.
        val steps = steps("upload-source-changed-uses-a-new-transfer-id").filterIndexed { index, _ -> index != 4 }
        val problems = replay(rig, steps, { rig.blocking { rig.engine.enqueueUpload(listOf(ShareSource(uri))) } }) { _, step, _ ->
            // The photo is edited while the first attempt is in flight: its size differs at the retry.
            if (step.response == null) rig.sources.fileOf(uri).writeBytes(TransferRig.bytes(3_200_000, 5))
            brokenOrServed(step)
        }
        assertEquals(emptyList<String>(), problems.toList())
        rig.awaitStatus("0193a5c2-8000-7a11-8b22-0123456789ac", TransferStatus.DONE)
    }

    @Test
    fun downloadResumeAfterBrokenConnection() {
        val rig = TransferRig().also { rigs += it; it.connect() }
        val id = "0193a5c2-8000-7a11-8b22-0123456789ab"
        val bytes = TransferRig.bytes(3_145_728)
        val offer = """{"type":"offer","id":"$id","files":[{"fileId":"f1","name":"video.mp4","size":3145728,"mime":"video/mp4"}],"totalSize":3145728}"""
        val problems = replay(rig, steps("download-resume-after-broken-connection"), { rig.pc.send(offer) }) { _, step, _ ->
            val response = step.response
            if (response == null) {
                // The connection broke after 1 MiB: the real bytes up to there, with the full length announced.
                MockResponse.Builder().code(200)
                    .body(Buffer().write(bytes.copyOfRange(0, step.interruptedAfterBytes!!.toInt()).toByteString()))
                    .setHeader("Content-Length", bytes.size.toString()).onResponseEnd(SocketEffect.CloseSocket()).build()
            } else {
                // The resumed GET: the real rest of the file, with the vector's Content-Range.
                val range = header(response["headers"] as JsonObject, "Content-Range")!!
                val from = Regex("bytes ([0-9]+)-").find(range)!!.groupValues[1].toInt()
                MockResponse.Builder().code(response.getValue("status").jsonPrimitive.int).setHeader("Content-Range", range)
                    .body(Buffer().write(bytes.copyOfRange(from, bytes.size).toByteString())).build()
            }
        }
        assertEquals(emptyList<String>(), problems.toList())
        rig.awaitStatus(id, TransferStatus.DONE)
        assertArrayEquals(bytes, rig.sink.visible().single().file.readBytes())
    }
}
