//! Media controls in both directions (milestone 2B.3, docs/phases/phase-2b-3-design.md, protocol.md "Media controls", changelog 57).
//!
//! * **PC → phone.** What plays on the PC (Windows' media sessions, read through GSMTC) is sent to the phone, and the phone's buttons and volume keys can control it.
//! * **Phone → PC.** What plays on the phone is shown in the Windows media flyout (a session of Kin's own, on a hidden window) and in the tray panel, and the
//!   flyout's buttons and the bar send `media.cmd` back to the phone.
//!
//! Both are **off by default**, each with its own switch, and **nothing about what plays is ever stored or logged** (only counts): now-playing is personal.
//!
//! | file | job |
//! |---|---|
//! | `wire.rs` | the `media.*` frames: validation, cleaning, the normalized message (pure) |
//! | `position.rs`, `pick.rs`, `limits.rs` | the position rule, which session is shown, the rates (pure) |
//! | `map.rs` | GSMTC status and controls ⇄ the wire, the wire → the flyout (pure) |
//! | `source.rs` | the engine that decides what the PC sends and when (pure, times as arguments) |
//! | `viewer.rs` | what a phone's session looks like in the flyout and in the panel (pure) |
//! | `guard.rs` | **only the installed Kin touches the user's real sessions and volume** (pure) |
//! | `reader.rs` | the GSMTC reader on a thread of its own, behind a trait and the guard |
//! | `art.rs` | a thumbnail as a picture the wire accepts (WIC) |
//! | `surface.rs` | the flyout session on a hidden top-level window (SMTC), behind a trait |
//! | `volume.rs` | the master volume behind a trait: Core Audio, or a fake in a test run |
//! | this file | the state in memory, the pump (one task, one wake-up at a time) and what the core does with a message, a press or a switch |
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::connections::ConnCommand;
use crate::core::{Core, Event, lock};
use crate::encoding::{unix_ms, uuid_v7};
use crate::messages::error_message;

pub mod art;
pub mod guard;
pub mod limits;
pub mod map;
pub mod pick;
pub mod policy;
pub mod position;
pub mod reader;
pub mod source;
pub mod surface;
pub mod viewer;
pub mod volume;
pub mod wire;

use art::Art;
use guard::Mode;
use limits::{Coalesce, Limiter};
use map::SmtcStatus;
use reader::{Control, Guarded, SessionBackend, WinBackend};
use source::{Engine, Out};
use surface::{Button, MediaSurface, SurfaceEvent, SurfaceView, WinSurface};
use viewer::{ImageStore, ViewSession};
use volume::{VolumeEndpoint, VolumeState};
use wire::{CmdKind, Inbound, MediaMsg, Status, VolumeOp};

/// After an event of a session the reader waits this long before it looks (a track change fires several events one after the other).
pub const SNAPSHOT_DEBOUNCE_MS: u64 = 300;
/// A thumbnail that is not there yet is looked for once more after this.
pub const THUMBNAIL_RETRY_MS: u64 = 1500;
/// `media.status` goes out at most once per this (a change of a switch right after another waits).
pub const STATUS_SPACING_MS: u64 = 5000;
/// While the flyout shows `Playing` its timeline is refreshed this often (the position runs on; whether the flyout moves its bar by itself is not known).
pub const TIMELINE_REFRESH_MS: u64 = 5000;
/// A seek or a volume dragged on the PC is one command per this.
pub const CMD_COALESCE_MS: u64 = 150;

/// Milliseconds on a monotonic clock (virtual under a paused test clock).
pub fn mono_ms() -> u64 {
    static START: OnceLock<tokio::time::Instant> = OnceLock::new();
    START.get_or_init(tokio::time::Instant::now).elapsed().as_millis() as u64
}

/// Numbers only, for `status.json` and the debug window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub states_sent: u64,
    pub images_sent: u64,
    pub clears_sent: u64,
    pub volumes_sent: u64,
    pub states_in: u64,
    pub images_in: u64,
    pub cmds_in: u64,
    pub cmds_refused: u64,
    pub cmds_out: u64,
    pub dropped_offline: u64,
    pub dropped_peer_off: u64,
    pub dropped_rate: u64,
    pub dropped_viewer_off: u64,
    pub dropped_by_receiver: u64,
    pub images_dropped: u64,
    pub surface_failures: u64,
}

struct PhoneMedia {
    /// The peer's latest `media.status` on this connection.
    status: Option<Status>,
    /// What the phone shows on this PC (the phone is the source).
    session: Option<ViewSession>,
    images: ImageStore,
    state_in: Limiter,
    cmd_out: Limiter,
    cmd_in: Limiter,
    state_out: Limiter,
    image_out: Limiter,
    sent_images: HashSet<String>,
    seek: Coalesce<u64>,
    vol: Coalesce<VolumeOp>,
}

impl PhoneMedia {
    fn new() -> PhoneMedia {
        PhoneMedia {
            status: None,
            session: None,
            images: ImageStore::default(),
            state_in: Limiter::new(limits::VIEWER_STATE),
            cmd_out: Limiter::new(limits::VIEWER_CMD),
            cmd_in: Limiter::new(limits::SOURCE_CMD),
            state_out: Limiter::new(limits::SOURCE_STATE),
            image_out: Limiter::new(limits::IMAGE),
            sent_images: HashSet::new(),
            seek: Coalesce::new(CMD_COALESCE_MS),
            vol: Coalesce::new(CMD_COALESCE_MS),
        }
    }
}

struct Inner {
    engine: Engine,
    phones: HashMap<String, PhoneMedia>,
    /// The phone whose session is in the flyout and the panel.
    viewing: Option<String>,
    /// `(aumid, title key)` of the picture read last, and the picture (or none).
    art_key: Option<(String, String)>,
    /// The track whose missing picture was already looked for once more.
    art_retry_key: Option<(String, String)>,
    art: Option<Art>,
    snapshot_due: Option<u64>,
    status_last_ms: Option<u64>,
    status_due: Option<u64>,
    timeline_due: Option<u64>,
    counters: Counters,
}

impl Inner {
    fn new() -> Inner {
        Inner {
            engine: Engine::new(),
            phones: HashMap::new(),
            viewing: None,
            art_key: None,
            art_retry_key: None,
            art: None,
            snapshot_due: None,
            status_last_ms: None,
            status_due: None,
            timeline_due: None,
            counters: Counters::default(),
        }
    }

    /// The earliest time the pump has to wake up.
    fn next_due(&self) -> Option<u64> {
        let mut due = vec![self.snapshot_due, self.engine.next_due(), self.status_due, self.timeline_due];
        for p in self.phones.values() {
            due.push(p.seek.next_due());
            due.push(p.vol.next_due());
        }
        due.into_iter().flatten().min()
    }
}

