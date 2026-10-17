package app.kinrelay.android

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import android.webkit.MimeTypeMap
import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.data.KinDatabase
import app.kinrelay.android.data.KinSettings
import app.kinrelay.android.data.SeenMessageEntity
import app.kinrelay.android.history.HistoryItem
import app.kinrelay.android.history.HistoryRepository
import app.kinrelay.android.history.SuccessRate
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.identity.IdentityFailure
import app.kinrelay.android.identity.IdentityLoad
import app.kinrelay.android.identity.IdentityStore
import app.kinrelay.android.identity.IdentityUnavailableException
import app.kinrelay.android.identity.KeystoreKeyWrapper
import app.kinrelay.android.link.ArrivalSink
import app.kinrelay.android.link.Clock
import app.kinrelay.android.data.QueuePolicy
import app.kinrelay.android.link.LinkController
import app.kinrelay.android.link.OutboxFlusher
import app.kinrelay.android.link.QueuedUpload
import app.kinrelay.android.link.RoomTextStore
import app.kinrelay.android.link.TextOutbox
import app.kinrelay.android.link.UploadQueue
import app.kinrelay.android.link.Notifications
import app.kinrelay.android.link.OutgoingSender
import app.kinrelay.android.link.TextFileSender
import app.kinrelay.android.log.FileLog
import app.kinrelay.android.log.LogFingerprint
import app.kinrelay.android.log.LogKeyStore
import app.kinrelay.android.notifmirror.MirrorController
import app.kinrelay.android.pairing.DeviceInfo
import app.kinrelay.android.pairing.PairingController
import app.kinrelay.android.protocol.DedupMemory
import app.kinrelay.android.protocol.MessageIds
import app.kinrelay.android.share.ShareShortcuts
import app.kinrelay.android.transfer.AndroidSourceFiles
import app.kinrelay.android.transfer.AndroidWakeLocks
import app.kinrelay.android.transfer.MediaStoreSink
import app.kinrelay.android.transfer.RoomTransferStore
import app.kinrelay.android.transfer.TransferDeps
import app.kinrelay.android.transfer.TransferEngine
import app.kinrelay.android.transfer.TransferNotifications
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.File

/** Everything the app shares, wired by hand. One per process. */
class AppGraph(private val context: Context) {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    /** The application context, for code that has no context of its own. */
    val appContext: Context = context.applicationContext

    /** One thread, in order: the service's "running" marker is written here, so a stop can't overtake the next start. */
    @OptIn(ExperimentalCoroutinesApi::class)
    val serviceBookkeeping: CoroutineDispatcher = Dispatchers.IO.limitedParallelism(1)

    val log = FileLog(File(context.filesDir, "logs"), mirror = KinApp::mirrorToLogcat)
    val database: KinDatabase by lazy { KinDatabase.create(context) }
    val settings = KinSettings(context)

    private val identityStore = IdentityStore(context.filesDir, KeystoreKeyWrapper())
    private val mutableIdentity = MutableStateFlow<Identity?>(null)
    val identityFlow: StateFlow<Identity?> = mutableIdentity

    /** Why the identity can't be used right now (a Keystore error that may pass), or null. Nothing is deleted meanwhile. */
    private val mutableIdentityProblem = MutableStateFlow<String?>(null)
    val identityProblem: StateFlow<String?> = mutableIdentityProblem

    val pairing = PairingController(this)
    val link = LinkController(this)

    /** Notifications and toasts; also the [app.kinrelay.android.link.Alerts] the sender and the receiver use. */
    val notifications: Notifications by lazy { Notifications(appContext) }

    /** Notification mirroring, phone to PC (2B.2): the user's choices, the sender and the listener's engine. Reacts only; starts nothing by itself. */
    val mirror: MirrorController by lazy { MirrorController(this) }

    /** Remembers what the PC just put on the clipboard, so "send clipboard" doesn't send it back. */
    val clipboardGuard = ClipboardGuard(System::currentTimeMillis)

