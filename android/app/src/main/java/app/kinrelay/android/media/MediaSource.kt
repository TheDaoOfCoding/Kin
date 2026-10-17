package app.kinrelay.android.media

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSession
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.core.graphics.get
import app.kinrelay.android.BuildConfig
import app.kinrelay.android.link.Clock
import app.kinrelay.android.log.KinLog
import app.kinrelay.android.notifmirror.AndroidIcons
import app.kinrelay.android.notifmirror.MirrorListenerService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Phone -> PC: reads this phone's media sessions (design 4.2). **Permission: the notification access of 2B.2, nothing new**: `MediaSessionManager.getActiveSessions` and
 * `addOnActiveSessionsChangedListener` take Kin's `MirrorListenerService` component, which the system accepts for an enabled notification listener; without that access they throw a
 * `SecurityException` ([start] returns false and the hub says `access: missing`). Only the callbacks of the platform wake it: `OnActiveSessionsChangedListener` and one
 * `MediaController.Callback` per session (`onPlaybackStateChanged`, `onMetadataChanged`, `onSessionDestroyed`, `onAudioInfoChanged`). **No polling, no timer, no wake lock**; the only `delay` is
 * the one that drops a session paused for 30 minutes. Every callback only queues an event; one coroutine takes them in order and decides the active session ([MediaPick]).
 *
 * What it reads: package (only for `sid` and the label), title, artist, album, duration, the art bitmap, state, position, speed, update time, actions, volume. **Never** the `*_URI` keys, the
 * queue, the session's extras (except the debug marker), media ids or ratings. Kin's own sessions are skipped (the PC-media session would echo back), except in a debug build the test session
 * ([EXTRA_DEBUG]); a debug build reads only that one unless `MEDIA_READ_ALL` was sent (design 6.1: the emulator is not the user's phone, but the rule is the same on both).
 */
class MediaSource(
    private val context: Context,
    private val scope: CoroutineScope,
    private val clock: Clock,
    private val log: KinLog,
    private val icons: AndroidIcons,
    private val volume: MediaVolumeControl,
    private val readAll: () -> Boolean,
    private val onActive: (MediaSnapshot, List<MediaImageData>) -> Unit,
    private val onNone: () -> Unit,
) : SourceControl {
    private sealed interface Event {
        class Sessions(val controllers: List<MediaController>) : Event
        data object Changed : Event
        class Destroyed(val token: MediaSession.Token) : Event
    }

    private class Meta(val title: String?, val artist: String?, val album: String?, val durationMs: Long?, val artSig: String?)

    private class ArtEntry(val image: MediaImageData?)

    private inner class Entry(val controller: MediaController, val packageName: String, private val events: Channel<Event>) {
        val sid = MediaWire.sidOf(packageName)
        @Volatile var playback: PlaybackState? = null
        @Volatile var meta: Meta? = null
        var pausedSince = NOT_PAUSED
        val token: MediaSession.Token = controller.sessionToken
        val callback = object : MediaController.Callback() {
            override fun onPlaybackStateChanged(state: PlaybackState?) {
                playback = state
                events.trySend(Event.Changed)
            }

            override fun onMetadataChanged(metadata: MediaMetadata?) {
                meta = extract(metadata)
                events.trySend(Event.Changed)
            }

            override fun onSessionDestroyed() {
                events.trySend(Event.Destroyed(token))
            }

            override fun onAudioInfoChanged(info: MediaController.PlaybackInfo) {
                events.trySend(Event.Changed)
            }
        }
    }

    private val manager: MediaSessionManager = context.getSystemService(MediaSessionManager::class.java)
    private val component = ComponentName(context, MirrorListenerService::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private val encoder = ArtworkEncoder()
    private val packages: PackageManager = context.packageManager
    private val labels = HashMap<String, String?>()
    private val artCache = object : LinkedHashMap<String, ArtEntry>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ArtEntry>?): Boolean = size > ART_CACHE_SIZE
    }

    @Volatile private var events: Channel<Event>? = null
    private var listener: MediaSessionManager.OnActiveSessionsChangedListener? = null
    private var worker: Job? = null
    private val gate = GenerationGate()

    // Touched only under [gate] (the worker's passes, `start` and `stop`), except [activeEntry] which the command runner reads.
    private var expiry: Job? = null
    private var entries: List<Entry> = emptyList()
    private var currentSid: String? = null
    @Volatile private var activeEntry: Entry? = null

    override val isRunning: Boolean get() = worker != null

    /**
     * Starts listening. False when the notification access is missing (the platform throws `SecurityException`): nothing was registered. Idempotent.
     */
    @Synchronized
    override fun start(): Boolean {
        if (worker != null) return true
        val initial = try {
            manager.getActiveSessions(component)
        } catch (_: SecurityException) {
            return false
        }
        val channel = Channel<Event>(Channel.UNLIMITED)
        val changed = MediaSessionManager.OnActiveSessionsChangedListener { list -> channel.trySend(Event.Sessions(list.orEmpty())) }
        try {
            manager.addOnActiveSessionsChangedListener(changed, component, handler)
        } catch (_: SecurityException) {
            return false
        }
        listener = changed
        events = channel
        // A new run starts from nothing: whatever an earlier run left behind (it can't hold anything after `stop`, but a late pass of its worker is now stale too).
        val gen = gate.advance {
            entries = emptyList()
            activeEntry = null
            currentSid = null
        }
        channel.trySend(Event.Sessions(initial))
        worker = scope.launch(Dispatchers.Default) {
            for (first in channel) {
                try {
                    // Under the gate and only while this run is the current one: after `stop()` a pass that was already underway can't register a callback again.
                    val alive = gate.ifCurrent(gen) {
                        apply(first, channel)
                        // A burst of callbacks (a track change fires several) is one recompute.
                        while (true) apply(channel.tryReceive().getOrNull() ?: break, channel)
                        recompute(channel)
                        true
                    } ?: false
                    if (!alive) break
                } catch (e: Exception) {
                    // One bad session must not stop the rest. The log has the class, never a title.
                    log.w("media", "media: a session failed (${e.javaClass.simpleName})")
                }
            }
        }
        log.i("media", "media: reading sessions")
        return true
    }

    /** Stops listening and lets go of every controller (design 4.2: no reference outlives the listener). Idempotent. */
    @Synchronized
    override fun stop() {
        val running = worker ?: return
        worker = null
        running.cancel()
        listener?.let { runCatching { manager.removeOnActiveSessionsChangedListener(it) } }
        listener = null
        // One lock with the worker, and the run is over afterwards: a pass that was underway finishes before this, a later one sees a stale generation and does nothing.
        gate.advance {
            expiry?.cancel()
            expiry = null
            entries.forEach { runCatching { it.controller.unregisterCallback(it.callback) } }
            entries = emptyList()
            activeEntry = null
            currentSid = null
            artCache.clear()
        }
        events?.close()
        events = null
        log.i("media", "media: stopped reading sessions")
    }

    /** Takes the platform's list of sessions again (a setting that decides which are read changed). */
    fun refresh() {
        val channel = events ?: return
        val list = try {
            manager.getActiveSessions(component)
        } catch (_: SecurityException) {
            return
        }
        channel.trySend(Event.Sessions(list))
    }

    // ------------------------------------------------------------------ for the command runner

    /** The active session read **once**: its `sid`, actions, state and controls all belong to the same entry, even if the worker switches sessions meanwhile. */
    fun activeSession(): ActiveSession? {
        val e = activeEntry ?: return null
        val p = e.playback ?: return null
        val state = MediaMapping.playStateOf(p.state) ?: return null
        val controls = e.controller.transportControls
        val transport = object : MediaTransport {
            override fun play() = controls.play()
            override fun pause() = controls.pause()
            override fun next() = controls.skipToNext()
            override fun prev() = controls.skipToPrevious()
            override fun seekTo(positionMs: Long) = controls.seekTo(positionMs)
        }
        return ActiveSession(e.sid, MediaMapping.actionsOf(p.actions, e.meta?.durationMs != null).toSet(), state, transport)
    }

    fun activeController(): MediaController? = activeEntry?.controller

    // ------------------------------------------------------------------ the worker

    private fun accepts(controller: MediaController): Boolean {
        val own = controller.packageName == context.packageName
        val flagged = BuildConfig.DEBUG && runCatching { controller.extras?.getBoolean(EXTRA_DEBUG) == true }.getOrDefault(false)
        if (own) return flagged
        return !BuildConfig.DEBUG || readAll()
    }

    private fun apply(event: Event, channel: Channel<Event>) {
        when (event) {
            is Event.Changed -> Unit
            is Event.Destroyed -> {
                entries.firstOrNull { it.token == event.token }?.let { gone ->
                    runCatching { gone.controller.unregisterCallback(gone.callback) }
                    entries = entries - gone
                }
            }
            is Event.Sessions -> {
                val wanted = event.controllers.filter(::accepts)
                val keep = ArrayList<Entry>(wanted.size)
                for (controller in wanted) {
                    val existing = entries.firstOrNull { it.token == controller.sessionToken }
                    if (existing != null) {
                        keep += existing
                        continue
                    }
                    val entry = Entry(controller, controller.packageName, channel)
                    controller.registerCallback(entry.callback, handler)
                    entry.playback = controller.playbackState
                    entry.meta = extract(controller.metadata)
                    keep += entry
                }
                entries.filter { old -> keep.none { it === old } }.forEach { runCatching { it.controller.unregisterCallback(it.callback) } }
                entries = keep
            }
        }
    }

    private fun recompute(channel: Channel<Event>) {
        val now = clock.elapsedMillis()
        // The first of each package, in the platform's priority order (two sessions of one app count as one).
        val list = entries.distinctBy { it.sid }
        for (e in list) {
            val state = MediaMapping.playStateOf(e.playback?.state ?: MediaMapping.STATE_NONE)
            if (state == PlayState.PAUSED) {
                if (e.pausedSince == NOT_PAUSED) e.pausedSince = now
            } else {
                e.pausedSince = NOT_PAUSED
            }
        }
        val picks = list.map { e ->
            PickSession(e.sid, own = false, title = e.meta?.title != null, state = MediaMapping.playStateOf(e.playback?.state ?: MediaMapping.STATE_NONE), pausedMs = if (e.pausedSince == NOT_PAUSED) 0L else now - e.pausedSince)
        }
        armExpiry(picks, channel)
        val chosen = MediaPick.choose(picks, currentSid)
        val entry = list.firstOrNull { it.sid == chosen }
        val playback = entry?.playback
        val state = playback?.let { MediaMapping.playStateOf(it.state) }
        if (entry == null || playback == null || state == null) {
            currentSid = null
            activeEntry = null
            onNone()
            return
        }
        currentSid = entry.sid
        activeEntry = entry
        val meta = entry.meta
        val duration = meta?.durationMs
        val art = artFor(entry)
        val icon = icons.forPackage(entry.packageName, null, disk = false)?.let { MediaImageData(it.hash, MediaWire.KIND_ICON, it.png) }
        val snapshot = MediaSnapshot(
            sid = entry.sid,
            app = labelOf(entry),
            state = state,
            title = meta?.title,
            artist = meta?.artist,
            album = meta?.album,
            durationMs = duration,
            position = playback.position,
            positionUpdatedAt = playback.lastPositionUpdateTime,
            speed = playback.playbackSpeed,
            actions = MediaMapping.actionsOf(playback.actions, duration != null),
            artHash = art?.hash,
            iconHash = icon?.hash,
            volume = runCatching { volume.read() }.getOrNull(),
        )
        onActive(snapshot, listOfNotNull(art, icon))
    }

    /** One `delay` for the earliest session that will be dropped after 30 minutes paused (3.5): re-armed on every recompute, never a poll. */
    private fun armExpiry(picks: List<PickSession>, channel: Channel<Event>) {
        expiry?.cancel()
        expiry = null
        val next = picks.filter { it.state == PlayState.PAUSED && it.title && it.pausedMs < MediaLimits.PAUSED_EXPIRY_MILLIS }
            .minOfOrNull { MediaLimits.PAUSED_EXPIRY_MILLIS - it.pausedMs } ?: return
        expiry = scope.launch {
            delay(next + 1)
            channel.trySend(Event.Changed)
        }
    }

    private fun labelOf(entry: Entry): String {
        val fromPackage = labels.getOrPut(entry.packageName) {
            try {
                packages.getApplicationInfo(entry.packageName, 0).loadLabel(packages).toString()
            } catch (_: PackageManager.NameNotFoundException) {
                null
            } catch (_: SecurityException) {
                null
            }
        }
        if (!fromPackage.isNullOrBlank()) return fromPackage
        // `getTag` is API 30; an app Kin can't see (no launcher entry) falls back to the session's own tag, then to "Media" (design 4.2).
        val tag = if (Build.VERSION.SDK_INT >= 30) runCatching { entry.controller.tag }.getOrNull() else null
        return tag?.takeIf { it.isNotBlank() } ?: "Media"
    }

    // ------------------------------------------------------------------ art

    /** Re-encoded only when the art changed (same session, same title, same picture signature): the last few results stay in memory, never on disk. */
    private fun artFor(entry: Entry): MediaImageData? {
        val meta = entry.meta ?: return null
        val sig = meta.artSig ?: return null
        val key = "${entry.sid}|${meta.title}|$sig"
        artCache[key]?.let { return it.image }
        val bitmap = try {
            entry.controller.metadata?.let(::bitmapOf)
        } catch (_: Exception) {
            null
        }
        val image = bitmap?.let(encoder::encode)
        artCache[key] = ArtEntry(image)
        return image
    }

    private fun extract(md: MediaMetadata?): Meta? {
        md ?: return null
        val title = MediaWire.clean(md.getString(MediaMetadata.METADATA_KEY_TITLE) ?: md.getString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE), MediaWire.TITLE_MAX)
        val artist = MediaWire.clean(md.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: md.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST) ?: md.getString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE), MediaWire.ARTIST_MAX)
        val album = MediaWire.clean(md.getString(MediaMetadata.METADATA_KEY_ALBUM), MediaWire.ALBUM_MAX)
        val duration = md.getLong(MediaMetadata.METADATA_KEY_DURATION).takeIf { it in 1..MediaWire.MAX_DURATION_MS }
        // Never a `*_URI` key: a URI from another app is not fetched (no network, no ContentResolver read on its behalf).
        val art = bitmapOf(md)
        return Meta(title, artist, album, duration, art?.let(::signature))
    }

    private fun bitmapOf(md: MediaMetadata): Bitmap? =
        md.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART) ?: md.getBitmap(MediaMetadata.METADATA_KEY_ART) ?: md.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON)

    /** A cheap signature of a picture (its size and 64 sampled pixels): a metadata callback hands over a fresh copy of the bitmap each time, so its identity says nothing. */
    private fun signature(bitmap: Bitmap): String {
        val w = bitmap.width
        val h = bitmap.height
        var hash = 17L * 31 + w
        hash = hash * 31 + h
        runCatching {
            for (i in 0 until 8) for (j in 0 until 8) hash = hash * 31 + bitmap[w * (2 * i + 1) / 16, h * (2 * j + 1) / 16]
        }
        return "${w}x$h:$hash"
    }

    companion object {
        /** The debug session's marker (design 4.9): the one own session a debug build reads. */
        const val EXTRA_DEBUG = "kin.debug_media"
        private const val NOT_PAUSED = -1L
        private const val ART_CACHE_SIZE = 8
    }
}
