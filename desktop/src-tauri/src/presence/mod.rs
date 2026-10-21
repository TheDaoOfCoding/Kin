//! Presence, Pause and Ring on the PC (milestone 2B.4, protocol.md "Presence, pause and ring", design section 6): what this PC knows about the phone's battery and pause,
//! what it tells the phone, this PC's own pause (a master gate next to every automatic feature) and the ring.
//!
//! * The pure parts are `wire` (the frames), `pause` (choices, the gate table, the peer's pause), `nudge` (the low-battery machine), `limits` (when a
//!   `presence` goes out and how many are taken in) and `battery` (the maps and the power source).
//! * This file is the core's side: one `ConnPresence` per open connection (the sender, the receiver's bucket and the peer's last snapshot, **in memory only**),
//!   the last known battery of each phone for the "last seen" line (in memory until Kin quits), one pump task that wakes at the next due time (a trailing send, the
//!   end of a pause, the 60 s battery poll while it matters, the ring's watchdog), and the commands (`set_pause`, `ring_start`, ...).
//! * Nothing here stores a battery level or a pause of the phone, and no log line has one (counts and enums only).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

pub mod battery;
pub mod limits;
pub mod nudge;
pub mod pause;
pub mod wire;

use crate::connections::{ConnCommand, SendOutcome};
use crate::core::{Core, Event, lock, short};
use crate::encoding::{unix_ms, uuid_v7};
use crate::features;
use crate::messages::error_message;
use crate::ring::{RingClient, RingState};
use battery::{FakePower, PowerSource, WinPower};
use limits::{PresenceBucket, PresenceSender, Send, Taken};
use nudge::LowBatteryNudge;
use pause::{PauseChoice, PauseState, PeerPauseView, WindowsZone, Zone};
use wire::{Battery, Dnd, PresenceMsg, RingStatus};

/// Milliseconds on a monotonic clock (virtual under a paused test clock).
pub fn mono_ms() -> i64 {
    static START: OnceLock<tokio::time::Instant> = OnceLock::new();
    START.get_or_init(tokio::time::Instant::now).elapsed().as_millis() as i64
}

fn now_ms() -> i64 {
    unix_ms() as i64
}

/// What the PC knows of one connected phone's last `presence`.
#[derive(Clone, Copy, Debug, Default)]
struct PeerView {
    battery: Option<Battery>,
    pause: Option<PeerPauseView>,
    /// 2B.5: the phone's Do Not Disturb filter while it is on. Memory only, forgotten when the connection closes, never in a file but the current word in `status.json`.
    dnd: Option<Dnd>,
}

struct ConnPresence {
    sender: PresenceSender,
    bucket: PresenceBucket,
    /// The same bucket (10, one back per 3 s) for the phone's `ring.status` (2B.4 review: it had none): over it the status is dropped and counted.
    ring_bucket: PresenceBucket,
    view: PeerView,
}

/// The last battery a phone reported, kept in memory until Kin quits for "Pixel 8 offline, last seen 14:32 · 12%".
#[derive(Clone, Copy, Debug)]
struct Known {
    battery: Battery,
    at_ms: i64,
}

/// Numbers only, for `status.json` and the debug window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub sent: u64,
    pub received: u64,
    pub dropped_rate: u64,
    pub dropped_by_receiver: u64,
    pub dropped_not_listed: u64,
    pub nudges: u64,
    pub quiet_arrivals: u64,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<String, ConnPresence>,
    known: HashMap<String, Known>,
    nudges: HashMap<String, LowBatteryNudge>,
    rings: HashMap<String, RingClient>,
    /// Things that arrived while this PC was paused, for "3 received while paused" (in memory, reset on resume).
    quiet_arrivals: u32,
    /// Sensitive texts that arrived while paused: never kept, so only the count is (same reset).
    quiet_sensitive: u32,
    next_poll_ms: Option<i64>,
    was_paused: bool,
}

pub struct Presence {
    inner: Mutex<Inner>,
    counts: Mutex<Counts>,
    power: Mutex<Arc<dyn PowerSource>>,
    zone: Mutex<Arc<dyn Zone>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    pump_running: AtomicBool,
    wake: Arc<tokio::sync::Notify>,
    /// 2B.5: this PC's Focus (the source and the 3 s filter), read only while the feature is on and a phone is connected.
    pub(crate) focus: crate::focus::FocusRuntime,
}

impl Presence {
    pub fn new() -> Presence {
        Presence {
            inner: Mutex::new(Inner::default()),
            counts: Mutex::new(Counts::default()),
            power: Mutex::new(Arc::new(WinPower)),
            zone: Mutex::new(Arc::new(WindowsZone)),
            runtime: Mutex::new(None),
            pump_running: AtomicBool::new(false),
            wake: Arc::new(tokio::sync::Notify::new()),
            focus: crate::focus::FocusRuntime::new(),
        }
    }

    /// A handle that wakes the pump (the Windows Focus event uses it: it only wakes, it decides nothing).
    pub(crate) fn wake_handle(&self) -> Arc<tokio::sync::Notify> {
        self.wake.clone()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }

    fn count(&self, f: impl FnOnce(&mut Counts)) {
        f(&mut lock(&self.counts));
    }

    pub fn counts(&self) -> Counts {
        *lock(&self.counts)
    }

    fn power(&self) -> Arc<dyn PowerSource> {
        lock(&self.power).clone()
    }

    fn zone(&self) -> Arc<dyn Zone> {
        lock(&self.zone).clone()
    }

