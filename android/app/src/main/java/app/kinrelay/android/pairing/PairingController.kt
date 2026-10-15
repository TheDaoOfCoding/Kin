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

    /** A link from outside the app is waiting for the user to confirm it. Nothing has been contacted. */
    data class Confirm(val confirmation: PairingConfirmation) : PairingUiState
}

/**
 * Takes pairing text (a scan, a pasted link, a deep link), asks the user to confirm links that came from
 * outside the app ([PairingIntake]), runs the pairing and saves the PC on success.
 */
class PairingController(private val graph: AppGraph) {
    private val mutableState = MutableStateFlow<PairingUiState>(PairingUiState.Idle)
    val state: StateFlow<PairingUiState> = mutableState
    private var job: Job? = null

    /** Clears a finished state (an error, a cancelled confirmation). A running pairing is left alone. */
    fun reset() {
        if (job?.isActive != true) mutableState.value = PairingUiState.Idle
    }

    fun fail(message: String) {
        mutableState.value = PairingUiState.Failed(message)
    }

    /**
     * Decides what to do with [text]: show an error, ask the user (the state becomes [PairingUiState.Confirm]),
     * or say "pair now", in which case the caller makes sure the permission is there and calls [start].
     * Returns null when a pairing is already running.
     */
    suspend fun intake(text: String, source: PairingSource, autoConfirm: Boolean = false): PairingIntake? {
        if (job?.isActive == true) {
            graph.log.w("pairing", "a link was ignored: a pairing is already running")
            return null
        }
        val current = graph.pairedDevice()?.let { it.deviceId to it.name }
        val decision = PairingIntake.decide(PairingUrl.parse(text), source, current, autoConfirm)
        when (decision) {
            is PairingIntake.Reject -> {
                graph.log.w("pairing", decision.logLine)
                mutableState.value = PairingUiState.Failed(decision.message)
            }
            is PairingIntake.AskUser -> {
                val c = decision.confirmation
                graph.log.i("pairing", "link from outside the app: waiting for confirmation (pc ${c.target.id.take(8)}… replacing=${c.replacing != null})")
                mutableState.value = PairingUiState.Confirm(c)
            }
            is PairingIntake.PairNow -> Unit
        }
        return decision
    }

    /**
     * The user tapped Confirm on the screen that showed [expected]: the waiting target, to be passed to [start].
     * If another link has replaced it since (a second deep link arrived while the screen was up), the tap was for the
     * old PC: nothing is confirmed, and the screen now shows the new one.
     */
    fun confirm(expected: PairingConfirmation): PairingTarget? {
        val target = PairingIntake.confirmable(mutableState.value, expected)
        if (target == null) {
            graph.log.w("pairing", "confirm ignored: the link on screen is not the one that was confirmed")
            return null
        }
        mutableState.value = PairingUiState.Idle
        graph.log.i("pairing", "pairing confirmed by the user")
        return target
    }

    fun cancel() {
        if (mutableState.value is PairingUiState.Confirm) graph.log.i("pairing", "pairing cancelled by the user")
        mutableState.value = PairingUiState.Idle
    }

    /** Starts the pairing itself. Call it only after [intake] said so or after the user confirmed. */
    fun start(target: PairingTarget, context: android.content.Context) {
        if (job?.isActive == true) return
        mutableState.value = PairingUiState.Working("Connecting to the PC…")
        job = graph.scope.launch { pair(target, context.applicationContext) }
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
                devices.all().filter { it.deviceId != target.id }.forEach {
                    devices.delete(it.deviceId)
                    // The old PC's transfers end with its pairing: an upload must not go to the PC that replaces it.
                    graph.peerGone(it.deviceId)
                }
                devices.upsert(DeviceEntity(target.id, name, "windows", now, now, ordered.joinToString(",")))
                log.i("pairing", "paired with $name (${target.id.take(8)}…) via ${result.address}")
                mutableState.value = PairingUiState.Idle
                LinkService.start(context)
                graph.link.devicePaired()
            }
        }
    }
}
