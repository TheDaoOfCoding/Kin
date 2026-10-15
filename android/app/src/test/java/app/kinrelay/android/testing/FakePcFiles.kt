package app.kinrelay.android.testing

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import mockwebserver3.MockResponse
import mockwebserver3.RecordedRequest
import mockwebserver3.SocketEffect
import okio.Buffer
import okio.ByteString.Companion.toByteString

/**
 * The file endpoints of the fake PC (protocol.md, "Files"): `POST /v1/transfers`, `PUT …/files/{fileId}?offset=N`, `DELETE
 * /v1/transfers/{id}` for uploads from the phone, and `GET …/files/{fileId}` (with `Range`) for the offers it sends. It keeps what it
 * received in memory, can fail on demand (`507`, `413`, `500`, a stale offset, a dropped connection) and remembers every call.
 */
class FakePcFiles {
    class UploadedFile(val fileId: String, val name: String, val size: Long, val mime: String) {
        @Volatile var data = ByteArray(0)
        val complete: Boolean get() = data.size.toLong() == size
    }

    class Upload(val id: String, val files: LinkedHashMap<String, UploadedFile>)

    class OfferedFile(val fileId: String, val name: String, val mime: String, val bytes: ByteArray) {
        /** The file changed after the offer: `GET` answers `409 source_changed`. */
        @Volatile var changed = false
    }

    class Offered(val id: String, val files: LinkedHashMap<String, OfferedFile>) {
        /** The offer expired or was cancelled: every `GET` answers `404`. */
        @Volatile var gone = false
    }

    class Call(val method: String, val path: String, val offset: Long?, val range: String?, val contentLength: Long?, val bodyBytes: Long)

    val uploads = ConcurrentHashMap<String, Upload>()
    val offers = ConcurrentHashMap<String, Offered>()
    val deleted = CopyOnWriteArrayList<String>()

    /** Transfer IDs the PC has used up (canceled, failed, or forgotten after 24 hours): a `POST`, `PUT` or `DELETE` with one is `404`, and nothing is created. */
    val usedIds: MutableSet<String> = ConcurrentHashMap.newKeySet()
    val calls = CopyOnWriteArrayList<Call>()

    /** Answer `POST /v1/transfers` with this status (`507 disk_full`, `413 too_large`, `500 internal`). */
    @Volatile var postStatus: Int? = null

    /** Answer `PUT` with this status. */
    @Volatile var putStatus: Int? = null

    /** The first `POST` for an existing transfer reports this offset for its first file (a stale answer, so the next `PUT` gets `409`). */
    @Volatile var staleOffsetOnce: Long? = null

    /** The next `PUT` keeps only this many bytes of its body and drops the connection without an answer (the PC "keeps what arrived"). */
    @Volatile var dropPutKeeping: Long? = null

    /** The next `PUT` stores everything and drops the connection without answering: the final `200` is lost. */
    @Volatile var loseFinalResponseOnce = false

    /** Runs when a `PUT` is dropped (before the connection closes): a test changes the shared file here, so the retry finds it changed. */
    @Volatile var afterDrop: (() -> Unit)? = null

    /** Every `GET` body is sent at most this many bytes per 100 ms (a slow download, to cancel it in flight). */
    @Volatile var throttleGetBytesPer100Ms: Long? = null

    /** The next `GET` sends only this many bytes of the body (with the full Content-Length) and closes the connection. */
    @Volatile var dropGetAfter: Long? = null

    /** Answer a `GET` that has a `Range` with `200` and the whole file (the PC ignores the range). */
    @Volatile var ignoreRange = false

    /** Messages from the phone about transfers (`complete`, `cancel`), as `type:transferId:reason`. */
    val messages = CopyOnWriteArrayList<String>()

    /** What to answer to `complete` and `cancel` from the phone: ack them (`true`), or say nothing. */
    @Volatile var ackMessages = true

    fun offered(id: String, vararg files: OfferedFile): Offered =
        Offered(id, LinkedHashMap(files.associateBy { it.fileId })).also { offers[id] = it }