    /// The runtime the pump runs on, for calls that come from a thread that has none (the tray menu).
    pub fn set_runtime(&self, handle: tokio::runtime::Handle) {
        *lock(&self.runtime) = Some(handle);
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

impl Default for Presence {
    fn default() -> Self {
        Presence::new()
    }
}

/// What the tray, the status line and the drop window say about presence. A copy: the windows never hold the lock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresenceView {
    /// This PC's own pause (the icon goes grey, the tooltip says it).
    pub pc_pause: Option<PauseState>,
    /// "until 14:30", "until tomorrow 08:00" or "until resumed", as the user's clock says it.
    pub pc_pause_text: Option<String>,
    /// The same for the phone's pause ("" when it ends at a time this PC can't say).
    pub phone_pause_text: Option<String>,
    /// The headline phone's battery while it is connected and "show my phone's battery" is on.
    pub phone_battery: Option<(u8, bool)>,
    /// The last known battery of an offline phone, with the time it was last heard from (Unix ms).
    pub phone_last_known: Option<(u8, bool, i64)>,
    /// The phone says it is paused: `Some(text)` is the part after "paused": "until 14:30", "until resumed" or "" when the time is not known.
    pub phone_pause: Option<PhonePause>,
    /// "Pixel 8 is ringing" while it is.
    pub ring_text: Option<String>,
    /// "Ring my phone" is available (the phone is connected and lists `ring`).
    pub can_ring: bool,
    /// Why it is not, for the status line.
    pub ring_reason: Option<String>,
    /// A ring is on its way or going: the tray offers "Stop ringing phone".
    pub ring_active: bool,
    /// How many things arrived quietly while this PC was paused.
    pub quiet_arrivals: u32,
    /// How many sensitive texts arrived while paused and were not kept (the count only: nothing of them is kept).
    pub quiet_sensitive: u32,
    /// The phone doesn't list `presence` (an old Kin), so pausing can't be told to it.
    pub phone_knows_pause: bool,
    /// 2B.5: "Phone: Do Not Disturb": the phone's filter while it is on and the phone is connected (shown when "Show my phone's battery here" is on).
    pub phone_dnd: Option<Dnd>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhonePause {
    /// When the phone's pause ends (Unix ms on this PC's clock), `None` for "until resumed".
    pub ends_ms: Option<i64>,
}

// ===== the core's side ===================================================================================================

impl Core {
    // ----- the switches --------------------------------------------------------------------------------------------------

    /// The resolver of the per-feature settings (design section 4): the override of `peer` for `feature`, else the default of the flat key.
    pub fn feature_enabled(&self, feature: &str, peer: Option<&str>) -> bool {
        let s = lock(&self.settings);
        features::resolve(feature, peer, s.feature_default(feature), &s.feature_overrides)
    }

    /// Sets (`Some`) or clears (`None`) the per-peer override of a feature (design section 4). There is **no UI for it in 2B.4**: the store exists so that the
    /// several devices of 2D need a screen and nothing else. The next call of `feature_enabled` for that peer sees it.
    pub fn set_feature_override(&self, peer: &str, feature: &str, value: Option<bool>) {
        let settings = {
            let mut s = lock(&self.settings);
            match value {
                Some(v) => {
                    s.feature_overrides.entry(peer.to_string()).or_default().insert(feature.to_string(), v);
                }
                None => {
                    if let Some(m) = s.feature_overrides.get_mut(peer) {
                        m.remove(feature);
                        if m.is_empty() {
                            s.feature_overrides.remove(peer);
                        }
                    }
                }
            }
            s.clone()
        };
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json (overrides) failed: {e}");
        }
    }

    // ----- this PC's pause -----------------------------------------------------------------------------------------------

    /// This PC's own pause, if it is on now: a wall-clock comparison on each use, no timer.
    pub fn pause_state(&self) -> Option<PauseState> {
        lock(&self.settings).pause.filter(|p| p.is_on(now_ms()))
    }

    /// The gate. Checked next to a feature's own switch, at the sender and at the receiver.
    pub fn paused(&self) -> bool {
        self.pause_state().is_some()
    }

    /// Whether the phone says it is paused (its last `presence` of this connection, and not past its expiry).
    pub fn peer_paused(&self, device_id: &str) -> bool {
        let inner = self.presence.inner();
        inner.conns.get(device_id).and_then(|c| c.view.pause).is_some_and(|p| p.is_paused(mono_ms()))
    }

    /// Pause (`Some(choice)`) or resume (`None`). Persisted in `settings.json`; the phone is told within a second; the media session is taken down or put
    /// back; nothing else is stopped (the gate is an `if` at each entry). A second pause replaces the first.
    pub fn set_pause(self: &Arc<Self>, choice: Option<PauseChoice>) {
        let now = now_ms();
        let state = choice.map(|c| pause::pause_for(c, now, self.presence.zone().as_ref()));
        // The new state and `was_paused` change in **one critical section** (2B.4 review): the pump's tick compares the two, and in the gap between "the file says resumed" and
        // "`was_paused` says so" it used to see a pause that ended, run `pause_ended` and start the media, which this call then started a second time (a leaked WinRT session).
        // The order of the two locks is the one `presence_peer_hello` takes them in (the presence state, then the settings).
        let settings = {
            let mut inner = self.presence.inner();
            let mut s = lock(&self.settings);
            s.pause = state;
            inner.was_paused = state.is_some_and(|p| p.is_on(now));
            s.clone()
        };
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json (pause) failed: {e}");
        }
        log::info!(
            "pause: {}",
            match choice {
                Some(PauseChoice::OneHour) => "on for 1 hour",
                Some(PauseChoice::Tomorrow) => "on until tomorrow morning",
                Some(PauseChoice::UntilResumed) => "on until resumed",
                None => "resumed",
            }
        );
        self.pause_changed(now);
    }

    /// The state of the gate changed (a choice, a resume, or the end of a pause): tell the phones, the media and the windows.
    fn pause_changed(self: &Arc<Self>, now: i64) {
        let (on, sends): (bool, Vec<(String, Vec<Send>)>) = {
            // `on` is read **under the presence lock** (the order of every other place: the presence state, then the settings), so the `was_paused` written below can't be a
            // value that a concurrent `set_pause` has already replaced (2B.4 review: the resume race).
            let mut inner = self.presence.inner();
            let state = lock(&self.settings).pause;
            let on = self.paused();
            if !on {
                inner.quiet_arrivals = 0;
                inner.quiet_sensitive = 0;
            }
            inner.was_paused = on;
            let sends = inner.conns.iter_mut().map(|(id, c)| (id.clone(), c.sender.set_pause(now, state))).collect();
            (on, sends)
        };
        for (id, out) in sends {
            self.presence_send(&id, out);
        }
        self.ensure_presence_pump();
        self.presence.wake();
        self.media_apply_pause(on);
        self.emit(Event::StateChanged);
    }

    /// The pause ended by itself (the pump noticed): the same things as a resume, without writing anything.
    fn pause_ended(self: &Arc<Self>) {
        log::info!("pause: over");
        let now = now_ms();
        if lock(&self.settings).pause.is_some_and(|p| !p.is_on(now)) {
            // Clear a pause that ran out, so the file doesn't keep a time in the past (a clock moved back would bring it to life).
            let settings = {
                let mut s = lock(&self.settings);
                s.pause = None;
                s.clone()
            };
            let _ = settings.save(&self.config.data_dir);
        }
        self.pause_changed(now);
    }

