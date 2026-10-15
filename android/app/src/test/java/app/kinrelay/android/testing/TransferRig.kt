package app.kinrelay.android.testing

import app.kinrelay.android.data.TransferEntity
import app.kinrelay.android.data.TransferFileEntity
import app.kinrelay.android.data.TransferStatus
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.link.DeliveryQueue
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.link.LinkSession
import app.kinrelay.android.link.LocalHello
import app.kinrelay.android.link.SessionListener
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.pairing.PairingAddress
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.Hello
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.protocol.Offer
import app.kinrelay.android.tls.KinTls
import app.kinrelay.android.transfer.NameTakenException
import app.kinrelay.android.transfer.PeerInfo
import app.kinrelay.android.transfer.SaveFolder
import app.kinrelay.android.transfer.SaveSink
import app.kinrelay.android.transfer.SavedEntry
import app.kinrelay.android.transfer.SourceFiles
import app.kinrelay.android.transfer.SourceInfo
import app.kinrelay.android.transfer.TransferDeps
import app.kinrelay.android.transfer.TransferEngine
import app.kinrelay.android.transfer.TransferLink
import app.kinrelay.android.transfer.TransferNotifier
import app.kinrelay.android.transfer.TransferStore
import app.kinrelay.android.transfer.TransferTimings
import app.kinrelay.android.transfer.WakeLockPort
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.io.RandomAccessFile
import java.nio.file.Files
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

/** A store that keeps everything in memory (the app uses Room). */
class InMemoryTransferStore : TransferStore {
    val transfers = ConcurrentHashMap<String, TransferEntity>()
    val files = ConcurrentHashMap<String, MutableList<TransferFileEntity>>()

    override suspend fun create(transfer: TransferEntity, files: List<TransferFileEntity>) {
        transfers[transfer.id] = transfer
        this.files[transfer.id] = files.toMutableList()
    }

    override suspend fun transfer(id: String) = transfers[id]
    override suspend fun files(id: String): List<TransferFileEntity> = files[id]?.sortedBy { it.ordinal }.orEmpty()

    private val lock = Any()

    override suspend fun updateFile(file: TransferFileEntity) = synchronized(lock) {
        val list = files.getOrPut(file.transferId) { mutableListOf() }
        val index = list.indexOfFirst { it.fileId == file.fileId }
        if (index >= 0) list[index] = file else list += file
        Unit
    }

    override suspend fun setBytesDone(id: String, fileId: String, bytesDone: Long) = synchronized(lock) {
        files[id]?.let { list -> val i = list.indexOfFirst { it.fileId == fileId }; if (i >= 0) list[i] = list[i].copy(bytesDone = bytesDone) }
        Unit
    }

    override suspend fun setStatus(id: String, status: String, errorCode: String?, message: String?, finishedAt: Long?) {
        transfers.computeIfPresent(id) { _, t -> t.copy(status = status, errorCode = errorCode, message = message, finishedAtMillis = finishedAt) }
    }

    override suspend fun setCompleteAcked(id: String, acked: Boolean) {
        transfers.computeIfPresent(id) { _, t -> t.copy(completeAcked = acked) }
    }

    override suspend fun setTotalBytes(id: String, totalBytes: Long) {
        transfers.computeIfPresent(id) { _, t -> t.copy(totalBytes = totalBytes) }
    }

    override suspend fun rename(oldId: String, newId: String) = synchronized(lock) {
        transfers.remove(oldId)?.let { transfers[newId] = it.copy(id = newId) }
        files.remove(oldId)?.let { list -> files[newId] = list.map { it.copy(transferId = newId) }.toMutableList() }
        Unit
    }

    override suspend fun unfinished() = transfers.values.filter { !TransferStatus.isFinished(it.status) }.sortedBy { it.createdAtMillis }
    override suspend fun recent(limit: Int) = transfers.values.sortedByDescending { it.createdAtMillis }.take(limit)
    override suspend fun claimedNames() = files.values.flatten().filter { !it.finished && it.finalName != null }.mapNotNull { it.finalName }.toSet()

    private fun unfinishedDownloads() = transfers.values.filter { it.direction == "down" && !TransferStatus.isFinished(it.status) }

    override suspend fun pendingDownloadBytes() =
        unfinishedDownloads().sumOf { t -> files[t.id].orEmpty().filter { !it.finished }.sumOf { maxOf(it.size - it.bytesDone, 0) } }

    override suspend fun ownedPartialUris() =
        unfinishedDownloads().flatMap { t -> files[t.id].orEmpty().filter { !it.finished }.mapNotNull { it.localUri } }.toSet()

    override suspend fun liveIds() = transfers.values.filter { it.status in setOf("queued", "active", "failed") }.map { it.id }.toSet()

