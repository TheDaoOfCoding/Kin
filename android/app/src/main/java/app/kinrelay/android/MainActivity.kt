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
import app.kinrelay.android.ui.KinRoot
import app.kinrelay.android.ui.theme.KinTheme
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val localNetworkAllowed = mutableStateOf(true)
    private var pendingPairing: String? = null
    private var askedOnce = false
    private var requestInFlight = false

    private val permissionRequest =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
            requestInFlight = false
            graph.log.i("app", "permissions: $result")
            refreshPermissions()
            pendingPairing?.let {
                pendingPairing = null
                graph.pairing.start(it, this)
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            KinTheme {
                KinRoot(
                    graph, this,
                    localNetworkAllowed = localNetworkAllowed.value,
                    onAllowLocalNetwork = ::allowLocalNetwork,
                    onScan = ::scan,
                    onPair = ::startPairing,
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

    override fun onStart() {
        super.onStart()
        refreshPermissions()
        if (!askedOnce) {
            askedOnce = true
            requestMissingPermissions()
        }
        lifecycleScope.launch {
            graph.link.ensureRunning(this@MainActivity)
            graph.link.userAction()
        }
    }

    private fun refreshPermissions() {
        localNetworkAllowed.value = Permissions.localNetworkGranted(this)
    }

    private fun handleLink(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        val text = intent.dataString ?: return
        graph.log.i("app", "pairing link opened (${intent.data?.scheme})")
        startPairing(text)
    }

    /** Pairing needs the local network permission on Android 17+, so it is asked for first. */
    private fun startPairing(text: String) {
        if (Permissions.localNetworkGranted(this)) {
            graph.pairing.start(text, this)
        } else {
            pendingPairing = text
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
            .addOnSuccessListener { barcode -> barcode.rawValue?.let(::startPairing) }
            .addOnCanceledListener { graph.log.i("app", "scan canceled") }
            .addOnFailureListener { e ->
                graph.log.w("app", "code scanner unavailable: ${e.javaClass.simpleName}: ${e.message}")
                graph.pairing.fail("The QR scanner isn't available on this phone. Paste the pairing link instead.")
            }
    }
}