    /** The `offer` frame for [offered]. */
    fun offerFrame(offered: Offered): String = buildJsonObject {
        put("type", "offer")
        put("id", offered.id)
        put("files", buildJsonArray {
            offered.files.values.forEach { f ->
                add(buildJsonObject { put("fileId", f.fileId); put("name", f.name); put("size", f.bytes.size.toLong()); put("mime", f.mime) })
            }
        })
        put("totalSize", offered.files.values.sumOf { it.bytes.size.toLong() })
    }.toString()

    /** A frame from the phone about a transfer. Returns the ack to send, if any. */
    fun onFrame(frame: JsonObject): String? {
        val type = frame["type"]?.jsonPrimitive?.content
        val id = frame["id"]?.jsonPrimitive?.content
        if ((type == "complete" || type == "cancel") && id != null) {
            messages += "$type:${frame["transferId"]?.jsonPrimitive?.content}:${frame["reason"]?.jsonPrimitive?.content}"
            if (type == "cancel") offers[frame["transferId"]?.jsonPrimitive?.content]?.gone = true
            return if (ackMessages) """{"type":"ack","ref":"$id"}""" else null
        }
        return null
    }

    fun handle(request: RecordedRequest): MockResponse {
        val segments = request.url.encodedPath.trim('/').split('/')
        val bodyBytes = request.body?.toByteArray() ?: ByteArray(0)
        val query = request.url.queryParameter("offset")?.toLongOrNull()
        calls += Call(request.method, request.url.encodedPath, query, request.headers["Range"], request.headers["Content-Length"]?.toLongOrNull(), bodyBytes.size.toLong())
        return when {
            segments.size == 2 && request.method == "POST" -> create(bodyBytes)
            segments.size == 3 && request.method == "DELETE" -> delete(segments[2])
            segments.size == 5 && segments[3] == "files" && request.method == "PUT" -> put(segments[2], segments[4], query, bodyBytes, request)
            segments.size == 5 && segments[3] == "files" && request.method == "GET" -> get(segments[2], segments[4], request.headers["Range"])
            else -> error(404, "not_found", "Unknown route.")
        }
    }

    private fun create(body: ByteArray): MockResponse {
        postStatus?.let { return error(it, codeFor(it), "test") }
        val root = try { Json.parseToJsonElement(String(body, Charsets.UTF_8)).jsonObject } catch (e: Exception) { return error(400, "bad_request", "not json") }
        val id = root["id"]?.jsonPrimitive?.content ?: return error(400, "bad_request", "no id")
        val files = (root["files"] as? JsonArray)?.map { it.jsonObject }?.map {
            UploadedFile(it.getValue("fileId").jsonPrimitive.content, it.getValue("name").jsonPrimitive.content, it.getValue("size").jsonPrimitive.content.toLong(), it.getValue("mime").jsonPrimitive.content)
        } ?: return error(400, "bad_request", "no files")
        if (id in usedIds) return error(404, "not_found", "Transfer not found.")
        val existing = uploads[id]
        val upload = if (existing == null) {
            Upload(id, LinkedHashMap(files.associateBy { it.fileId })).also { uploads[id] = it }
        } else {
            if (existing.files.keys != files.map { it.fileId }.toSet()) return error(400, "bad_request", "different files")
            existing
        }
        val stale = staleOffsetOnce.also { if (existing != null) staleOffsetOnce = null }
        val answer = buildJsonObject {
            put("id", id)
            put("files", buildJsonArray {
                // The order of the request, with how many bytes the PC has of each file.
                files.forEachIndexed { index, f ->
                    val stored = upload.files.getValue(f.fileId)
                    val offset = if (index == 0 && stale != null) stale else stored.data.size.toLong()
                    add(buildJsonObject { put("fileId", f.fileId); put("offset", offset) })
                }
            })
        }
        return json(200, answer.toString())
    }