    /** The transfers: uploads from a share, downloads from an `offer`. Needs the link, the shared files, MediaStore and the notifications. */
    val transfers: TransferEngine by lazy {
        val sources = AndroidSourceFiles(appContext)
        val wake = AndroidWakeLocks(appContext)
        val deps = TransferDeps(
            store = RoomTransferStore(database.transfers()),
            link = link,
            sources = sources,
            sink = MediaStoreSink(appContext),
            notifier = TransferNotifications(appContext),
            wake = wake,
            log = log,
            newId = MessageIds::newId,
        )
        wakeLocks = wake
        TransferEngine(scope, deps, mimeForExtension = { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) })
    }

    /** Kept for the debug state dump. Set when [transfers] is first used. */
    @Volatile var wakeLocks: AndroidWakeLocks? = null
        private set

    /** The text queue and the text history (milestone 1.4). */
    val textOutbox: TextOutbox by lazy { TextOutbox(RoomTextStore(database.texts()), log) }

    /** Sends the queue (texts and uploads, oldest first) after the PC's `hello` and after a new send while connected. */
    val flusher: OutboxFlusher by lazy {
        OutboxFlusher(
            texts = textOutbox,
            uploads = object : UploadQueue {
                override suspend fun queued() = transfers.queuedUploads().map { QueuedUpload(it.id, it.createdAtMillis, it.peerDeviceId) }
                override fun start(id: String) = transfers.startQueuedUpload(id)
                override suspend fun expire(maxAgeMillis: Long) = transfers.expireQueued(maxAgeMillis)
            },
            pairedPeerDeviceId = { pairedDevice()?.deviceId },
            alerts = notifications,
            log = log,
        )
    }

    /** Sends text to the PC: saved in the queue first, then sent. Used by the share target, the text menu, the tile and the notification button. */
    val sender: OutgoingSender by lazy {
        OutgoingSender(
            link = link,
            alerts = notifications,
            guard = clipboardGuard,
            arrivals = ArrivalSink { settings.setLastArrival(it) },
            clock = Clock { SystemClock.elapsedRealtime() },
            log = log,
            outbox = textOutbox,
            flusher = flusher,
            textFiles = TextFileSender { wire ->
                transfers.enqueueText(wire, TransferEngine.textFileName(System.currentTimeMillis())) != null
            },
        )
    }

    /** Name and model sent to the PC. */
    val deviceInfo: DeviceInfo by lazy {
        val name = Settings.Global.getString(context.contentResolver, "device_name")?.takeIf { it.isNotBlank() } ?: Build.MODEL
        DeviceInfo(name = name, model = "${Build.MANUFACTURER} ${Build.MODEL}", appVersion = BuildConfig.VERSION_NAME)
    }

    /** The history screen's list, and what it does with an item. */
    val history: HistoryRepository by lazy { HistoryRepository(database.transfers(), database.texts()) }

    /**
     * Retry in the history (the user asked): a failed upload goes through [TransferEngine.retry] (its old ID is asked about first, a new ID
     * only if the PC has used it up), a failed text goes out again as a new message.
     */
    fun retryItem(item: HistoryItem) {
        scope.launch(Dispatchers.IO) {
            if (!item.isText) {
                transfers.retry(item.id)
                return@launch
            }
            if (!textOutbox.retry(item.id)) return@launch
            link.ensureRunning(appContext)
            val peer = pairedDevice() ?: return@launch
            link.await(OutgoingSender.CONNECT_WAIT_MILLIS, peer.deviceId)?.let { flusher.flush(it) }
        }
    }

    /** The first-try success rate of the sends from this phone (targets.md #1). */
    suspend fun successRate(): SuccessRate.Result = SuccessRate.compute(database.transfers().outgoingOutcomes(), database.texts().outgoingOutcomes())

    /** Cancel in the history: a queued item never goes out. */
    fun cancelItem(item: HistoryItem) {
        scope.launch(Dispatchers.IO) { if (item.isText) textOutbox.cancel(item.id) else transfers.cancel(item.id) }
    }

    /** The name sent in `hello`: the one the user chose in Settings, or the system's. */
    suspend fun deviceName(): String = settings.deviceName() ?: deviceInfo.name

    /** A PC is gone (unpaired from either side, or replaced): what is unfinished with it ends, and nothing queued for it can go to another PC. */
    suspend fun peerGone(deviceId: String) {
        transfers.onUnpaired(deviceId)
        textOutbox.onUnpaired(deviceId)
    }

    private val dedupLoad = CompletableDeferred<DedupMemory>()

    private companion object {
        const val PRUNE_INTERVAL_MILLIS = 24L * 60 * 60 * 1000
    }

    /** Creates the identity on first run, without blocking the main thread. */
    fun warmUp() {
        // The key of the log fingerprints (a keyed hash of a text, never the text): random per install, wrapped by the Keystore, never exported.
        scope.launch(Dispatchers.IO) {
            runCatching { LogFingerprint.install(LogKeyStore(File(context.filesDir, "log-key.bin"), KeystoreKeyWrapper("kin_log_wrap")).loadOrCreate()) }
                .onFailure { log.w("app", "no log key (${it.javaClass.simpleName}): texts are logged by length only") }
        }
        // Texts that were being sent when the app stopped are queued again, and what waited too long expires (it never goes out).
        scope.launch(Dispatchers.IO) {
            runCatching { textOutbox.recoverAtStart() }
            runCatching { flusher.expire() }
            prune()
        }
        // The same housekeeping once a day, for a phone that stays connected for weeks (it also runs at every connect).
        scope.launch(Dispatchers.IO) {
            while (true) {
                delay(PRUNE_INTERVAL_MILLIS)
                prune()
            }
        }
        // What a crash left behind and no transfer owns (pending gallery entries, and cached copies of transfers that are not live: a queued,
        // active or failed upload keeps its copies, whatever their age) goes now, before downloads resume.
        scope.launch(Dispatchers.IO) { runCatching { transfers.sweepOrphans() } }
        // Follows the user's notification-mirroring choices (turning it off or an app off takes its toasts down on the PC).
        mirror.start()
        // The paired PC shows up in the share sheet's top row (the direct-share shortcut) and goes away when it is unpaired.
        scope.launch {
            database.devices().observeAll().collect { ShareShortcuts.sync(appContext, it.firstOrNull(), log) }
        }
        scope.launch(Dispatchers.IO) {
            val load = loadIdentity()
            if (load.replaced != null) {
                // The old identity is lost for good and the PC knows its device ID, so the old pairing can never work again.
                log.w("identity", "the stored identity was replaced: ${load.replaced}")
                val gone = database.devices().all()
                database.devices().deleteAll()
                gone.forEach { peerGone(it.deviceId) }
                log.w("identity", "the new identity replaces the old one; paired PCs were removed")
            }
            val identity = load.identity
            log.i("identity", "device ${identity.shortId}")
            mutableIdentity.value = identity
            dedupLoad.complete(loadDedup())
        }
    }

    /**
     * Loads or creates the identity. A temporary Keystore problem is retried with a growing delay and shown in
     * [identityProblem]; the identity files and the paired PCs are not touched while it lasts.
     */
    private suspend fun loadIdentity(): IdentityLoad {
        var attempt = 0
        while (true) {
            try {
                return identityStore.loadOrCreate().also { mutableIdentityProblem.value = null }
            } catch (e: IdentityUnavailableException) {
                val wait = IdentityFailure.retryDelayMillis(attempt++)
                log.w("identity", "identity unavailable (attempt $attempt): ${e.message}; retrying in ${wait / 1000.0} s")
                mutableIdentityProblem.value = "Kin can't unlock its identity right now (${e.message}). It will try again."
                delay(wait)
            }
        }
    }

    /**
     * Keeps the database small (at every connect, at start, and once a day): the file rows of finished transfers and the bodies of finished
     * texts beyond the 500 the history shows go. The transfer rows stay for ever: a transfer ID is never used twice.
     */
    suspend fun prune() {
        runCatching { database.transfers().trim(QueuePolicy.HISTORY_LIMIT) }
        runCatching { textOutbox.prune() }
    }

    suspend fun identity(): Identity = mutableIdentity.filterNotNull().first()

    /** The de-duplication memory, shared by all connections and persisted in the database. */
    suspend fun dedup(): DedupMemory = dedupLoad.await()

    private suspend fun loadDedup(): DedupMemory {
        val dao = database.seenMessages()
        val initial = dao.all().map { DedupMemory.Entry(it.id, it.response, it.atMillis) }
        return DedupMemory(System::currentTimeMillis, initial) { added, removed ->
            scope.launch(Dispatchers.IO) {
                added?.let { dao.insert(SeenMessageEntity(it.id, it.response, it.atMillis)) }
                if (removed.isNotEmpty()) dao.delete(removed)
            }
        }
    }

    suspend fun pairedDevice(): DeviceEntity? = database.devices().all().firstOrNull()
}
