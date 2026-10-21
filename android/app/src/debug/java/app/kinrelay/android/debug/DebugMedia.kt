package app.kinrelay.android.debug

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import app.kinrelay.android.AppGraph
import app.kinrelay.android.media.MediaAction
import app.kinrelay.android.media.MediaMapping
import app.kinrelay.android.media.MediaState
import app.kinrelay.android.media.MediaVolume
import app.kinrelay.android.media.PlayState
import app.kinrelay.android.media.debugClearPc
import app.kinrelay.android.media.debugPcCommand
import app.kinrelay.android.media.debugReadAll
import app.kinrelay.android.media.debugReadAllOn
import app.kinrelay.android.media.debugShowPc
import app.kinrelay.android.media.debugState
import app.kinrelay.android.media.pcSessionShown
import app.kinrelay.android.notifmirror.MirrorListenerService
import kotlinx.coroutines.delay

/**
 * The 2B.3 debug commands (design 4.9). Every one prints a `CMD ...` line with counts and settings, never a title, artist, album or art. The test session is
 * [DebugMediaSession]: silent, no audio. Nothing here touches another app's session.
 *
 *   MEDIA_SET [--ez share b] [--ez show b] [--ez lockscreen b] [--ez volkeys b]
 *   MEDIA_TEST_START [--es title t] [--es artist a] [--ei durationMs n] [--ez art false]
 *   MEDIA_TEST_TRACK [--es title t] [--es artist a] [--ei durationMs n]
 *   MEDIA_TEST_STATE --es state playing|paused|buffering|stopped [--ei positionMs n] [--ei rateMilli n] [--es actions play,pause,toggle,next,prev,seek]
 *   MEDIA_TEST_STOP
 *   MEDIA_TEST_BURST [--ei count 200]  (count state changes in about 3 s)
 *   MEDIA_STATE
 *   MEDIA_PC_CMD --es cmd play|pause|next|prev|seek|volume [--ei positionMs n] [--ei level n] [--ei delta n]
 *   MEDIA_PC_FAKE --es state s [--es title t] [--es artist a] [--es album a] [--ei durationMs n] [--ei positionMs n] [--ei rateMilli n] [--es actions a,b] [--ez art true]
 *   MEDIA_PC_FAKE_CLEAR
 *   MEDIA_VOLUME --ei level n          (the phone's STREAM_MUSIC; the emulator is not the user's device)
 *   MEDIA_READ_ALL [--ez on b]         (the reader takes every session, not only the debug one: emulator only)
 *   MEDIA_PROBE                        (spike S3: what the platform lets Kin do; counts and results only)
 */