    private fun put(id: String, fileId: String, offset: Long?, body: ByteArray, request: RecordedRequest): MockResponse {
        putStatus?.let { return error(it, codeFor(it), "test") }
        val file = uploads[id]?.files?.get(fileId) ?: return error(404, "not_found", "Unknown transfer or file.")
        if (offset == null) return error(400, "bad_request", "offset")
        val stored = file.data.size.toLong()
        if (offset != stored) return json(409, """{"error":"offset_mismatch","message":"The PC has $stored bytes.","offset":$stored}""")
        if (offset + body.size != file.size) return error(400, "bad_request", "Content-Length is not size - offset")
        dropPutKeeping?.let { keep ->
            dropPutKeeping = null
            file.data += body.copyOfRange(0, keep.toInt().coerceAtMost(body.size))
            afterDrop?.invoke()
            return MockResponse.Builder().onResponseStart(SocketEffect.CloseSocket()).build()
        }
        file.data += body
        if (loseFinalResponseOnce) {
            loseFinalResponseOnce = false
            return MockResponse.Builder().onResponseStart(SocketEffect.CloseSocket()).build()
        }
        return json(200, """{"received":${file.size}}""")
    }

    private fun delete(id: String): MockResponse {
        deleted += id
        usedIds += id
        return if (uploads.remove(id) != null) MockResponse.Builder().code(204).build() else error(404, "not_found", "Transfer not found.")
    }

    private fun get(id: String, fileId: String, range: String?): MockResponse {
        val offered = offers[id]
        if (offered == null || offered.gone) return error(404, "not_found", "The offer is gone.")
        val file = offered.files[fileId] ?: return error(404, "not_found", "Unknown file.")
        if (file.changed) return error(409, "source_changed", "The file changed after it was offered.")
        val size = file.bytes.size
        var from = 0
        var partial = false
        if (range != null && !ignoreRange) {
            val n = Regex("bytes=([0-9]+)-").matchEntire(range)?.groupValues?.get(1)?.toLongOrNull() ?: return rangeError(size)
            if (n >= size) return rangeError(size)
            from = n.toInt()
            partial = true
        }
        val body = file.bytes.copyOfRange(from, size)
        val dropAfter = dropGetAfter
        val sent = if (dropAfter != null) body.copyOfRange(0, dropAfter.toInt().coerceAtMost(body.size)) else body
        val builder = MockResponse.Builder()
            .code(if (partial) 206 else 200)
            .body(Buffer().write(sent.toByteString()))
            .setHeader("Content-Type", "application/octet-stream")
            .setHeader("Accept-Ranges", "bytes")
            .setHeader("Content-Length", body.size.toString())
        if (partial) builder.setHeader("Content-Range", "bytes $from-${size - 1}/$size")
        throttleGetBytesPer100Ms?.let { builder.throttleBody(it, 100, java.util.concurrent.TimeUnit.MILLISECONDS) }
        if (dropAfter != null) {
            dropGetAfter = null
            builder.onResponseEnd(SocketEffect.CloseSocket())
        }
        return builder.build()
    }

    private fun rangeError(size: Int): MockResponse = MockResponse.Builder()
        .code(416)
        .setHeader("Content-Range", "bytes */$size")
        .setHeader("Content-Type", "application/json; charset=utf-8")
        .body("""{"error":"bad_request","message":"Range not satisfiable."}""")
        .build()

    private fun codeFor(status: Int) = when (status) {
        507 -> "disk_full"
        413 -> "too_large"
        500 -> "internal"
        401 -> "unauthorized"
        404 -> "not_found"
        else -> "bad_request"
    }

    fun error(status: Int, code: String, message: String): MockResponse = json(status, """{"error":"$code","message":"$message"}""")

    private fun json(status: Int, body: String) = MockResponse.Builder()
        .code(status)
        .addHeader("Content-Type", "application/json; charset=utf-8")
        .body(body)
        .build()

    @Suppress("unused")
    private fun primitive(value: String) = JsonPrimitive(value)
}