    /// A manual item (a text, a link, a file) arrived while this PC is paused and was handled quietly: counted for "3 received while paused".
    pub fn presence_note_quiet_arrival(&self) {
        self.presence.inner().quiet_arrivals += 1;
        self.presence.count(|c| c.quiet_arrivals += 1);
        self.emit(Event::StateChanged);
    }

    /// Windows woke from sleep (`powerwatch`): a timer does not count the time asleep, so a pause that ended meanwhile is looked at now. The pump runs a tick (the end of
    /// the pause is noticed, the phone told, the media started) and the windows and the tray are drawn from the state as it is.
    pub fn presence_system_resumed(self: &Arc<Self>) {
        log::info!("the PC woke from sleep: the pause is checked again");
        self.ensure_presence_pump();
        self.presence.wake();
        self.emit(Event::StateChanged);
    }

    /// A **sensitive** text arrived while paused: it is neither kept nor put on the clipboard, so the window can only say that one came (a count, no content).
    pub fn presence_note_quiet_sensitive(&self) {
        self.presence.inner().quiet_sensitive += 1;
        self.emit(Event::StateChanged);
    }

    /// Unlinking the last phone clears the pause (a pause belongs to a PC that has someone to pause for).
    pub(crate) fn clear_pause_if_no_phone_left(self: &Arc<Self>) {
        if self.db.list_devices().map(|d| d.is_empty()).unwrap_or(false) && lock(&self.settings).pause.is_some() {
            self.set_pause(None);
        }
    }

    // ----- connections ---------------------------------------------------------------------------------------------------

    /// A phone's `hello` was accepted (`first`: the first one of this connection). The PC sends its snapshot; a new connection starts with nothing known.
    pub fn presence_peer_hello(self: &Arc<Self>, device_id: &str, first: bool) {
        // A `hello` that is handled on a socket that is closing, after its phone was unlinked (2B.4 review): no snapshot goes to a phone that is no longer paired.
        if !self.db.is_paired(device_id).unwrap_or(false) {
            self.presence.inner().conns.remove(device_id);
            return;
        }
        if !self.connections.has_capability(device_id, wire::CAPABILITY) {
            // An old phone has no presence: nothing is sent, nothing is shown, and what was known goes.
            self.presence.inner().conns.remove(device_id);
            return;
        }
        let battery = self.own_battery_for(device_id);
        // 2B.5: whether this PC is in Focus, read before the presence lock is taken (the first reading since nothing was being read touches Windows).
        let focus_now = self.focus_for_new_connection(now_ms());
        let sends = {
            let mut inner = self.presence.inner();
            let fresh = first || !inner.conns.contains_key(device_id);
            if !fresh {
                return;
            }
            let state = lock(&self.settings).pause;
            let mut sender = PresenceSender::new();
            let now = now_ms();
            sender.set_battery(now, battery);
            sender.set_pause(now, state.filter(|p| p.is_on(now)));
            // 2B.5: a phone that connects hears whether this PC is in Focus in its first snapshot (a paused PC puts none in). Decided before the lock below.
            sender.set_focus(now, focus_now);
            let sends = sender.connected(now);
            inner.conns.insert(
                device_id.to_string(),
                ConnPresence { sender, bucket: PresenceBucket::new(), ring_bucket: PresenceBucket::new(), view: PeerView::default() },
            );
            sends
        };
        self.presence_send(device_id, sends);
        self.ensure_presence_pump();
        self.presence.wake();
    }

    /// The connection of a phone ended: its pause is forgotten (nothing is paused as far as this side knows), its last battery stays for the "last seen" line.
    pub fn presence_connection_closed(&self, device_id: &str) {
        // A phone that was unlinked leaves nothing behind: its last battery is not kept.
        let paired = self.db.is_paired(device_id).unwrap_or(false);
        let mut inner = self.presence.inner();
        if let Some(c) = inner.conns.remove(device_id)
            && let (Some(b), true) = (c.view.battery, paired)
        {
            inner.known.insert(device_id.to_string(), Known { battery: b, at_ms: now_ms() });
        }
        if let Some(r) = inner.rings.get_mut(device_id) {
            r.connection_lost();
        }
        drop(inner);
        // The pump looks again: with no phone connected the Focus source is dropped (no timer, no registered event otherwise).
        self.presence.wake();
        self.emit(Event::StateChanged);
    }

    /// How many phones have a presence connection now (a connection whose `hello` lists `presence`).
    pub fn presence_connection_count(&self) -> usize {
        self.presence.inner().conns.len()
    }

    /// The Do Not Disturb word of a connected phone, if any says one (the headline phone first: there is one phone in 0.2.0). **None while this PC is paused**: a
    /// paused Kin does nothing on its own, so the phone's word is not used (it is kept, and counts again when the pause ends).
    pub fn presence_phone_dnd(&self) -> Option<Dnd> {
        if self.paused() {
            return None;
        }
        self.presence.inner().conns.values().find_map(|c| c.view.dnd)
    }

    /// Tells every connection's sender the stable Focus value (within 1 s, trailing; never held back by the battery rule). Called on **every** turn of the pump, not only when
    /// the value changed: a sender that already has it does nothing, one that has another (a connection created while a reconnect raced the pump, 2B.5 review round 3) is armed
    /// to send within a second. It does not wake the pump (the caller is the pump).
    pub(crate) fn presence_sync_focus(self: &Arc<Self>, now: i64, on: bool) {
        let sends: Vec<(String, Vec<Send>)> = {
            let mut inner = self.presence.inner();
            inner.conns.iter_mut().map(|(id, c)| (id.clone(), c.sender.set_focus(now, on))).collect()
        };
        for (id, out) in sends {
            self.presence_send(&id, out);
        }
    }

    /// A clean quit (2B.5 review round 3): one last snapshot **without `focus`** goes to every connected phone before the sockets close, so the phone lets go at once instead of
    /// after its 60 s without a snapshot. Never blocks: the frame goes into each connection's own queue, ahead of the close that follows it.
    pub(crate) fn presence_quit_snapshot(&self) {
        let now = now_ms();
        let sends: Vec<(String, Send)> = {
            let mut inner = self.presence.inner();
            inner.conns.iter_mut().filter_map(|(id, c)| c.sender.quit_snapshot(now).map(|s| (id.clone(), s))).collect()
        };
        for (id, s) in sends {
            self.presence_send(&id, vec![s]);
        }
    }