object DebugMedia {
    suspend fun handle(command: String, intent: Intent, context: Context, graph: AppGraph) {
        when (command) {
            "MEDIA_SET" -> {
                val store = graph.media.store
                if (intent.hasExtra("share")) store.setShare(intent.getBooleanExtra("share", false))
                if (intent.hasExtra("show")) store.setShow(intent.getBooleanExtra("show", false))
                if (intent.hasExtra("lockscreen")) store.setShowOnLockScreen(intent.getBooleanExtra("lockscreen", false))
                if (intent.hasExtra("volkeys")) store.setUseVolumeKeys(intent.getBooleanExtra("volkeys", false))
                delay(300)
                val p = store.current()
                graph.log.i("debug", "CMD MEDIA_SET share=${p.share} show=${p.show} lockscreen=${p.showOnLockScreen} volkeys=${p.useVolumeKeys}")
            }
            "MEDIA_TEST_START" -> {
                DebugMediaSession.start(
                    context, intent.getStringExtra("title"), intent.getStringExtra("artist"),
                    intent.takeIf { it.hasExtra("durationMs") }?.getIntExtra("durationMs", 0)?.toLong(), art = intent.getBooleanExtra("art", true),
                )
                graph.log.i("debug", "CMD MEDIA_TEST_START running=${DebugMediaSession.isRunning}")
            }
            "MEDIA_TEST_TRACK" -> {
                DebugMediaSession.setTrack(
                    intent.getStringExtra("title"), intent.getStringExtra("artist"),
                    intent.takeIf { it.hasExtra("durationMs") }?.getIntExtra("durationMs", 0)?.toLong(),
                )
                graph.log.i("debug", "CMD MEDIA_TEST_TRACK done")
            }
            "MEDIA_TEST_STATE" -> {
                val actions = intent.getStringExtra("actions")?.split(',')?.mapNotNull { MediaAction.fromWire(it.trim()) }
                val mask = if (actions == null) DebugMediaSession.ALL_ACTIONS else MediaMapping.maskOf(actions)
                DebugMediaSession.setState(intent.getStringExtra("state") ?: "paused", intent.getIntExtra("positionMs", 0).toLong(), intent.getIntExtra("rateMilli", 1000), mask)
                graph.log.i("debug", "CMD MEDIA_TEST_STATE done")
            }
            "MEDIA_TEST_STOP" -> {
                DebugMediaSession.stop()
                graph.log.i("debug", "CMD MEDIA_TEST_STOP running=${DebugMediaSession.isRunning}")
            }
            "MEDIA_STATE" -> {
                val audio = context.getSystemService(AudioManager::class.java)
                graph.log.i(
                    "debug",
                    "CMD MEDIA_STATE ${graph.media.debugState()} testSession=${DebugMediaSession.isRunning} cmdsReceived={${DebugMediaSession.commands.entries.sortedBy { it.key }.joinToString(",") { "${it.key}:${it.value.get()}" }}} " +
                        "lastCmd=${DebugMediaSession.lastCommand} musicVolume=${audio.getStreamVolume(AudioManager.STREAM_MUSIC)}/${audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)} musicMuted=${audio.isStreamMute(AudioManager.STREAM_MUSIC)} sdk=${android.os.Build.VERSION.SDK_INT}",
                )
            }
            // 200 changes of the test session in about 3 s (the sender's rate rules, design 9.2 step 8).
            "MEDIA_TEST_BURST" -> {
                val count = intent.getIntExtra("count", 200)
                for (i in 0 until count) {
                    DebugMediaSession.setState(if (i % 2 == 0) "playing" else "paused", i * 1000L, 1000, DebugMediaSession.ALL_ACTIONS)
                    delay(15)
                }
                graph.log.i("debug", "CMD MEDIA_TEST_BURST done count=$count")
            }
            "MEDIA_PC_CMD" -> {
                val result = graph.media.debugPcCommand(intent.getStringExtra("cmd") ?: "play", intent.getIntExtra("positionMs", 0).toLong(), intent.getIntExtra("level", 50), intent.getIntExtra("delta", 5))
                graph.log.i("debug", "CMD MEDIA_PC_CMD result=$result")
            }
            "MEDIA_PC_FAKE" -> {
                val state = PlayState.fromWire(intent.getStringExtra("state") ?: "paused") ?: PlayState.PAUSED
                val actions = intent.getStringExtra("actions")?.split(',')?.mapNotNull { MediaAction.fromWire(it.trim()) } ?: MediaAction.entries
                val fake = MediaState(
                    sid = app.kinrelay.android.media.MediaWire.sidOf("debug.fake.pc.player"), app = "Fake Player", state = state,
                    title = intent.getStringExtra("title") ?: "PC test song", artist = intent.getStringExtra("artist") ?: "PC test artist", album = intent.getStringExtra("album"),
                    durationMs = intent.takeIf { it.hasExtra("durationMs") }?.getIntExtra("durationMs", 0)?.toLong() ?: 200_000L,
                    positionMs = intent.takeIf { it.hasExtra("positionMs") }?.getIntExtra("positionMs", 0)?.toLong() ?: 30_000L,
                    rateMilli = intent.getIntExtra("rateMilli", 1000), actions = actions, volume = MediaVolume(40, false),
                )
                graph.media.debugShowPc(fake, if (intent.getBooleanExtra("art", true)) DebugMediaSession.pcArtJpeg() else null, intent.getBooleanExtra("volkeys", false))
                graph.log.i("debug", "CMD MEDIA_PC_FAKE shown=${graph.media.pcSessionShown()}")
            }
            "MEDIA_PC_FAKE_CLEAR" -> {
                graph.media.debugClearPc()
                graph.log.i("debug", "CMD MEDIA_PC_FAKE_CLEAR shown=${graph.media.pcSessionShown()}")
            }
            "MEDIA_VOLUME" -> {
                val audio = context.getSystemService(AudioManager::class.java)
                val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                val index = Math.round(intent.getIntExtra("level", 50) * max / 100f)
                val result = try {
                    audio.setStreamVolume(AudioManager.STREAM_MUSIC, index, 0)
                    "ok"
                } catch (e: SecurityException) {
                    "SecurityException"
                }
                graph.log.i("debug", "CMD MEDIA_VOLUME result=$result index=${audio.getStreamVolume(AudioManager.STREAM_MUSIC)}/$max")
            }
            "MEDIA_READ_ALL" -> {
                graph.media.debugReadAll(intent.getBooleanExtra("on", true))
                graph.log.i("debug", "CMD MEDIA_READ_ALL on=${graph.media.debugReadAllOn()}")
            }
            "MEDIA_PROBE" -> probe(context, graph)
        }
    }