/// The parts that talk to Windows (or are fakes in a test). Each can be missing: the feature carries on without it.
#[derive(Default)]
struct Parts {
    guarded: Option<Arc<Guarded>>,
    backend: Option<Arc<WinBackend>>,
    surface: Option<Arc<dyn MediaSurface>>,
    win_surface: Option<Arc<WinSurface>>,
    volume: Option<Arc<dyn VolumeEndpoint>>,
    watch: Option<volume::Watch>,
    /// Why the reader can't run, and why the flyout can't be used (shown in the settings window).
    reader_unavailable: Option<String>,
    surface_unavailable: Option<String>,
}

pub struct Media {
    inner: Mutex<Inner>,
    parts: Mutex<Parts>,
    mode: Mutex<Option<Mode>>,
    wake: Notify,
    pump_running: AtomicBool,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
}

impl Default for Media {
    fn default() -> Self {
        Media::new()
    }
}

impl Media {
    pub fn new() -> Media {
        Media {
            inner: Mutex::new(Inner::new()),
            parts: Mutex::new(Parts::default()),
            mode: Mutex::new(None),
            wake: Notify::new(),
            pump_running: AtomicBool::new(false),
            runtime: Mutex::new(None),
        }
    }

    /// The runtime for what the Windows threads report (the app gives it at start; a caller already inside a runtime needn't).
    pub fn set_runtime(&self, handle: tokio::runtime::Handle) {
        *lock(&self.runtime) = Some(handle);
    }

