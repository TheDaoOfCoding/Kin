package app.kinrelay.android

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import app.kinrelay.android.clipboard.ClipboardGuard
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.data.KinDatabase
import app.kinrelay.android.data.KinSettings
import app.kinrelay.android.data.SeenMessageEntity
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.identity.IdentityFailure
import app.kinrelay.android.identity.IdentityLoad
import app.kinrelay.android.identity.IdentityStore
import app.kinrelay.android.identity.IdentityUnavailableException
import app.kinrelay.android.identity.KeystoreKeyWrapper
import app.kinrelay.android.link.ArrivalSink
import app.kinrelay.android.link.Clock
import app.kinrelay.android.link.LinkController
import app.kinrelay.android.link.Notifications
import app.kinrelay.android.link.OutgoingSender
import app.kinrelay.android.log.FileLog
import app.kinrelay.android.pairing.DeviceInfo
import app.kinrelay.android.pairing.PairingController
import app.kinrelay.android.protocol.DedupMemory
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

    /** Remembers what the PC just put on the clipboard, so "send clipboard" doesn't send it back. */
    val clipboardGuard = ClipboardGuard(System::currentTimeMillis)

    /** Sends text to the PC and waits for the ack. Used by the share target, the text menu, the tile and the notification button. */
    val sender: OutgoingSender by lazy {
        OutgoingSender(
            link = link,
            alerts = notifications,
            guard = clipboardGuard,
            arrivals = ArrivalSink { settings.setLastArrival(it) },
            clock = Clock { SystemClock.elapsedRealtime() },
            log = log,
        )
    }

    /** Name and model sent to the PC. */
    val deviceInfo: DeviceInfo by lazy {
        val name = Settings.Global.getString(context.contentResolver, "device_name")?.takeIf { it.isNotBlank() } ?: Build.MODEL
        DeviceInfo(name = name, model = "${Build.MANUFACTURER} ${Build.MODEL}", appVersion = BuildConfig.VERSION_NAME)
    }

    private val dedupLoad = CompletableDeferred<DedupMemory>()

    /** Creates the identity on first run, without blocking the main thread. */
    fun warmUp() {
        scope.launch(Dispatchers.IO) {
            val load = loadIdentity()
            if (load.replaced != null) {
                // The old identity is lost for good and the PC knows its device ID, so the old pairing can never work again.
                log.w("identity", "the stored identity was replaced: ${load.replaced}")
                database.devices().deleteAll()
                log.w("identity", "the new identity replaces the old one; paired PCs were removed")
            }
            val identity = load.identity
            log.i("identity", "device ${identity.deviceId} (short ${identity.shortId})")
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
