package app.kinrelay.android.ring

import android.app.NotificationManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Build
import android.os.PowerManager
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import app.kinrelay.android.AppGraph
import app.kinrelay.android.DebugHooks
import app.kinrelay.android.R
import app.kinrelay.android.dnd.AlarmAudibility
import app.kinrelay.android.features.FeatureIds
import app.kinrelay.android.link.Clock
import app.kinrelay.android.presence.CoroutineTimers
import app.kinrelay.android.presence.PresenceWire
import app.kinrelay.android.protocol.Frames
import app.kinrelay.android.protocol.MessageIds
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/** What the Stop screen and Settings show: whether the phone rings now, for which PC, and when the 30 s end (monotonic ms). */
data class RingUi(val ringing: Boolean = false, val pcName: String = "", val endsAtElapsed: Long = 0L, val audible: Boolean = true)

/**
 * Ring my phone as the app sees it (design 5.4): one per process, made by [AppGraph], living in the process the link service holds (**no new foreground-service type**, no
 * `FOREGROUND_SERVICE_MEDIA_PLAYBACK`: Kin plays no media). It starts nothing by itself. The sound is a `MediaPlayer` on `USAGE_ALARM` (independent of the ringer mode, so a phone on silent
 * rings), with a vibration on the alarm usage; **no audio-focus request**. The user's alarm volume is saved before it is raised and put back on every stop and at every service start.
 */
class RingController(private val graph: AppGraph) {
    private val context: Context = graph.appContext
    private val clock = Clock { SystemClock.elapsedRealtime() }
    private val audio = context.getSystemService(AudioManager::class.java)
    private val notifications = context.getSystemService(NotificationManager::class.java)

    @Volatile private var allowedNow: Boolean? = null

    @Volatile private var loudNow: Boolean? = null

    private val mutableUi = MutableStateFlow(RingUi())
    val ui: StateFlow<RingUi> = mutableUi

    private val ports = AndroidRingPorts()

    val machine = RingMachine(
        ports, clock, CoroutineTimers(graph.scope),
        allowed = { allowedNow ?: runBlocking { graph.features.enabled(FeatureIds.RING_ALLOW) }.also { allowedNow = it } },
        loud = { loudNow ?: runBlocking { graph.presenceStore.ringLoudNow() }.also { loudNow = it } },
        pcName = { graph.link.runtime?.currentSession?.peerName ?: "Your PC" },
        log = graph.log,
    )

    /** Called once when the app starts: follows "Let my PC ring this phone" and says it in the `hello` (the `ring` capability exists only while it is on). Reacts only. */
    fun start() {
        graph.scope.launch {
            graph.features.flow(FeatureIds.RING_ALLOW).collect { on ->
                allowedNow = on
                if (!on) machine.allowedChanged()
                graph.link.runtime?.currentSession?.updateCapabilities(Frames.withRing(on))
            }
        }
        graph.scope.launch { graph.presenceStore.ringLoud.collect { loudNow = it } }
    }

    /** At every `LinkService` start: an alarm volume left raised by an interrupted ring is put back (nothing rings in a fresh process). */
    fun restoreAtStart() {
        if (machine.restoreAtStart()) graph.log.i("ring", "ring: restored the alarm volume after an interrupted ring")
    }

    suspend fun onStartFromPc(id: String, reply: (String) -> Unit) {
        machine.start { r ->
            reply(
                when (r) {
                    RingReply.Ack -> Frames.ack(id)
                    is RingReply.Error -> Frames.error(r.code, r.message, id)
                },
            )
        }
    }

    suspend fun onStopFromPc(id: String, reply: (String) -> Unit) {
        machine.stopFromPc { reply(Frames.ack(id)) }
    }

    /** The Stop button of the notification, the full-screen screen, and the app. */
    fun stopByUser() = machine.stopByUser()

    /** Unlinking: no ring, no raised volume, nobody to tell. */
    fun onUnlink() = machine.onUnlink()

    /** `Settings > Special app access > Full-screen intents` can be granted for Kin on Android 14+ (always true before). */
    fun canFullScreen(): Boolean = Build.VERSION.SDK_INT < 34 || notifications.canUseFullScreenIntent()

    /** The debug state (`RING_STATE`). */
    fun debugLine(): String =
        "state=${if (machine.isRinging) "ringing" else "idle"} audible=${machine.audible} ${machine.stats.line()} lastReason=${machine.lastStopReason} " +
            "savedAlarmIndex=${ports.savedIndex()} canFullScreen=${canFullScreen()} silent=${DebugHooks.ringSilent(context)}"