    fn runtime(&self) -> Option<tokio::runtime::Handle> {
        lock(&self.runtime).clone().or_else(|| tokio::runtime::Handle::try_current().ok())
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn parts(&self) -> MutexGuard<'_, Parts> {
        self.parts.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn counters(&self) -> Counters {
        self.inner().counters.clone()
    }

    fn count(&self, f: impl FnOnce(&mut Counters)) {
        f(&mut self.inner().counters);
    }

    /// The numbers of `status.json`.
    pub fn counters_json(&self) -> Value {
        let c = self.counters();
        json!({
            "sent": { "states": c.states_sent, "images": c.images_sent, "clears": c.clears_sent, "volumes": c.volumes_sent },
            "received": { "states": c.states_in, "images": c.images_in },
            "cmds": { "in": c.cmds_in, "refused": c.cmds_refused, "out": c.cmds_out },
            "dropped": {
                "offline": c.dropped_offline,
                "peerOff": c.dropped_peer_off,
                "rate": c.dropped_rate,
                "viewerOff": c.dropped_viewer_off,
                "byReceiver": c.dropped_by_receiver,
                "images": c.images_dropped,
            },
            "surfaceFailures": c.surface_failures,
        })
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

fn state_word_of(session: &ViewSession, now_ms: u64) -> &'static str {
    viewer::state_word(session.shown(now_ms).state)
}

// ===== the core's side ===================================================================================================

impl Core {
    /// "Share this PC's media with the phone".
    pub fn media_share_on(&self) -> bool {
        lock(&self.settings).media_share
    }

    /// "Show the phone's media on this PC".
    pub fn media_show_on(&self) -> bool {
        lock(&self.settings).media_show
    }

    /// Which kind of run this is (design 6.1): the installed Kin, or a test run that touches no real session and no real volume.
    pub fn media_mode(&self) -> Mode {
        let mut mode = lock(&self.media.mode);
        mode.get_or_insert_with(|| Mode::for_config(&self.config, &std::env::current_exe().unwrap_or_default())).clone()
    }

    fn media_name_of(&self, device_id: &str) -> String {
        self.db.get_device(device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name)
    }

    fn media_status_frame(&self) -> Value {
        json!({ "type": "media.status", "id": uuid_v7(unix_ms()), "show": self.media_show_on(), "share": self.media_share_on() })
    }

    /// Sends a `media.*` frame. `false` if the phone isn't connected or doesn't list `media`.
    fn media_send(&self, device_id: &str, frame: Value) -> bool {
        if !self.connections.has_capability(device_id, wire::CAPABILITY) {
            return false;
        }
        self.connections.open_sender(device_id).is_some_and(|tx| tx.send(ConnCommand::Media { frame }).is_ok())
    }

    /// A `media.*` message that the receiver dropped silently (a side without the capability, or a bad picture).
    pub fn media_dropped_by_receiver(&self) {
        self.media.count(|c| c.dropped_by_receiver += 1);
    }

    // ----- connections --------------------------------------------------------------------------------------------------

    /// A phone's `hello` was accepted (`first`: the first one of this connection). The PC tells it its `media.status`; a new connection starts with nothing known.
    pub fn media_peer_hello(&self, device_id: &str, first: bool) {
        let lists = self.connections.has_capability(device_id, wire::CAPABILITY);
        if !lists {
            self.media_forget_phone(device_id);
            return;
        }
        {
            let mut inner = self.media.inner();
            if first || !inner.phones.contains_key(device_id) {
                inner.phones.insert(device_id.to_string(), PhoneMedia::new());
            }
        }
        self.media_send(device_id, self.media_status_frame());
    }

    /// The connection of a phone ended: what it showed goes.
    pub fn media_connection_closed(&self, device_id: &str) {
        self.media_forget_phone(device_id);
    }

    fn media_forget_phone(&self, device_id: &str) {
        let was_viewing = {
            let mut inner = self.media.inner();
            let had = inner.phones.remove(device_id);
            let viewing = inner.viewing.as_deref() == Some(device_id);
            if viewing {
                inner.viewing = None;
                inner.timeline_due = None;
            }
            viewing && had.is_some_and(|p| p.session.is_some())
        };
        if was_viewing {
            self.media_surface_clear();
        }
        self.emit(Event::StateChanged);
    }

    // ----- the settings -----------------------------------------------------------------------------------------------------

    fn save_media_settings(&self, what: &str) {
        let settings = lock(&self.settings).clone();
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json ({what}) failed: {e}");
        }
    }

    /// "Share this PC's media with the phone". Turning it on starts the reader of the PC's sessions; turning it off sends one `media.clear` and stops it.
    pub fn set_media_share(self: &Arc<Self>, on: bool) {
        let changed = {
            let mut s = lock(&self.settings);
            let changed = s.media_share != on;
            s.media_share = on;
            changed
        };
        if !changed {
            return;
        }
        log::info!("media: share this PC's media {}", if on { "on" } else { "off" });
        self.save_media_settings("mediaShare");
        if on {
            self.media_start_source();
        } else {
            self.media_stop_source();
        }
        self.media_send_status_to_all();
        self.emit(Event::StateChanged);
    }

    /// "Show the phone's media on this PC". Turning it off removes what is shown at once.
    pub fn set_media_show(self: &Arc<Self>, on: bool) {
        let changed = {
            let mut s = lock(&self.settings);
            let changed = s.media_show != on;
            s.media_show = on;
            changed
        };
        if !changed {
            return;
        }
        log::info!("media: show the phone's media {}", if on { "on" } else { "off" });
        self.save_media_settings("mediaShow");
        if on {
            self.media_start_surface();
        } else {
            self.media_stop_surface();
        }
        self.media_send_status_to_all();
        self.emit(Event::StateChanged);
    }

    /// Starts what the switches that are on need (at the start of the app).
    pub fn media_start(self: &Arc<Self>) {
        if self.media_share_on() {
            self.media_start_source();
        }
        if self.media_show_on() {
            self.media_start_surface();
        }
    }

    /// Stops everything (the app quits). The flyout session goes with the process anyway; this makes it go at once.
    pub fn media_stop(&self) {
        self.media.pump_running.store(false, Ordering::SeqCst);
        self.media.wake();
        self.media_stop_backend();
        self.media_stop_surface_parts();
    }

    /// `media.status` to every connected phone that lists `media`: at once if the last one went more than 5 s ago, else once the 5 s are over.
    fn media_send_status_to_all(&self) {
        let now = mono_ms();
        let send_now = {
            let mut inner = self.media.inner();
            match inner.status_last_ms {
                Some(at) if now.saturating_sub(at) < STATUS_SPACING_MS => {
                    inner.status_due = Some(at + STATUS_SPACING_MS);
                    false
                }
                _ => {
                    inner.status_last_ms = Some(now);
                    inner.status_due = None;
                    true
                }
            }
        };
        if send_now {
            let ids: Vec<String> = self.media.inner().phones.keys().cloned().collect();
            for id in ids {
                self.media_send(&id, self.media_status_frame());
            }
        }
        self.media.wake();
    }

    // ----- the source: the PC's media to the phone ---------------------------------------------------------------------

    fn media_start_source(self: &Arc<Self>) {
        if self.media.parts().guarded.is_some() {
            return;
        }
        let mode = self.media_mode();
        // Kin's own flyout session must never be read back: if Windows can't say the process's AUMID, it is the one the app gave itself at start.
        let own = crate::integration::process_app_user_model_id()
            .or_else(|| Some(crate::instance::app_user_model_id(self.config.instance_key().as_deref())));
        let weak: Weak<Core> = Arc::downgrade(self);
        // The reader tells the core that something changed; the core looks 300 ms later (a track change is several events).
        let on_change: reader::OnChange = Arc::new(move || {
            if let Some(core) = weak.upgrade() {
                core.media_changed();
            }
        });
        let allowed_mode = mode.clone();
        let allowed_own = own.clone();
        let allowed: Arc<dyn Fn(&str) -> bool + Send + Sync> =
            Arc::new(move |aumid| allowed_mode.may_use_session(aumid, allowed_own.as_deref()));
        let started = WinBackend::start(allowed, on_change);
        {
            let mut parts = self.media.parts();
            match started {
                Ok(backend) => {
                    let backend = Arc::new(backend);
                    parts.guarded = Some(Arc::new(Guarded::new(backend.clone(), mode.clone(), own)));
                    parts.backend = Some(backend);
                    parts.reader_unavailable = None;
                }
                Err(e) => {
                    log::warn!("media: the PC's sessions can't be read: {e}");
                    parts.reader_unavailable = Some(e);
                }
            }
            if parts.volume.is_none() {
                parts.volume = Some(volume::for_mode(&mode));
            }
        }
        self.media_start_volume_watch(&mode);
        log::info!("media: sharing the PC's media with the phone ({} run)", mode.name());
        self.media_ensure_pump();
        self.media_changed();
        self.media_read_volume();
    }

    /// The real volume is watched only where it is real: a test run has a fake that changes only when a command changes it.
    fn media_start_volume_watch(self: &Arc<Self>, mode: &Mode) {
        if !mode.real_volume() || self.media.parts().watch.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        let runtime = self.media.runtime();
        let on_change: volume::OnChange = Arc::new(move |state| {
            if let (Some(core), Some(rt)) = (weak.upgrade(), runtime.clone()) {
                rt.spawn(async move { core.media_volume_changed(state) });
            }
        });
        match volume::Watch::start(on_change) {
            Ok(w) => self.media.parts().watch = Some(w),
            Err(e) => log::warn!("media: changes of the PC's volume are not watched: {e}"),
        }
    }

    /// Tells the engine what the volume is now (at start, or when a change was reported).
    fn media_read_volume(self: &Arc<Self>) {
        let Some(endpoint) = self.media.parts().volume.clone() else { return };
        // From the start of the app there is no runtime on this thread: the app gave one (`Media::set_runtime`).
        let Some(runtime) = self.media.runtime() else { return };
        let core = self.clone();
        runtime.spawn_blocking(move || {
            if let Ok(state) = endpoint.get() {
                core.media_volume_changed(state);
            }
        });
    }

    fn media_volume_changed(&self, state: VolumeState) {
        let outs = self.media.inner().engine.on_volume(mono_ms(), state);
        self.media_fanout(outs);
        self.media.wake();
    }

    /// Share was turned off: one `media.clear` to the phones that show it, and then nothing.
    fn media_stop_source(&self) {
        let ids: Vec<String> = {
            let inner = self.media.inner();
            inner
                .phones
                .iter()
                .filter(|(id, p)| {
                    let peer = p.status.map(|s| policy::PeerStatus { show: s.show, share: s.share });
                    policy::may_send(policy::Kind::Clear, false, true, self.connections.has_capability(id, wire::CAPABILITY), peer)
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in ids {
            if self.media_send(&id, json!({ "type": "media.clear", "id": uuid_v7(unix_ms()) })) {
                self.media.count(|c| c.clears_sent += 1);
            }
        }
        self.media_stop_backend();
        let mut inner = self.media.inner();
        inner.engine = Engine::new();
        inner.art = None;
        inner.art_key = None;
        inner.snapshot_due = None;
        for p in inner.phones.values_mut() {
            p.sent_images.clear();
        }
    }

    fn media_stop_backend(&self) {
        let (backend, watch) = {
            let mut parts = self.media.parts();
            parts.guarded = None;
            (parts.backend.take(), parts.watch.take())
        };
        if let Some(b) = backend {
            b.stop();
        }
        drop(watch);
    }

    /// The reader's callback: something about a session changed. The core looks in 300 ms.
    pub fn media_changed(&self) {
        {
            let mut inner = self.media.inner();
            inner.snapshot_due.get_or_insert(mono_ms() + SNAPSHOT_DEBOUNCE_MS);
        }
        self.media.wake();
    }

    /// Reads the sessions and gives them to the engine (the pump calls it when the 300 ms are over).
    async fn media_snapshot(self: &Arc<Self>) {
        let Some(guarded) = self.media.parts().guarded.clone() else { return };
        let g = guarded.clone();
        let Ok(sessions) = tokio::task::spawn_blocking(move || g.snapshot()).await else { return };
        let now = mono_ms();
        let want = self.media.inner().engine.preview(now, &sessions);
        // The picture is read again only when the track changed; a thumbnail that isn't there yet is looked for once more after 1.5 s.
        let mut retry = false;
        let art_hash = match want {
            None => None,
            Some((aumid, key)) => {
                let wanted = (aumid.clone(), key);
                let known = self.media.inner().art_key.clone();
                if known.as_ref() != Some(&wanted) {
                    let g = guarded.clone();
                    let art = tokio::task::spawn_blocking(move || g.thumbnail(&aumid).and_then(|t| art::prepare(&t))).await.ok().flatten();
                    let mut inner = self.media.inner();
                    // No picture: look once more after 1.5 s (once per track: a player that has none is not asked again and again).
                    retry = art.is_none() && inner.art_retry_key.as_ref() != Some(&wanted);
                    if retry {
                        inner.art_retry_key = Some(wanted.clone());
                    }
                    inner.art_key = if retry { None } else { Some(wanted) };
                    inner.art = art;
                }
                self.media.inner().art.as_ref().map(|a| a.hash.clone())
            }
        };
        let outs = {
            let mut inner = self.media.inner();
            if retry {
                inner.snapshot_due = Some(now + THUMBNAIL_RETRY_MS);
            }
            inner.engine.on_sessions(now, sessions, art_hash)
        };
        self.media_fanout(outs);
        self.emit(Event::StateChanged);
    }

    /// Whether the PC may send `media.state`, `media.image`, `media.clear` or `media.volume` to this phone now (design 3.1: its own `share` is on, the phone lists
    /// `media` and the phone's latest status says `show: true`).
    fn media_may_send(&self, inner: &Inner, device_id: &str) -> bool {
        policy::may_send(
            policy::Kind::State,
            self.media_share_on(),
            false,
            self.connections.has_capability(device_id, wire::CAPABILITY),
            inner.phones.get(device_id).and_then(|p| p.status).map(|s| policy::PeerStatus { show: s.show, share: s.share }),
        )
    }

    /// Sends what the engine decided to every phone that may get it. A picture goes before the first state that uses it, once per connection.
    fn media_fanout(&self, outs: Vec<Out>) {
        if outs.is_empty() {
            return;
        }
        let now = mono_ms();
        let mut inner = self.media.inner();
        let devices: Vec<String> = inner.phones.keys().cloned().collect();
        for out in outs {
            for device in &devices {
                if !self.media_may_send(&inner, device) {
                    if self.media_share_on() {
                        if self.connections.has_capability(device, wire::CAPABILITY) {
                            inner.counters.dropped_peer_off += 1;
                        } else {
                            inner.counters.dropped_offline += 1;
                        }
                    }
                    continue;
                }
                match &out {
                    Out::Clear => {
                        if self.media_send(device, json!({ "type": "media.clear", "id": uuid_v7(unix_ms()) })) {
                            inner.counters.clears_sent += 1;
                        }
                    }
                    Out::Volume(v) => {
                        let frame = json!({ "type": "media.volume", "id": uuid_v7(unix_ms()), "level": v.level, "muted": v.muted });
                        if self.media_send(device, frame) {
                            inner.counters.volumes_sent += 1;
                        }
                    }
                    Out::State(state) => {
                        let art = inner.art.clone();
                        self.media_send_state_to(&mut inner, device, (**state).clone(), art.as_ref(), now);
                    }
                }
            }
        }
    }

    /// One state (and its picture, first) to one phone, under that phone's budgets.
    fn media_send_state_to(&self, inner: &mut Inner, device: &str, mut state: wire::State, art: Option<&Art>, now: u64) {
        let Some(phone) = inner.phones.get_mut(device) else { return };
        if let Some(hash) = state.art_hash.clone() {
            let sendable = art.filter(|a| a.hash == hash);
            match sendable {
                Some(a) if phone.sent_images.contains(&hash) => {
                    let _ = a;
                }
                Some(a) => {
                    if phone.image_out.check(now).allowed {
                        let frame = wire::image_frame(&uuid_v7(unix_ms()), wire::ImageKind::Art, &a.bytes);
                        if self.media_send(device, frame) {
                            phone.sent_images.insert(hash);
                            inner.counters.images_sent += 1;
                        } else {
                            state.art_hash = None;
                        }
                    } else {
                        // Over the picture budget: the state goes without a picture rather than naming one the phone doesn't have.
                        inner.counters.dropped_rate += 1;
                        state.art_hash = None;
                    }
                }
                None => state.art_hash = None,
            }
        }
        let Some(phone) = inner.phones.get_mut(device) else { return };
        if !phone.state_out.check(now).allowed {
            inner.counters.dropped_rate += 1;
            return;
        }
        if self.media_send(device, state.to_frame(&uuid_v7(unix_ms()))) {
            inner.counters.states_sent += 1;
        } else {
            inner.counters.dropped_offline += 1;
        }
    }

    /// The phone has just said it shows our media: it gets the current session afresh (the picture first, then the state, then the volume).
    fn media_send_current(&self, device_id: &str) {
        let now = mono_ms();
        let mut inner = self.media.inner();
        if !self.media_may_send(&inner, device_id) {
            return;
        }
        if let Some(state) = inner.engine.current_wire(now) {
            let art = inner.art.clone();
            self.media_send_state_to(&mut inner, device_id, state, art.as_ref(), now);
        }
        if let Some(v) = inner.engine.volume() {
            let frame = json!({ "type": "media.volume", "id": uuid_v7(unix_ms()), "level": v.level, "muted": v.muted });
            if self.media_send(device_id, frame) {
                inner.counters.volumes_sent += 1;
            }
        }
    }

    // ----- what arrives -----------------------------------------------------------------------------------------------------

    /// A `media.*` message that the receiver let through. Returns a frame to send back, if one is due (an `error`).
    pub async fn media_message(self: &Arc<Self>, device_id: &str, inbound: Inbound) -> Option<Value> {
        let Inbound { id, msg } = inbound;
        match msg {
            MediaMsg::Status(status) => {
                self.media_on_status(device_id, status);
                None
            }
            MediaMsg::Cmd(cmd) => self.media_on_cmd(device_id, &id, cmd).await,
            // The four that flow source → viewer: a viewer whose `show` is off drops them silently (design 3.1).
            other => {
                let kind = match &other {
                    MediaMsg::State(_) => policy::Kind::State,
                    MediaMsg::Image(_) => policy::Kind::Image,
                    MediaMsg::Clear => policy::Kind::Clear,
                    _ => policy::Kind::Volume,
                };
                if !policy::viewer_accepts(kind, self.media_show_on()) {
                    self.media.count(|c| c.dropped_viewer_off += 1);
                    return None;
                }
                match other {
                    MediaMsg::State(state) => self.media_viewer_state(device_id, *state).await,
                    MediaMsg::Image(image) => {
                        self.media_viewer_image(device_id, image).await;
                        None
                    }
                    MediaMsg::Clear => {
                        self.media_viewer_clear(device_id);
                        None
                    }
                    MediaMsg::Volume(v) => {
                        self.media_viewer_volume(device_id, v);
                        None
                    }
                    MediaMsg::Status(_) | MediaMsg::Cmd(_) => None,
                }
            }
        }
    }

    fn media_on_status(self: &Arc<Self>, device_id: &str, status: Status) {
        let (became_show, lost_show) = {
            let mut inner = self.media.inner();
            let phone = inner.phones.entry(device_id.to_string()).or_insert_with(PhoneMedia::new);
            let before = phone.status.map(|s| s.show).unwrap_or(false);
            phone.status = Some(status);
            if !status.show {
                phone.sent_images.clear();
            }
            (status.show && !before, !status.show && before)
        };
        log::info!(
            "media status of device {}: show {}, share {}, access {:?}",
            crate::core::short(device_id),
            status.show,
            status.share,
            status.access
        );
        if became_show {
            self.media_send_current(device_id);
        }
        if lost_show {
            // The source simply sends nothing more.
        }
        self.emit(Event::StateChanged);
    }

    async fn media_viewer_state(self: &Arc<Self>, device_id: &str, state: wire::State) -> Option<Value> {
        let now = mono_ms();
        {
            let mut inner = self.media.inner();
            let phone = inner.phones.entry(device_id.to_string()).or_insert_with(PhoneMedia::new);
            let verdict = phone.state_in.check(now);
            if !verdict.allowed {
                inner.counters.dropped_rate += 1;
                return verdict.send_error.then(|| error_message("rate_limited", "Too many media updates. Some were dropped.", None));
            }
            let playing = state.state == wire::PlayState::Playing;
            phone.session = Some(ViewSession { state, anchor_ms: now });
            inner.viewing = Some(device_id.to_string());
            inner.counters.states_in += 1;
            inner.timeline_due = playing.then_some(now + TIMELINE_REFRESH_MS);
        }
        self.media_refresh_surface().await;
        self.emit(Event::StateChanged);
        self.media.wake();
        None
    }

    async fn media_viewer_image(self: &Arc<Self>, device_id: &str, image: wire::Image) {
        let refresh = {
            let mut inner = self.media.inner();
            let phone = inner.phones.entry(device_id.to_string()).or_insert_with(PhoneMedia::new);
            phone.images.put(image.hash.clone(), image.kind, image.data);
            let uses_it = phone.session.as_ref().is_some_and(|s| s.state.art_hash.as_deref() == Some(image.hash.as_str()));
            inner.counters.images_in += 1;
            uses_it && inner.viewing.as_deref() == Some(device_id)
        };
        if refresh {
            self.media_refresh_surface().await;
        }
        self.emit(Event::StateChanged);
    }

    fn media_viewer_clear(self: &Arc<Self>, device_id: &str) {
        let was_viewing = {
            let mut inner = self.media.inner();
            if let Some(p) = inner.phones.get_mut(device_id) {
                p.session = None;
            }
            let viewing = inner.viewing.as_deref() == Some(device_id);
            if viewing {
                inner.viewing = None;
                inner.timeline_due = None;
            }
            viewing
        };
        if was_viewing {
            self.media_surface_clear();
        }
        self.emit(Event::StateChanged);
    }

    fn media_viewer_volume(self: &Arc<Self>, device_id: &str, v: wire::Volume) {
        {
            let mut inner = self.media.inner();
            if let Some(s) = inner.phones.get_mut(device_id).and_then(|p| p.session.as_mut()) {
                s.state.volume = Some(v);
            }
        }
        self.emit(Event::StateChanged);
    }

    // ----- the flyout --------------------------------------------------------------------------------------------------------

    fn media_surface(&self) -> Option<Arc<dyn MediaSurface>> {
        self.media.parts().surface.clone()
    }

    /// The view of the session in the flyout now (`None` when there is none, or the switch is off).
    fn media_surface_view(&self) -> Option<SurfaceView> {
        if !self.media_show_on() {
            return None;
        }
        let now = mono_ms();
        let inner = self.media.inner();
        let device = inner.viewing.as_ref()?;
        let phone = inner.phones.get(device)?;
        let session = phone.session.as_ref()?;
        let art = session.state.art_hash.as_deref().and_then(|h| phone.images.get(h));
        let name = self.media_name_of(device);
        Some(viewer::surface_view(session, now, &name, art, self.media_mode().never_playing()))
    }

    /// Puts the session of the phone in the flyout. The call waits for the flyout's thread (up to 10 s), so it is made on the blocking pool.
    async fn media_refresh_surface(self: &Arc<Self>) {
        let (Some(surface), Some(view)) = (self.media_surface(), self.media_surface_view()) else { return };
        let core = self.clone();
        let result = tokio::task::spawn_blocking(move || surface.show(view)).await;
        if !matches!(result, Ok(Ok(()))) {
            core.media.count(|c| c.surface_failures += 1);
            log::warn!("media: the Windows media flyout could not be updated");
        }
    }

    fn media_surface_clear(&self) {
        if let Some(surface) = self.media_surface() {
            surface.clear();
        }
    }

    fn media_start_surface(self: &Arc<Self>) {
        if self.media.parts().surface.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        let runtime = self.media.runtime();
        let events: surface::Events = Arc::new(move |event| {
            if let (Some(core), Some(rt)) = (weak.upgrade(), runtime.clone()) {
                rt.spawn(async move { core.media_surface_event(event).await });
            }
        });
        match WinSurface::start(events) {
            Ok(win) => {
                let win = Arc::new(win);
                let mut parts = self.media.parts();
                parts.win_surface = Some(win.clone());
                parts.surface = Some(win);
                parts.surface_unavailable = None;
            }
            Err(e) => {
                log::warn!("media: {e}");
                self.media.parts().surface_unavailable = Some("Windows can't show media here".to_string());
            }
        }
        self.media_ensure_pump();
        let core = self.clone();
        if let Some(rt) = self.media.runtime() {
            rt.spawn(async move { core.media_refresh_surface().await });
        }
    }

    /// Show was turned off: the flyout session goes, and every phone's session and pictures with it.
    fn media_stop_surface(&self) {
        self.media_stop_surface_parts();
        let mut inner = self.media.inner();
        inner.viewing = None;
        inner.timeline_due = None;
        for p in inner.phones.values_mut() {
            p.session = None;
            p.images.clear();
        }
    }

    fn media_stop_surface_parts(&self) {
        let (surface, win) = {
            let mut parts = self.media.parts();
            (parts.surface.take(), parts.win_surface.take())
        };
        if let Some(s) = surface {
            s.clear();
        }
        if let Some(w) = win {
            w.stop();
        }
    }

    /// A press in the flyout, a drag of its bar.
    pub async fn media_surface_event(self: &Arc<Self>, event: SurfaceEvent) {
        let kind = match event {
            SurfaceEvent::Button(Button::Play) => CmdKind::Play,
            SurfaceEvent::Button(Button::Pause) => CmdKind::Pause,
            SurfaceEvent::Button(Button::Next) => CmdKind::Next,
            SurfaceEvent::Button(Button::Prev) => CmdKind::Prev,
            SurfaceEvent::Seek(ms) => CmdKind::Seek(ms),
            // The flyout's thread handles a lock itself (it disables and clears the session).
            SurfaceEvent::Locked(_) => return,
        };
        self.media_send_cmd(kind);
    }

    // ----- commands to the phone (the PC is the viewer) -----------------------------------------------------------------------

    /// What a press in the flyout, the tray, the panel or a developer command does: a `media.cmd` to the phone whose media is shown. Never when there is none, the
    /// phone doesn't share, or the PC is over its budget. A seek and a volume that are dragged are one command per 150 ms.
    pub fn media_send_cmd(self: &Arc<Self>, kind: CmdKind) -> bool {
        let now = mono_ms();
        let (device, sid, sends) = {
            let mut inner = self.media.inner();
            let Some(device) = inner.viewing.clone() else { return false };
            let Some(phone) = inner.phones.get_mut(&device) else { return false };
            let Some(session) = phone.session.as_ref() else { return false };
            let peer = phone.status.map(|s| policy::PeerStatus { show: s.show, share: s.share });
            if !policy::may_send(policy::Kind::Cmd, false, false, self.connections.has_capability(&device, wire::CAPABILITY), peer) {
                return false;
            }
            let sid = session.state.sid.clone();
            let sends: Vec<CmdKind> = match kind {
                CmdKind::Seek(ms) => {
                    let ms = wire::clamp_seek(ms, session.state.duration_ms);
                    phone.seek.push(now, ms).into_iter().map(CmdKind::Seek).collect()
                }
                CmdKind::Volume(op @ VolumeOp::Level(_)) => phone.vol.push(now, op).into_iter().map(CmdKind::Volume).collect(),
                other => vec![other],
            };
            (device, sid, sends)
        };
        let mut sent_any = false;
        for kind in sends {
            sent_any |= self.media_emit_cmd(&device, &sid, kind);
        }
        self.media.wake();
        sent_any
    }

    /// One command, under the viewer's budget of 20 per 10 s.
    fn media_emit_cmd(&self, device: &str, sid: &str, kind: CmdKind) -> bool {
        let now = mono_ms();
        {
            let mut inner = self.media.inner();
            let Some(phone) = inner.phones.get_mut(device) else { return false };
            if !phone.cmd_out.check(now).allowed {
                inner.counters.dropped_rate += 1;
                return false;
            }
        }
        let cmd = wire::Cmd { sid: (!matches!(kind, CmdKind::Volume(_))).then(|| sid.to_string()), kind };
        let ok = self.media_send(device, cmd.to_frame(&uuid_v7(unix_ms())));
        if ok {
            self.media.count(|c| c.cmds_out += 1);
        }
        ok
    }

    /// A press in the tray menu or the panel: `toggle` (play or pause, whichever the phone's player offers and the state calls for), `next` or `prev`.
    pub fn media_press(self: &Arc<Self>, name: &str) -> bool {
        let kind = match name {
            "next" => CmdKind::Next,
            "prev" => CmdKind::Prev,
            "play" => CmdKind::Play,
            "pause" => CmdKind::Pause,
            "toggle" => {
                let now = mono_ms();
                let inner = self.media.inner();
                let Some(session) = inner.viewing.as_ref().and_then(|d| inner.phones.get(d)).and_then(|p| p.session.as_ref()) else {
                    return false;
                };
                if session.can(wire::Action::Toggle) {
                    CmdKind::Toggle
                } else if session.shown(now).state == wire::PlayState::Playing {
                    CmdKind::Pause
                } else {
                    CmdKind::Play
                }
            }
            _ => return false,
        };
        self.media_send_cmd(kind)
    }

    /// The developer hook `--dev-media <action> [value]`: what a button of the flyout does, without Windows.
    pub fn media_dev(self: &Arc<Self>, action: &str, arg: Option<String>) {
        if !self.config.dev_hooks_enabled() {
            log::warn!("refused --dev-media: it works only when the app was started with a developer setting");
            return;
        }
        let number = || arg.as_deref().and_then(|a| a.parse::<i64>().ok());
        let kind = match action {
            "play" => Some(CmdKind::Play),
            "pause" => Some(CmdKind::Pause),
            "toggle" => Some(CmdKind::Toggle),
            "next" => Some(CmdKind::Next),
            "prev" => Some(CmdKind::Prev),
            "seek" => number().filter(|n| *n >= 0).map(|n| CmdKind::Seek(n as u64)),
            "volume" => number().filter(|n| (0..=100).contains(n)).map(|n| CmdKind::Volume(VolumeOp::Level(n as u8))),
            "volume-delta" => number().filter(|n| *n != 0 && (-100..=100).contains(n)).map(|n| CmdKind::Volume(VolumeOp::Delta(n))),
            "mute" => Some(CmdKind::Volume(VolumeOp::Mute(true))),
            "unmute" => Some(CmdKind::Volume(VolumeOp::Mute(false))),
            _ => None,
        };
        match kind {
            Some(k) => {
                let sent = self.media_send_cmd(k);
                log::info!("developer media command {action}: {}", if sent { "sent" } else { "not sent" });
            }
            None => log::warn!(
                "--dev-media: no command {action:?} with that value (play, pause, toggle, next, prev, seek <ms>, volume <0-100>, volume-delta <n>, mute, unmute)"
            ),
        }
    }

    // ----- commands from the phone (the PC is the source) -------------------------------------------------------------------

    /// A `media.cmd` from a phone. In this order (protocol.md): `share` off is `disabled`; over the budget `rate_limited`; for every command but the volume no
    /// active session is `not_found`, another `sid` or an action the session doesn't list `stale`; a refusal is `action_failed`. **Success has no reply.**
    async fn media_on_cmd(self: &Arc<Self>, device_id: &str, id: &str, cmd: wire::Cmd) -> Option<Value> {
        self.media.count(|c| c.cmds_in += 1);
        let refuse = |this: &Core, code: &str, message: &str| {
            this.media.count(|c| c.cmds_refused += 1);
            Some(error_message(code, message, Some(id)))
        };
        // The answer comes from the one function that knows the order (`policy::cmd_result`): disabled, rate_limited, not_found, stale, action_failed. A source whose
        // `share` is off spends none of its budget.
        let share = self.media_share_on();
        let now = mono_ms();
        let (budget, active) = {
            let mut inner = self.media.inner();
            let budget = share.then(|| inner.phones.entry(device_id.to_string()).or_insert_with(PhoneMedia::new).cmd_in.check(now));
            let active = inner.engine.active().map(|a| (wire::sid_of(&a.aumid), a.aumid.clone(), a.state.actions.clone()));
            (budget, active)
        };
        let verdict = policy::cmd_result(
            share,
            budget.is_none_or(|b| b.allowed),
            active.as_ref().map(|(sid, _, actions)| (sid.as_str(), actions.as_slice())),
            &cmd,
            true,
        );
        match verdict {
            policy::Verdict::Ok => {}
            policy::Verdict::RateLimited => {
                self.media.count(|c| c.dropped_rate += 1);
                return budget
                    .is_some_and(|b| b.send_error)
                    .then(|| error_message("rate_limited", "Too many commands. Wait a moment.", Some(id)));
            }
            other => {
                let code = other.code().unwrap_or("action_failed");
                let message = match other {
                    policy::Verdict::Disabled => "This PC is not sharing its media.",
                    policy::Verdict::NotFound => "Nothing is playing on this PC.",
                    _ => "That is not what is playing now.",
                };
                return refuse(self, code, message);
            }
        }
        if let CmdKind::Volume(op) = cmd.kind {
            let Some(endpoint) = self.media.parts().volume.clone() else {
                return refuse(self, "action_failed", "The volume is not available here.");
            };
            return match tokio::task::spawn_blocking(move || volume::apply(endpoint.as_ref(), op)).await {
                Ok(Ok(state)) => {
                    self.media_volume_changed(state);
                    None
                }
                _ => refuse(self, "action_failed", "The volume could not be changed."),
            };
        }
        // The policy said ok for a command that is not the volume, so there is an active session.
        let Some((_, aumid, _)) = active else {
            return refuse(self, "not_found", "Nothing is playing on this PC.");
        };
        let control = match cmd.kind {
            CmdKind::Play => Control::Play,
            CmdKind::Pause => Control::Pause,
            CmdKind::Toggle => Control::Toggle,
            CmdKind::Next => Control::Next,
            CmdKind::Prev => Control::Prev,
            CmdKind::Seek(ms) => Control::SeekTo(ms),
            CmdKind::Volume(_) => unreachable!("handled above"),
        };
        let Some(guarded) = self.media.parts().guarded.clone() else {
            return refuse(self, "not_found", "Nothing is playing on this PC.");
        };
        match tokio::task::spawn_blocking(move || guarded.control(&aumid, control)).await {
            Ok(Ok(true)) => {
                // The effect comes back as the next state; the reader's events say so, and this makes sure.
                self.media_changed();
                None
            }
            _ => refuse(self, "action_failed", "The player did not do that."),
        }
    }

    // ----- the pump ----------------------------------------------------------------------------------------------------------

    /// Starts the one task that wakes up at the next due time (the debounce of a read, the throttles, a trailing status, a held command, the timeline refresh).
    fn media_ensure_pump(self: &Arc<Self>) {
        if self.media.pump_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(runtime) = self.media.runtime() else {
            self.media.pump_running.store(false, Ordering::SeqCst);
            return;
        };
        let core = self.clone();
        runtime.spawn(async move {
            while core.media.pump_running.load(Ordering::SeqCst) {
                let due = core.media.inner().next_due();
                match due {
                    Some(at) => {
                        let wait = at.saturating_sub(mono_ms());
                        tokio::select! {
                            () = core.media.wake.notified() => {}
                            () = tokio::time::sleep(std::time::Duration::from_millis(wait)) => {}
                        }
                    }
                    None => core.media.wake.notified().await,
                }
                if core.media.pump_running.load(Ordering::SeqCst) {
                    core.media_pump_once().await;
                }
            }
        });
    }

    /// Does what is due.
    async fn media_pump_once(self: &Arc<Self>) {
        let now = mono_ms();
        let snapshot_due = {
            let mut inner = self.media.inner();
            if inner.snapshot_due.is_some_and(|d| d <= now) {
                inner.snapshot_due = None;
                true
            } else {
                false
            }
        };
        if snapshot_due {
            self.media_snapshot().await;
        }
        let now = mono_ms();
        let outs = self.media.inner().engine.tick(now);
        if !outs.is_empty() {
            self.media_fanout(outs);
            self.emit(Event::StateChanged);
        }
        // A trailing `media.status`.
        let status_due = {
            let mut inner = self.media.inner();
            if inner.status_due.is_some_and(|d| d <= now) {
                inner.status_due = None;
                inner.status_last_ms = Some(now);
                true
            } else {
                false
            }
        };
        if status_due {
            let ids: Vec<String> = self.media.inner().phones.keys().cloned().collect();
            for id in ids {
                self.media_send(&id, self.media_status_frame());
            }
        }
        self.media_flush_cmds(now);
        self.media_refresh_timeline(now).await;
    }

    /// A seek or a volume that was held goes now.
    fn media_flush_cmds(self: &Arc<Self>, now: u64) {
        let held: Vec<(String, String, CmdKind)> = {
            let mut inner = self.media.inner();
            let mut out = Vec::new();
            for (device, phone) in inner.phones.iter_mut() {
                let Some(sid) = phone.session.as_ref().map(|s| s.state.sid.clone()) else { continue };
                if let Some(ms) = phone.seek.flush(now) {
                    out.push((device.clone(), sid.clone(), CmdKind::Seek(ms)));
                }
                if let Some(op) = phone.vol.flush(now) {
                    out.push((device.clone(), sid, CmdKind::Volume(op)));
                }
            }
            out
        };
        for (device, sid, kind) in held {
            self.media_emit_cmd(&device, &sid, kind);
        }
    }

    /// While the flyout shows `Playing`, its timeline is refreshed every 5 s with the extrapolated position (and a track that ran past its end is shown paused).
    async fn media_refresh_timeline(self: &Arc<Self>, now: u64) {
        let due = {
            let mut inner = self.media.inner();
            if inner.timeline_due.is_some_and(|d| d <= now) {
                inner.timeline_due = None;
                true
            } else {
                false
            }
        };
        if !due {
            return;
        }
        let Some(surface) = self.media_surface() else { return };
        let Some(view) = self.media_surface_view() else { return };
        let playing = view.buttons.status == SmtcStatus::Playing;
        let (status, timeline) = (view.buttons.status, view.timeline);
        let _ = tokio::task::spawn_blocking(move || surface.update_timeline(status, timeline)).await;
        if playing {
            self.media.inner().timeline_due = Some(now + TIMELINE_REFRESH_MS);
        }
    }

    // ----- what the windows show -----------------------------------------------------------------------------------------------

    /// What the tray says about the phone's media: `Some("playing")` or `Some("paused")` while a phone's session is shown, else `None`. (Never the title.)
    pub fn media_tray_state(&self) -> Option<&'static str> {
        let now = mono_ms();
        let inner = self.media.inner();
        let session = inner.phones.get(inner.viewing.as_ref()?)?.session.as_ref()?;
        Some(state_word_of(session, now))
    }

    fn media_phone_lines(&self) -> Vec<Value> {
        let open = self.connections.open_device_ids();
        let share = self.media_share_on();
        let show = self.media_show_on();
        let inner = self.media.inner();
        self.db
            .list_devices()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let connected = open.contains(&d.device_id);
                let lists = self.connections.has_capability(&d.device_id, wire::CAPABILITY);
                let status = inner.phones.get(&d.device_id).and_then(|p| p.status);
                let text = if !connected {
                    "offline".to_string()
                } else if !lists {
                    "Update Kin on your phone".to_string()
                } else if let Some(s) = status {
                    let mut parts: Vec<String> = Vec::new();
                    if share && !s.show {
                        parts.push(format!("{} isn't showing your media: turn it on in Kin on the phone", d.name));
                    }
                    if show && !s.share {
                        parts.push(format!("{} isn't sharing: turn it on in Kin on the phone", d.name));
                    }
                    if show && s.share && s.access == Some(crate::mirror::wire::Access::Missing) {
                        parts.push("notification access is off on the phone".to_string());
                    }
                    if parts.is_empty() { "connected".to_string() } else { parts.join("; ") }
                } else {
                    "waiting for the phone".to_string()
                };
                json!({ "name": d.name, "status": text })
            })
            .collect()
    }

    /// The "Media" part of the settings window and of `status.json`: the switches, which kind of run this is, and a line per phone. Numbers and words only.
    pub fn media_view(&self) -> Value {
        let now = mono_ms();
        let mode = self.media_mode();
        let parts = self.media.parts();
        let reader = parts.guarded.is_some();
        let flyout = parts.surface.is_some();
        let (reader_unavailable, surface_unavailable) = (parts.reader_unavailable.clone(), parts.surface_unavailable.clone());
        drop(parts);
        let shown = {
            let inner = self.media.inner();
            inner.viewing.as_ref().and_then(|d| inner.phones.get(d)).and_then(|p| p.session.as_ref()).map(|s| state_word_of(s, now))
        };
        let shared = self.media.inner().engine.active().map(|a| viewer::state_word(a.state.state));
        json!({
            "share": self.media_share_on(),
            "show": self.media_show_on(),
            "mode": mode.name(),
            "reading": reader,
            "flyout": flyout,
            "readerUnavailable": reader_unavailable,
            "flyoutUnavailable": surface_unavailable,
            "phoneMedia": shown.unwrap_or("none"),
            "pcMedia": shared.unwrap_or("none"),
            "phones": self.media_phone_lines(),
        })
    }

    /// What the drop window's media strip shows: the phone's session (with the title, in a window the user opened), the PC's own being shared (read-only) and the PC's
    /// master volume.
    pub fn media_panel(&self) -> Value {
        let now = mono_ms();
        let (phone, pc) = {
            let inner = self.media.inner();
            let phone = inner.viewing.as_ref().and_then(|d| inner.phones.get(d).map(|p| (d.clone(), p))).and_then(|(device, p)| {
                let s = p.session.as_ref()?;
                let art = s.state.art_hash.as_deref().and_then(|h| p.images.get(h)).map(|b| viewer::data_url(&b));
                Some(viewer::panel_json(s, now, &self.media_name_of(&device), art, None))
            });
            let pc = self.media_share_on().then(|| inner.engine.current_wire(now)).flatten().map(|s| {
                json!({ "title": s.title.clone().unwrap_or_else(|| s.app.clone()), "artist": s.artist, "app": s.app, "state": viewer::state_word(s.state) })
            });
            (phone, pc)
        };
        // The engine's cached volume (kept by the change callback, `media_read_volume` and the commands): no Core Audio call on the thread that serves the window.
        let volume = self.media.inner().engine.volume().map(|v| json!({ "level": v.level, "muted": v.muted }));
        json!({ "phone": phone, "pc": pc, "volume": volume, "testMode": self.media_mode().is_test() })
    }

    /// The drop window's PC volume slider: the user's own action in the app. In a test run it moves the fake.
    pub fn media_set_pc_volume(self: &Arc<Self>, op: VolumeOp) -> Result<(), String> {
        let endpoint = self.media.parts().volume.clone().ok_or_else(|| "The volume is not available.".to_string())?;
        let state = volume::apply(endpoint.as_ref(), op)?;
        self.media_volume_changed(state);
        self.emit(Event::StateChanged);
        Ok(())
    }

    // ----- for tests -------------------------------------------------------------------------------------------------------

    /// For tests: the switches on, and fakes in place of Windows (no GSMTC, no flyout, no Core Audio).
    #[doc(hidden)]
    pub fn media_for_tests(
        self: &Arc<Self>,
        backend: Arc<dyn SessionBackend>,
        mode: Mode,
        surface: Option<Arc<dyn MediaSurface>>,
        volume: Arc<dyn VolumeEndpoint>,
    ) {
        *lock(&self.media.mode) = Some(mode.clone());
        {
            let mut s = lock(&self.settings);
            s.media_share = true;
            s.media_show = true;
        }
        {
            let mut parts = self.media.parts();
            parts.guarded = Some(Arc::new(Guarded::new(backend, mode, Some("app.kinrelay.desktop.dev-test".to_string()))));
            parts.surface = surface;
            parts.volume = Some(volume);
        }
        self.media_ensure_pump();
    }

    /// For tests: the runtime the app gives at start (`Media::set_runtime`), so a test can start the media from a thread that has none, as the app does.
    #[doc(hidden)]
    pub fn media_runtime_for_tests(&self, handle: tokio::runtime::Handle) {
        self.media.set_runtime(handle);
    }

    /// For tests: the numbers that `status.json` shows.
    #[doc(hidden)]
    pub fn media_counters(&self) -> Counters {
        self.media.counters()
    }

    /// For tests: what the flyout would show now.
    #[doc(hidden)]
    pub fn media_current_view(&self) -> Option<SurfaceView> {
        self.media_surface_view()
    }
}
