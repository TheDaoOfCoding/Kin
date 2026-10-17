package app.kinrelay.android.media

import android.Manifest
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.SystemClock
import androidx.core.content.ContextCompat
import app.kinrelay.android.AppGraph
import app.kinrelay.android.link.Clock
import app.kinrelay.android.link.LinkConnection
import app.kinrelay.android.link.LinkState
import app.kinrelay.android.protocol.Frames
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/** What the Settings card and the Home row show (design 4.7): the user's choices and what the PC and the system say. No content of any session. */
data class MediaUi(
    val policy: MediaPolicy = MediaPolicy(),
    /** Notification access is on for Kin (needed to read other apps' media; the same access as notification mirroring). */
    val accessGranted: Boolean = false,
    val connected: Boolean = false,
    /** The PC's latest `hello` lists `media` (a 0.2.0 build), and its latest `media.status`, if any came. */
    val peerListsMedia: Boolean = false,
    val peerStatus: MediaStatus? = null,
    /** This phone's media is being read for the PC right now. */
    val sharing: Boolean = false,
    /** The PC's media is shown on this phone right now. */
    val showing: Boolean = false,
    val notificationsAllowed: Boolean = true,
    val pcName: String? = null,
    /** The PC's volume as its source reports it (the slider on Home), and whether its media is playing: the words on Home say "playing", never the title. */
    val pcVolume: MediaVolume? = null,
    val pcPlaying: Boolean = false,
)

/**
 * Media controls as the rest of the app sees them (design 4.1): one per process, made by [AppGraph]. **It starts nothing by itself**: it reacts to a setting, a PC message, the link and
 * the platform's callbacks, so an idle phone spends nothing on it. It never starts `LinkService`; with no connection it drops.
 *
 * Phone -> PC: [MediaSource] reads the sessions while `share` is on, the notification access is granted, a connection is ready and the PC says `show: true`; [MediaSender] is the only writer.
 * PC -> phone: a `media.state` becomes one `MediaSession` and its `MediaStyle` notification ([PcMediaSession]); its buttons send `media.cmd`. Nothing is stored or logged but counts.
 */
class MediaHub(private val graph: AppGraph) {
    private val context: Context = graph.appContext
    private val clock = Clock { SystemClock.elapsedRealtime() }
    private val log = graph.log

    val store = MediaPolicyStore(context)
    val stats = MediaStats()
    val sender = MediaSender(graph.scope, clock, link = { graph.link.runtime?.currentSession }, log = log, stats = stats)

    @Volatile private var policy = MediaPolicy()

    private val volume = AndroidVolume(context) { source.activeController() }

    internal val source: MediaSource by lazy {
        MediaSource(
            context, graph.scope, clock, log, graph.mirror.icons, volume,
            readAll = { readAllSessions },
            onActive = { snapshot, images -> sender.update(snapshot, images) },
            onNone = { sender.clear() },
        )
    }

    private val runner: MediaCommandRunner by lazy {
        MediaCommandRunner(
            active = { source.activeSession() }, volume = volume,
            shareOn = { policy.share }, clock = clock, stats = stats, volumeChanged = { sender.volume(it) },
        )
    }

    private val sourceSwitch = SourceSwitch({ source }, sender)

    // PC -> phone. The viewer holds the model, the session and what the user swiped away, and is synchronized on itself; [viewerLock] covers the connection and the rate bucket.
    private val viewerLock = Any()
    private var viewerBucket = MediaLimits.viewerState(clock)
    private val viewerErrors = ErrorSpacer(MediaLimits.ERROR_SPACING_MILLIS, clock)
    private var viewerConnection: LinkConnection? = null
    private var endTimer: Job? = null

    // Made on first use: a phone that never shows the PC's media never creates the session or its notification channel.
    internal val pcSessionLazy: Lazy<PcMediaSession> = lazy {
        PcMediaSession(context, object : PcMediaControls {
            override fun transport(kind: MediaCmdKind) = sendCommand(MediaCmd(kind, sid = viewer.current?.sid))

            override fun seek(positionMs: Long) = sendCommand(MediaCmd(MediaCmdKind.SEEK, sid = viewer.current?.sid, seekMs = positionMs))

            override fun volumeTo(level: Int) = sendCommand(MediaCmd(MediaCmdKind.VOLUME, level = level))

            override fun volumeAdjust(delta: Int) = sendCommand(MediaCmd(MediaCmdKind.VOLUME, delta = delta))
        }, log)
    }

    internal val viewer: PcMediaViewer = PcMediaViewer(
        clock,
        display = { pcSessionLazy.value },
        displayIfCreated = { if (pcSessionLazy.isInitialized()) pcSessionLazy.value else null },
        scheduleEnd = ::scheduleEnd,
        deviceName = ::deviceName,
    )

    private val incoming = Channel<MediaMsg>(Channel.UNLIMITED)

    private val mutableUi = MutableStateFlow(MediaUi())
    val ui: StateFlow<MediaUi> = mutableUi