    /// For tests: a connection that was created with a Focus value this side doesn't hold (what a reconnect racing the pump did): its sender is made to say `focus` at once.
    #[doc(hidden)]
    pub fn presence_force_stale_focus(self: &Arc<Self>, device_id: &str, focus: bool) {
        let now = now_ms();
        let sends = {
            let mut inner = self.presence.inner();
            inner.conns.get_mut(device_id).map(|c| {
                c.sender.disconnected();
                let mut out = c.sender.set_focus(now, focus);
                out.extend(c.sender.connected(now));
                out
            })
        };
        if let Some(sends) = sends {
            self.presence_send(device_id, sends);
        }
    }

    /// A phone was unlinked (or unlinked itself): everything this side knows about it goes, and so do its per-feature overrides.
    pub(crate) fn presence_forget_device(&self, device_id: &str) {
        {
            let mut inner = self.presence.inner();
            inner.conns.remove(device_id);
            inner.known.remove(device_id);
            inner.nudges.remove(device_id);
            inner.rings.remove(device_id);
        }
        let changed = {
            let mut s = lock(&self.settings);
            s.feature_overrides.remove(device_id).is_some()
        };
        if changed {
            let settings = lock(&self.settings).clone();
            if let Err(e) = settings.save(&self.config.data_dir) {
                log::warn!("saving settings.json (overrides) failed: {e}");
            }
        }
    }

    /// A `presence` or `ring.status` that the receiver dropped silently (a side that doesn't list `presence`).
    pub fn presence_dropped_by_receiver(&self) {
        self.presence.count(|c| c.dropped_by_receiver += 1);
    }

    /// What a peer's `presence` or `ring.status` changes here. A frame goes back only for `rate_limited`.
    pub fn presence_message(self: &Arc<Self>, device_id: &str, msg: PresenceMsg) -> Option<Value> {
        let now = mono_ms();
        match msg {
            PresenceMsg::Presence(body) => {
                let mut nudge_for: Option<(String, Battery)> = None;
                {
                    let mut inner = self.presence.inner();
                    let Some(conn) = inner.conns.get_mut(device_id) else {
                        self.presence.count(|c| c.dropped_not_listed += 1);
                        return None;
                    };
                    match conn.bucket.take(now) {
                        Taken::Accepted => {}
                        Taken::Dropped { error } => {
                            drop(inner);
                            self.presence.count(|c| c.dropped_rate += 1);
                            return error.then(|| error_message("rate_limited", "Too many presence messages.", None));
                        }
                    }
                    // A snapshot replaces the last one: a missing `pause` is "not paused", a missing `battery` is "unknown".
                    conn.view = PeerView { battery: body.battery, pause: limits::peer_pause_of(&body, now), dnd: body.dnd };
                    if let Some(b) = body.battery {
                        inner.known.insert(device_id.to_string(), Known { battery: b, at_ms: now_ms() });
                        nudge_for = Some((device_id.to_string(), b));
                    }
                }
                self.presence.count(|c| c.received += 1);
                if let Some((id, b)) = nudge_for {
                    self.low_battery_check(&id, b);
                }
                self.emit(Event::StateChanged);
                None
            }
            PresenceMsg::RingStatus(status) => {
                // The same receiver bucket as `presence` (2B.4 review), per connection: a phone that floods `ring.status` is dropped and counted, not obeyed.
                let taken = {
                    let mut inner = self.presence.inner();
                    inner.conns.get_mut(device_id).map(|c| c.ring_bucket.take(now))
                };
                match taken {
                    Some(Taken::Accepted) => self.ring_status(device_id, status),
                    Some(Taken::Dropped { .. }) | None => self.presence.count(|c| c.dropped_rate += 1),
                }
                None
            }
        }
    }

    /// The low-battery machine of this phone: one toast "Pixel 8 is at 14% and not charging" per discharge episode. Held back (and not used up) while this PC is paused
    /// and while a fullscreen program, a presentation or quiet time is on: a toast must never steal a game's focus.
    fn low_battery_check(&self, device_id: &str, b: Battery) {
        if !self.feature_enabled(features::PRESENCE_NUDGE, Some(device_id)) {
            return;
        }
        let held = self.paused() || self.user_state().state().holds_back_windows();
        let nudge = {
            let mut inner = self.presence.inner();
            inner.nudges.entry(device_id.to_string()).or_default().snapshot(b.level, b.charging, held)
        };
        // 2B.5: while the phone is in Do Not Disturb Kin's own optional toast waits. It is **not lost and not delayed**: the machine has already disarmed, so the toast does
        // not come later either (that is what "hold" means, design 3.6). A held nudge was not shown, so it is not counted as shown (`nudges`): the hold has its own count.
        if nudge && self.hold_own_toast(crate::focus::hold::ToastKind::LowBattery) {
            return;
        }
        if nudge {
            self.presence.count(|c| c.nudges += 1);
            let name = self.presence_name(device_id);
            // The level is on the toast and nowhere else: the log gets a plain line.
            self.emit(Event::ToastRedacted {
                text: format!("{name} is at {}% and not charging", b.level),
                log_text: format!("low battery warning for {name}"),
            });
        }
    }

    fn presence_name(&self, device_id: &str) -> String {
        self.db.get_device(device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name)
    }

    // ----- sending -------------------------------------------------------------------------------------------------------

    fn presence_send(&self, device_id: &str, sends: Vec<Send>) {
        for s in sends {
            let frame = wire::presence_frame(&uuid_v7(unix_ms()), &s.body);
            let sent = self.connections.has_capability(device_id, wire::CAPABILITY)
                && self.connections.open_sender(device_id).is_some_and(|tx| tx.send(ConnCommand::Presence { frame }).is_ok());
            if sent {
                self.presence.count(|c| c.sent += 1);
            }
        }
    }

    /// This PC's battery for a phone: only if "share" is on for it and the PC has a battery (a desktop shares nothing).
    fn own_battery_for(&self, device_id: &str) -> Option<Battery> {
        if !self.feature_enabled(features::PRESENCE_SHARE, Some(device_id)) {
            return None;
        }
        let power = self.presence.power();
        if !power.has_battery() {
            return None;
        }
        power.read()
    }