    override suspend fun delete(id: String) {
        transfers.remove(id)
        files.remove(id)
    }
}

/** Shared files that are plain files in a temporary folder, named by `content://test/<name>` URIs. */
class TempSources(private val root: File) : SourceFiles {
    private val byUri = ConcurrentHashMap<String, File>()
    val cacheDir = File(root, "cache").also { it.mkdirs() }
    val cached = CopyOnWriteArrayList<String>()
    @Volatile var lost = false

    /** Every `read` of a shared file waits this long first (a slow upload, to cancel it in flight). */
    @Volatile var readDelayMillis = 0L
    val reads = AtomicInteger()

    private val mimes = ConcurrentHashMap<String, String>()

    /** Writes [bytes] as a shared file and returns its URI. */
    fun add(name: String, bytes: ByteArray, mime: String? = null): String {
        val file = File(root, name).also { it.writeBytes(bytes) }
        return "content://test/$name".also { byUri[it] = file; if (mime != null) mimes[it] = mime }
    }

    fun fileOf(uri: String): File = byUri.getValue(uri)

    override fun stat(uri: String): SourceInfo? {
        if (lost) return null
        val file = byUri[uri]?.takeIf { it.isFile } ?: return null
        return SourceInfo(file.length(), file.lastModified(), file.name, mimes[uri])
    }

    override fun open(uri: String, offset: Long): InputStream {
        if (lost) throw SecurityException("access lost")
        val file = byUri[uri] ?: throw java.io.FileNotFoundException(uri)
        val stream = java.io.FileInputStream(file).also { var skipped = 0L; while (skipped < offset) skipped += it.skip(offset - skipped) }
        return object : InputStream() {
            override fun read(): Int = stream.read()
            override fun read(b: ByteArray, off: Int, len: Int): Int {
                reads.incrementAndGet()
                if (readDelayMillis > 0) Thread.sleep(readDelayMillis)
                return stream.read(b, off, minOf(len, CHUNK))
            }
            override fun close() = stream.close()
        }
    }

    override fun cacheCopy(uri: String, transferId: String, fileId: String): String? {
        val source = byUri[uri] ?: return null
        val target = File(File(cacheDir, transferId).also { it.mkdirs() }, fileId)
        source.copyTo(target, overwrite = true)
        val cachedUri = "content://test-cache/$transferId/$fileId"
        byUri[cachedUri] = target
        cached += cachedUri
        return cachedUri
    }

    override fun cacheText(text: String, transferId: String, fileId: String): String {
        val target = File(File(cacheDir, transferId).also { it.mkdirs() }, fileId).also { it.writeBytes(text.toByteArray(Charsets.UTF_8)) }
        return "content://test-cache/$transferId/$fileId".also { byUri[it] = target }
    }

    override fun deleteCache(transferId: String) {
        File(cacheDir, transferId).deleteRecursively()
        cached.removeAll { it.startsWith("content://test-cache/$transferId/") }
    }

    /** Pretends every cached copy is this old when the sweep asks (the real one reads the folder's modified time). */
    @Volatile var cacheAgeMillis = Long.MAX_VALUE

    override fun deleteCacheExcept(owned: Set<String>, minAgeMillis: Long): Int {
        val orphans = cacheDir.listFiles()?.filter { it.name !in owned && cacheAgeMillis >= minAgeMillis }.orEmpty()
        orphans.forEach { dir -> dir.deleteRecursively(); cached.removeAll { it.startsWith("content://test-cache/${dir.name}/") } }
        return orphans.size
    }
}

private const val CHUNK = 64 * 1024

/** A "gallery" in a temporary folder. It renames a clashing name the way MediaStore does ("name (1).ext"). */
class TempSink(private val root: File) : SaveSink {
    class Entry(val folder: SaveFolder, val name: String, val mime: String, val file: File) {
        @Volatile var pending = true
    }

    val entries = ConcurrentHashMap<String, Entry>()
    @Volatile var free = Long.MAX_VALUE
    @Volatile var renameClashes = true

    /** `length` says every entry is gone (a system that can't hold the entry). */
    @Volatile var vanish = false
    val created: Int get() = counter.get()
    private val counter = AtomicInteger()

    override fun existingNames(folder: SaveFolder): Set<String> = entries.values.filter { it.folder == folder }.map { it.name }.toSet()

    /** Names that a file this app can't see already has: `create` refuses them the way MediaStore's unique path index does. */
    val hiddenNames: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

    /** Every name is refused as taken (a system that never finds a free name); [refusals] counts the calls. */
    @Volatile var refuseAllNames = false
    val refusals = AtomicInteger()