    /** A debug build's reader takes every session instead of only the test one (set by the debug-only `MEDIA_READ_ALL`); always false in a release build. */
    @Volatile internal var readAllSessions = false

    internal val currentPolicy: MediaPolicy get() = policy

    // ------------------------------------------------------------------ start and the user's choices

    /** Called once when the app starts: follows the user's choices and the link, and takes the PC's messages in order. */
    fun start() {
        // The PC-media notification outlives nothing: after a process death (a crash, the system killing the app) the one posted before can't be updated or removed by its session any
        // more, so it is cancelled here (the session and its buttons are gone with the process).
        runCatching { context.getSystemService(NotificationManager::class.java)?.cancel(PcMediaNotification.ID) }
        graph.scope.launch(Dispatchers.Default) {
            for (message in incoming) {
                try {
                    handle(message)
                } catch (e: Exception) {
                    log.w("media", "media: a message failed (${e.javaClass.simpleName})")
                }
            }
        }
        graph.scope.launch {
            var last: MediaPolicy? = null
            store.policy.distinctUntilChanged().collect { now ->
                val before = last
                last = now
                policy = now
                sender.setOwnShare(now.share)
                if (before != null) {
                    if (now.share != before.share) log.i("media", "media: share ${if (now.share) "on" else "off"}")
                    if (now.show != before.show) log.i("media", "media: show ${if (now.show) "on" else "off"}")
                    if (!now.show) viewer.forgetAll()
                    // A change of the lock-screen or volume-key choice redraws what is shown.
                    if (now.show && (now.showOnLockScreen != before.showOnLockScreen || now.useVolumeKeys != before.useVolumeKeys)) viewer.render(now)
                }
                reevaluate()
                sender.statusChanged(status())
                publish()
            }
        }
        graph.scope.launch {
            graph.link.state.collect { if (it !is LinkState.Connected) onLinkClosed() }
        }
    }

    fun status(): MediaStatus = MediaStatus(policy.show, policy.share, if (accessGranted()) "granted" else "missing")

    fun accessGranted(): Boolean = graph.mirror.accessGranted()

    /**
     * Decides whether this phone's sessions are read: only while `share` is on, the notification access is granted, a connection is ready and the PC lists `media` and says `show: true`
     * (design 4.2). The moment any of those goes away the listener and every controller are let go.
     */
    @Synchronized
    fun reevaluate() {
        val access = accessGranted()
        val want = policy.share && access && sender.isOpen()
        // A reader that stops (access revoked while sharing, the PC stopped showing) also clears what the PC shows of this phone: no stale card is left there.
        if (sourceSwitch.apply(want) == SourceSwitch.Result.START_FAILED) sender.statusChanged(status())
        publish()
    }

    /** The notification-listener binding changed (access granted or revoked): the source may start or must stop, and the status says whether access is there. */
    fun accessChanged() {
        reevaluate()
        sender.statusChanged(status())
    }

    /** The PC's `hello` was accepted (also a later one that changed its capabilities): `media.status` goes out, and the connection's per-connection state starts clean. */
    fun onLinkReady(connection: LinkConnection) {
        synchronized(viewerLock) {
            // A different connection object is a new PC session: the viewer drops the old PC's card and model (a fast reconnect can come before the old link's close is seen).
            if (viewer.onLinkReady(connection)) {
                viewerConnection = connection
                viewerBucket = MediaLimits.viewerState(clock)
            }
        }
        publish()
        graph.scope.launch(Dispatchers.Default) {
            sender.onConnected(status())
            reevaluate()
        }
    }

    /** The connection ended: nothing about the PC or its session is kept (design 3.1, 3.7). */
    private fun onLinkClosed() {
        if (source.isRunning) source.stop()
        sender.reset()
        synchronized(viewerLock) {
            viewerConnection = null
            viewer.onLinkClosed()
        }
        publish()
    }

    // ------------------------------------------------------------------ the PC's messages

    /** A valid `media.*` message (ephemeral). Queued: one coroutine takes them in order, so a state and the clear after it can't swap places. */
    fun onMedia(message: MediaMsg) {
        incoming.trySend(message)
    }

    /** A `media.*` message was dropped silently by the receiver (counted): [notListed] when a side's `hello` doesn't list `media`, else an image that failed its checks. */
    fun onDropped(notListed: Boolean) {
        stats.drop(if (notListed) MediaStats.NOT_LISTED else MediaStats.BAD_IMAGE)
    }

    private fun handle(message: MediaMsg) {
        when (message) {
            is MediaMsg.Status -> {
                sender.onPeerStatus(message.status)
                reevaluate()
            }
            is MediaMsg.Cmd -> onCommand(message)
            else -> onViewer(message)
        }
        stats.maybeLog(clock.elapsedMillis(), log)
        publish()
    }

