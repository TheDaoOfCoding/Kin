package app.kinrelay.android

import android.content.Context
import android.os.Build
import android.provider.Settings
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.data.KinDatabase
import app.kinrelay.android.data.KinSettings
import app.kinrelay.android.data.SeenMessageEntity
import app.kinrelay.android.identity.Identity
import app.kinrelay.android.identity.IdentityStore
import app.kinrelay.android.identity.KeystoreKeyWrapper
import app.kinrelay.android.link.LinkController
import app.kinrelay.android.log.FileLog
import app.kinrelay.android.pairing.DeviceInfo
import app.kinrelay.android.pairing.PairingController
import app.kinrelay.android.protocol.DedupMemory
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.File

/** Everything the app shares, wired by hand. One per process. */
class AppGraph(private val context: Context) {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val log = FileLog(File(context.filesDir, "logs"), mirror = KinApp::mirrorToLogcat)
    val database: KinDatabase by lazy { KinDatabase.create(context) }
    val settings = KinSettings(context)

    @Volatile private var identityRegenerated = false
    private val identityStore = IdentityStore(context.filesDir, KeystoreKeyWrapper()) {
        identityRegenerated = true
        log.w("identity", it)
    }
    private val mutableIdentity = MutableStateFlow<Identity?>(null)
    val identityFlow: StateFlow<Identity?> = mutableIdentity

    val pairing = PairingController(this)
    val link = LinkController(this)

    /** Name and model sent to the PC. */
    val deviceInfo: DeviceInfo by lazy {
        val name = Settings.Global.getString(context.contentResolver, "device_name")?.takeIf { it.isNotBlank() } ?: Build.MODEL
        DeviceInfo(name = name, model = "${Build.MANUFACTURER} ${Build.MODEL}", appVersion = BuildConfig.VERSION_NAME)
    }

    private val dedupLoad = CompletableDeferred<DedupMemory>()

    /** Creates the identity on first run, without blocking the main thread. */
    fun warmUp() {
        scope.launch(Dispatchers.IO) {
            val identity = identityStore.loadOrCreate()
            if (identityRegenerated) {
                // The PC knows the old device ID, so the old pairing can never work again.
                database.devices().deleteAll()
                log.w("identity", "the new identity replaces the old one; paired PCs were removed")
            }
            log.i("identity", "device ${identity.deviceId} (short ${identity.shortId})")
            mutableIdentity.value = identity
            dedupLoad.complete(loadDedup())
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
