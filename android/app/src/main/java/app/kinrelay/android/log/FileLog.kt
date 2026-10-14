package app.kinrelay.android.log

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.io.File
import java.time.Instant

/**
 * A rotating text log: `kin.log`, and `kin.log.1` once the first reaches [maxBytes].
 * Lines look like `2026-10-14T12:00:00.123Z I link: connected`.
 */
class FileLog(
    private val directory: File,
    private val maxBytes: Long = 256 * 1024,
    private val now: () -> Instant = Instant::now,
    private val mirror: (level: Char, line: String) -> Unit = { _, _ -> },
) : KinLog {
    private val file get() = File(directory, "kin.log")
    private val previous get() = File(directory, "kin.log.1")
    private val recentLines = MutableStateFlow<List<String>>(emptyList())

    /** The newest lines, oldest first, for the debug screen. */
    val recent: StateFlow<List<String>> = recentLines

    val logFile: File get() = file

    init {
        directory.mkdirs()
        recentLines.value = readTail(RECENT_LINES)
    }

    @Synchronized
    override fun log(level: Char, component: String, message: String) {
        val line = "${now()} $level $component: ${Redactor.redact(message).replace('\n', ' ')}"
        mirror(level, line)
        runCatching {
            if (file.length() >= maxBytes) rotate()
            file.appendText(line + "\n")
        }
        recentLines.value = (recentLines.value + line).takeLast(RECENT_LINES)
    }

    private fun rotate() {
        previous.delete()
        file.renameTo(previous)
    }

    private fun readTail(count: Int): List<String> {
        val lines = ArrayDeque<String>()
        for (source in listOf(previous, file)) {
            if (!source.exists()) continue
            source.forEachLine {
                lines.addLast(it)
                if (lines.size > count) lines.removeFirst()
            }
        }
        return lines.toList()
    }

    private companion object {
        const val RECENT_LINES = 200
    }
}