    private inner class AndroidRingPorts : RingPorts {
        private var player: MediaPlayer? = null
        private var wake: PowerManager.WakeLock? = null
        private val vibrator: Vibrator? = if (Build.VERSION.SDK_INT >= 31) context.getSystemService(VibratorManager::class.java)?.defaultVibrator else @Suppress("DEPRECATION") context.getSystemService(Vibrator::class.java)

        @Volatile private var saved: Int? = null
        private var savedLoaded = false

        private val silent: Boolean get() = DebugHooks.ringSilent(context)

        // A debug "RING_SILENT" ring touches neither the player, the vibrator nor the alarm volume (the logic is the same; the save/raise/restore is exercised on a fake in the unit tests).
        override fun volumeFixed(): Boolean = silent || audio.isVolumeFixed

        override fun alarmVolume(): Int = audio.getStreamVolume(AudioManager.STREAM_ALARM)

        override fun alarmMax(): Int = audio.getStreamMaxVolume(AudioManager.STREAM_ALARM)

        override fun setAlarmVolume(index: Int) {
            // No FLAG_SHOW_UI: the user sees nothing change on screen.
            audio.setStreamVolume(AudioManager.STREAM_ALARM, index.coerceIn(0, alarmMax()), 0)
        }

        override fun startSound() {
            if (silent) return
            val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build()
            val p = MediaPlayer()
            try {
                p.setAudioAttributes(attributes)
                val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM) ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
                var prepared = false
                if (uri != null) {
                    prepared = try {
                        p.setDataSource(context, uri)
                        p.prepare()
                        true
                    } catch (_: Exception) {
                        p.reset()
                        p.setAudioAttributes(attributes)
                        false
                    }
                }
                if (!prepared) {
                    // The bundled tone (res/raw/kin_ring): a small WAV, so a phone with no alarm or ringtone still rings.
                    context.resources.openRawResourceFd(R.raw.kin_ring).use { p.setDataSource(it.fileDescriptor, it.startOffset, it.length) }
                    p.prepare()
                }
                p.isLooping = true
                p.setVolume(1f, 1f)
                p.start()
            } catch (e: Exception) {
                runCatching { p.release() }
                throw e
            }
            player = p
            vibrate()
        }

        private fun vibrate() {
            val v = vibrator ?: return
            if (!v.hasVibrator()) return
            // 800 ms on, 400 ms off, repeating, on the alarm usage (VIBRATE is a normal permission).
            val effect = VibrationEffect.createWaveform(longArrayOf(0, 800, 400), 1)
            runCatching {
                if (Build.VERSION.SDK_INT >= 33) {
                    v.vibrate(effect, android.os.VibrationAttributes.Builder().setUsage(android.os.VibrationAttributes.USAGE_ALARM).build())
                } else {
                    @Suppress("DEPRECATION")
                    v.vibrate(effect, AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).build())
                }
            }
        }

        override fun stopSound() {
            player?.let { p ->
                runCatching { p.stop() }
                runCatching { p.release() }
            }
            player = null
            runCatching { vibrator?.cancel() }
        }

        // "Total silence", or Priority mode whose policy does not allow alarms (the 2B.4 review: that one used to be reported as audible). Reading the policy needs no access.
        override fun totalSilence(): Boolean = try {
            AlarmAudibility.silenced(notifications.currentInterruptionFilter, notifications.notificationPolicy.priorityCategories)
        } catch (_: Exception) {
            notifications.currentInterruptionFilter == NotificationManager.INTERRUPTION_FILTER_NONE
        }

        override fun showStop(pcName: String, endsInMillis: Long): Boolean {
            mutableUi.value = RingUi(ringing = true, pcName = pcName, endsAtElapsed = clock.elapsedMillis() + endsInMillis, audible = !totalSilence())
            return RingNotification.show(context, pcName)
        }

        override fun hideStop() {
            mutableUi.value = RingUi()
            RingNotification.hide(context)
        }

        override fun holdAwake(on: Boolean) {
            if (!on) {
                wake?.let { if (it.isHeld) runCatching { it.release() } }
                wake = null
                return
            }
            val w = wake ?: context.getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "kin:ring").also { it.setReferenceCounted(false); wake = it }
            // A timeout, so a lost stop can't hold the phone awake for ever.
            w.acquire(WAKE_LOCK_MILLIS)
        }

        // The saved index is mirrored in memory and persisted **before** the volume is raised (a crash mid-ring must not leave the alarm at maximum).
        override fun savedIndex(): Int? {
            if (!savedLoaded) {
                saved = runBlocking { graph.presenceStore.savedAlarmIndex() }
                savedLoaded = true
            }
            return saved
        }

        override fun saveIndex(index: Int) {
            runBlocking { graph.presenceStore.saveAlarmIndex(index) }
            saved = index
            savedLoaded = true
        }

        override fun clearSavedIndex() {
            runBlocking { graph.presenceStore.clearSavedAlarmIndex() }
            saved = null
            savedLoaded = true
        }

        override fun announce(state: String, reason: String?, audible: Boolean?) {
            val c = graph.link.runtime?.currentSession ?: return
            if (!c.isReady) return
            c.sendEphemeral(PresenceWire.ringStatus(MessageIds.newId(), state, reason, audible))
        }
    }

    private companion object {
        const val WAKE_LOCK_MILLIS = 35_000L
    }
}
