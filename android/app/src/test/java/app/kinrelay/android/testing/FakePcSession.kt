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
 *                 Milestone 1.3 (files): `offer <id> <spec>[,<spec>...]` where a spec is `name:size[:mime]` (deterministic bytes) or `name@WxH` (a
 *                 real JPEG or PNG), `cancel <transferId> [reason]` (a `cancel` from the PC), `gone <id>` (the offer expires: every GET is 404),
 *                 `changed <id> <fileId>` (a file changed: GET is 409), `postfail <status>` / `putfail <status>` (answer the phone's POST or PUT
 *                 with 507, 413, 500 ... ; `postfail 0` / `putfail 0` clears it), `dropput <bytes>` (the next PUT keeps that many bytes and drops
 *                 the connection), `dropget <bytes>` (the next GET sends only that many bytes), `throttle <bytes per 100 ms>` (slow GETs, 0 = off),
 *                 `stale <offset>` (the next repeated POST reports that offset), `ackmsg` / `noackmsg` (ack or ignore the phone's `complete` and `cancel`).
 *                 The events file also gets what the PC stored: `stored <transfer> <file> name='..' size=N complete=<bool> sha=<hex>`, the phone's
 *                 `complete` / `cancel` messages and DELETEs.
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
            val reported = HashMap<String, String>()
            var messagesSeen = 0
            var deletedSeen = 0
            var callsSeen = 0
            val deadline = System.currentTimeMillis() + seconds * 1000
            while (System.currentTimeMillis() < deadline) {
                if (pc.requests.size > seen) {
                    pc.requests.drop(seen).forEach { event("http ${it.method} ${it.path} phone=${it.clientDeviceId} ct=${it.contentType} body=${it.body.replace(Regex("\"token\":\"[^\"]*\""), "\"token\":\"[redacted]\"")}") }
                    seen = pc.requests.size
                }
                // What the PC stored and was told, once per change.
                pc.files.uploads.values.forEach { up ->
                    up.files.values.forEach { f ->
                        val state = "stored ${up.id} ${f.fileId} name='${f.name}' size=${f.size} have=${f.data.size} complete=${f.complete} sha=${if (f.complete) sha(f.data) else "-"}"
                        if (reported[up.id + f.fileId] != state) { reported[up.id + f.fileId] = state; event(state) }
                    }
                }
                pc.files.messages.drop(messagesSeen).forEach { event("phone message $it") }
                messagesSeen = pc.files.messages.size
                pc.files.deleted.drop(deletedSeen).forEach { event("phone deleted transfer $it") }
                deletedSeen = pc.files.deleted.size
                pc.files.calls.drop(callsSeen).forEach { event("files ${it.method} ${it.path} offset=${it.offset} range=${it.range} contentLength=${it.contentLength}") }
                callsSeen = pc.files.calls.size
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
                            "offer" -> {
                                val id = argument.substringBefore(' ')
                                val files = argument.substringAfter(' ').split(',').mapIndexed { i, spec -> offered("f${i + 1}", spec.trim()) }
                                pc.sendOffer(id, *files.toTypedArray())
                                files.forEach { event("offered $id ${it.fileId} name='${it.name}' size=${it.bytes.size} mime=${it.mime} sha=${sha(it.bytes)}") }
                            }
                            "cancel" -> pc.sendCancel(argument.substringBefore(' '), argument.substringAfter(' ', "user"))
                            "gone" -> pc.files.offers[argument.trim()]?.gone = true
                            "changed" -> pc.files.offers[argument.substringBefore(' ')]?.files?.get(argument.substringAfter(' '))?.changed = true
                            "postfail" -> pc.files.postStatus = argument.trim().toInt().takeIf { it > 0 }
                            "putfail" -> pc.files.putStatus = argument.trim().toInt().takeIf { it > 0 }
                            "dropput" -> pc.files.dropPutKeeping = argument.trim().toLong()
                            "dropget" -> pc.files.dropGetAfter = argument.trim().toLong()
                            "throttle" -> pc.files.throttleGetBytesPer100Ms = argument.trim().toLong().takeIf { it > 0 }
                            "stale" -> pc.files.staleOffsetOnce = argument.trim().toLong()
                            "ackmsg" -> pc.files.ackMessages = true
                            "noackmsg" -> pc.files.ackMessages = false
                            "stop" -> return
                        }
                    }.onFailure { event("command failed: $it") }
                }
                Thread.sleep(200)
            }
        }
    }

    /** A spec of the `offer` command: `name:size[:mime]` or `name@WxH`. */
    private fun offered(fileId: String, spec: String): FakePcFiles.OfferedFile {
        val picture = spec.substringAfter('@', "")
        if (picture.isNotEmpty()) {
            val name = spec.substringBefore('@')
            val (w, h) = picture.split('x').map { it.toInt() }
            val image = java.awt.image.BufferedImage(w, h, java.awt.image.BufferedImage.TYPE_INT_RGB)
            val random = java.util.Random(name.hashCode().toLong())
            for (y in 0 until h step 4) for (x in 0 until w step 4) image.setRGB(x, y, random.nextInt(0xFFFFFF))
            val format = if (name.endsWith(".png", true)) "png" else "jpg"
            val out = java.io.ByteArrayOutputStream().also { javax.imageio.ImageIO.write(image, format, it) }
            return FakePcFiles.OfferedFile(fileId, name, if (format == "png") "image/png" else "image/jpeg", out.toByteArray())
        }
        val parts = spec.split(':')
        val size = parts.getOrNull(1)?.toInt() ?: 0
        return FakePcFiles.OfferedFile(fileId, parts[0], parts.getOrNull(2) ?: "application/octet-stream", ByteArray(size).also { java.util.Random(parts[0].hashCode().toLong()).nextBytes(it) })
    }

    private fun sha(bytes: ByteArray): String = java.security.MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}