    /// The power source changed or was polled: every connection's sender hears the new battery (and applies its 2-point and 30 s rules).
    pub fn presence_battery_changed(self: &Arc<Self>) {
        self.presence_battery_update(false);
    }

    /// `immediate`: the user switched sharing (the sender sends at once, see `PresenceSender::set_battery_now`).
    fn presence_battery_update(self: &Arc<Self>, immediate: bool) {
        let now = now_ms();
        let ids: Vec<String> = self.presence.inner().conns.keys().cloned().collect();
        let mut all = Vec::new();
        for id in ids {
            let battery = self.own_battery_for(&id);
            if let Some(c) = self.presence.inner().conns.get_mut(&id) {
                let out = if immediate { c.sender.set_battery_now(now, battery) } else { c.sender.set_battery(now, battery) };
                all.push((id.clone(), out));
            }
        }
        for (id, out) in all {
            self.presence_send(&id, out);
        }
        self.presence.wake();
    }

    /// The setting "Share this PC's battery with the phone".
    pub fn set_presence_share_battery(self: &Arc<Self>, on: bool) {
        self.set_presence_switch("presenceShareBattery", on, |s| &mut s.presence_share_battery);
        self.presence_battery_update(true);
    }

    /// The setting "Show my phone's battery here".
    pub fn set_presence_show_phone(self: &Arc<Self>, on: bool) {
        self.set_presence_switch("presenceShowPhone", on, |s| &mut s.presence_show_phone);
    }

    /// The setting "Warn me when the phone's battery is low".
    pub fn set_low_battery_nudge(self: &Arc<Self>, on: bool) {
        self.set_presence_switch("lowBatteryNudge", on, |s| &mut s.low_battery_nudge);
    }

