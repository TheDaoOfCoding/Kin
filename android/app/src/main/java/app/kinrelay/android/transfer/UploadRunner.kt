package app.kinrelay.android.transfer

import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.link.HttpEndpoint
import app.kinrelay.android.link.LinkConnection
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Sends one phone → PC transfer (protocol.md, "Files: phone → PC"):
 *
 * 1. `POST /v1/transfers` with names, sizes and MIME types; the answer says how many bytes of each file the PC has.
 * 2. `PUT` each file, from that offset to the end (an already complete file gets an empty `PUT` at its size). Up to
 *    [TransferTimings.maxParallelFiles] files are on the wire at once, started in the order of the list ([uploadAll]).
 * 3. `409 offset_mismatch`: carry on from the offset the PC returns. `507`, `413`, `500` (and any other answer that
 *    isn't a success) fail the transfer for good, with no automatic retry.
 * 4. A dropped connection: wait for the PC to come back (no wake lock, no polling), then `POST` again, which resumes.
 * 5. A shared file that changed since it was shared: `DELETE` the old transfer and start over under a new transfer ID.
 * 6. `404` to a `POST` means the PC ended that transfer for good, or the ID belongs to another device, or it was used before and is over
 *    (protocol.md, "A transfer ID is never reused"). An **automatic** resume (a reconnect, the queue) marks the transfer failed ("the PC no
 *    longer has it") and never sends again. Only an explicit **user Retry** ([userRetry]) may: it has already `POST`ed the old ID (so a
 *    transfer the PC finished within 24 hours is recognised and just finishes), and on `404` it sends **only the files that never got a
 *    `200`** under a new transfer ID, as a new transfer ([TransferStore.supersede]).
 *
 * It stops (throws a cancellation) when its coroutine is cancelled. Cancelling on the PC (`DELETE`) is [TransferEngine]'s job.
 */
class UploadRunner(
    private val d: TransferDeps,
    private val timings: TransferTimings = TransferTimings(),
    private val httpFor: (HttpEndpoint) -> TransferHttp = ::TransferHttp,
    /** Called with the old and the new transfer ID when the transfer went on under a new ID (a changed source, or a user Retry after `404`). */
    private val onRenamed: (oldId: String, newId: String) -> Unit = { _, _ -> },
    /**
     * The user asked for this run (Retry). Only then may a `404` to the `POST` lead to a new transfer ID. A Retry made while the PC was
     * away is stored on the transfer ([TransferEntity.userRetry]) and counts too: the run that the queue flush starts later is still the
     * user's, until it has reached its `POST`.
     */
    private val userRetry: Boolean = false,
    /** The shared files were looked at a moment ago (the transfer was just made from them): the first attempt skips the "did it change" look. */
    private val sourcesJustChecked: Boolean = false,
) {
    /** The connection ended while a request was in flight: the attempt is abandoned and resumes on the next connection. */
    private class ConnectionLost : Exception("the connection to the PC ended")

    /** One file ended the attempt for good (or as a retry); it unwinds the other files that are in flight. Never leaves the runner. */
    private class StopAttempt(val attempt: Attempt) : RuntimeException(null, null, false, false)

    /**
     * A file that is about to go out, with its source already opened at [offset] so the request doesn't wait for the provider (the
     * "pipelining": the next files are made ready while the current ones are on the wire). A failure to open is not reported here: the
     * request opens the source again and meets the same failure where it is handled.
     */
    private class Prepared(val file: TransferFileEntity, val offset: Long, private var stream: InputStream?) {
        @Synchronized fun take(atOffset: Long): InputStream? =
            if (atOffset == offset) stream.also { stream = null } else null

        @Synchronized fun close() {
            runCatching { stream?.close() }
            stream = null
        }
    }

    private sealed interface Attempt {
        data object Done : Attempt
        data class Fatal(val code: String, val message: String) : Attempt

        /** Try again after the PC is back. [moved]: bytes went through in this attempt, so it wasn't stuck. */
        data class Retry(val moved: Boolean) : Attempt
    }

    suspend fun run(transferId: String): RunResult {
        val transfer = d.store.transfer(transferId) ?: return RunResult.Failed("not_found", "That transfer is gone.")
        return Run(transfer, d.store.files(transferId)).execute()
    }

    private inner class Run(var transfer: TransferEntity, initialFiles: List<TransferFileEntity>) {
        /** Replaced as a whole (never changed in place); several files finish at once, so every change goes through [markFileDone]. */
        @Volatile var files: List<TransferFileEntity> = initialFiles
        private val filesLock = Any()

        /** Cleared by the first attempt that uses it ([sourcesJustChecked]). */
        private var skipSourceCheck = sourcesJustChecked

        /** The PC may already know this transfer ID (a `POST` worked, or this is a recovery of an active transfer). */
        var started = transfer.status == TransferStatus.ACTIVE

        /** A user Retry found its ID used up and the unfinished files went on as a new transfer (only once per run). */
        var startedOver = false

        /** This run is the user's: asked for by the caller, or by a Retry that was stored while the PC was away. */
        val userRun = userRetry || transfer.userRetry
        val done = AtomicLong(0)
        val sentThisAttempt = AtomicLong(0)

        val throttle = ProgressThrottle()

        suspend fun execute(): RunResult {
            // Someone may have ended the transfer between the launch and now (a cancel from the queue screen): then there is nothing to run.
            if (!setStatus(TransferStatus.ACTIVE)) return RunResult.Canceled
            var stalled = 0
            // The link was found dead (a silent network drop): the next wait for a connection asks the loop to try at once.
            var kickNext = false
            while (true) {
                currentCoroutineContext().ensureActive()
                // Before every attempt: this transfer belongs to one PC, and a different PC must never get its files.
                if (!d.isPairedWith(transfer)) return peerChanged()
                val first = !started
                val connection = d.connectionFor(transfer, if (first) timings.connectWaitMillis else timings.reconnectWaitMillis, kick = first || kickNext)
                kickNext = false
                if (connection == null) return park(first)
                if (!d.isPairedWith(transfer)) return peerChanged()
                val endpoint = connection.httpEndpoint ?: return fail("internal", "The connection has no file channel.")

                d.store.addAttempt(transfer.id)
                sentThisAttempt.set(0)
                val attempt = try {
                    whileConnected(connection) { attempt(connection, httpFor(endpoint)) }
                } catch (_: ConnectionLost) {
                    d.log.w("transfer", "upload ${transfer.id}: the connection ended; waiting for the PC")
                    Attempt.Retry(sentThisAttempt.get() > 0)
                }
                when (attempt) {
                    Attempt.Done -> return finish()
                    is Attempt.Fatal -> return fail(attempt.code, attempt.message)
                    is Attempt.Retry -> {
                        // A broken attempt on a link that still says "ready" may be a network that went silent: ask the PC before counting
                        // this against the transfer. A dead link is the network's doing, not the transfer's: the attempts it cost don't count,
                        // and the transfer waits for the new connection (the queue's own limits apply) instead of failing.
                        if (!connection.foundDead && connection.isReady) connection.checkAlive(timings.probeTimeoutMillis)
                        if (connection.foundDead) {
                            d.log.w("transfer", "upload ${transfer.id}: the PC doesn't answer; waiting for a new connection")
                            stalled = 0
                            kickNext = true
                        } else {
                            stalled = if (attempt.moved) 0 else stalled + 1
                        }
                        if (stalled >= timings.maxStalledAttempts) {
                            return fail("connection_lost", "The connection to ${transfer.peerName} keeps dropping. Try again.")
                        }
                        d.notifier.progress(transfer, files, done.get(), waiting = true)
                        delay(timings.retryDelayMillis)
                    }
                }
            }
        }

        /** Runs [block] and abandons it if [connection] ends first. */
        private suspend fun <T> whileConnected(connection: LinkConnection, block: suspend () -> T): T = coroutineScope {
            val watcher = launch {
                connection.awaitEnd()
                throw ConnectionLost()
            }
            val guard = launch {
                watchForStall(connection, timings, d.log, "upload ${transfer.id}", sentThisAttempt::get)
            }
            try {
                block()
            } finally {
                watcher.cancel()
                guard.cancel()
            }
        }

        private suspend fun attempt(connection: LinkConnection, http: TransferHttp): Attempt {
            try {
                if (skipSourceCheck) skipSourceCheck = false else checkSources(http)?.let { return it }
                done.set(files.filter { it.finished }.sumOf { it.size })
                val offsets = when (val response = createOrStartOver(http)) {
                    is ApiResult.Error -> return fatalFor(response)
                    is ApiResult.Ok -> response.value
                }
                started = true
                if (!transfer.started) {
                    d.store.markStarted(transfer.id)
                    transfer = transfer.copy(started = true)
                }
                setStatus(TransferStatus.ACTIVE)
                val work = ArrayList<Pair<TransferFileEntity, Long>>()
                for (file in files) {
                    val offset = offsets[file.fileId] ?: return Attempt.Fatal("bad_response", "${transfer.peerName} answered with a different list of files.")
                    if (file.finished && file.size > 0) continue
                    work += file to offset
                }
                return uploadAll(http, work)
            } catch (e: CancellationException) {
                throw e
            } catch (e: SourceShortException) {
                return Attempt.Fatal("source_changed", "A file changed while it was being sent. Share it again.")
            } catch (e: SecurityException) {
                return Attempt.Fatal("source_lost", "Kin can't read the shared file any more. Share it again.")
            } catch (e: IOException) {
                // A cancelled call surfaces as an IOException ("Canceled"): that is a cancel, not a failed connection.
                currentCoroutineContext().ensureActive()
                d.log.w("transfer", "upload ${transfer.id}: ${e.javaClass.simpleName}: ${e.message}")
                return Attempt.Retry(sentThisAttempt.get() > 0)
            }
        }

        /**
         * Sends [work] (a file and the offset to continue from) with up to [TransferTimings.maxParallelFiles] `PUT`s at a time, in the order
         * of the list: a new file starts the moment a slot frees. A producer opens the source of the next files ahead of time, so a small
         * file doesn't wait for its provider between two requests. One request per file; a file is never split.
         *
         * - A file that ends the attempt for good (`Fatal`) stops the others that are in flight and ends the attempt.
         * - A file whose connection broke only fails *that* file for this attempt (`Retry`): the others go on and finish, and the next
         *   attempt (a new `POST` tells the PC's offsets) sends what is missing.
         */
        private suspend fun uploadAll(http: TransferHttp, work: List<Pair<TransferFileEntity, Long>>): Attempt {
            if (work.isEmpty()) return Attempt.Done
            val parallel = timings.maxParallelFiles.coerceIn(1, work.size)
            val retry = AtomicBoolean(false)
            // Sources that were opened ahead but never taken (the attempt ended early) are closed by `cancel` below.
            val prepared = Channel<Prepared>(capacity = parallel, onUndeliveredElement = { it.close() })
            try {
                coroutineScope {
                    launch {
                        try {
                            for ((file, offset) in work) prepared.send(prepare(file, offset))
                        } finally {
                            prepared.close()
                        }
                    }
                    repeat(parallel) {
                        launch {
                            for (next in prepared) {
                                val outcome = try {
                                    uploadGuarded(http, next)
                                } finally {
                                    next.close()
                                }
                                when (outcome) {
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
            } finally {
                prepared.cancel()
            }
            return if (retry.get()) Attempt.Retry(sentThisAttempt.get() > 0) else Attempt.Done
        }

        /** The source of a file opened at its offset, ahead of its request. If it can't be opened the request will try again and report it. */
        private fun prepare(file: TransferFileEntity, offset: Long): Prepared {
            val uri = file.localUri
            val stream = if (uri == null || offset < 0 || offset > file.size || file.size == offset) {
                null
            } else {
                try {
                    d.sources.open(uri, offset)
                } catch (_: IOException) {
                    null
                } catch (_: SecurityException) {
                    null
                }
            }
            return Prepared(file, offset, stream)
        }

        /** [upload] for one of several files: what ends the whole attempt is returned, a broken connection fails only this file. */
        private suspend fun uploadGuarded(http: TransferHttp, next: Prepared): Attempt? = try {
            upload(http, next)
        } catch (e: CancellationException) {
            throw e
        } catch (e: SourceShortException) {
            Attempt.Fatal("source_changed", "A file changed while it was being sent. Share it again.")
        } catch (e: SecurityException) {
            Attempt.Fatal("source_lost", "Kin can't read the shared file any more. Share it again.")
        } catch (e: IOException) {
            currentCoroutineContext().ensureActive()
            d.log.w("transfer", "upload ${transfer.id}: ${e.javaClass.simpleName}: ${e.message}")
            Attempt.Retry(sentThisAttempt.get() > 0)
        }

        /** A file is on the PC: its row says so. Several files finish at once, so the list is changed under a lock. */
        private suspend fun markFileDone(fileId: String, size: Long) {
            val finished = synchronized(filesLock) {
                files = files.map { if (it.fileId == fileId) it.copy(bytesDone = size, finished = true) else it }
                files.first { it.fileId == fileId }
            }
            d.store.updateFile(finished)
        }

        /** The PUT of one file, with the `409 offset_mismatch` loop. Null means the file is done. */
        private suspend fun upload(http: TransferHttp, first: Prepared): Attempt? {
            val file = first.file
            var offset = first.offset
            var counted = 0L
            var mismatches = 0
            val uri = file.localUri ?: return Attempt.Fatal("source_lost", "Kin can't read ${file.name} any more. Share it again.")
            while (true) {
                if (offset < 0 || offset > file.size) return Attempt.Fatal("bad_response", "${transfer.peerName} reports a wrong offset for ${file.name}.")
                done.addAndGet(offset - counted)
                counted = offset
                val at = offset
                // The pre-opened source serves the first request (same offset); a request after a `409` opens its own at the new offset.
                val body = SourceRequestBody({ first.take(at) ?: d.sources.open(uri, at) }, file.size - offset) { n ->
                    done.addAndGet(n)
                    counted += n
                    sentThisAttempt.addAndGet(n)
                    report()
                }
                d.wake.acquire()
                val result = try {
                    http.put(transfer.id, file.fileId, offset, body)
                } finally {
                    d.wake.release()
                }
                when (result) {
                    is ApiResult.Ok -> {
                        done.addAndGet(file.size - counted)
                        markFileDone(file.fileId, file.size)
                        return null
                    }
                    is ApiResult.Error -> {
                        if (result.status == 409 && result.code == "offset_mismatch") {
                            val actual = result.offset ?: return Attempt.Fatal("bad_response", "${transfer.peerName} didn't say where to continue.")
                            if (++mismatches > timings.maxOffsetMismatches) {
                                return Attempt.Fatal("offset_mismatch", "${transfer.peerName} and this phone don't agree where ${file.name} stands.")
                            }
                            d.log.i("transfer", "upload ${transfer.id}: offset mismatch for ${file.fileId}, continuing from $actual")
                            offset = actual
                            continue
                        }
                        d.store.setBytesDone(transfer.id, file.fileId, counted)
                        return fatalFor(result)
                    }
                }
            }
        }

        /**
         * Before every attempt: is each shared file still readable and unchanged? A change on a transfer the PC already knows
         * means `DELETE` and a new transfer ID (protocol.md). Returns a result to end the attempt with, or null to go on.
         */
        private suspend fun checkSources(http: TransferHttp): Attempt? {
            var changed = false
            // Looked at several at a time (each is a round trip to the app that owns the file); judged in the order of the files.
            val infos = files.mapParallel(timings.statParallelism) { file -> file.localUri?.let { d.sources.stat(it) } }
            for ((index, file) in files.withIndex()) {
                if (file.localUri == null) return lost(file)
                val info = infos[index] ?: return lost(file)
                val timeChanged = info.modifiedMillis != null && file.sourceModifiedMillis != null && info.modifiedMillis != file.sourceModifiedMillis
                if (info.size != file.size || timeChanged) changed = true
            }
            if (!changed) return null
            d.log.w("transfer", "upload ${transfer.id}: a shared file changed; starting over")
            if (started) {
                // The PC drops the partial files. A 404 (a retried DELETE) means it is already gone: done.
                when (val response = http.delete(transfer.id)) {
                    is ApiResult.Error -> return fatalFor(response)
                    is ApiResult.Ok -> Unit
                }
            }
            return startOverUnderNewId()
        }

        private fun lost(file: TransferFileEntity) =
            Attempt.Fatal("source_lost", "Kin can't read ${file.name} any more. Share it again.")

        /**
         * The same files under a new transfer ID, from byte 0 (the sizes are read again). The PC never takes an ID back once it has used it.
         * The cached copies stay where they are: they are keyed by [TransferEntity.localId], which doesn't change with the ID. A file that
         * can't be read now ends the attempt (`source_lost`) before anything is renamed. Returns null when it worked.
         */
        private suspend fun startOverUnderNewId(): Attempt.Fatal? {
            val looked = files.mapParallel(timings.statParallelism) { file -> file.localUri?.let { d.sources.stat(it) } }
            val infos = files.mapIndexed { index, file -> file to (looked[index] ?: return lost(file)) }
            val newId = d.newId()
            d.store.rename(transfer.id, newId)
            d.notifier.clear(transfer.id)
            onRenamed(transfer.id, newId)
            transfer = transfer.copy(id = newId)
            files = infos.map { (file, info) ->
                file.copy(transferId = newId, size = info.size, sourceModifiedMillis = info.modifiedMillis, bytesDone = 0, finished = false)
                    .also { d.store.updateFile(it) }
            }
            val total = files.sumOf { it.size }
            d.store.setTotalBytes(newId, total)
            transfer = transfer.copy(totalBytes = total)
            started = false
            return null
        }

        /**
         * `POST /v1/transfers`. A `404` means the PC ended that transfer for good, or the ID belongs to another device, or it was used and
         * is over (protocol.md): it will never start it again.
         * - An automatic run (a reconnect, the queue) ends there: the transfer fails, and nothing is sent again.
         * - A user Retry has just asked the PC about the old ID, which is how a transfer the PC finished within 24 hours is recognised
         *   (`200`, every file complete). On `404` it goes on under a **new ID with only the files that never got a `200`**; a second `404`
         *   is final.
         */
        private suspend fun createOrStartOver(http: TransferHttp): ApiResult<Map<String, Long>> {
            val first = http.createTransfer(transfer.id, files.map { UploadFileSpec(it.fileId, it.name, it.size, it.mime) }, transfer.origin)
            // The run has reached its POST: whatever the answer, the stored "this is the user's Retry" has done its work.
            if (transfer.userRetry) {
                d.store.clearUserRetry(transfer.id)
                transfer = transfer.copy(userRetry = false)
            }
            if (first !is ApiResult.Error || first.status != 404) return first
            if (!userRun || startedOver) {
                d.log.w("transfer", "upload ${transfer.id}: the PC answered 404 to the POST; it no longer has this transfer")
                return first
            }
            startedOver = true
            val remaining = files.filter { !it.finished }
            if (remaining.isEmpty()) return first
            d.log.w("transfer", "upload ${transfer.id}: the PC has used this ID up; the ${remaining.size} files without a 200 go as a new transfer")
            val old = transfer
            val newId = d.newId()
            val now = d.nowMillis()
            val newFiles = remaining.mapIndexed { i, f -> f.copy(transferId = newId, ordinal = i, bytesDone = 0, finished = false) }
            val replacement = old.copy(
                id = newId, status = TransferStatus.ACTIVE, createdAtMillis = now, finishedAtMillis = null, totalBytes = newFiles.sumOf { it.size },
                errorCode = null, message = null, retriedAs = null, attempts = maxOf(old.attempts, 1) + 1, // a retry is never a first-try success
                userRetry = false, started = false,
            )
            d.store.supersede(old.id, replacement, newFiles, "${old.peerName} no longer has it. The files that weren't sent went as a new transfer.", now)
            d.notifier.clear(old.id)
            onRenamed(old.id, newId)
            transfer = replacement
            files = newFiles
            started = false
            done.set(0)
            return http.createTransfer(transfer.id, files.map { UploadFileSpec(it.fileId, it.name, it.size, it.mime) }, transfer.origin)
        }

        private fun fatalFor(error: ApiResult.Error): Attempt.Fatal {
            val pc = transfer.peerName
            val message = when (error.status) {
                507 -> "$pc doesn't have enough free space."
                413 -> "$pc won't take a transfer this large."
                500 -> "$pc couldn't save the file."
                401 -> "$pc no longer trusts this phone. Pair again."
                404 -> "$pc no longer has this transfer."
                400 -> "$pc refused the transfer (${error.code})."
                else -> "$pc answered ${error.status} (${error.code})."
            }
            return Attempt.Fatal(error.code, message)
        }

        /** At most one update per 500 ms and never forced: the result goes out under its own notification ID. */
        private fun report() {
            if (throttle.tryReport(d.nowMillis())) d.notifier.progress(transfer, files, done.get(), waiting = false)
        }

        /** The status of a transfer that isn't over; false when it was already over (canceled, done): then nothing here changes it. */
        private suspend fun setStatus(status: String, code: String? = null, message: String? = null): Boolean {
            val finishedAt = if (TransferStatus.isFinished(status)) d.nowMillis() else null
            val changed = d.store.setStatus(transfer.id, status, code, message, finishedAt)
            if (changed) transfer = transfer.copy(status = status, errorCode = code, message = message, finishedAtMillis = finishedAt)
            return changed
        }

        private suspend fun finish(): RunResult {
            if (!setStatus(TransferStatus.DONE)) return RunResult.Canceled
            d.sources.deleteCache(transfer.localId)
            d.log.i("transfer", "upload ${transfer.id} done: ${files.size} files, ${files.sumOf { it.size }} bytes")
            d.notifier.uploaded(transfer, files)
            return RunResult.Done
        }

        private suspend fun fail(code: String, message: String): RunResult {
            if (!setStatus(TransferStatus.FAILED, code, message)) return RunResult.Canceled
            // The message names files and the PC; the log gets the code only.
            d.log.w("transfer", "upload ${transfer.id} failed: $code")
            d.notifier.failed(transfer, message)
            return RunResult.Failed(code, message)
        }

        /** The paired PC is not the one this transfer was made for: nothing is sent, and the copies in the cache go. */
        private suspend fun peerChanged(): RunResult {
            val message = "These files were meant for ${transfer.peerName}, which is no longer paired. Share them again."
            d.sources.deleteCache(transfer.localId)
            if (!setStatus(TransferStatus.FAILED, DownloadRunner.PEER_CHANGED, message)) return RunResult.Canceled
            d.log.w("transfer", "upload ${transfer.id} failed: ${DownloadRunner.PEER_CHANGED}")
            d.notifier.failed(transfer, message)
            return RunResult.Failed(DownloadRunner.PEER_CHANGED, message)
        }

        /** No connection: keep the transfer ready to retry. Small files are copied into the cache so access can't be lost (android.md). */
        private suspend fun park(first: Boolean): RunResult {
            if (first) cacheSmallFiles()
            val reason = d.link.unreachableReason()
            if (!setStatus(TransferStatus.QUEUED, "unreachable", reason)) return RunResult.Canceled
            d.log.w("transfer", "upload ${transfer.id}: PC not reachable ($reason); kept ready to retry")
            d.notifier.queued(transfer, reason)
            return RunResult.Waiting
        }

        private suspend fun cacheSmallFiles() {
            files = files.map { file ->
                val uri = file.localUri
                if (uri == null || file.size >= timings.cacheCopyLimitBytes || uri.startsWith("file:")) return@map file
                val cached = d.sources.cacheCopy(uri, transfer.localId, file.fileId) ?: return@map file
                // The copy has its own modified time: that is what "did it change" compares against from now on.
                val info = d.sources.stat(cached) ?: return@map file
                file.copy(localUri = cached, size = info.size, sourceModifiedMillis = info.modifiedMillis).also { d.store.updateFile(it) }
            }
        }
    }
}
