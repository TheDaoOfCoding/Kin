package app.kinrelay.android.scan

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Size
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContract
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.kinrelay.android.graph
import app.kinrelay.android.ui.theme.KinAppTheme
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The QR scanner: a camera preview, and every frame goes to [QrDecoder]. The first code found is returned as text
 * ([ScanContract]); the caller treats it exactly like pasted text. Nothing is stored, and no frame leaves this class.
 *
 * It does not ask for the camera permission: [app.kinrelay.android.MainActivity] asks when the user taps "Scan", and
 * starts this screen only once it is granted. If the permission is missing anyway, the screen closes as "canceled".
 */
class ScanActivity : ComponentActivity() {
    private var analysisExecutor: ExecutorService? = null
    private val finished = AtomicBoolean(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            setResult(RESULT_CANCELED)
            finish()
            return
        }
        val executor = Executors.newSingleThreadExecutor().also { analysisExecutor = it }
        setContent {
            KinAppTheme(applicationContext.graph.settings) {
                ScanScreen(executor, onCode = ::onCode, onCancel = { finishScan(null) })
            }
        }
    }

    /** Called on the analysis thread. Only the first code counts. */
    private fun onCode(text: String) {
        if (text.length > MAX_TEXT) return // not ours: a pairing link is a few hundred characters
        runOnUiThread { finishScan(text) }
    }

    private fun finishScan(text: String?) {
        if (!finished.compareAndSet(false, true)) return
        if (text == null) {
            setResult(RESULT_CANCELED)
        } else {
            setResult(RESULT_OK, Intent().putExtra(EXTRA_TEXT, text))
        }
        finish()
    }

    override fun onDestroy() {
        super.onDestroy()
        analysisExecutor?.shutdown()
    }

    companion object {
        const val EXTRA_TEXT = "app.kinrelay.android.SCANNED_TEXT"
        private const val MAX_TEXT = 4096
    }
}

/** `launch(Unit)`; the result is the scanned text, or null when the user went back. */
class ScanContract : ActivityResultContract<Unit, String?>() {
    override fun createIntent(context: Context, input: Unit) = Intent(context, ScanActivity::class.java)
    override fun parseResult(resultCode: Int, intent: Intent?): String? =
        if (resultCode == android.app.Activity.RESULT_OK) intent?.getStringExtra(ScanActivity.EXTRA_TEXT) else null
}

@Composable
private fun ScanScreen(analysisExecutor: ExecutorService, onCode: (String) -> Unit, onCancel: () -> Unit) {
    val lifecycleOwner = LocalLifecycleOwner.current
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        AndroidView(
            modifier = Modifier.fillMaxSize(),
            factory = { ctx ->
                PreviewView(ctx).apply {
                    scaleType = PreviewView.ScaleType.FILL_CENTER
                    val decoder = QrDecoder()
                    val providerFuture = ProcessCameraProvider.getInstance(ctx)
                    providerFuture.addListener({
                        val preview = Preview.Builder().build().also { it.surfaceProvider = surfaceProvider }
                        val analysis = ImageAnalysis.Builder()
                            .setResolutionSelector(
                                // About 720p: enough for a QR code on a screen, and a full-size frame (12 MP) would take seconds to decode.
                                ResolutionSelector.Builder()
                                    .setResolutionStrategy(ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER))
                                    .build(),
                            )
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                        analysis.setAnalyzer(analysisExecutor) { image -> analyse(image, decoder, onCode) }
                        // No back camera, or the camera is in use: the screen stays black and the user goes back.
                        runCatching {
                            val provider = providerFuture.get()
                            provider.unbindAll()
                            provider.bindToLifecycle(lifecycleOwner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
                        }
                    }, ContextCompat.getMainExecutor(ctx))
                }
            },
        )
        Column(
            modifier = Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(24.dp),
            verticalArrangement = Arrangement.SpaceBetween,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                "Point the camera at the code on your PC",
                style = MaterialTheme.typography.titleMedium,
                color = Color.White,
                // A dark plate: the camera picture behind can be as bright as the text.
                modifier = Modifier.fillMaxWidth().background(Color.Black.copy(alpha = 0.6f)).padding(12.dp),
            )
            OutlinedButton(
                onClick = onCancel,
                colors = ButtonDefaults.outlinedButtonColors(containerColor = Color.Black.copy(alpha = 0.6f)),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Cancel", color = Color.White)
            }
        }
    }
}

private fun analyse(image: ImageProxy, decoder: QrDecoder, onCode: (String) -> Unit) {
    try {
        val plane = image.planes[0] // Y: the brightness, which is all a QR code needs
        val buffer = plane.buffer
        val bytes = ByteArray(buffer.remaining())
        buffer.get(bytes)
        decoder.decodeLuminance(bytes, image.width, image.height, plane.rowStride)?.let(onCode)
    } catch (_: Exception) {
        // One bad frame is not worth a crash: the next one comes in a moment.
    } finally {
        image.close()
    }
}