    fn set_presence_switch(&self, name: &str, on: bool, field: impl FnOnce(&mut crate::settings::Settings) -> &mut bool) {
        let settings = {
            let mut s = lock(&self.settings);
            let f = field(&mut s);
            if *f == on {
                return;
            }
            *f = on;
            s.clone()
        };
        log::info!("{name} turned {}", if on { "on" } else { "off" });
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json failed: {e}");
        }
        self.emit(Event::StateChanged);
    }

    /// For tests and `--dev-battery`: replaces where this PC's battery comes from.
    pub fn set_power_source(&self, source: Arc<dyn PowerSource>) {
        *lock(&self.presence.power) = source;
    }

    /// `--dev-battery` / `--dev-set devBattery`: a fixed fake battery (or none), pushed to the phones now.
    pub fn dev_set_battery(self: &Arc<Self>, battery: Option<Battery>) {
        self.set_power_source(Arc::new(FakePower::new(battery)));
        self.presence_battery_update(true);
        self.emit(Event::StateChanged);
    }

    /// For tests: the zone of the clock that "until tomorrow" is read in (the real one is Windows').
    #[doc(hidden)]
    pub fn set_presence_zone(&self, zone: Arc<dyn Zone>) {
        *lock(&self.presence.zone) = zone;
    }

    // ----- the pump --------------------------------------------------------------------------------------------------------

    /// Starts the one task that wakes at the next due time. It runs on the runtime it was first asked on, and ends when the core is gone.
    pub fn ensure_presence_pump(self: &Arc<Self>) {
        if self.presence.pump_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let handle = lock(&self.presence.runtime).clone().or_else(|| tokio::runtime::Handle::try_current().ok());
        let Some(handle) = handle else {
            self.presence.pump_running.store(false, Ordering::SeqCst);
            return;
        };
        let weak = Arc::downgrade(self);
        let wake = self.presence.wake.clone();
        handle.spawn(async move {
            loop {
                // On the blocking pool: the end of a pause starts the media source (WinRT), which must not run on an async worker.
                let wait = {
                    let Some(core) = weak.upgrade() else { return };
                    tokio::task::spawn_blocking(move || core.presence_tick(now_ms())).await.unwrap_or(Duration::from_secs(1))
                };
                tokio::select! {
                    () = wake.notified() => {}
                    () = tokio::time::sleep(wait) => {}
                }
            }
        });
    }

    /// Everything that is due at `now`: the trailing sends, the end of a pause, the battery poll and the ring's watchdog. Returns how long until the next one.
    pub fn presence_tick(self: &Arc<Self>, now: i64) -> Duration {
        let mut due: Option<i64> = None;
        let mut min_due = |at: i64| due = Some(due.map_or(at, |d| d.min(at)));
        // The end of this PC's own pause.
        // Read and swapped in one critical section, under the presence lock (the settings are read inside it: the order everywhere else), so a user's `set_pause`
        // can't slip between "is it paused" and "was it paused" (2B.4 review: the resume race).
        let (was, on) = {
            let mut inner = self.presence.inner();
            let on = self.paused();
            (std::mem::replace(&mut inner.was_paused, on), on)
        };
        if was && !on {
            self.pause_ended();
        }
        if let Some(PauseState::Until(end)) = self.pause_state() {
            min_due(end);
        }
        // 2B.5: this PC's Focus: read on the event, on the 15 s tick and when a phone connects, only while the feature is on and a phone that lists `presence` is connected.
        // A change goes to the senders (trailing, within 1 s), so it comes before the trailing sends are fired.
        if let Some(at) = self.focus_tick(now) {
            min_due(at);
        }
        // The trailing sends.
        let sends: Vec<(String, Vec<Send>)> = {
            let mut inner = self.presence.inner();
            inner.conns.iter_mut().map(|(id, c)| (id.clone(), c.sender.advance(now))).collect()
        };
        for (id, out) in sends {
            self.presence_send(&id, out);
        }
        for c in self.presence.inner().conns.values() {
            if let Some(at) = c.sender.next_due() {
                min_due(at);
            }
        }
        // The battery poll: only while a phone is connected, the switch is on and this PC has a battery.
        let polling = {
            let inner = self.presence.inner();
            !inner.conns.is_empty()
        } && self.feature_enabled(features::PRESENCE_SHARE, None)
            && self.presence.power().has_battery();
        if polling {
            let poll_due = {
                let mut inner = self.presence.inner();
                *inner.next_poll_ms.get_or_insert(now + battery::POLL_SECS as i64 * 1000)
            };
            if now >= poll_due {
                self.presence_battery_changed();
                self.presence.inner().next_poll_ms = Some(now + battery::POLL_SECS as i64 * 1000);
                min_due(now + battery::POLL_SECS as i64 * 1000);
            } else {
                min_due(poll_due);
            }
        } else {
            self.presence.inner().next_poll_ms = None;
        }
        // The ring's watchdog.
        let ring_due = {
            let mut inner = self.presence.inner();
            let mut changed = false;
            let mut next: Option<i64> = None;
            for r in inner.rings.values_mut() {
                let before = r.state().clone();
                r.tick(now);
                changed |= before != *r.state();
                if let RingState::Requested { since } | RingState::Ringing { since, .. } = r.state() {
                    next = Some(next.map_or(since + crate::ring::WATCHDOG_MS, |n: i64| n.min(since + crate::ring::WATCHDOG_MS)));
                }
            }
            (changed, next)
        };
        if ring_due.0 {
            self.emit(Event::StateChanged);
        }
        if let Some(at) = ring_due.1 {
            min_due(at);
        }
        Duration::from_millis(due.map_or(3_600_000, |d| (d - now).clamp(1, 3_600_000)) as u64)
    }

    // ----- ring my phone ---------------------------------------------------------------------------------------------------

    /// The phone a ring would go to: the connected one whose `hello` lists `ring`, newest first.
    fn ring_target(&self) -> Option<String> {
        self.connections
            .open_device_ids()
            .into_iter()
            // A phone that was just unlinked may still have its connection open for a moment (it has not answered the `unpair`): it can not be rung.
            .filter(|id| self.connections.has_capability(id, wire::RING_CAPABILITY) && self.db.is_paired(id).unwrap_or(false))
            .max_by_key(|id| self.db.get_device(id).ok().flatten().and_then(|d| d.last_seen).unwrap_or(0))
    }

    /// Why a ring is not possible now, for the status line and the buttons. `None`: it is.
    pub fn ring_unavailable_reason(&self) -> Option<String> {
        if self.ring_target().is_some() {
            return None;
        }
        let headline = self.snapshot_headline();
        let Some((id, name)) = headline else {
            return Some("No phone is paired, so there is nothing to ring.".to_string());
        };
        crate::ring::unavailable_reason(
            &name,
            self.connections.open_sender(&id).is_some(),
            self.connections.has_capability(&id, wire::CAPABILITY),
            self.connections.has_capability(&id, wire::RING_CAPABILITY),
        )
    }

    fn snapshot_headline(&self) -> Option<(String, String)> {
        let devices = self.db.list_devices().ok()?;
        let open = self.connections.open_device_ids();
        devices
            .iter()
            .find(|d| open.contains(&d.device_id))
            .or_else(|| devices.iter().max_by_key(|d| d.paired_at))
            .map(|d| (d.device_id.clone(), d.name.clone()))
    }

    /// "Ring my phone" (the tray, the drop window, `Kin.exe --ring`). The answer is the line the user sees (also a toast). The PC makes no sound.
    pub async fn ring_start(self: &Arc<Self>) -> String {
        let Some(device) = self.ring_target() else {
            let why = self.ring_unavailable_reason().unwrap_or_else(|| "Ringing isn't available.".to_string());
            self.emit(Event::Toast(why.clone()));
            return why;
        };
        let name = self.presence_name(&device);
        let pressed = self.presence.inner().rings.entry(device.clone()).or_default().press(now_ms());
        match pressed {
            Ok(()) => {}
            Err(crate::ring::Refusal::AlreadyRinging) => return format!("{name} is already ringing"),
            Err(crate::ring::Refusal::TooSoon { wait_ms }) => {
                let text = format!("Wait {} s before ringing {name} again", (wait_ms + 999) / 1000);
                self.emit(Event::Toast(text.clone()));
                return text;
            }
        }
        self.emit(Event::StateChanged);
        let id = uuid_v7(unix_ms());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sent = self
            .connections
            .open_sender(&device)
            .is_some_and(|c| c.send(ConnCommand::Ring { frame: wire::ring_start_frame(&id), reply: Some(tx) }).is_ok());
        let outcome = if sent {
            // The connection answers within 5 s (or says it has no reply). The extra seconds are for the channel.
            match tokio::time::timeout(Duration::from_secs(8), rx).await {
                Ok(Ok(o)) => o,
                _ => SendOutcome::NoReply,
            }
        } else {
            SendOutcome::NotConnected
        };
        log::info!(
            "ring: start to device {} -> {}",
            short(&device),
            match &outcome {
                SendOutcome::Ack(_) => "acked".to_string(),
                SendOutcome::Failed(code) => format!("error {}", crate::messages::error_code_word(code)),
                SendOutcome::NoReply => "no answer".to_string(),
                SendOutcome::NotConnected => "not connected".to_string(),
                SendOutcome::Lost => "lost".to_string(),
            }
        );
        let text = {
            let mut inner = self.presence.inner();
            let client = inner.rings.entry(device.clone()).or_default();
            match outcome {
                // Acked: the phone says `ring.status` next. The state stays `Requested` until it does (or the watchdog lets go).
                SendOutcome::Ack(_) => None,
                SendOutcome::Failed(code) => Some(client.on_error(&code, &name)),
                SendOutcome::NoReply | SendOutcome::Lost => Some(client.on_no_answer(&name)),
                SendOutcome::NotConnected => Some(client.on_not_connected(&name)),
            }
        };
        self.ensure_presence_pump();
        self.presence.wake();
        self.emit(Event::StateChanged);
        match text {
            Some(t) => {
                self.emit(Event::Toast(t.clone()));
                t
            }
            None => format!("Ringing {name}"),
        }
    }

    /// "Stop ringing phone": a `ring.stop`, which the phone always acks. Whatever happens the PC stops showing "ringing" at once.
    pub async fn ring_stop(self: &Arc<Self>) -> String {
        let device = {
            let inner = self.presence.inner();
            inner.rings.iter().find(|(_, r)| r.state().is_active()).map(|(id, _)| id.clone())
        };
        let Some(device) = device.or_else(|| self.ring_target()) else {
            return "Nothing is ringing.".to_string();
        };
        let name = self.presence_name(&device);
        if let Some(r) = self.presence.inner().rings.get_mut(&device) {
            r.stop_pressed();
        }
        let sent = self.connections.has_capability(&device, wire::RING_CAPABILITY)
            && self
                .connections
                .open_sender(&device)
                .is_some_and(|c| c.send(ConnCommand::Ring { frame: wire::ring_stop_frame(&uuid_v7(unix_ms())), reply: None }).is_ok());
        log::info!("ring: stop to device {} -> {}", short(&device), if sent { "sent" } else { "not sent" });
        self.emit(Event::StateChanged);
        if sent { format!("Stopping {name}") } else { format!("{name} isn't connected") }
    }

    fn ring_status(&self, device_id: &str, status: RingStatus) {
        // A phone that was unlinked has nothing to say here any more.
        if !self.db.is_paired(device_id).unwrap_or(false) {
            return;
        }
        let name = self.presence_name(device_id);
        let notice = {
            let mut inner = self.presence.inner();
            // An unsolicited status (no ring was asked for in the last 35 s, none is going) is ignored (2B.4 review): the phone does not get to put "ringing" on the PC's
            // tray by itself. A `stopped` that follows the PC's own `ring.stop` comes within the same window.
            let client = inner.rings.entry(device_id.to_string()).or_default();
            if !client.expecting(now_ms()) {
                drop(inner);
                self.presence.count(|c| c.dropped_not_listed += 1);
                log::info!("ring: a status nobody asked for was ignored");
                return;
            }
            client.on_status(status.ringing, status.audible, status.reason.as_deref(), now_ms(), &name)
        };
        log::info!(
            "ring: device {} says {}",
            short(device_id),
            if status.ringing { "ringing".to_string() } else { format!("stopped ({})", reason_word(status.reason.as_deref())) }
        );
        if let Some(text) = notice {
            self.emit(Event::Toast(text));
        }
        self.presence.wake();
        self.emit(Event::StateChanged);
    }

    // ----- what the windows and status.json show ----------------------------------------------------------------------------

    /// The copy of the presence state that the tray and the windows draw from.
    pub fn presence_view(&self) -> PresenceView {
        let now = now_ms();
        let show = self.feature_enabled(features::PRESENCE_SHOW, None);
        let headline = self.snapshot_headline();
        let inner = self.presence.inner();
        let mut view = PresenceView {
            pc_pause: self.pause_state(),
            quiet_arrivals: inner.quiet_arrivals,
            quiet_sensitive: inner.quiet_sensitive,
            ..Default::default()
        };
        let until_text = |p: PauseState| match p {
            PauseState::UntilResumed => "until resumed".to_string(),
            PauseState::Until(ms) => format!("until {}", crate::localtime::clock_text_for(ms, now).unwrap_or_else(|| "later".to_string())),
        };
        view.pc_pause_text = view.pc_pause.map(until_text);
        if let Some((id, name)) = &headline {
            let conn = inner.conns.get(id);
            if show {
                if let Some(c) = conn {
                    view.phone_battery = c.view.battery.map(|b| (b.level, b.charging));
                } else if let Some(k) = inner.known.get(id) {
                    view.phone_last_known = Some((k.battery.level, k.battery.charging, k.at_ms));
                }
            }
            // Not while this PC is paused (see `presence_phone_dnd`).
            if show && view.pc_pause.is_none() {
                view.phone_dnd = conn.and_then(|c| c.view.dnd);
            }
            if let Some(c) = conn {
                let mono = mono_ms();
                view.phone_pause =
                    c.view.pause.filter(|p| p.is_paused(mono)).map(|p| PhonePause { ends_ms: p.remaining_ms(mono).map(|r| now + r) });
                view.phone_pause_text = view.phone_pause.map(|p| match p.ends_ms {
                    None => "until resumed".to_string(),
                    Some(ms) => until_text(PauseState::Until(ms)),
                });
            }
            view.phone_knows_pause = self.connections.has_capability(id, wire::CAPABILITY);
            if let Some(r) = inner.rings.get(id) {
                view.ring_text = r.text(name);
                view.ring_active = r.state().is_active();
            }
        }
        drop(inner);
        view.can_ring = self.ring_target().is_some();
        if !view.can_ring {
            view.ring_reason = self.ring_unavailable_reason();
        }
        view
    }

    /// What the Settings and the drop window show (2B.4): the pause, the phone's battery and pause, the ring. For the person at the PC: the windows may show the
    /// level, a log and `status.json` never get a name or a time of the phone's pause.
    pub fn presence_panel(&self) -> Value {
        let v = self.presence_view();
        let headline = self.snapshot_headline();
        let connected = headline.as_ref().is_some_and(|(id, _)| self.connections.open_sender(id).is_some());
        json!({
            "pause": {
                "on": v.pc_pause.is_some(),
                "untilResumed": v.pc_pause == Some(PauseState::UntilResumed),
                "text": v.pc_pause_text,
                "quietArrivals": v.quiet_arrivals,
                "quietSensitive": v.quiet_sensitive,
            },
            "phone": {
                "name": headline.as_ref().map(|(_, n)| n.clone()),
                "deviceId": headline.as_ref().map(|(id, _)| id.clone()),
                "connected": connected,
                "battery": v.phone_battery.map(|(level, charging)| json!({ "level": level, "charging": charging })),
                "lastKnown": v.phone_last_known.map(|(level, charging, _)| json!({ "level": level, "charging": charging })),
                "paused": v.phone_pause.is_some(),
                "pausedText": v.phone_pause_text,
                "knowsPause": v.phone_knows_pause,
                "dnd": v.phone_dnd.map(crate::focus::dnd_text),
            },
            "focus": self.focus_panel(),
            "ring": { "can": v.can_ring, "active": v.ring_active, "text": v.ring_text, "reason": v.ring_reason },
            "shareBattery": self.feature_enabled(features::PRESENCE_SHARE, None),
            "showPhone": self.feature_enabled(features::PRESENCE_SHOW, None),
            "lowBatteryNudge": self.feature_enabled(features::PRESENCE_NUDGE, None),
            "hasBattery": self.presence.power().has_battery(),
        })
    }

    /// For `status.json`: numbers and words only, never a name or a time of the phone's pause.
    pub fn presence_status_json(&self) -> Value {
        let view = self.presence_view();
        let c = self.presence.counts();
        let ring = {
            let inner = self.presence.inner();
            inner.rings.values().next().map(|r| r.to_json()).unwrap_or_else(|| json!({ "state": "idle", "counts": {} }))
        };
        let now = now_ms();
        json!({
            "pause": {
                "on": view.pc_pause.is_some(),
                "untilResumed": view.pc_pause == Some(PauseState::UntilResumed),
                "untilMs": match view.pc_pause { Some(PauseState::Until(ms)) => json!(ms), _ => Value::Null },
                "quietArrivals": view.quiet_arrivals,
                "quietSensitive": view.quiet_sensitive,
            },
            "presence": {
                "phone": {
                    "level": view.phone_battery.map(|b| b.0),
                    "charging": view.phone_battery.map(|b| b.1),
                    "lastKnownLevel": view.phone_last_known.map(|b| b.0),
                    "paused": view.phone_pause.is_some(),
                    "pausedUntilResumed": view.phone_pause.is_some_and(|p| p.ends_ms.is_none()),
                    "knowsPause": view.phone_knows_pause,
                    // The phone's current Do Not Disturb word (design 10.2), never a history: `status.json` is rewritten on every change.
                    "dnd": view.phone_dnd.map(Dnd::word),
                },
                "shareBattery": self.feature_enabled(features::PRESENCE_SHARE, None),
                "showPhone": self.feature_enabled(features::PRESENCE_SHOW, None),
                "nudge": self.feature_enabled(features::PRESENCE_NUDGE, None),
                "hasBattery": self.presence.power().has_battery(),
                "counts": {
                    "sent": c.sent, "received": c.received, "droppedRate": c.dropped_rate, "droppedByReceiver": c.dropped_by_receiver,
                    "droppedNotListed": c.dropped_not_listed, "nudges": c.nudges, "quietArrivals": c.quiet_arrivals,
                },
            },
            "ring": ring,
            "focus": self.focus_status_json(),
            "nowMs": now,
        })
    }

    /// The phone-side status for the extension's `status` (design 3.12): `paused: true` while this PC is paused.
    pub fn presence_paused_for_extension(&self) -> bool {
        self.paused()
    }
}

