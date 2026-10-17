package app.kinrelay.android.testing

import java.io.File
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
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
 *                 Milestone 1.4: `holdtext <text>` (a text the PC kept while the phone was away: it goes out right after the phone's next `hello`), `used <transferId>` (that ID is used up: 404).
 *                 Milestone 1.3 (files): `offer <id> <spec>[,<spec>...]` where a spec is `name:size[:mime]` (deterministic bytes) or `name@WxH` (a
 *                 real JPEG or PNG), `cancel <transferId> [reason]` (a `cancel` from the PC), `gone <id>` (the offer expires: every GET is 404),
 *                 `changed <id> <fileId>` (a file changed: GET is 409), `postfail <status>` / `putfail <status>` (answer the phone's POST or PUT
 *                 with 507, 413, 500 ... ; `postfail 0` / `putfail 0` clears it), `dropput <bytes>` (the next PUT keeps that many bytes and drops
 *                 the connection), `dropget <bytes>` (the next GET sends only that many bytes), `throttle <bytes per 100 ms>` (slow GETs, 0 = off),
 *                 `stale <offset>` (the next repeated POST reports that offset), `ackmsg` / `noackmsg` (ack or ignore the phone's `complete` and `cancel`).
 *                 The events file also gets what the PC stored: `stored <transfer> <file> name='..' size=N complete=<bool> sha=<hex>`, the phone's
 *                 `complete` / `cancel` messages and DELETEs.
 *                 Milestone 2B.2 (notifications): this PC lists the `notif` capability from the start (a 0.2.0 PC with "Show phone notifications" on); every
 *                 frame from the phone, `notif.*` and the `ack` or `error` of a `notif.action`, is in the events file as `frame from phone: ...` (a test
 *                 stub: it keeps the test strings the scripts post, so never point it at real notifications). `notifoff` / `notifon` send a new `hello`
 *                 without or with `notif`; `dismiss <nid>` sends `notif.dismiss`; `action <nid> <i> <label>` a button press; `reply <nid> <i> <label>|<text>` a reply.
 *                 Milestone 2B.3 (media): the hello also lists `media` (`mediaoff` / `mediaon` send a new hello without or with it). Every frame from the
 *                 phone is in the events file as before (the debug sessions carry test strings only). `mstatus <show> <share>` (true|false each) sends a
 *                 `media.status`; `mstate <state> [title|artist|durationMs|positionMs|actions|art]` a `media.state` of a made-up PC player (actions are
 *                 comma separated, `art` is 0 or 1: a `media.image` goes first); `mclear`; `mvolume <level> <muted>`; `mcmd <cmd> [arg]` a `media.cmd` for the
 *                 `sid` of the phone's last `media.state` (`seek`'s arg is the position in ms, `volume`'s is `level=N`, `delta=N` or `mute=true|false`, which
 *                 need no session); `mcmdsid <sid> <cmd> [arg]` the same with a `sid` of your choice (a stale one); `mraw <json>` any frame as it is.
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

        FakePc(name = "Fake PC", fixedPort = 47831).apply { autoPair = true; helloCapabilities = listOf("link", "notif", "media") }.start().use { pc ->
            val token = "AAECAwQFBgcICQoLDA0ODw"
            File(dir, "pair-url.txt").writeText(
                "kinrelay://pair?v=1&a=10.0.2.2:${pc.port}&id=${pc.identity.deviceId}&tk=$token&n=Fake%20PC"
            )
            event("listening on 127.0.0.1:${pc.port} pcId=${pc.identity.deviceId}")
            // `holdtext`: texts the PC "queued" while the phone was away; they go out right after the phone's next hello (the PC's own queue, milestone 1.4).
            val held = java.util.concurrent.ConcurrentLinkedQueue<String>()
            // The sid of the phone's last `media.state`: what a `mcmd` of this stub is for.
            var phoneSid: String? = null
            pc.onFrame = { _, frame ->
                event("frame from phone: $frame")
                if (frame["type"]?.jsonPrimitive?.content == "media.state") phoneSid = frame["sid"]?.jsonPrimitive?.content
                if (frame["type"]?.jsonPrimitive?.content == "hello") {
                    while (true) { val t = held.poll() ?: break; pc.sendText("fake-pc-held-${System.nanoTime()}", t, isUrl = false); event("sent held text after the phone's hello") }
                }
            }
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
                // The driver (a script) may be writing the file right now: read it again at the next round.
                val line = runCatching { command.readText().trim() }.getOrDefault("")
                if (line.isNotEmpty()) {
                    runCatching { command.writeText("") }
                    event("command ${line.substringBefore(' ')}")
                    val text = line.substringBefore(' ')
                    val argument = line.substringAfter(' ', "")
                    runCatching {
                        when (text) {
                            "clipboard" -> pc.sendClipboard("fake-pc-clip-${System.nanoTime()}", argument)
                            "text" -> pc.sendText("fake-pc-text-${System.nanoTime()}", argument, isUrl = false)
                            "url" -> pc.sendText("fake-pc-text-${System.nanoTime()}", argument, isUrl = true)
                            "holdtext" -> held += argument
                            "used" -> pc.files.usedIds += argument.trim() // the PC has used this transfer ID up (canceled, failed or forgotten): POST, PUT and DELETE with it are 404
                            "notifon" -> { pc.helloCapabilities = listOf("link", "notif"); pc.send(pc.helloFrame()) }
                            "notifoff" -> { pc.helloCapabilities = listOf("link", "media"); pc.send(pc.helloFrame()) }
                            "mediaon" -> { pc.helloCapabilities = listOf("link", "notif", "media"); pc.send(pc.helloFrame()) }
                            "mediaoff" -> { pc.helloCapabilities = listOf("link", "notif"); pc.send(pc.helloFrame()) }
                            "mstatus" -> {
                                val (show, share) = argument.trim().split(' ')
                                pc.send(kotlinx.serialization.json.buildJsonObject { put("type", "media.status"); put("id", "fake-pc-mstatus-${System.nanoTime()}"); put("show", show.toBoolean()); put("share", share.toBoolean()) }.toString())
                            }
                            "mstate" -> {
                                val parts = argument.trim().split(' ', limit = 2)
                                val fields = parts.getOrElse(1) { "" }.split('|')
                                val art = fields.getOrElse(5) { "0" } == "1"
                                val artBytes = if (art) fakeArt() else null
                                if (artBytes != null) pc.send(kotlinx.serialization.json.buildJsonObject {
                                    put("type", "media.image"); put("id", "fake-pc-mimage-${System.nanoTime()}"); put("hash", sha(artBytes).take(32)); put("kind", "art")
                                    put("data", java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(artBytes))
                                }.toString())
                                pc.send(kotlinx.serialization.json.buildJsonObject {
                                    put("type", "media.state"); put("id", "fake-pc-mstate-${System.nanoTime()}"); put("sid", sha("fake.pc.player".toByteArray()).take(32)); put("app", "Fake Player")
                                    put("state", parts[0])
                                    fields.getOrNull(0)?.takeIf { it.isNotEmpty() }?.let { put("title", it) }
                                    fields.getOrNull(1)?.takeIf { it.isNotEmpty() }?.let { put("artist", it) }
                                    fields.getOrNull(2)?.takeIf { it.isNotEmpty() }?.let { put("durationMs", it.toLong()) }
                                    fields.getOrNull(3)?.takeIf { it.isNotEmpty() }?.let { put("positionMs", it.toLong()) }
                                    fields.getOrNull(4)?.takeIf { it.isNotEmpty() }?.let { list -> put("actions", kotlinx.serialization.json.JsonArray(list.split(',').map { kotlinx.serialization.json.JsonPrimitive(it) })) }
                                    if (artBytes != null) put("artHash", sha(artBytes).take(32))
                                    put("volume", kotlinx.serialization.json.buildJsonObject { put("level", 40); put("muted", false) })
                                }.toString())
                            }
                            "mclear" -> pc.send("""{"type":"media.clear","id":"fake-pc-mclear-${System.nanoTime()}"}""")
                            "mvolume" -> {
                                val (level, muted) = argument.trim().split(' ')
                                pc.send(kotlinx.serialization.json.buildJsonObject { put("type", "media.volume"); put("id", "fake-pc-mvolume-${System.nanoTime()}"); put("level", level.toInt()); put("muted", muted.toBoolean()) }.toString())
                            }
                            "mcmd", "mcmdsid" -> {
                                val parts = argument.trim().split(' ')
                                val sidArg = if (text == "mcmdsid") parts[0] else phoneSid
                                val cmd = parts[if (text == "mcmdsid") 1 else 0]
                                val arg = parts.getOrNull(if (text == "mcmdsid") 2 else 1)
                                pc.send(kotlinx.serialization.json.buildJsonObject {
                                    put("type", "media.cmd"); put("id", "fake-pc-mcmd-${System.nanoTime()}"); put("cmd", cmd)
                                    if (sidArg != null && cmd != "volume") put("sid", sidArg)
                                    when {
                                        cmd == "seek" -> put("seekMs", arg!!.toLong())
                                        cmd == "volume" && arg!!.startsWith("level=") -> put("level", arg.substringAfter('=').toInt())
                                        cmd == "volume" && arg!!.startsWith("delta=") -> put("delta", arg.substringAfter('=').toInt())
                                        cmd == "volume" -> put("mute", arg!!.substringAfter('=').toBoolean())
                                    }
                                }.toString())
                            }
                            "mraw" -> pc.send(argument)
                            "dismiss" -> pc.send(kotlinx.serialization.json.buildJsonObject {
                                put("type", "notif.dismiss"); put("id", "fake-pc-dismiss-${System.nanoTime()}"); put("nid", argument.trim())
                            }.toString())
                            "action", "reply" -> {
                                val parts = argument.split(' ', limit = 3)
                                val rest = parts.getOrElse(2) { "" }
                                val label = if (text == "reply") rest.substringBefore('|') else rest
                                pc.send(kotlinx.serialization.json.buildJsonObject {
                                    put("type", "notif.action"); put("id", "fake-pc-action-${System.nanoTime()}"); put("nid", parts[0]); put("i", parts[1].toInt()); put("label", label)
                                    if (text == "reply") put("text", rest.substringAfter('|'))
                                }.toString())
                            }
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

    /** A made-up cover: a 96 x 96 PNG of a generated gradient (what a PC's `media.image` of `art` is, small enough for the 24,576-byte cap). */
    private fun fakeArt(): ByteArray {
        val image = java.awt.image.BufferedImage(96, 96, java.awt.image.BufferedImage.TYPE_INT_RGB)
        for (y in 0 until 96) for (x in 0 until 96) image.setRGB(x, y, (x * 2) shl 16 or ((y * 2) shl 8) or 0x60)
        return java.io.ByteArrayOutputStream().also { javax.imageio.ImageIO.write(image, "png", it) }.toByteArray()
    }

    private fun sha(bytes: ByteArray): String = java.security.MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}