    override fun create(folder: SaveFolder, name: String, mime: String): SavedEntry {
        if (refuseAllNames) {
            refusals.incrementAndGet()
            throw NameTakenException("$name is taken")
        }
        if (name in hiddenNames) throw NameTakenException("$name is taken")
        var finalName = name
        if (renameClashes) {
            var n = 0
            while (entries.values.any { it.folder == folder && it.name.equals(finalName, ignoreCase = true) }) {
                n++
                val dot = name.lastIndexOf('.')
                finalName = if (dot > 0) "${name.substring(0, dot)} ($n)${name.substring(dot)}" else "$name ($n)"
            }
        }
        val uri = "mem://${folder.name.lowercase()}/${counter.incrementAndGet()}"
        val file = File(File(root, folder.name).also { it.mkdirs() }, "${counter.get()}.bin")
        file.writeBytes(ByteArray(0))
        entries[uri] = Entry(folder, finalName, mime, file)
        return SavedEntry(uri, finalName)
    }

    override fun length(uri: String): Long? = if (vanish) null else entries[uri]?.file?.length()

    /** `openForWrite` fails with a bug-like exception (not an `IOException`), to test what a crash leaves behind. */
    @Volatile var crashOnWrite = false

    override fun openForWrite(uri: String, at: Long): OutputStream {
        if (crashOnWrite) throw IllegalStateException("a bug in the sink")
        val entry = entries[uri] ?: throw java.io.FileNotFoundException(uri)
        val raf = RandomAccessFile(entry.file, "rw")
        if (raf.length() > at) raf.setLength(at)
        raf.seek(at)
        return object : OutputStream() {
            override fun write(b: Int) = raf.write(b)
            override fun write(b: ByteArray, off: Int, len: Int) = raf.write(b, off, len)
            override fun close() = raf.close()
        }
    }

    override fun finish(uri: String) {
        entries[uri]?.pending = false
    }

    override fun delete(uri: String) {
        entries.remove(uri)?.file?.delete()
    }

    override fun pendingEntries(): List<String> = entries.filter { it.value.pending }.keys.toList()

    override fun freeBytes(): Long = free

    fun bytes(uri: String): ByteArray = entries.getValue(uri).file.readBytes()
    fun visible(): List<Entry> = entries.values.filter { !it.pending }
}

class RecordingNotifier : TransferNotifier {
    val events = CopyOnWriteArrayList<String>()
    val progress = CopyOnWriteArrayList<Pair<Long, Boolean>>()
    val received = CopyOnWriteArrayList<Pair<TransferEntity, List<TransferFileEntity>>>()
    val queued = CopyOnWriteArrayList<Pair<TransferEntity, String>>()
    val failed = CopyOnWriteArrayList<Pair<TransferEntity, String>>()
    val messages = CopyOnWriteArrayList<String>()

    override fun progress(transfer: TransferEntity, files: List<TransferFileEntity>, bytesDone: Long, waiting: Boolean) {
        progress += bytesDone to waiting
        events += "progress"
    }

    override fun uploaded(transfer: TransferEntity, files: List<TransferFileEntity>) { events += "uploaded:${transfer.id}" }
    override fun received(transfer: TransferEntity, files: List<TransferFileEntity>) { received += transfer to files; events += "received:${transfer.id}" }
    override fun queued(transfer: TransferEntity, reason: String) { queued += transfer to reason; events += "queued:${transfer.id}" }
    override fun failed(transfer: TransferEntity, message: String) { failed += transfer to message; events += "failed:${transfer.id}" }
    override fun clear(transferId: String) { events += "clear:$transferId" }
    override fun message(text: String) { messages += text }
}

class CountingWake : WakeLockPort {
    val acquired = AtomicInteger()
    val released = AtomicInteger()
    val held: Boolean get() = acquired.get() > released.get()

    override fun acquire() { acquired.incrementAndGet() }
    override fun release() { released.incrementAndGet() }
}

/**
 * Everything a transfer test needs: a fake PC (real TLS 1.3, real WebSocket, the file endpoints), a phone session connected to it, and
 * a [TransferEngine] wired to in-memory fakes of the Android parts.
 */
