package app.kinrelay.android.transfer

import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.log.KinLog
import kotlinx.coroutines.delay

/**
 * Runs beside one attempt of a transfer (a coroutine of [UploadRunner] / [DownloadRunner]) and asks "does the PC still answer?" whenever
 * no byte has moved for [TransferTimings.stallProbeAfterMillis]. A network that goes silent without telling the phone (a router or access
 * point that fails: the packets just stop, nothing resets) leaves the WebSocket looking alive for minutes, because its own ping only
 * goes out every 240 s (battery) and its pong is waited for another 240 s. Without this, a transfer would sit on requests that time out
 * one by one and give up for good after about a minute while the link still says "connected".
 *
 * The probe is [LinkConnection.checkAlive]: a new TLS connection, which costs the PC nothing visible (no `ping` toast) and costs the phone
 * nothing at all while bytes move or nothing is being transferred. If the PC doesn't answer, the connection is abandoned: it ends, the
 * attempt is dropped (the runner's own watcher sees the end), the link reconnects, and the transfer resumes on the new connection.
 * The connection remembers the verdict ([LinkConnection.foundDead]), so the runner knows this was the network and not a failure of
 * the transfer.
 *
 * Returns after it found the link dead; runs until cancelled otherwise.
 */
internal suspend fun watchForStall(
    connection: LinkConnection,
    timings: TransferTimings,
    log: KinLog,
    what: String,
    bytesMoved: () -> Long,
) {
    var seen = bytesMoved()
    while (true) {
        delay(timings.stallProbeAfterMillis)
        val now = bytesMoved()
        if (now != seen) {
            seen = now
            continue
        }
        log.i("transfer", "$what: no byte moved for ${timings.stallProbeAfterMillis / 1000.0} s; checking that the PC answers")
        if (!connection.checkAlive(timings.probeTimeoutMillis)) {
            log.w("transfer", "$what: the PC doesn't answer; the connection is dropped and the transfer waits for a new one")
            return
        }
    }
}
