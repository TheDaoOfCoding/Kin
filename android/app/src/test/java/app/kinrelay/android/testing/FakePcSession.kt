package app.kinrelay.android.testing

import java.io.File
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Not a unit test: a fake PC for the emulator smoke test. It runs only when KIN_FAKE_PC=1, listens on
 * 127.0.0.1:47831 (loopback only; the emulator reaches it as 10.0.2.2), accepts any pairing, and
 * serves for KIN_FAKE_PC_SECONDS (default 300). Files in `app/build/fake-pc/`:
 *   pair-url.txt  the deep link to open on the phone
 *   events.txt    what the PC saw, one line per event
 *   command.txt   write `ping`, `close4001`, `close4003`, `unpair`, `hello2` (future-protocol hello) or `stop` here to drive the PC.
 *                 Milestone 1.2: `clipboard <text>` (a `clipboard` message), `text <text>`, `url <link>` (a `text` with isUrl),
 *                 `noack` / `ack` (stop / resume acking the phone's `text`) and `errtext` (answer the phone's `text` with an `error`)
 *
 *   KIN_FAKE_PC=1 gradlew.bat testDebugUnitTest --tests "*FakePcSession*" --rerun
 */
class FakePcSession {
    @Test
    fun serve() {
        assumeTrue(System.getenv("KIN_FAKE_PC") == "1")
        val seconds = System.getenv("KIN_FAKE_PC_SECONDS")?.toLongOrNull() ?: 300
        val dir = File("build/fake-pc").also { it.mkdirs() }
        val events = File(dir, "events.txt").also { it.writeText("") }
        val command = File(dir, "command.txt").also { it.writeText("") }
        fun event(text: String) = events.appendText("${System.currentTimeMillis()} $text\n")

        FakePc(name = "Fake PC", fixedPort = 47831).apply { autoPair = true }.start().use { pc ->
            val token = "AAECAwQFBgcICQoLDA0ODw"
            File(dir, "pair-url.txt").writeText(
                "kinrelay://pair?v=1&a=10.0.2.2:${pc.port}&id=${pc.identity.deviceId}&tk=$token&n=Fake%20PC"
            )
            event("listening on 127.0.0.1:${pc.port} pcId=${pc.identity.deviceId}")
            pc.onFrame = { _, frame -> event("frame from phone: $frame") }
            var seen = 0
            val deadline = System.currentTimeMillis() + seconds * 1000
            while (System.currentTimeMillis() < deadline) {
                if (pc.requests.size > seen) {
                    pc.requests.drop(seen).forEach { event("http ${it.method} ${it.path} phone=${it.clientDeviceId} ct=${it.contentType} body=${it.body.replace(Regex("\"token\":\"[^\"]*\""), "\"token\":\"[redacted]\"")}") }
                    seen = pc.requests.size
                }
                val line = command.readText().trim()
                if (line.isNotEmpty()) {
                    command.writeText("")
                    event("command ${line.substringBefore(' ')}")
                    val text = line.substringBefore(' ')
                    val argument = line.substringAfter(' ', "")
                    runCatching {
                        when (text) {
                            "clipboard" -> pc.sendClipboard("fake-pc-clip-${System.nanoTime()}", argument)
                            "text" -> pc.sendText("fake-pc-text-${System.nanoTime()}", argument, isUrl = false)
                            "url" -> pc.sendText("fake-pc-text-${System.nanoTime()}", argument, isUrl = true)
                            "noack" -> pc.ackTexts = false
                            "ack" -> { pc.ackTexts = true; pc.errorForText = null }
                            "errtext" -> pc.errorForText = "internal"
                            "ping" -> pc.send("""{"type":"ping","id":"fake-pc-ping-${System.nanoTime()}","sentAt":${System.currentTimeMillis()}}""")
                            "close4001" -> pc.closeSocket(4001, "replaced")
                            "close4003" -> pc.closeSocket(4003, "unauthorized")
                            "unpair" -> pc.send("""{"type":"unpair","id":"fake-pc-unpair"}""")
                            "hello2" -> pc.send("""{"type":"hello","id":"h2","protocol":2}""")
                            "stop" -> return
                        }
                    }.onFailure { event("command failed: $it") }
                }
                Thread.sleep(200)
            }
        }
    }
}
