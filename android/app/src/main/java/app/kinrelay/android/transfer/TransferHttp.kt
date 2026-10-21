package app.kinrelay.android.transfer

import app.kinrelay.android.link.HttpEndpoint
import app.kinrelay.android.protocol.Fields
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.JsonArray
import okhttp3.Call
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okio.BufferedSink
import okio.Buffer
import okio.source
import java.io.IOException
import java.io.InputStream

/** One file of `POST /v1/transfers`. */
data class UploadFileSpec(val fileId: String, val name: String, val size: Long, val mime: String)

/** The answer to a request: a value, or the PC's `{ error, message }` (plus `offset` for `offset_mismatch`). */
sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>
    data class Error(val status: Int, val code: String, val message: String, val offset: Long? = null) : ApiResult<Nothing>
}

/** The shared file ended before the declared size (it was shortened or lost) while its bytes were being sent. */
class SourceShortException(message: String) : IOException(message)

/**
 * The HTTPS endpoints of protocol.md, "Files": `POST /v1/transfers`, `PUT …/files/{fileId}?offset=N`, `GET …/files/{fileId}`
 * (with `Range: bytes=N-`) and `DELETE /v1/transfers/{id}`. Every call can be cancelled with its coroutine: the call is
 * cancelled, which also stops a body that is being sent or read.
 */
class TransferHttp(private val endpoint: HttpEndpoint) {
    private fun base(): HttpUrl.Builder =
        HttpUrl.Builder().scheme("https").host(endpoint.address.host).port(endpoint.address.port)

    /** `POST /v1/transfers`: every file's offset, by file ID. */
    suspend fun createTransfer(id: String, files: List<UploadFileSpec>, origin: String? = null): ApiResult<Map<String, Long>> {
        val json = buildJsonObject {
            put("id", id)
            // 2B.4: `screenshot` or `camera` for an image the phone sent by itself; an old PC ignores the unknown field. Left out for everything else.
            if (origin != null) put("origin", origin)
            put("files", JsonArray(files.map { f ->
                buildJsonObject { put("fileId", f.fileId); put("name", f.name); put("size", f.size); put("mime", f.mime) }
            }))
        }.toString()
        val request = Request.Builder()
            .url(base().addPathSegments("v1/transfers").build())
            .post(json.toRequestBody(JSON))
            .build()
        return call(request) { response ->
            if (response.code != 200) return@call errorOf(response)
            val parsed = parse(response)
            val array = parsed?.get("files") as? JsonArray ?: return@call ApiResult.Error(200, "bad_response", "The PC's answer has no files.")
            val offsets = HashMap<String, Long>()
            for (entry in array) {
                val file = entry as? JsonObject
                val fileId = Fields.string(file?.get("fileId"))
                val offset = Fields.integer(file?.get("offset"))
                if (fileId == null || offset == null) return@call ApiResult.Error(200, "bad_response", "The PC's answer is not valid.")
                offsets[fileId] = offset
            }
            ApiResult.Ok(offsets)
        }
    }

    /** `PUT`: the bytes of [body] from [offset] to the end. `200` gives the byte count the PC has (`received`). */
    suspend fun put(id: String, fileId: String, offset: Long, body: RequestBody): ApiResult<Long> {
        val request = Request.Builder()
            .url(base().addPathSegments("v1/transfers").addPathSegment(id).addPathSegment("files").addPathSegment(fileId)
                .addQueryParameter("offset", offset.toString()).build())
            .put(body)
            .build()
        return call(request) { response ->
            if (response.code != 200) return@call errorOf(response)
            val received = Fields.integer(parse(response)?.get("received"))
                ?: return@call ApiResult.Error(200, "bad_response", "The PC's answer has no received count.")
            ApiResult.Ok(received)
        }
    }

    /** `DELETE /v1/transfers/{id}`: `204`, or `404` (a retried DELETE: the transfer is already gone), both mean done. */
    suspend fun delete(id: String): ApiResult<Unit> {
        val request = Request.Builder()
            .url(base().addPathSegments("v1/transfers").addPathSegment(id).build())
            .delete()
            .build()
        return call(request) { response ->
            when (response.code) {
                204, 404 -> ApiResult.Ok(Unit)
                else -> errorOf(response)
            }
        }
    }

    /**
     * `GET …/files/{fileId}`, with `Range: bytes=[rangeFrom]-` when resuming. [handle] reads the response (any status) and
     * runs on an IO thread; reading the body there is cancelled with the coroutine. The caller never passes a range for a
     * 0-byte file (protocol.md).
     */
    suspend fun <T> download(id: String, fileId: String, rangeFrom: Long?, handle: (Response) -> T): T {
        val builder = Request.Builder()
            .url(base().addPathSegments("v1/transfers").addPathSegment(id).addPathSegment("files").addPathSegment(fileId).build())
            .get()
            // No compression: the bytes are saved as they come, and Content-Length counts them.
            .header("Accept-Encoding", "identity")
        if (rangeFrom != null) builder.header("Range", "bytes=$rangeFrom-")
        return call(builder.build(), handle)
    }

    private suspend fun <T> call(request: Request, handle: (Response) -> T): T {
        val call = endpoint.client.newCall(request)
        return coroutineScope {
            // When the coroutine is cancelled the call is cancelled too, which unblocks a blocked read or write.
            val watcher = launch(start = CoroutineStart.UNDISPATCHED) {
                try { awaitCancellation() } finally { call.cancel() }
            }
            try {
                withContext(Dispatchers.IO) { call.execute().use(handle) }
            } finally {
                watcher.cancel()
            }
        }
    }

    private fun parse(response: Response): JsonObject? = try {
        Json.parseToJsonElement(response.peekBody(MAX_JSON_BYTES).string()) as? JsonObject
    } catch (_: Exception) {
        null
    }

    private fun errorOf(response: Response): ApiResult.Error {
        val body = if (response.code in 200..299) null else parse(response)
        return ApiResult.Error(
            status = response.code,
            code = Fields.string(body?.get("error")) ?: "http_${response.code}",
            message = Fields.string(body?.get("message")) ?: response.message.ifEmpty { "HTTP ${response.code}" },
            offset = Fields.integer(body?.get("offset")),
        )
    }

    private companion object {
        val JSON = "application/json; charset=utf-8".toMediaType()
        const val MAX_JSON_BYTES = 64L * 1024
    }
}

/**
 * The raw bytes of one file from [offset] to its end, read from a stream opened at [offset]. `Content-Length` is
 * `length` (no chunked encoding, as protocol.md says). A source that ends early is [SourceShortException]. [onBytes] gets
 * the count of every chunk sent, which is how progress is shown. Never retried by OkHttp on its own ([isOneShot]).
 */
class SourceRequestBody(
    private val open: () -> InputStream,
    private val length: Long,
    private val onBytes: (Long) -> Unit,
) : RequestBody() {
    override fun contentType() = OCTET_STREAM
    override fun contentLength(): Long = length
    override fun isOneShot(): Boolean = true

    override fun writeTo(sink: BufferedSink) {
        if (length == 0L) return
        open().use { stream ->
            val source = stream.source()
            val buffer = Buffer()
            var remaining = length
            while (remaining > 0) {
                val read = source.read(buffer, minOf(CHUNK_BYTES, remaining))
                if (read < 0) throw SourceShortException("the file ended $remaining bytes early")
                sink.write(buffer, read)
                remaining -= read
                onBytes(read)
            }
        }
    }

    private companion object {
        val OCTET_STREAM = "application/octet-stream".toMediaType()
        const val CHUNK_BYTES = 256L * 1024
    }
}
