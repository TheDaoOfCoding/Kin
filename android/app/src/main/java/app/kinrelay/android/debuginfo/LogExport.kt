package app.kinrelay.android.debuginfo

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.core.content.FileProvider
import app.kinrelay.android.AppGraph
import app.kinrelay.android.Permissions
import app.kinrelay.android.history.HistoryBuilder
import app.kinrelay.android.history.HistoryStatus
import app.kinrelay.android.history.SuccessRate
import app.kinrelay.android.identity.DeviceIds
import app.kinrelay.android.setup.OemTable
import app.kinrelay.android.setup.SetupActions
import kotlinx.coroutines.flow.first
import java.io.File
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/**
 * "Export logs": a zip of the log files and a redacted state dump, handed to the share sheet through a [FileProvider] URI (a read grant on
 * the intent, no storage permission). No secrets and no text contents: the log never has them ([app.kinrelay.android.log.Redactor], and text
 * is only ever logged as a length and a short keyed hash, not at all when sensitive), and the dump is made of counts, states and codes.
 * **Every file is redacted again on its way into the zip** ([ExportRedactor]): the PC's IP address and port, the PC's and the phone's names,
 * and full device IDs are taken out, as in the desktop's export.
 */
object LogExport {
    const val DIR = "exports"
    private const val KEEP = 3

    /** Everything the Debug screen shows, as numbers (also used by the state dump). */
    suspend fun stateInput(graph: AppGraph): StateDump.Input {
        val context = graph.appContext
        val stats = graph.settings.stats.first()
        val device = graph.pairedDevice()
        val transfers = graph.database.transfers()
        val texts = graph.database.texts()
        val outcomes = SuccessRate.compute(transfers.outgoingOutcomes(), texts.outgoingOutcomes())
        val history = HistoryBuilder.build(transfers.recent(500), emptyList(), texts.recent(500))
        val errors = history.filter { it.status == HistoryStatus.FAILED || it.status == HistoryStatus.EXPIRED }
            .mapNotNull { item -> item.errorCode?.let { it to isoTime(item.timeMillis) } }
            .take(20)
        val guide = OemTable.select(Build.MANUFACTURER, Build.BRAND)
        return StateDump.Input(
            appVersion = app.kinrelay.android.BuildConfig.VERSION_NAME, sdk = Build.VERSION.SDK_INT, manufacturer = Build.MANUFACTURER, model = Build.MODEL,
            linkState = graph.link.state.value.toString(), paired = device != null,
            phoneShortId = graph.identityFlow.value?.shortId?.let(DeviceIds::display),
            pcShortId = device?.deviceId?.let { DeviceIds.shortIdOfDeviceId(it) }?.let(DeviceIds::display),
            connects = stats.connects, disconnects = stats.disconnects, kills = stats.kills, successRate = outcomes.label(),
            rtt = LatencySummary.of(stats.recentRtts), arrival = LatencySummary.of(stats.recentArrivals.map { it.toDouble() }),
            queued = history.count { it.status == HistoryStatus.QUEUED }, recentErrors = errors,
            notificationsAllowed = Permissions.notificationsGranted(context), localNetworkAllowed = Permissions.localNetworkGranted(context),
            batteryUnrestricted = SetupActions.batteryOptimisationIgnored(context), oemGuide = guide?.id,
        )
    }

    /** Writes the zip into the cache folder (older exports go) and returns its content URI. */
    suspend fun create(graph: AppGraph): Uri {
        val context = graph.appContext
        val redactor = redactorFor(graph)
        val dir = File(context.cacheDir, DIR).also { it.mkdirs() }
        dir.listFiles()?.sortedByDescending { it.lastModified() }?.drop(KEEP - 1)?.forEach { it.delete() }
        val dump = StateDump.build(stateInput(graph), Instant.now().toString())
        val logDir = File(context.filesDir, "logs")
        val entries = buildList {
            add("state.txt" to redactor.apply(dump).toByteArray(Charsets.UTF_8))
            for (name in listOf("kin.log.1", "kin.log")) {
                val file = File(logDir, name)
                if (file.isFile) add(name to redactor.apply(file.readText(Charsets.UTF_8)).toByteArray(Charsets.UTF_8))
            }
        }
        val stamp = DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss").withZone(ZoneOffset.UTC).format(Instant.now())
        val zip = File(dir, "kin-logs-$stamp.zip")
        zip.outputStream().use { LogZip.write(it, entries) }
        graph.log.i("debug", "logs exported: ${zip.name}, ${zip.length()} bytes")
        return FileProvider.getUriForFile(context, "${context.packageName}.exports", zip)
    }

    /** The names the log may mention: every paired PC (there is one) and this phone's own names. */
    suspend fun redactorFor(graph: AppGraph): ExportRedactor {
        val pcNames = graph.database.devices().all().map { it.name }
        val phoneNames = listOf(graph.deviceName(), graph.deviceInfo.name)
        return ExportRedactor(pcNames, phoneNames)
    }

    /** The share sheet for [uri]. */
    fun shareIntent(uri: Uri): Intent {
        val send = Intent(Intent.ACTION_SEND)
            .setType("application/zip")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .putExtra(Intent.EXTRA_SUBJECT, "Kin logs")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        send.clipData = android.content.ClipData.newRawUri("Kin logs", uri)
        return Intent.createChooser(send, "Export Kin logs").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

    private fun isoTime(millis: Long): String = Instant.ofEpochMilli(millis).toString()
}