/// `--dev-dump-view settings`: the file lands in the data directory, where no battery level may be found (design T1), so the phone's level and its pause
/// text are replaced by a word, like the titles of the media panel.
pub fn redact_panel(panel: &mut Value) {
    for key in ["battery", "lastKnown"] {
        if panel["phone"][key].is_object() {
            panel["phone"][key] = json!("<redacted>");
        }
    }
    if panel["phone"]["pausedText"].is_string() {
        panel["phone"]["pausedText"] = json!("<redacted>");
    }
    // 2B.5: the phone's Do Not Disturb state is not written to a file in the data directory by a dump either.
    if panel["phone"]["dnd"].is_string() {
        panel["phone"]["dnd"] = json!("<redacted>");
    }
}

/// The export copy of `status.json` (2B.4 review): the phone's battery level, whether it charges, its last known level and its Do Not Disturb word are nulled, and the tray
/// texts (which carry "62% charging" and "phone: Do Not Disturb") are replaced. The file in the data directory keeps them (numbers for the person who runs a test);
/// a zip that is sent to someone else does not.
pub fn redact_status_for_export(status: &mut Value) {
    for key in ["level", "charging", "lastKnownLevel", "dnd"] {
        if !status["presence"]["phone"][key].is_null() {
            status["presence"]["phone"][key] = Value::Null;
        }
    }
    // Whether this PC is in Focus or Do Not Disturb right now is the person's own state, like the battery.
    if !status["focus"]["on"].is_null() {
        status["focus"]["on"] = Value::Null;
    }
    for key in ["status", "tooltip"] {
        if status["tray"][key].is_string() {
            status["tray"][key] = json!("<redacted: it may name the phone's battery>");
        }
    }
}

