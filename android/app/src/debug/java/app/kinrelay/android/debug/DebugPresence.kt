package app.kinrelay.android.debug

import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.provider.MediaStore
import androidx.core.graphics.createBitmap
import app.kinrelay.android.AppGraph
import app.kinrelay.android.data.Durable
import app.kinrelay.android.pause.PauseChoice
import app.kinrelay.android.pause.PauseState
import kotlinx.coroutines.delay
import java.io.File

/**
 * The 2B.4 debug commands (design 5.9): presence, pause, ring and auto-send. Every one prints a `CMD ...` line (debug builds only; the log export drops them). The ring can run silently
 * (`RING_SET --ez silent true`: no player, no vibrator, and the alarm volume is not touched). The emulator's battery is set from the shell (`adb shell dumpsys battery set level 14`, `set ac 0`,
 * `reset`: it changes the **emulator's** battery only). Screenshots are **generated** images inserted into the emulator's MediaStore: no screen capture, no real photo.
 *
 *   PAUSE_SET --es mode off|1h|tomorrow|resume|untilresumed [--el untilMs N]
 *   PAUSE_STATE
 *   PRESENCE_STATE
 *   PRESENCE_SET [--ez share b] [--ez show b]
 *   RING_SET [--ez allow b] [--ez silent b] [--ez loud b]
 *   RING_STATE
 *   RING_STOP_LOCAL
 *   SHOTS_SET [--ez screenshots b] [--ez camera b]   (takes the baseline first when turning one on; the permission is granted with `pm grant`)
 *   SHOTS_STATE
 *   SHOTS_INSERT --es kind screenshot|camera|other|own [--ei ageSec N]
 *   SHOTS_CANCEL --el id N
 *   DB_INFO                                           (journal mode, `PRAGMA synchronous`, and a checkpoint)
 */
object DebugPresence {
    private val COMMANDS = setOf(
        "PAUSE_SET", "PAUSE_STATE", "PRESENCE_STATE", "PRESENCE_SET", "RING_SET", "RING_STATE", "RING_STOP_LOCAL",
        "SHOTS_SET", "SHOTS_STATE", "SHOTS_INSERT", "SHOTS_CANCEL", "DB_INFO",
    )

    fun handles(command: String): Boolean = command in COMMANDS