    /** Spike S3 (a), (c), (e): what the platform allows Kin. **Counts only**: no package name, session tag or title of any session is printed or logged. */
    private suspend fun probe(context: Context, graph: AppGraph) {
        val sessions = context.getSystemService(MediaSessionManager::class.java)
        val listener = ComponentName(context, MirrorListenerService::class.java)
        val access = graph.mirror.accessGranted()
        val controllers: List<MediaController>? = try {
            sessions.getActiveSessions(listener)
        } catch (e: SecurityException) {
            graph.log.i("debug", "CMD MEDIA_PROBE getActiveSessions access=$access SecurityException")
            null
        }
        if (controllers != null) {
            val playing = controllers.count { it.playbackState?.state == android.media.session.PlaybackState.STATE_PLAYING }
            graph.log.i("debug", "CMD MEDIA_PROBE getActiveSessions access=$access ok n=${controllers.size} playing=$playing own=${controllers.count { it.packageName == context.packageName }}")
            val debug = controllers.firstOrNull { it.extras?.getBoolean(DebugMediaSession.EXTRA) == true }
            if (debug != null) {
                debug.transportControls.play()
                debug.transportControls.skipToNext()
                debug.transportControls.seekTo(12_345L)
                delay(800)
                graph.log.i("debug", "CMD MEDIA_PROBE transportControls via the listener's controller: cmdsReceived={${DebugMediaSession.commands.entries.sortedBy { it.key }.joinToString(",") { "${it.key}:${it.value.get()}" }}} last=${DebugMediaSession.lastCommand}")
            } else {
                graph.log.i("debug", "CMD MEDIA_PROBE no debug session found to try transportControls on")
            }
        }
        val audio = context.getSystemService(AudioManager::class.java)
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        val before = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
        fun trySet(index: Int): String = try {
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, index, 0)
            "ok(now ${audio.getStreamVolume(AudioManager.STREAM_MUSIC)})"
        } catch (e: SecurityException) {
            "SecurityException"
        }
        val to1 = trySet(1)
        val to0 = trySet(0)
        val mute = try {
            audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_MUTE, 0)
            "ok(muted=${audio.isStreamMute(AudioManager.STREAM_MUSIC)})"
        } catch (e: SecurityException) {
            "SecurityException"
        }
        val unmute = try {
            audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_UNMUTE, 0)
            "ok(muted=${audio.isStreamMute(AudioManager.STREAM_MUSIC)})"
        } catch (e: SecurityException) {
            "SecurityException"
        }
        trySet(before)
        val policy = context.getSystemService(android.app.NotificationManager::class.java)
        graph.log.i(
            "debug",
            "CMD MEDIA_PROBE volume max=$max before=$before to1=$to1 to0=$to0 mute=$mute unmute=$unmute fixed=${audio.isVolumeFixed} " +
                "dndAccess=${policy.isNotificationPolicyAccessGranted} interruptionFilter=${policy.currentInterruptionFilter} ringer=${audio.ringerMode}",
        )
    }
}
