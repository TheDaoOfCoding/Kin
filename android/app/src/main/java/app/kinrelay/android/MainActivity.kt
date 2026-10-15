package app.kinrelay.android

import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.mutableStateOf
import androidx.core.net.toUri
import androidx.lifecycle.lifecycleScope
import app.kinrelay.android.pairing.PairingIntake
import app.kinrelay.android.pairing.PairingSource
import app.kinrelay.android.pairing.PairingTarget
import app.kinrelay.android.ui.KinRoot
import app.kinrelay.android.ui.theme.KinTheme
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val permissions = mutableStateOf(PermissionSnapshot())
    private var pendingTarget: PairingTarget? = null
    private var askedOnce = false
    private var requestInFlight = false

    private val permissionRequest =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
            requestInFlight = false
            graph.log.i("app", "permissions: $result")
            refreshPermissions()
            pendingTarget?.let {
                pendingTarget = null
                graph.pairing.start(it, this)
            }
            // The permission may just have been granted: try the connection now.
            graph.link.userAction()
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            KinTheme {
                KinRoot(
                    graph, this,
                    permissions = permissions.value,
                    onAskPermissions = ::askPermissions,
                    onAllowLocalNetwork = ::allowLocalNetwork,
                    onScan = ::scan,
                    onPair = { onPairingText(it, PairingSource.IN_APP) },
                    onConfirmPairing = { shown -> graph.pairing.confirm(shown)?.let(::beginPairing) },
                    onCancelPairing = { graph.pairing.cancel() },
                )
            }
        }
        if (savedInstanceState == null) handleLink(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleLink(intent)
    }

    override fun onResume() {
        super.onResume()
        // Back from a settings screen (battery, notifications, Nearby devices): the setup shows what changed.
        refreshPermissions()
    }

    /** The setup's "Allow": the system prompt for what is missing, or the app's settings page when the system won't ask again. */
    private fun askPermissions() {
        if (Permissions.missing(this).isEmpty()) {
            allowLocalNetwork()
        } else {
            requestMissingPermissions()
        }
    }

    override fun onStart() {
        super.onStart()
        refreshPermissions()
        if (!askedOnce) {
            askedOnce = true
            requestMissingPermissions()
        }
        graph.notifications.textsSeen()
        lifecycleScope.launch {
            graph.link.ensureRunning(this@MainActivity)
            graph.link.userAction()
            // What waited in the queue for more than 7 days has expired (it never goes out).
            runCatching { graph.flusher.expire() }
        }
    }

    private fun refreshPermissions() {
        permissions.value = Permissions.snapshot(this)
    }

    /**
     * A link opened from outside the app (a browser, a message, `adb`). Any app or web page can fire one, so it never
     * pairs on its own: it opens the confirmation screen. (The debug build can skip that, see [DebugHooks].)
     */
    private fun handleLink(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        // Re-opening the app from the recents list re-delivers its first intent. That isn't a new link.
        if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        val text = intent.dataString ?: return
        graph.log.i("app", "pairing link opened from outside the app (${intent.data?.scheme})")
        onPairingText(text, PairingSource.EXTERNAL_LINK, autoConfirm = DebugHooks.autoConfirmPairing(intent))
    }

    /** Text that was scanned or pasted in the app ([PairingSource.IN_APP]) or opened as a link. */
    private fun onPairingText(text: String, source: PairingSource, autoConfirm: Boolean = false) {
        lifecycleScope.launch {
            val decision = graph.pairing.intake(text, source, autoConfirm)
            if (decision is PairingIntake.PairNow) beginPairing(decision.target)
        }
    }

    /** Pairing needs the local network permission on Android 17+, so it is asked for first. */
    private fun beginPairing(target: PairingTarget) {
        if (Permissions.localNetworkGranted(this)) {
            graph.pairing.start(target, this)
        } else {
            pendingTarget = target
            requestMissingPermissions()
        }
    }

    /** One system prompt at a time: a second request while one is showing is answered with an empty result. */
    private fun requestMissingPermissions() {
        val missing = Permissions.missing(this)
        if (requestInFlight || missing.isEmpty()) return
        requestInFlight = true
        permissionRequest.launch(missing.toTypedArray())
    }

    /** The system stops showing the prompt after a denial, so this opens the app's settings page. */
    private fun allowLocalNetwork() {
        startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, "package:$packageName".toUri()))
    }

    /** The Google code scanner (ML Kit through Play services). It needs no camera permission. */
    private fun scan() {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        GmsBarcodeScanning.getClient(this, options).startScan()
            .addOnSuccessListener { barcode -> barcode.rawValue?.let { onPairingText(it, PairingSource.IN_APP) } }
            .addOnCanceledListener { graph.log.i("app", "scan canceled") }
            .addOnFailureListener { e ->
                graph.log.w("app", "code scanner unavailable: ${e.javaClass.simpleName}: ${e.message}")
                graph.pairing.fail("The QR scanner isn't available on this phone. Paste the pairing link instead.")
            }
    }
}