    private fun onViewer(message: MediaMsg) {
        if (!MediaGate.viewerAccepts(typeOf(message), policy.show)) {
            // A race (the PC didn't see our `show: false` yet): dropped silently, counted, never an error.
            stats.drop(MediaStats.VIEWER_OFF)
            return
        }
        if (message is MediaMsg.State && !viewerBucket.tryTake()) {
            stats.drop(MediaStats.RATE)
            if (viewerErrors.mayAnswer()) sender.answer(Frames.error("rate_limited", "Too many media states.", null))
            return
        }
        when (message) {
            is MediaMsg.State -> {
                viewer.onState(message.state, policy)
                stats.statesShown.incrementAndGet()
            }
            is MediaMsg.Image -> viewer.onImage(message.image, policy)
            is MediaMsg.Clear -> viewer.onClear()
            is MediaMsg.Volume -> viewer.onVolume(message.volume)
            else -> Unit
        }
    }

    private fun typeOf(message: MediaMsg): String = when (message) {
        is MediaMsg.Status -> MediaWire.STATUS
        is MediaMsg.State -> MediaWire.STATE
        is MediaMsg.Image -> MediaWire.IMAGE
        is MediaMsg.Clear -> MediaWire.CLEAR
        is MediaMsg.Volume -> MediaWire.VOLUME
        is MediaMsg.Cmd -> MediaWire.CMD
    }

    /** A PC's `media.cmd` for this phone's media: the runner decides and the answer, if any, is an `error` with the command's `ref`; success has no reply. */
    private fun onCommand(message: MediaMsg.Cmd) {
        when (val outcome = runner.run(message.cmd)) {
            CmdOutcome.Done, CmdOutcome.Silent -> Unit
            is CmdOutcome.Error -> sender.answer(Frames.error(outcome.code, outcome.message, message.id))
        }
    }

    // ------------------------------------------------------------------ PC -> phone: the session and the notification

    private fun deviceName(): String = PcMediaModel.deviceName(viewerConnection?.peerName ?: graph.link.runtime?.currentSession?.peerName)

    /** One `delay` until a playing state with no new message passes the end of the track by 5 s and must be shown paused at the end (3.4); null cancels it. */
    private fun scheduleEnd(wait: Long?) {
        endTimer?.cancel()
        endTimer = null
        if (wait == null) return
        endTimer = graph.scope.launch {
            delay(wait)
            viewer.render(policy)
        }
    }

    /** A button of Kin's session or notification (and the headset): one `media.cmd`, no local effect: the PC's next `media.state` moves the display. */
    private fun sendCommand(cmd: MediaCmd) {
        if (cmd.kind != MediaCmdKind.VOLUME && cmd.sid == null) return
        sender.command(cmd)
        stats.maybeLog(clock.elapsedMillis(), log)
    }

    /** The buttons and the swipe of the notification ([PcMediaReceiver]). */
    fun onNotificationAction(action: String) {
        val current = viewer.current
        val actions = current?.actions?.toSet().orEmpty()
        val sid = current?.sid
        when (action) {
            PcMediaNotification.ACTION_PLAY -> MediaMapping.playOrToggle(actions)?.let { sendCommand(MediaCmd(it, sid = sid)) }
            PcMediaNotification.ACTION_PAUSE -> MediaMapping.pauseOrToggle(actions)?.let { sendCommand(MediaCmd(it, sid = sid)) }
            PcMediaNotification.ACTION_NEXT -> sendCommand(MediaCmd(MediaCmdKind.NEXT, sid = sid))
            PcMediaNotification.ACTION_PREV -> sendCommand(MediaCmd(MediaCmdKind.PREV, sid = sid))
            PcMediaNotification.ACTION_DISMISSED -> viewer.dismiss()
        }
    }

    // ------------------------------------------------------------------ the UI and the debug state

    private fun publish() {
        val peer = sender.peerStatus.value
        val connection = graph.link.runtime?.currentSession
        mutableUi.value = MediaUi(
            policy = policy,
            accessGranted = accessGranted(),
            connected = connection?.isReady == true,
            peerListsMedia = connection?.isReady == true && connection.peerUnderstandsMedia,
            peerStatus = peer,
            sharing = source.isRunning,
            showing = pcShown(),
            notificationsAllowed = android.os.Build.VERSION.SDK_INT < 33 ||
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED,
            pcName = connection?.peerName,
            // The PC's volume is the device's, not the session's: the slider is there whenever it is known, with or without a session.
            pcVolume = if (policy.show) viewer.volume else null,
            pcPlaying = policy.show && viewer.current?.state == PlayState.PLAYING,
        )
    }

    /** The volume slider of the Home card: one `media.cmd volume` with the level, coalesced (trailing 150 ms, last wins). */
    fun setPcVolume(level: Int) = sendCommand(MediaCmd(MediaCmdKind.VOLUME, level = level.coerceIn(0, 100)))

    private fun pcShown(): Boolean = viewer.current != null && policy.show

    /** The user came back from the system's notification access screen: look again. */
    fun refreshAccess() {
        accessChanged()
    }

    // The debug-only helpers (`MEDIA_PC_FAKE`, `MEDIA_STATE`, ...) are extension functions in the debug source set (`MediaHubDebug.kt`): none of them is in a release build.
}
