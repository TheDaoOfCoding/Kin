package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.link.HttpEndpoint
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.protocol.FileNames
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withContext
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Writing the received bytes failed (no space, the entry can't be written). It isn't the network's fault, so it isn't retried. */
class SinkException(message: String, cause: Throwable? = null) : IOException(message, cause)

/** The PC sent a different number of bytes than the file has. */
class SizeMismatchException(message: String) : IOException(message)

/**
 * Receives one PC → phone transfer (protocol.md, "Files: PC → phone"). The `offer` has been acked already. For each file, started in the
 * order of the offer and up to [TransferTimings.maxParallelFiles] at a time ([downloadAll]):
 * create a pending entry (photos to `Pictures/Kin`, videos to `Movies/Kin`, the rest to `Download/Kin`, through [SaveSink]), `GET` it
 * (with `Range: bytes=N-` when resuming, never for a 0-byte file), append to the entry, and when it is complete clear the pending flag.
 * After the last file it sends `complete`: the message is written down with the status ([OwedMessages]) and sent once; if the PC doesn't
 * ack it, it is sent again after every `hello` until it does.
 *
 * - A dropped connection resumes: the entry keeps its bytes, and the next `GET` asks for the rest.
 * - `200` to a `Range` request restarts the file; `416` starts it over once.
 * - `404` or `409` (the offer expired, was cancelled, or the file changed) fails the whole offer: the partial files are deleted, a `cancel`
 *   with reason `error` goes to the PC (written down and sent again after every `hello` until acked), and files already saved completely stay.
 */
class DownloadRunner(
    private val d: TransferDeps,
    private val timings: TransferTimings = TransferTimings(),
    private val httpFor: (HttpEndpoint) -> TransferHttp = ::TransferHttp,
    private val mimeForExtension: (String) -> String? = { null },
) {
    private class ConnectionLost : Exception("the connection to the PC ended")

    /** One file ended the attempt for good (or as a retry); it unwinds the other files that are in flight. Never leaves the runner. */
    private class StopAttempt(val attempt: Attempt) : RuntimeException(null, null, false, false)

    private sealed interface Attempt {
        data object Done : Attempt

        /** The offer can't be finished: the PC dropped it, the file changed, or the phone can't write. */
        data class Fatal(val code: String, val message: String) : Attempt
        data class Retry(val moved: Boolean) : Attempt
    }

    /** What one `GET` ended with. */
    private sealed interface Got {
        data class Saved(val bytes: Long) : Got
        data class Gone(val status: Int, val code: String) : Got
        data object Unsatisfiable : Got
        data class Other(val status: Int) : Got
        data class BadRange(val header: String?) : Got
    }

    private val owedMessages = OwedMessages(d, timings.ackTimeoutMillis)

    suspend fun run(transferId: String): RunResult {
        val transfer = d.store.transfer(transferId) ?: return RunResult.Failed("not_found", "That transfer is gone.")
        return Run(transfer, d.store.files(transferId)).execute()
    }

    private inner class Run(var transfer: TransferEntity, initialFiles: List<TransferFileEntity>) {
        /** Replaced as a whole (never changed in place); several files change at once, so every change goes through [replaceFile]. */
        @Volatile var files: List<TransferFileEntity> = initialFiles
        private val filesLock = Any()

        /** One entry is made at a time: two files of one offer may have the same name, and each must see the name the other just took. */
        private val entryLock = Mutex()

        /** The names in each gallery folder, read once per attempt (not once per file): what the app has plus what this attempt creates. */
        private var namesInFolder = HashMap<SaveFolder, MutableSet<String>>()
        private var claimedByOthers: Set<String>? = null

        val done = AtomicLong(0)
        val movedThisAttempt = AtomicLong(0)

        val throttle = ProgressThrottle()

        suspend fun execute(): RunResult = try {
            run()
        } finally {
            // What was really written is saved (the free-space check of a new offer reads it), then the live numbers go.
            withContext(NonCancellable) {
                persistProgress()
                d.progress.clear(transfer.id)
            }
        }

        private suspend fun run(): RunResult {
            if (!setStatus(TransferStatus.ACTIVE)) return RunResult.Canceled
            var stalled = 0
            // The link was found dead (a silent network drop): the next wait for a connection asks the loop to try at once.
            var kickNext = false
            while (true) {
                currentCoroutineContext().ensureActive()
                // Before every attempt: this transfer belongs to one PC, and only that PC is ever asked for it.
                if (!d.isPairedWith(transfer)) return peerChanged()
                val connection = d.connectionFor(transfer, timings.reconnectWaitMillis, kick = kickNext) ?: return park()
                kickNext = false
                if (!d.isPairedWith(transfer)) return peerChanged()
                val endpoint = connection.httpEndpoint ?: return failAndCancel(connection, "internal", "The connection has no file channel.")
                movedThisAttempt.set(0)
                val attempt = try {
                    whileConnected(connection) { attempt(httpFor(endpoint)) }
                } catch (_: ConnectionLost) {
                    d.log.w("transfer", "download ${transfer.id}: the connection ended; waiting for the PC")
                    Attempt.Retry(movedThisAttempt.get() > 0)
                }
                when (attempt) {
                    Attempt.Done -> return finish(connection)
                    is Attempt.Fatal -> return failAndCancel(connection, attempt.code, attempt.message)
                    is Attempt.Retry -> {
                        persistProgress()
                        // See UploadRunner: a broken attempt on a link that still says "ready" is checked before it counts against the
                        // transfer, and a network that went silent is waited out instead of failing the transfer.
                        if (!connection.foundDead && connection.isReady) connection.checkAlive(timings.probeTimeoutMillis)
                        if (connection.foundDead) {
                            d.log.w("transfer", "download ${transfer.id}: the PC doesn't answer; waiting for a new connection")
                            stalled = 0
                            kickNext = true
                        } else {
                            stalled = if (attempt.moved) 0 else stalled + 1
                        }
                        if (stalled >= timings.maxStalledAttempts) {
                            return failAndCancel(connection, "connection_lost", "The connection to ${transfer.peerName} keeps dropping.")
                        }
                        d.notifier.progress(transfer, files, done.get(), waiting = true)
                        delay(timings.retryDelayMillis)
                    }
                }
            }
        }

        private suspend fun <T> whileConnected(connection: LinkConnection, block: suspend () -> T): T = coroutineScope {
            val watcher = launch {
                connection.awaitEnd()
                throw ConnectionLost()
            }
            val guard = launch {
                watchForStall(connection, timings, d.log, "download ${transfer.id}", movedThisAttempt::get)
            }
            try {
                block()
            } finally {
                watcher.cancel()
                guard.cancel()
            }
        }

        private suspend fun attempt(http: TransferHttp): Attempt {
            try {
                done.set(files.filter { it.finished }.sumOf { it.size })
                namesInFolder = HashMap()
                claimedByOthers = null
                return downloadAll(http)
            } catch (e: CancellationException) {
                throw e
            } catch (e: SinkException) {
                d.log.w("transfer", "download ${transfer.id}: can't write: ${e.message}")
                val full = e.cause?.message?.contains("ENOSPC") == true || e.cause?.message?.contains("No space") == true
                return Attempt.Fatal(if (full) "disk_full" else "internal", if (full) "The phone is out of space." else "Kin couldn't save the file.")
            } catch (e: SizeMismatchException) {
                d.log.w("transfer", "download ${transfer.id}: ${e.message}")
                return Attempt.Fatal("internal", "${transfer.peerName} sent a file of the wrong size.")
            } catch (e: IOException) {
                currentCoroutineContext().ensureActive()
                d.log.w("transfer", "download ${transfer.id}: ${e.javaClass.simpleName}: ${e.message}")
                return Attempt.Retry(movedThisAttempt.get() > 0)
            }
        }

        /**
         * Fetches the unfinished files with up to [TransferTimings.maxParallelFiles] `GET`s at a time, one request per file (a file is never
         * split), in the order of the offer: a new file starts the moment a slot frees. One producer makes the entries (gallery rows) of the
         * next files ahead of time, so the requests don't wait for MediaStore between two files, and, being one, it also hands out names
         * one at a time.
         *
         * - A file that fails for good (`Fatal`) stops the files that are in flight and ends the attempt.
         * - A file whose connection broke, or that the PC answered oddly, only fails *that* file for this attempt (`Retry`): the others go on
         *   and finish, and the next attempt resumes what is missing with `Range`.
         */
        private suspend fun downloadAll(http: TransferHttp): Attempt {
            val todo = files.filter { !it.finished }
            if (todo.isEmpty()) return Attempt.Done
            val parallel = timings.maxParallelFiles.coerceIn(1, todo.size)
            val retry = AtomicBoolean(false)
            val ready = Channel<TransferFileEntity>(capacity = parallel)
            try {
                coroutineScope {
                    launch {
                        try {
                            for (file in todo) ready.send(ensureEntry(file))
                        } finally {
                            ready.close()
                        }
                    }
                    repeat(parallel) {
                        launch {
                            for (entry in ready) {
                                when (val outcome = receiveGuarded(http, entry)) {
                                    null -> Unit
                                    is Attempt.Retry -> retry.set(true)
                                    else -> throw StopAttempt(outcome)
                                }
                            }
                        }
                    }
                }
            } catch (e: StopAttempt) {
                return e.attempt
            }
            return if (retry.get()) Attempt.Retry(movedThisAttempt.get() > 0) else Attempt.Done
        }

        /** [receive] for one of several files: a broken connection fails only this file; a file that can't be written ends the attempt. */
        private suspend fun receiveGuarded(http: TransferHttp, entry: TransferFileEntity): Attempt? = try {
            receive(http, entry)
        } catch (e: CancellationException) {
            throw e
        } catch (e: SinkException) {
            throw e
        } catch (e: SizeMismatchException) {
            throw e
        } catch (e: IOException) {
            currentCoroutineContext().ensureActive()
            d.log.w("transfer", "download ${transfer.id}: ${e.javaClass.simpleName}: ${e.message}")
            Attempt.Retry(movedThisAttempt.get() > 0)
        }

        /** One file, whose entry exists. Null when it is saved completely. */
        private suspend fun receive(http: TransferHttp, start: TransferFileEntity): Attempt? {
            var file = start
            var restarted = false
            var recreated = 0
            // How many bytes of this file are counted in `done` now (files run side by side, so `done` is only ever added to).
            var credited = 0L
            while (true) {
                val uri = file.localUri!!
                val length = d.sink.length(uri)
                if (length == null) {
                    // The user deleted the half-written entry (or the gallery did): make a new one and start over. But not forever: an
                    // entry that is "gone" right after it was made means the system can't hold it, and that must fail the file, not fill
                    // the gallery with empty entries (seen on the emulator before this limit existed).
                    if (++recreated > MAX_RECREATED_ENTRIES) throw SinkException("the entry for file ${file.fileId} keeps disappearing")
                    d.sink.delete(uri)
                    file = ensureEntry(file.copy(localUri = null, finalName = null, bytesDone = 0))
                    continue
                }
                val have = if (length > file.size) 0L else length
                d.progress.set(transfer.id, file.fileId, have)
                done.addAndGet(have - credited)
                credited = have
                if (have == file.size && file.size > 0) return finishFile(file)
                // No Range for a 0-byte file (any Range on it is unsatisfiable), none when starting from 0.
                val from = if (file.size > 0 && have > 0) have else null

                d.wake.acquire()
                val got = try {
                    http.download(transfer.id, file.fileId, from) { response -> read(response, file, uri, from) }
                } finally {
                    d.wake.release()
                }
                when (got) {
                    is Got.Saved -> {
                        val total = d.sink.length(uri) ?: 0
                        if (total == file.size) return finishFile(file)
                        d.store.setBytesDone(transfer.id, file.fileId, total)
                        // The connection ended early (or the PC sent fewer bytes): resume with the next attempt.
                        return Attempt.Retry(movedThisAttempt.get() > 0)
                    }
                    is Got.Gone -> return Attempt.Fatal(got.code, goneMessage(got))
                    Got.Unsatisfiable -> {
                        if (restarted) return Attempt.Fatal("bad_range", "${transfer.peerName} can't send ${file.name}.")
                        restarted = true
                        file = file.copy(bytesDone = 0)
                        // Starting over: a plain GET after the entry is truncated.
                        d.sink.openForWrite(uri, 0).close()
                    }
                    is Got.BadRange -> {
                        if (restarted) return Attempt.Fatal("bad_range", "${transfer.peerName} answered the resume request wrongly.")
                        restarted = true
                        d.sink.openForWrite(uri, 0).close()
                    }
                    is Got.Other -> {
                        d.log.w("transfer", "download ${transfer.id}: GET answered ${got.status}")
                        return Attempt.Retry(false)
                    }
                }
            }
        }

        private fun goneMessage(got: Got.Gone): String = when (got.code) {
            "source_changed" -> "A file changed on ${transfer.peerName} after it was offered."
            else -> "${transfer.peerName} doesn't have this transfer any more."
        }

        /** Reads one response into the entry. Runs on an IO thread. */
        private fun read(response: okhttp3.Response, file: TransferFileEntity, uri: String, from: Long?): Got {
            when (response.code) {
                404, 409 -> {
                    val code = if (response.code == 409) "source_changed" else "not_found"
                    return Got.Gone(response.code, code)
                }
                416 -> return Got.Unsatisfiable
                200, 206 -> Unit
                else -> return Got.Other(response.code)
            }
            val body = response.body
            val writeFrom: Long = when {
                response.code == 200 -> 0L // a 200 to a Range request restarts the file at byte 0
                else -> {
                    val header = response.header("Content-Range")
                    val match = header?.let { CONTENT_RANGE.matchEntire(it.trim()) }
                    val first = match?.groupValues?.get(1)?.toLongOrNull()
                    val total = match?.groupValues?.get(3)?.toLongOrNull()
                    if (from == null || first != from || total != file.size) return Got.BadRange(header)
                    from
                }
            }
            // A `200` to a Range request starts the file over: what was counted for it so far is not part of the progress any more.
            val counted = from ?: 0L
            if (writeFrom < counted) {
                done.addAndGet(writeFrom - counted)
                d.progress.set(transfer.id, file.fileId, writeFrom)
            }
            val expected = file.size - writeFrom
            val declared = body.contentLength()
            if (declared >= 0 && declared != expected) throw SizeMismatchException("expected $expected bytes, the PC announced $declared")
            var written = 0L
            val out = try {
                d.sink.openForWrite(uri, writeFrom)
            } catch (e: IOException) {
                throw SinkException("can't open the entry", e)
            }
            out.use { stream ->
                val source = body.source()
                val buffer = ByteArray(BUFFER_BYTES)
                while (true) {
                    val n = source.read(buffer)
                    if (n < 0) break
                    if (written + n > expected) throw SizeMismatchException("the PC sent more than $expected bytes")
                    try {
                        stream.write(buffer, 0, n)
                    } catch (e: IOException) {
                        throw SinkException("can't write the entry", e)
                    }
                    written += n
                    movedThisAttempt.addAndGet(n.toLong())
                    d.progress.set(transfer.id, file.fileId, writeFrom + written)
                    done.addAndGet(n.toLong())
                    report()
                }
            }
            return Got.Saved(writeFrom + written)
        }

        /** The pending entry for [file]: the one it has, or a new one named per protocol.md, "File names". */
        private suspend fun ensureEntry(file: TransferFileEntity): TransferFileEntity = entryLock.withLock {
            val uri = file.localUri
            if (uri != null && d.sink.length(uri) != null) return@withLock file
            val cleaned = FileNames.clean(file.name, emptyList())
            val classified = MediaKinds.classify(cleaned, file.mime, mimeForExtension)
            // `existing`: what this app already has in the folder, what unfinished transfers claimed, what this transfer saved. The first two are
            // read once per attempt (a gallery query per file adds up over hundreds of files); the names this attempt creates are added as it goes.
            val inFolder = namesInFolder.getOrPut(classified.folder) { HashSet(d.sink.existingNames(classified.folder)) }
            val claimed = claimedByOthers ?: d.store.claimedNames().also { claimedByOthers = it }
            val existing = HashSet<String>().apply {
                addAll(inFolder)
                addAll(claimed)
                files.forEach { f -> if (f.fileId != file.fileId) f.finalName?.let(::add) }
            }
            var created: SavedEntry? = null
            var requested = cleaned
            val result = FileNames.createNew(file.name, existing) { candidate ->
                requested = candidate
                try {
                    val entry = d.sink.create(classified.folder, candidate, classified.mime)
                    created = entry
                    FileNames.Created.Name(entry.finalName)
                } catch (e: NameTakenException) {
                    FileNames.Created.AlreadyExists
                } catch (e: IOException) {
                    FileNames.Created.Failed(e.message ?: "create failed")
                }
            }
            val entry = created
            if (result is FileNames.CreateResult.Failed || entry == null) {
                throw SinkException("can't create file ${file.fileId}: ${(result as? FileNames.CreateResult.Failed)?.reason}")
            }
            // File names are never logged (they are the user's): only the fact.
            if (entry.finalName != requested) d.log.i("transfer", "the system saved file ${file.fileId} under another name than the one asked for")
            val updated = file.copy(localUri = entry.uri, finalName = entry.finalName, bytesDone = 0, finished = false)
            try {
                d.store.updateFile(updated)
            } catch (e: Throwable) {
                // The entry exists but no row owns it: it would stay pending for ever.
                runCatching { d.sink.delete(entry.uri) }
                throw e
            }
            inFolder += entry.finalName
            replaceFile(updated)
            updated
        }

        /** Several files change at once: the list is replaced under a lock, so no update is lost. */
        private fun replaceFile(updated: TransferFileEntity) {
            synchronized(filesLock) { files = files.map { if (it.fileId == updated.fileId) updated else it } }
        }

        private suspend fun finishFile(file: TransferFileEntity): Attempt? {
            val uri = file.localUri!!
            if (d.sink.length(uri) != file.size) throw SizeMismatchException("file ${file.fileId} doesn't have ${file.size} bytes")
            d.sink.finish(uri)
            val finished = file.copy(bytesDone = file.size, finished = true)
            d.store.updateFile(finished)
            replaceFile(finished)
            return null
        }

        /** At most one update per 500 ms and never forced: the result goes out under its own notification ID. */
        private fun report() {
            if (throttle.tryReport(d.nowMillis())) d.notifier.progress(transfer, files, done.get(), waiting = false)
        }

        /** The status of a transfer that isn't over; false when it was already over (canceled, done): then nothing here changes it. */
        private suspend fun setStatus(status: String, code: String? = null, message: String? = null, owed: String? = null, owedId: String? = null): Boolean {
            val finishedAt = if (TransferStatus.isFinished(status)) d.nowMillis() else null
            val changed = d.store.setStatus(transfer.id, status, code, message, finishedAt, owed, owedId)
            if (changed) transfer = transfer.copy(status = status, errorCode = code, message = message, finishedAtMillis = finishedAt, owed = owed, owedId = owedId)
            return changed
        }

        /** Saves how many bytes each unfinished file really has (the live numbers), so a stopped download is counted right. */
        private suspend fun persistProgress() {
            for (file in files) {
                if (file.finished) continue
                val live = d.progress.get(transfer.id, file.fileId) ?: continue
                runCatching { d.store.setBytesDone(transfer.id, file.fileId, live) }
            }
        }

        /**
         * Every file is saved: the transfer is done and owes the PC a `complete` (written in the same step as the status). It is sent now; if
         * nobody acks it, it is sent again after every `hello`, under the same ID, until the PC does.
         */
        private suspend fun finish(connection: LinkConnection): RunResult {
            if (!setStatus(TransferStatus.DONE, owed = OwedMessages.COMPLETE, owedId = OwedMessages.newMessageId())) return RunResult.Canceled
            d.log.i("transfer", "download ${transfer.id} done: ${files.size} files, ${files.sumOf { it.size }} bytes")
            d.notifier.received(transfer, files)
            owedMessages.send(connection, transfer.id)
            return RunResult.Done
        }

        /** A file failed for good: delete the partial files, fail the transfer, and tell the PC (`cancel`, reason `error`: owed until acked). */
        private suspend fun failAndCancel(connection: LinkConnection?, code: String, message: String): RunResult {
            discardPartials(d, transfer.id)
            files = d.store.files(transfer.id)
            if (!setStatus(TransferStatus.FAILED, code, message, owed = OwedMessages.cancel("error"), owedId = OwedMessages.newMessageId())) return RunResult.Canceled
            // The message names files and the PC; the log gets the code only.
            d.log.w("transfer", "download ${transfer.id} failed: $code")
            d.notifier.failed(transfer, message)
            owedMessages.send(connection, transfer.id)
            return RunResult.Failed(code, message)
        }

        /** The paired PC is not the one that offered these files (it was unpaired, or another PC was paired): nothing is asked of any PC. */
        private suspend fun peerChanged(): RunResult {
            val message = "These files came from ${transfer.peerName}, which is no longer paired."
            discardPartials(d, transfer.id)
            files = d.store.files(transfer.id)
            if (!setStatus(TransferStatus.FAILED, PEER_CHANGED, message)) return RunResult.Canceled
            d.log.w("transfer", "download ${transfer.id} failed: $PEER_CHANGED")
            d.notifier.failed(transfer, message)
            return RunResult.Failed(PEER_CHANGED, message)
        }

        private suspend fun park(): RunResult {
            val reason = d.link.unreachableReason()
            if (!setStatus(TransferStatus.QUEUED, "unreachable", reason)) return RunResult.Canceled
            d.log.w("transfer", "download ${transfer.id}: the PC is not reachable ($reason); it resumes when the PC is back")
            d.notifier.queued(transfer, reason)
            return RunResult.Waiting
        }
    }

    companion object {
        private val CONTENT_RANGE = Regex("bytes (\\d+)-(\\d+)/(\\d+)")
        private const val BUFFER_BYTES = 256 * 1024
        private const val MAX_RECREATED_ENTRIES = 2

        /** The error code of a transfer whose PC is not the paired one any more. */
        const val PEER_CHANGED = "peer_changed"

        /** Deletes the entries of the files that aren't saved completely and forgets them. Files saved completely stay. */
        suspend fun discardPartials(d: TransferDeps, transferId: String) {
            for (file in d.store.files(transferId)) {
                if (file.finished) continue
                file.localUri?.let { runCatching { d.sink.delete(it) } }
                d.store.updateFile(file.copy(localUri = null, finalName = null, bytesDone = 0))
            }
        }
    }
}