class TransferRig(
    val dir: File = Files.createTempDirectory("kin-transfer").toFile(),
    timings: TransferTimings = FAST,
    configurePc: FakePc.() -> Unit = {},
    newId: (() -> String)? = null,
) : AutoCloseable {
    val phone = Identity.generate()
    val pc: FakePc = FakePc().apply { pairedPhones += phone.deviceId }.apply(configurePc).start()
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val store = InMemoryTransferStore()
    val sources = TempSources(File(dir, "src").also { it.mkdirs() })
    val sink = TempSink(File(dir, "gallery"))
    val notifier = RecordingNotifier()
    val wake = CountingWake()
    val ids = AtomicInteger()
    val sessions = CopyOnWriteArrayList<LinkSession>()
    @Volatile var current: LinkSession? = null
    @Volatile var unreachable = "This phone isn't on Wi-Fi."
    private val deliveries = DeliveryQueue(KinLog.None).also { it.start(scope) }
    private val dedup = DedupMemory(System::currentTimeMillis)

    /** The PC that is paired now. A test changes it to pair another PC (or null to unpair). */
    @Volatile var pairedPeer: PeerInfo? = PeerInfo(pc.identity.deviceId, "Desk PC")

    val link = object : TransferLink {
        override suspend fun connection(waitMillis: Long, kick: Boolean): LinkConnection? {
            val deadline = System.nanoTime() + waitMillis * 1_000_000
            while (true) {
                current?.takeIf { it.isReady }?.let { return it }
                if (System.nanoTime() >= deadline) return null
                delay(10)
            }
        }

        override fun unreachableReason() = unreachable
        override suspend fun peer() = this@TransferRig.pairedPeer
    }

    val deps = TransferDeps(store, link, sources, sink, notifier, wake, KinLog.None, newId = newId ?: { "t-${ids.incrementAndGet()}-${MessageIds.newId().take(8)}" })
    lateinit var engine: TransferEngine

    init {
        engine = TransferEngine(scope, deps, timings, mimeForExtension = ::mime)
    }

    private fun mime(extension: String): String? = when (extension) {
        "jpg", "jpeg" -> "image/jpeg"
        "png" -> "image/png"
        "mp4" -> "video/mp4"
        "txt" -> "text/plain"
        "exe" -> "application/x-msdownload"
        else -> null
    }

    /** Opens a new session to the fake PC and waits for its `hello`. */
    fun connect(): LinkSession {
        val client = KinTls(phone, pc.identity.deviceId).httpClient().newBuilder().callTimeout(0, TimeUnit.MILLISECONDS).build()
        val hellos = LinkedBlockingQueue<Hello>()
        val listener = object : SessionListener {
            override fun onHelloAccepted(peer: Hello) { hellos.add(peer) }
            override fun onPingReceived(fromName: String) = Unit
            override fun onPeerUnpaired() = Unit
            override fun onTextReceived(fromName: String, id: String, text: String, isUrl: Boolean, sensitive: Boolean) = Unit
            override fun onClipboardReceived(id: String, text: String) = Unit
            override fun acceptOffer(offer: Offer): String? = engine.acceptOffer(offer)
            override fun onOfferReceived(fromName: String, offer: Offer) = deliveries.submit { engine.onOffer(fromName, offer) }
            override fun onCancelReceived(transferId: String, reason: String) = deliveries.submit { engine.onPeerCancel(transferId, reason) }
        }
        val session = LinkSession(
            client, PairingAddress("127.0.0.1", pc.port), LocalHello("Pixel 8", "android", "0.1.0"),
            dedup, scope, { System.nanoTime() / 1_000_000 }, KinLog.None, listener,
        )
        session.open()
        check(hellos.poll(5, TimeUnit.SECONDS) != null) { "no hello" }
        sessions += session
        current = session
        pc.nextFrame() // the phone's own hello
        return session
    }

    /** Drops the WebSocket (the PC closes it), like Wi-Fi going off. */
    fun dropConnection() {
        pc.closeSocket(1001, "going away")
        runBlocking { withTimeout(5_000) { current?.awaitEnd() } }
    }

    fun <T> blocking(timeoutMillis: Long = 20_000, block: suspend () -> T): T = runBlocking { withTimeout(timeoutMillis) { block() } }

    /** Polls until [condition] holds. */
    fun await(what: String, timeoutMillis: Long = 15_000, condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMillis
        while (!condition()) {
            check(System.currentTimeMillis() < deadline) { "timed out waiting for: $what" }
            Thread.sleep(10)
        }
    }

    fun awaitStatus(id: String, vararg statuses: String, timeoutMillis: Long = 15_000): TransferEntity {
        await("transfer $id to be ${statuses.toList()} (is ${store.transfers[id]?.status})", timeoutMillis) { store.transfers[id]?.status in statuses }
        return store.transfers.getValue(id)
    }

    override fun close() {
        scope.cancel()
        pc.close()
        dir.deleteRecursively()
    }

    companion object {
        val FAST = TransferTimings(
            connectWaitMillis = 1_000, reconnectWaitMillis = 8_000, ackTimeoutMillis = 2_000, retryDelayMillis = 20,
            cacheCopyLimitBytes = 1_000,
        )

        fun bytes(size: Int, seed: Int = 1): ByteArray = ByteArray(size) { ((it * 31 + seed) and 0xFF).toByte() }
    }
}