/// The phone's `ring.status` reason, for the log: only the words the protocol lists, anything else (text from the phone) is `other`.
pub(crate) fn reason_word(reason: Option<&str>) -> &'static str {
    match reason {
        None => "no reason",
        Some("timeout") => "timeout",
        Some("user") => "user",
        Some("pc") => "pc",
        Some("limit") => "limit",
        Some("error") => "error",
        Some(_) => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_reason_words_reach_the_log() {
        for word in ["timeout", "user", "pc", "limit", "error"] {
            assert_eq!(reason_word(Some(word)), word);
        }
        assert_eq!(reason_word(None), "no reason");
        assert_eq!(reason_word(Some("battery 14% at Home Wi-Fi\nfake log line")), "other");
        assert_eq!(reason_word(Some("TIMEOUT")), "other");
    }

    #[test]
    fn the_export_copy_of_status_json_has_no_phone_battery_or_dnd() {
        let mut status = json!({
            "presence": { "phone": { "level": 73, "charging": true, "lastKnownLevel": 12, "dnd": "priority", "paused": false, "knowsPause": true }, "counts": { "sent": 3 } },
            "tray": { "status": "Phone connected: Pixel 8 \u{b7} 73% charging", "tooltip": "Kin: Pixel 8 connected \u{b7} 73% charging \u{b7} phone: Do Not Disturb" },
            "state": "phone_connected",
            "focus": { "share": true, "on": true, "sourceActive": true, "counts": { "changes": 4 } },
        });
        redact_status_for_export(&mut status);
        assert!(status["focus"]["on"].is_null(), "whether this PC is in Focus is not exported");
        assert_eq!(
            (status["focus"]["share"].clone(), status["focus"]["counts"]["changes"].clone()),
            (json!(true), json!(4)),
            "the rest of focus stays"
        );
        let text = status.to_string();
        assert!(!text.contains("73") && !text.contains("12") && !text.contains("priority") && !text.contains("Do Not Disturb"), "{text}");
        assert_eq!(status["presence"]["phone"]["paused"], false, "what is not a level stays");
        assert_eq!(status["presence"]["counts"]["sent"], 3);
        assert_eq!(status["state"], "phone_connected");
    }

    #[test]
    fn a_dumped_panel_has_no_battery_level() {
        let mut panel = json!({ "phone": { "battery": { "level": 14, "charging": false }, "lastKnown": { "level": 12, "charging": true }, "pausedText": "until 14:30", "paused": true } });
        redact_panel(&mut panel);
        let text = panel.to_string();
        assert!(!text.contains("14") && !text.contains("12"), "{text}");
        assert_eq!(panel["phone"]["paused"], true, "a flag is not a time");
        let mut none = json!({ "phone": { "battery": null } });
        redact_panel(&mut none);
        assert!(none["phone"]["battery"].is_null());
    }

    #[test]
    fn the_monotonic_clock_never_goes_back() {
        let a = mono_ms();
        let b = mono_ms();
        assert!(b >= a);
    }
}