    suspend fun handle(command: String, intent: Intent, context: Context, graph: AppGraph) {
        when (command) {
            "PAUSE_SET" -> {
                if (intent.hasExtra("untilMs")) {
                    // An exact end (epoch ms), for the expiry and the 8-day guard: `--el untilMs N`.
                    graph.pause.setRaw(PauseState(untilMillis = intent.getLongExtra("untilMs", 0L)))
                } else {
                    when (val mode = intent.getStringExtra("mode") ?: "off") {
                        "off", "resume" -> graph.pause.resume()
                        "1h" -> graph.pause.set(PauseChoice.HOUR)
                        "tomorrow" -> graph.pause.set(PauseChoice.TOMORROW)
                        "untilresumed" -> graph.pause.set(PauseChoice.UNTIL_RESUMED)
                        else -> graph.log.w("debug", "CMD PAUSE_SET unknown mode $mode")
                    }
                }
                delay(300)
                pauseState(graph, "PAUSE_SET")
            }
            "PAUSE_STATE" -> pauseState(graph, "PAUSE_STATE")
            "PRESENCE_STATE" -> graph.log.i("debug", "CMD PRESENCE_STATE ${graph.presence.debugLine()}")
            "PRESENCE_SET" -> {
                if (intent.hasExtra("share")) graph.presenceStore.setShare(intent.getBooleanExtra("share", true))
                if (intent.hasExtra("show")) graph.presenceStore.setShow(intent.getBooleanExtra("show", true))
                delay(300)
                graph.log.i("debug", "CMD PRESENCE_SET ${graph.presence.debugLine()}")
            }
            "RING_SET" -> {
                if (intent.hasExtra("allow")) graph.presenceStore.setRingAllow(intent.getBooleanExtra("allow", true))
                if (intent.hasExtra("loud")) graph.presenceStore.setRingLoud(intent.getBooleanExtra("loud", true))
                if (intent.hasExtra("silent")) {
                    val flag = File(context.filesDir, "debug-ring-silent")
                    if (intent.getBooleanExtra("silent", false)) flag.writeText("1") else flag.delete()
                }
                delay(300)
                graph.log.i("debug", "CMD RING_SET ${graph.ring.debugLine()}")
            }
            "RING_STATE" -> graph.log.i("debug", "CMD RING_STATE ${graph.ring.debugLine()}")
            "RING_STOP_LOCAL" -> {
                graph.ring.stopByUser()
                graph.log.i("debug", "CMD RING_STOP_LOCAL ${graph.ring.debugLine()}")
            }
            "SHOTS_SET" -> {
                val screenshots = if (intent.hasExtra("screenshots")) intent.getBooleanExtra("screenshots", false) else null
                val camera = if (intent.hasExtra("camera")) intent.getBooleanExtra("camera", false) else null
                var refused = false
                if (screenshots == true || camera == true) {
                    refused = !graph.shots.turnOn(if (screenshots == true) true else null, if (camera == true) true else null)
                }
                graph.shots.turnOff(if (screenshots == false) false else null, if (camera == false) false else null)
                delay(300)
                graph.shots.reevaluate()
                delay(300)
                graph.log.i("debug", "CMD SHOTS_SET refused=$refused ${graph.shots.debugLine()}")
            }
            "SHOTS_STATE" -> graph.log.i("debug", "CMD SHOTS_STATE ${graph.shots.debugLine()}")
            "SHOTS_CANCEL" -> {
                graph.shots.cancel(intent.getLongExtra("id", -1L))
                delay(300)
                graph.log.i("debug", "CMD SHOTS_CANCEL ${graph.shots.debugLine()}")
            }
            "SHOTS_INSERT" -> insertImage(intent, context, graph)
            "DB_INFO" -> {
                Durable.flush(graph.database, graph.log)
                graph.log.i("debug", "CMD DB_INFO journal=${Durable.journalMode(graph.database)} synchronous=${Durable.synchronousMode(graph.database)} (0 OFF, 1 NORMAL, 2 FULL, 3 EXTRA)")
            }
        }
    }

    private fun pauseState(graph: AppGraph, name: String) {
        val v = graph.pause.view.value
        graph.log.i("debug", "CMD $name paused=${v.paused} untilResumed=${v.state.untilResumed} untilMs=${v.state.untilMillis} quiet=${v.receivedWhilePaused} peerPaused=${graph.presence.peerPaused()}")
    }

    /** Inserts a **generated** PNG into the emulator's MediaStore at the path of [kind] (`screenshot`: `Pictures/Screenshots/`, `camera`: `DCIM/Camera/`, `other`: `Pictures/Other/`, `own`: `Pictures/Kin/`). */
    private suspend fun insertImage(intent: Intent, context: Context, graph: AppGraph) {
        val kind = intent.getStringExtra("kind") ?: "screenshot"
        val folder = when (kind) {
            "screenshot" -> "Pictures/Screenshots/"
            "camera" -> "DCIM/Camera/"
            "own" -> "Pictures/Kin/"
            else -> "Pictures/Other/"
        }
        val ageSec = intent.getIntExtra("ageSec", 0)
        val flag = File(context.filesDir, "debug-shots-offset")
        if (ageSec > 0) flag.writeText(ageSec.toString())
        val name = "kin-debug-${kind}-${System.currentTimeMillis()}.png"
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.MIME_TYPE, "image/png")
            put(MediaStore.MediaColumns.RELATIVE_PATH, folder)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
        if (uri == null) {
            graph.log.w("debug", "CMD SHOTS_INSERT failed: no uri")
            return
        }
        val bitmap = createBitmap(120, 240)
        Canvas(bitmap).apply {
            drawColor(Color.rgb((kind.hashCode() and 0x7F) + 64, 90, 140))
            drawText("kin debug", 10f, 40f, Paint().apply { color = Color.WHITE; textSize = 22f })
        }
        resolver.openOutputStream(uri)?.use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        graph.log.i("debug", "CMD SHOTS_INSERT kind=$kind ageSec=$ageSec id=${uri.lastPathSegment}")
        if (ageSec > 0) {
            // The picker runs 500 ms after the signal; the moved clock is put back after it has looked.
            delay(3_000)
            flag.delete()
        }
    }
}
