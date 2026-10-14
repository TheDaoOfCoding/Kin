package app.kinrelay.android.pairing

import app.kinrelay.android.AppGraph
import app.kinrelay.android.Permissions
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.link.LinkService
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

sealed interface PairingUiState {
    data object Idle : PairingUiState
    data class Working(val message: String) : PairingUiState
    data class Failed(val message: String) : PairingUiState
}

/** Runs a pairing from scanned or pasted text (or a deep link) and saves the PC on success. */
class PairingController(private val graph: AppGraph) {
    private val mutableState = MutableStateFlow<PairingUiState>(PairingUiState.Idle)
    val state: StateFlow<PairingUiState> = mutableState
    private var job: Job? = null

    fun reset() {
        if (job?.isActive != true) mutableState.value = PairingUiState.Idle
    }

    fun fail(message: String) {
        mutableState.value = PairingUiState.Failed(message)
    }

    fun start(text: String, context: android.content.Context) {
        if (job?.isActive == true) return
        val log = graph.log
        when (val parsed = PairingUrl.parse(text)) {
            is PairingParse.Error -> {
                log.w("pairing", "link rejected: ${parsed.category} ${parsed.param.orEmpty()}")
                mutableState.value = PairingUiState.Failed(PairingMessages.forParseError(parsed))
            }
            is PairingParse.Ok -> {
                mutableState.value = PairingUiState.Working("Connecting to the PC…")
                job = graph.scope.launch { pair(parsed.target, context.applicationContext) }
            }
        }
    }

    private suspend fun pair(target: PairingTarget, context: android.content.Context) {
        val log = graph.log
        log.i("pairing", "pairing started: ${target.addresses.joinToString()} pc=${target.id.take(8)}…")
        val client = PairingClient(graph.identity(), graph.deviceInfo, log)
        when (val result = client.pair(target)) {
            is PairResult.Failure -> {
                log.w("pairing", "pairing failed: ${result.kind} (${result.detail})")
                val blocked = result.kind == PairFailure.UNREACHABLE && !Permissions.localNetworkGranted(context)
                mutableState.value = PairingUiState.Failed(
                    if (blocked) PairingMessages.LOCAL_NETWORK_DENIED else PairingMessages.forFailure(result.kind)
                )
            }
            is PairResult.Success -> {
                val now = System.currentTimeMillis()
                val ordered = listOf(result.address) + target.addresses.filter { it != result.address }
                val name = result.pcName ?: target.name ?: "PC"
                val devices = graph.database.devices()
                devices.all().filter { it.deviceId != target.id }.forEach { devices.delete(it.deviceId) }
                devices.upsert(DeviceEntity(target.id, name, "windows", now, now, ordered.joinToString(",")))
                log.i("pairing", "paired with $name (${target.id.take(8)}…) via ${result.address}")
                mutableState.value = PairingUiState.Idle
                LinkService.start(context)
            }
        }
    }
}
