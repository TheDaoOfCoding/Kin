package app.kinrelay.android.debuginfo

import app.kinrelay.android.log.Redactor
import java.io.ByteArrayOutputStream
import java.io.OutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** One connect or disconnect (or service event), read from the log. */
data class ConnectionEvent(val time: String, val kind: Kind, val detail: String) {
    enum class Kind { CONNECTED, DISCONNECTED, SERVICE_STARTED, SERVICE_STOPPED, KILL_DETECTED }
}

/**
 * The connect/disconnect history of the Debug screen. The local log already records every connect, disconnect (with its reason) and service
 * start or stop (milestone 1.1), so the history is read from it: nothing more is stored, and the two can never disagree.
 */
object ConnectionHistory {
    private val LINE = Regex("^(\\S+) [IWE] (link|service): (.*)$")

    /** The newest [limit] events of [lines] (log lines, oldest first), newest first. */
    fun parse(lines: List<String>, limit: Int = 40): List<ConnectionEvent> {
        val events = ArrayList<ConnectionEvent>()
        for (line in lines) {
            val m = LINE.matchEntire(line) ?: continue
            val (time, component, message) = m.destructured
            val kind = when {
                component == "link" && message.startsWith("connected to ") -> ConnectionEvent.Kind.CONNECTED
                component == "link" && message.startsWith("disconnected: ") -> ConnectionEvent.Kind.DISCONNECTED
                component == "service" && message.startsWith("kill detected") -> ConnectionEvent.Kind.KILL_DETECTED
                component == "service" && message.startsWith("service started") -> ConnectionEvent.Kind.SERVICE_STARTED
                component == "service" && message.startsWith("service stopped") -> ConnectionEvent.Kind.SERVICE_STOPPED
                else -> continue
            }
            events += ConnectionEvent(time, kind, message)
        }
        return events.takeLast(limit).asReversed()
    }
}

/** The numbers of a list: last, median, and how many. */
data class LatencySummary(val last: Double?, val median: Double?, val count: Int) {
    companion object {
        fun of(values: List<Double>): LatencySummary {
            if (values.isEmpty()) return LatencySummary(null, null, 0)
            val sorted = values.sorted()
            val median = if (sorted.size % 2 == 1) sorted[sorted.size / 2] else (sorted[sorted.size / 2 - 1] + sorted[sorted.size / 2]) / 2
            return LatencySummary(values.last(), median, values.size)
        }
    }
}

/**
 * The state dump that goes into the exported logs. **No secrets and no text contents**: it lists versions, counts, states and error codes,
 * never a key, a token, a text, a file name or a message of the user's. Whatever it is made of also goes through [Redactor].
 */
object StateDump {
    data class Input(
        val appVersion: String,
        val sdk: Int,
        val manufacturer: String,
        val model: String,
        val linkState: String,
        val paired: Boolean,
        val phoneShortId: String?,
        val pcShortId: String?,
        val connects: Int,
        val disconnects: Int,
        val kills: Int,
        val successRate: String,
        val rtt: LatencySummary,
        val arrival: LatencySummary,
        val queued: Int,
        val recentErrors: List<Pair<String, String>>,
        val notificationsAllowed: Boolean,
        val localNetworkAllowed: Boolean,
        val batteryUnrestricted: Boolean,
        val oemGuide: String?,
    )

    fun build(i: Input, nowIso: String): String = Redactor.redact(
        buildString {
            appendLine("Kin state dump, $nowIso")
            appendLine("app ${i.appVersion}, Android API ${i.sdk}, ${i.manufacturer} ${i.model}")
            appendLine("link: ${i.linkState}; paired=${i.paired}; phone=${i.phoneShortId ?: "?"} pc=${i.pcShortId ?: "?"}")
            appendLine("connects=${i.connects} disconnects=${i.disconnects} service kills=${i.kills}")
            appendLine("first-try success: ${i.successRate}")
            appendLine("ping rtt ms: last=${i.rtt.last?.let { "%.1f".format(java.util.Locale.US, it) } ?: "-"} median=${i.rtt.median?.let { "%.1f".format(java.util.Locale.US, it) } ?: "-"} n=${i.rtt.count}")
            appendLine("tap to ack ms: last=${i.arrival.last?.toLong() ?: "-"} median=${i.arrival.median?.toLong() ?: "-"} n=${i.arrival.count}")
            appendLine("queued items=${i.queued}")
            appendLine("permissions: notifications=${i.notificationsAllowed} nearbyDevices=${i.localNetworkAllowed} batteryUnrestricted=${i.batteryUnrestricted} oem=${i.oemGuide ?: "none"}")
            appendLine("recent errors (code, when):")
            if (i.recentErrors.isEmpty()) appendLine("  none")
            i.recentErrors.forEach { (code, at) -> appendLine("  $code  $at") }
        }
    )
}

/** Packs the log files and the state dump into one zip. */
object LogZip {
    /** [entries] is name to bytes, in the order to write them. */
    fun write(out: OutputStream, entries: List<Pair<String, ByteArray>>) {
        ZipOutputStream(out).use { zip ->
            for ((name, bytes) in entries) {
                zip.putNextEntry(ZipEntry(name))
                zip.write(bytes)
                zip.closeEntry()
            }
        }
    }

    fun toBytes(entries: List<Pair<String, ByteArray>>): ByteArray = ByteArrayOutputStream().also { write(it, entries) }.toByteArray()
}
