//! Phone notifications on the PC (milestone 2B.2, docs/phases/phase-2b-2-design.md, protocol.md "Notifications", changelog 56).
//!
//! The phone posts, updates and removes notifications; the PC shows them as Windows toasts under **its own app user model ID** (`app.kinrelay.desktop.phone`,
//! `registration.rs`), with a reply box and buttons that reach Kin through a COM class (`activator.rs`). Pressing X on a toast can dismiss it on the phone.
//!
//! **Nothing about a notification is ever stored by Kin**: not in the database, not in a file, not in a log. The pieces:
//!
//! | file | job |
//! |---|---|
//! | `wire.rs` | the `notif.*` frames: validation, cleaning, the normalized post (pure) |
//! | `limits.rs` | the receiver's token bucket, duplicate window and action limit (pure) |
//! | `args.rs` | the identifiers a toast carries for a click, read back strictly (pure) |
//! | `toast_xml.rs` | a post as toast XML, cut to fit, and a toast read back from Windows' history (pure) |
//! | `surface.rs` | the toast layer behind a trait: WinRT (`WinSurface`) and a fake |
//! | `activator.rs` | the COM class `INotificationActivationCallback` |
//! | `registration.rs` | the `HKCU` keys (installed Kin: fixed names, only in a normal release run; every other instance: its own names, all removed again) |
//! | `icons.rs` | the cache of app icons, the one thing kept on disk |
//! | `otp_clip.rs` | "Copy <code>" and the 60-second clear |
//! | this file | the ledger in memory, and what the core does with a message, a click or a closed toast |
//!
//! **The ledger** holds, per phone and `nid`, the generation of the toast on screen, when it was shown and what its buttons offer. It lives in memory only; after a
//! restart of Kin the toasts are still in Windows' history, and a click is checked against that (`ToastSurface::history`, `toast_xml::offered_from_xml`).
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::connections::{ConnCommand, SendOutcome};
use crate::core::{Core, Event, lock, short};
use crate::encoding::{self, short_id_from_hash, unix_ms};
use crate::messages::error_message;

pub mod activator;
pub mod args;
pub mod icons;
pub mod limits;
pub mod otp_clip;
pub mod registration;
pub mod surface;
pub mod toast_xml;
pub mod wire;

use activator::{Activation, Source};
use args::{ToastAction, ToastArgs};
use icons::{IconCache, IconChecker, Stored, WicChecker};
use limits::{ActionLimiter, PostLimiter, Verdict};
use otp_clip::{Cleared, OtpClip};
use surface::{DismissReason, SurfaceEvent, ToastSpec, ToastSurface};
use toast_xml::{Offered, ToastContext};
use wire::{NotifMsg, Post, Status};

/// Windows keeps these long: a normal toast 8 hours, a silent one 2, a private one, a redacted one or one with a code 10 minutes. (`ExpiresOnReboot` is always on.)
pub const EXPIRY_NORMAL: Duration = Duration::from_secs(8 * 3600);
pub const EXPIRY_SILENT: Duration = Duration::from_secs(2 * 3600);
pub const EXPIRY_PRIVATE: Duration = Duration::from_secs(10 * 60);
/// Kin keeps at most this many toasts of one phone: the oldest go first (Windows has a cap of its own in the Action Center).
pub const MAX_TOASTS_PER_PHONE: usize = 30;
/// A click that repeats the same arguments within this long is the same click (a popup can report it through COM and through the toast object).
pub const ACTIVATION_DEDUPE_MS: u64 = 3000;
/// A click that needs the phone waits this long for it to connect (Kin was started by the click, or the connection dropped for a moment).
pub const PHONE_WAIT: Duration = Duration::from_secs(10);

/// The short ID of a device (the 8 characters of the QR code and of the toast group), from its full ID: the base32 of the hash that the device ID is the
/// base64url of.
pub fn short_of(device_id: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(device_id).ok()?;
    let hash: [u8; 32] = bytes.try_into().ok()?;
    Some(short_id_from_hash(&hash))
}

/// The `nid`s of a phone's toasts that a `notif.sync` removes: the ones on screen that the phone's list does not have.
pub fn sync_remove(shown: &[String], listed: &[String]) -> Vec<String> {
    shown.iter().filter(|n| !listed.contains(n)).cloned().collect()
}

/// A hash of what a post would show, to tell an identical post from a changed one.
fn content_hash(post: &Post) -> u64 {
    let digest = encoding::sha256(post.to_json().to_string().as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("eight bytes"))
}

/// A button or a reply that the user pressed, as it goes to the phone: `text` is present exactly for a reply.
struct PressedAction {
    nid: String,
    i: u8,
    label: String,
    text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub shown: u64,
    pub replaced: u64,
    pub removed: u64,
    pub swept: u64,
    pub over_cap_removed: u64,
    pub dropped_feature_off: u64,
    pub dropped_by_receiver: u64,
    pub dropped_rate: u64,
    pub ignored_duplicates: u64,
    pub icons_stored: u64,
    pub icons_dropped: u64,
    pub actions_sent: u64,
    pub actions_failed: u64,
    pub dismissals_sent: u64,
    pub activations_refused: u64,
    pub codes_copied: u64,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub generation: u64,
    pub shown_at_ms: u64,
    pub offered: Offered,
}

struct Phone {
    short: String,
    limiter: PostLimiter,
    actions: ActionLimiter,
    ledger: HashMap<String, Entry>,
    /// The `nid`s in the order they were last shown, oldest first.
    order: VecDeque<String>,
    status: Option<Status>,
}

impl Phone {
    fn new(short: String) -> Phone {
        Phone {
            short,
            limiter: PostLimiter::default(),
            actions: ActionLimiter::default(),
            ledger: HashMap::new(),
            order: VecDeque::new(),
            status: None,
        }
    }
}

#[derive(Default)]
struct State {
    phones: HashMap<String, Phone>,
    counters: Counters,
    otp: OtpClip,
    /// The arguments of a click, when it came, and the reply text it carried (trimmed; empty for anything but a reply).
    recent_activations: VecDeque<(String, u64, String)>,
    next_generation: u64,
}

pub struct Mirror {
    state: Mutex<State>,
    surface: Mutex<Option<Arc<dyn ToastSurface>>>,
    win_surface: Mutex<Option<Arc<surface::WinSurface>>>,
    class: Mutex<Option<activator::ClassRegistration>>,
    identity: Mutex<Option<registration::Identity>>,
    icons: IconCache,
    checker: Mutex<Arc<dyn IconChecker>>,
    /// The COM class is registered and Windows can reach it: buttons and the reply box are offered. Off: text-only toasts (decision 2).
    activator_ok: AtomicBool,
    /// Why the mirror can't run in this process, if it can't (shown in the settings window).
    unavailable: Mutex<Option<String>>,
    /// The runtime that handles what the toast layer and the COM class report: they call from threads of Windows' own, which have none.
    runtime: Mutex<Option<tokio::runtime::Handle>>,
}

impl Mirror {
    pub fn new(data_dir: &Path) -> Mirror {
        Mirror {
            state: Mutex::new(State::default()),
            surface: Mutex::new(None),
            win_surface: Mutex::new(None),
            class: Mutex::new(None),
            identity: Mutex::new(None),
            icons: IconCache::new(data_dir),
            checker: Mutex::new(Arc::new(WicChecker)),
            activator_ok: AtomicBool::new(false),
            unavailable: Mutex::new(None),
            runtime: Mutex::new(None),
        }
    }

    /// The runtime for the events of the toast layer (the app gives it at start; a caller already inside a runtime needn't).
    pub fn set_runtime(&self, handle: tokio::runtime::Handle) {
        *lock(&self.runtime) = Some(handle);
    }

    fn runtime(&self) -> Option<tokio::runtime::Handle> {
        lock(&self.runtime).clone().or_else(|| tokio::runtime::Handle::try_current().ok())
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_surface(&self, surface: Option<Arc<dyn ToastSurface>>) {
        *lock(&self.surface) = surface;
    }

    pub fn surface(&self) -> Option<Arc<dyn ToastSurface>> {
        lock(&self.surface).clone()
    }

    pub fn has_surface(&self) -> bool {
        lock(&self.surface).is_some()
    }

    pub fn set_activator_ok(&self, ok: bool) {
        self.activator_ok.store(ok, Ordering::SeqCst);
    }

    pub fn activator_ok(&self) -> bool {
        self.activator_ok.load(Ordering::SeqCst)
    }

    pub fn icons(&self) -> &IconCache {
        &self.icons
    }

    /// For tests: a decoder that doesn't need Windows.
    pub fn set_icon_checker(&self, checker: Arc<dyn IconChecker>) {
        *lock(&self.checker) = checker;
    }

    pub fn counters(&self) -> Counters {
        self.state().counters.clone()
    }

    pub fn unavailable(&self) -> Option<String> {
        lock(&self.unavailable).clone()
    }

    pub fn entry(&self, device_id: &str, nid: &str) -> Option<Entry> {
        self.state().phones.get(device_id).and_then(|p| p.ledger.get(nid)).cloned()
    }

    /// The phone and entry of a `nid` that is on screen (for the developer commands).
    pub fn find(&self, nid: &str) -> Option<(String, String, Entry)> {
        let state = self.state();
        state.phones.iter().find_map(|(d, p)| p.ledger.get(nid).map(|e| (d.clone(), p.short.clone(), e.clone())))
    }

    pub fn status_of(&self, device_id: &str) -> Option<Status> {
        self.state().phones.get(device_id).and_then(|p| p.status.clone())
    }

    /// How many toasts of this phone are on screen according to the ledger.
    pub fn on_screen(&self, device_id: &str) -> usize {
        self.state().phones.get(device_id).map_or(0, |p| p.ledger.len())
    }

    fn count(&self, f: impl FnOnce(&mut Counters)) {
        f(&mut self.state().counters);
    }

    /// Forgets a phone that is gone: its ledger, its limiters, its status.
    pub fn forget_phone(&self, device_id: &str) -> Option<String> {
        self.state().phones.remove(device_id).map(|p| p.short)
    }

    /// Forgets every toast of every phone (the switch was turned off).
    pub fn forget_all(&self) {
        self.state().phones.clear();
    }

    /// The numbers of `status.json`.
    pub fn counters_json(&self) -> Value {
        let c = self.counters();
        json!({
            "shown": c.shown,
            "replaced": c.replaced,
            "removed": c.removed,
            "swept": c.swept,
            "overCapRemoved": c.over_cap_removed,
            "dropped": {
                "featureOff": c.dropped_feature_off,
                "byReceiver": c.dropped_by_receiver,
                "rate": c.dropped_rate,
                "duplicate": c.ignored_duplicates,
            },
            "icons": { "stored": c.icons_stored, "dropped": c.icons_dropped },
            "actions": { "sent": c.actions_sent, "failed": c.actions_failed, "dismissalsSent": c.dismissals_sent, "refused": c.activations_refused },
            "codesCopied": c.codes_copied,
        })
    }
}

/// What the user did to the surface, or Windows told us: one channel for the two producers (the toast layer and the COM class).
#[derive(Clone, Debug)]
pub enum MirrorEvent {
    Surface(SurfaceEvent),
    Activation(Activation),
}

// ===== the core's side ===================================================================================================

impl Core {
    /// What this PC puts in its `hello`: `link`, and `notif` only while "Show phone notifications" is on **and** the toast layer runs.
    pub fn capabilities(&self) -> Vec<&'static str> {
        let mut caps = crate::messages::CAPABILITIES.to_vec();
        if self.mirror_on() && self.mirror.has_surface() {
            caps.push(wire::CAPABILITY);
        }
        caps
    }

    /// The setting "Show phone notifications".
    pub fn mirror_on(&self) -> bool {
        lock(&self.settings).mirror_notifications
    }

    fn phone_label(&self, device_id: &str) -> String {
        self.db.get_device(device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name)
    }

    /// A `notif.*` message that the receiver let through (validation step 7 and later). Returns a frame to send back, if one is due (`rate_limited`).
    pub async fn mirror_message(&self, device_id: &str, msg: NotifMsg) -> Option<Value> {
        if !self.mirror_on() || !self.mirror.has_surface() {
            self.mirror.count(|c| c.dropped_feature_off += 1);
            return None;
        }
        match msg {
            NotifMsg::Status(status) => {
                self.mirror_status(device_id, status);
                None
            }
            NotifMsg::Icon { hash, png } => {
                self.mirror_icon(hash, png).await;
                None
            }
            NotifMsg::Post(post) => self.mirror_post(device_id, *post).await,
            NotifMsg::Remove { nid } => {
                self.mirror_remove(device_id, &nid);
                None
            }
            NotifMsg::Sync { nids } => {
                self.mirror_sync(device_id, nids).await;
                None
            }
            // These two go from the PC to the phone; the receiver of the PC never gets one.
            NotifMsg::Dismiss { .. } | NotifMsg::Action(_) => None,
        }
    }

    /// A `notif.*` the receiver dropped silently (the feature is off, or the sender didn't list `notif`).
    pub fn mirror_dropped_by_receiver(&self) {
        self.mirror.count(|c| c.dropped_by_receiver += 1);
    }

    /// A `notif.icon` the receiver dropped (wrong hash, not a PNG, wrong size).
    pub fn mirror_icon_dropped(&self) {
        self.mirror.count(|c| c.icons_dropped += 1);
    }

    fn mirror_status(&self, device_id: &str, status: Status) {
        let Some(short) = short_of(device_id) else { return };
        log::info!(
            "phone notifications, status of device {}: mirroring {}, access {:?}, {} apps",
            short_prefix(device_id),
            status.enabled,
            status.access,
            status.apps
        );
        self.mirror.state().phones.entry(device_id.to_string()).or_insert_with(|| Phone::new(short)).status = Some(status);
        self.emit(Event::StateChanged);
    }

    async fn mirror_icon(&self, hash: String, png: Vec<u8>) {
        let cache = IconCache::new(&self.config.data_dir);
        let checker = lock(&self.mirror.checker).clone();
        let stored = tokio::task::spawn_blocking(move || cache.store(&hash, &png, checker.as_ref())).await;
        match stored {
            Ok(Stored::Written) => self.mirror.count(|c| c.icons_stored += 1),
            Ok(Stored::AlreadyThere) => {}
            Ok(Stored::Rejected(why)) => {
                log::warn!("a phone notification icon was refused: {why}");
                self.mirror.count(|c| c.icons_dropped += 1);
            }
            Err(e) => log::error!("the icon task failed: {e}"),
        }
    }

    async fn mirror_post(&self, device_id: &str, post: Post) -> Option<Value> {
        let surface = self.mirror.surface()?;
        let short = short_of(device_id)?;
        let (hide_content, sound, text_only) = {
            let s = lock(&self.settings);
            (s.mirror_hide_content, s.mirror_sound, s.mirror_text_only)
        };
        let now = self.clock.now_ms();
        let phone_name = self.phone_label(device_id);
        let hash = content_hash(&post);

        let generation = {
            let mut state = self.mirror.state();
            let phone = state.phones.entry(device_id.to_string()).or_insert_with(|| Phone::new(short.clone()));
            match phone.limiter.post(now, &post.nid, hash) {
                Verdict::Accepted => {}
                Verdict::IgnoredDuplicate => {
                    state.counters.ignored_duplicates += 1;
                    return None;
                }
                Verdict::DroppedRate { send_error } => {
                    state.counters.dropped_rate += 1;
                    return send_error.then(|| error_message("rate_limited", "Too many notifications. Some were dropped.", None));
                }
            }
            state.next_generation += 1;
            state.next_generation
        };

        let icon = post.icon_hash.as_deref().and_then(|h| self.mirror.icons.path_of(h));
        let effective_text_only = text_only || !self.mirror.activator_ok();
        let ctx = ToastContext {
            phone_name: &phone_name,
            device: &short,
            icon: icon.as_deref(),
            hide_content,
            sound: sound && !self.config.dev_toast_quiet,
            text_only: effective_text_only,
        };
        let built = toast_xml::build(&post, &ctx);
        let silent = post.silent || post.importance <= 2 || self.config.dev_toast_quiet;
        let expires_in = if built.private || post.redacted || post.code.is_some() {
            EXPIRY_PRIVATE
        } else if silent {
            EXPIRY_SILENT
        } else {
            EXPIRY_NORMAL
        };
        let spec = ToastSpec {
            tag: args::tag_of(&post.nid),
            group: args::group_of(&short),
            xml: built.xml,
            suppress_popup: silent,
            expires_in,
            generation,
        };
        let group = spec.group.clone();
        // The oldest toasts of this phone go first when there are more than 30.
        let evicted: Vec<String> = {
            let mut state = self.mirror.state();
            let existed = state.phones.get(device_id).is_some_and(|p| p.ledger.contains_key(&post.nid));
            let phone = state.phones.entry(device_id.to_string()).or_insert_with(|| Phone::new(short.clone()));
            phone.order.retain(|n| *n != post.nid);
            phone.order.push_back(post.nid.clone());
            phone.ledger.insert(post.nid.clone(), Entry { generation, shown_at_ms: now, offered: built.offered });
            let mut evicted = Vec::new();
            while phone.order.len() > MAX_TOASTS_PER_PHONE {
                if let Some(old) = phone.order.pop_front() {
                    phone.ledger.remove(&old);
                    evicted.push(old);
                }
            }
            if existed {
                state.counters.replaced += 1;
            } else {
                state.counters.shown += 1;
            }
            state.counters.over_cap_removed += evicted.len() as u64;
            evicted
        };
        for old in evicted {
            surface.remove(&old, &group);
        }
        // Counts and a keyed tag only: nothing the phone sent goes into the log.
        log::info!(
            "phone notification {} from device {}: {}{}",
            crate::privacy::tag(post.nid.as_bytes()),
            short_prefix(device_id),
            if built.private { "private form" } else { "shown" },
            if silent { ", no popup" } else { "" }
        );
        // Showing waits for the toast thread (up to 10 s): not on a thread of the runtime. The connection that delivered this post waits for it, so the
        // posts and removes of one phone stay in order.
        match tokio::task::spawn_blocking(move || surface.show(spec)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log::warn!("showing a phone notification failed: {e}"),
            Err(e) => log::warn!("showing a phone notification failed: the task ended ({e})"),
        }
        self.emit(Event::StateChanged);
        None
    }

    fn mirror_remove(&self, device_id: &str, nid: &str) {
        let Some(surface) = self.mirror.surface() else { return };
        let Some(short) = short_of(device_id) else { return };
        {
            let mut state = self.mirror.state();
            if let Some(phone) = state.phones.get_mut(device_id) {
                phone.ledger.remove(nid);
                phone.order.retain(|n| n != nid);
            }
            state.counters.removed += 1;
        }
        // Not a `Dismissed` for the user: removing from the history raises none (spike), and the ledger entry is gone, so nothing would act on one anyway.
        surface.remove(nid, &args::group_of(&short));
        self.emit(Event::StateChanged);
    }

    /// `notif.sync`: what the phone still has. Every toast of this phone that Windows holds (also from before a restart of Kin) and is not in the list goes.
    async fn mirror_sync(&self, device_id: &str, nids: Vec<String>) {
        let (Some(surface), Some(short)) = (self.mirror.surface(), short_of(device_id)) else { return };
        let group = args::group_of(&short);
        let listed: std::collections::HashSet<String> = nids.into_iter().collect();
        let held = {
            let surface = surface.clone();
            tokio::task::spawn_blocking(move || surface.history()).await.unwrap_or_default()
        };
        let mut gone: Vec<String> = held.into_iter().filter(|h| h.group == group && !listed.contains(&h.tag)).map(|h| h.tag).collect();
        {
            let mut state = self.mirror.state();
            if let Some(phone) = state.phones.get_mut(device_id) {
                let stale: Vec<String> = phone.ledger.keys().filter(|n| !listed.contains(*n)).cloned().collect();
                for n in stale {
                    phone.ledger.remove(&n);
                    phone.order.retain(|o| *o != n);
                    if !gone.contains(&n) {
                        gone.push(n);
                    }
                }
            }
            state.counters.swept += gone.len() as u64;
        }
        for tag in &gone {
            surface.remove(tag, &group);
        }
        log::info!("phone notifications synced with device {}: {} toast(s) removed", short_prefix(device_id), gone.len());
        self.emit(Event::StateChanged);
    }

    /// A phone was unpaired: its toasts go, and with the last phone the icon cache.
    pub fn mirror_forget_device(&self, device_id: &str) {
        let short = short_of(device_id);
        self.mirror.forget_phone(device_id);
        if let (Some(surface), Some(short)) = (self.mirror.surface(), short) {
            surface.remove_group(&args::group_of(&short));
        }
        if self.db.list_devices().map(|d| d.is_empty()).unwrap_or(false) {
            self.mirror.icons.remove_all();
        }
    }

    // ----- what happens on the PC ---------------------------------------------------------------------------------------

    pub async fn mirror_event(self: &Arc<Self>, event: MirrorEvent) {
        match event {
            MirrorEvent::Surface(e) => self.mirror_surface_event(e).await,
            MirrorEvent::Activation(a) => self.mirror_activation(a).await,
        }
    }

    /// A toast was closed, or clicked in the process that showed it.
    pub async fn mirror_surface_event(self: &Arc<Self>, event: SurfaceEvent) {
        match event {
            SurfaceEvent::Activated { arguments, inputs } => {
                self.mirror_activation(Activation { source: Source::InProcess, arguments, inputs }).await;
            }
            SurfaceEvent::Dismissed { tag, group, generation, reason } => {
                // Only the exact "the user pressed X" (design decision 7). A popup that timed out, a toast the app hid and a suppressed one that was
                // never shown say nothing about what the user wants.
                if reason != DismissReason::UserCanceled {
                    return;
                }
                let Some(short) = group.strip_prefix("kin-").map(str::to_string) else { return };
                let device_id = {
                    let mut state = self.mirror.state();
                    let found = state.phones.iter_mut().find(|(_, p)| p.short == short);
                    let Some((device_id, phone)) = found else { return };
                    // A superseded toast (an older generation of this `nid`) or one Kin already removed is not the user's doing.
                    if phone.ledger.get(&tag).is_none_or(|e| e.generation != generation) {
                        return;
                    }
                    phone.ledger.remove(&tag);
                    phone.order.retain(|n| *n != tag);
                    device_id.clone()
                };
                if !lock(&self.settings).mirror_dismiss_to_phone {
                    return;
                }
                let frame = json!({ "type": "notif.dismiss", "id": encoding::uuid_v7(unix_ms()), "nid": tag });
                if self.send_ephemeral(&device_id, frame) {
                    self.mirror.count(|c| c.dismissals_sent += 1);
                    log::info!("a closed phone notification was dismissed on device {} too", short_prefix(&device_id));
                }
            }
        }
    }

    /// Sends a message that is never queued, acked or re-sent. `false` if the phone is not connected or doesn't list `notif`.
    fn send_ephemeral(&self, device_id: &str, frame: Value) -> bool {
        if !self.connections.has_capability(device_id, wire::CAPABILITY) {
            return false;
        }
        self.connections.open_sender(device_id).is_some_and(|tx| tx.send(ConnCommand::Notif { frame, reply: None }).is_ok())
    }

    /// The one function that a click ends in, whatever way it came (the COM class, the toast object, a developer command).
    pub async fn mirror_activation(self: &Arc<Self>, activation: Activation) {
        self.mirror_activation_inner(activation).await;
        // The numbers of `status.json` moved (an action sent, a click refused).
        self.emit(Event::StateChanged);
    }

    async fn mirror_activation_inner(self: &Arc<Self>, activation: Activation) {
        let now = self.clock.now_ms();
        // The same click reported twice (COM and the toast object) is one click.
        {
            let mut state = self.mirror.state();
            while state.recent_activations.front().is_some_and(|(_, at, _)| now.saturating_sub(*at) >= ACTIVATION_DEDUPE_MS) {
                state.recent_activations.pop_front();
            }
            // The arguments alone: the two routes report the same click with the same arguments, but not always with the same inputs, and a reply must
            // never be sent twice.
            let key = activation.arguments.clone();
            let typed = reply_text(&activation);
            if let Some((_, _, first)) = state.recent_activations.iter().find(|(k, _, _)| *k == key) {
                // A different text under the same button within seconds is not the same click (the user typed again): it is not sent, and the user is
                // told so instead of seeing nothing happen.
                let another_reply = !typed.is_empty() && typed != *first;
                drop(state);
                if another_reply {
                    self.emit(Event::Toast(
                        "A reply to this notification was sent a moment ago. Wait a few seconds, then send this one.".to_string(),
                    ));
                }
                return;
            }
            state.recent_activations.push_back((key, now, typed));
        }
        let refuse = |why: &str| {
            log::warn!("a click on a phone notification was refused: {why}");
            self.mirror.count(|c| c.activations_refused += 1);
        };
        if !self.mirror_on() {
            return refuse("phone notifications are off");
        }
        let Some(parsed) = args::parse(&activation.arguments) else {
            return refuse("the arguments are not ours");
        };
        // The phone has to be a paired one.
        let device = self
            .db
            .list_devices()
            .ok()
            .and_then(|ds| ds.into_iter().find(|d| short_of(&d.device_id).as_deref() == Some(parsed.device.as_str())));
        let Some(device) = device else {
            return refuse("no paired phone has that short ID");
        };
        if parsed.action == ToastAction::Tap {
            // A click on the toast does nothing visible: Kin doesn't come to the front. (The button for it is the reply.)
            return;
        }
        let Some(offered) = self.offered_for(&device.device_id, &parsed).await else {
            return refuse("no such notification on screen");
        };
        let phone_name = device.name.clone();
        match parsed.action {
            ToastAction::Tap => {}
            ToastAction::Copy => self.mirror_copy_code(offered.copy_code),
            ToastAction::Reply => {
                let text = reply_text(&activation);
                let Some((i, hint)) = offered.reply else {
                    return refuse("the toast offered no reply");
                };
                if text.is_empty() {
                    self.forget_recent(&activation.arguments);
                    self.emit(Event::Toast("Type your reply in the box first, then press Send.".to_string()));
                    return;
                }
                // The phone would answer `bad_request` to a longer one: say so here, and send nothing.
                let length = text.chars().count();
                if length > wire::MAX_ACTION_TEXT {
                    self.forget_recent(&activation.arguments);
                    self.emit(Event::Toast(format!(
                        "That reply is too long ({length} characters; at most {} are sent). Shorten it and press Send again.",
                        wire::MAX_ACTION_TEXT
                    )));
                    return;
                }
                let action = PressedAction { nid: parsed.nid.clone(), i, label: hint, text: Some(text) };
                self.mirror_send_action(&device.device_id, &phone_name, action).await;
            }
            ToastAction::Button => {
                let Some(label) = parsed.index.and_then(|i| offered.buttons.iter().find(|(b, _)| *b == i).map(|(_, l)| l.clone())) else {
                    return refuse("the toast offered no such button");
                };
                let action = PressedAction { nid: parsed.nid.clone(), i: parsed.index.unwrap_or(0), label, text: None };
                self.mirror_send_action(&device.device_id, &phone_name, action).await;
            }
        }
    }

    /// A click that sent nothing is not a click to be de-duplicated: the user may fix it and press again at once.
    fn forget_recent(&self, arguments: &str) {
        self.mirror.state().recent_activations.retain(|(k, _, _)| k != arguments);
    }

    /// What the toast of this `nid` offers: from the ledger, else from Windows' history (a toast from before a restart of Kin).
    async fn offered_for(&self, device_id: &str, parsed: &ToastArgs) -> Option<Offered> {
        if let Some(entry) = self.mirror.entry(device_id, &parsed.nid) {
            return Some(entry.offered);
        }
        let surface = self.mirror.surface()?;
        let group = args::group_of(&parsed.device);
        let tag = parsed.nid.clone();
        let held = tokio::task::spawn_blocking(move || surface.history()).await.ok()?;
        held.into_iter().find(|h| h.group == group && h.tag == tag).map(|h| toast_xml::offered_from_xml(&h.xml, &parsed.nid))
    }

    /// "Copy <code>": the code goes to the clipboard as sensitive, and leaves it again after 60 seconds if nothing else was copied.
    fn mirror_copy_code(self: &Arc<Self>, code: Option<String>) {
        let Some(code) = code else {
            self.mirror.count(|c| c.activations_refused += 1);
            return;
        };
        let now = self.clock.now_ms();
        let result = self.mirror.state().otp.copy(self.clipboard.as_ref(), &code, now);
        match result {
            Ok(due) => {
                self.mirror.count(|c| c.codes_copied += 1);
                log::info!("a one-time code was copied (it leaves the clipboard again in 60 s if nothing else is copied)");
                self.emit(Event::Toast("Code copied. It leaves the clipboard in a minute.".to_string()));
                let core = self.clone();
                let wait = Duration::from_millis(due.saturating_sub(now));
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    core.mirror_clear_code_if_due();
                });
            }
            Err(e) => {
                log::warn!("copying a one-time code failed: {e}");
                self.emit(Event::Toast("Couldn't copy the code: the clipboard is busy.".to_string()));
            }
        }
    }

    /// The 60-second clear, if it is due (a later Copy has moved the time on).
    pub fn mirror_clear_code_if_due(&self) {
        let now = self.clock.now_ms();
        let cleared = self.mirror.state().otp.clear_if_due(self.clipboard.as_ref(), now);
        self.log_cleared(cleared);
    }

    /// When Kin quits the same check runs once at once.
    pub fn mirror_clear_code_now(&self) {
        let cleared = self.mirror.state().otp.clear_now(self.clipboard.as_ref());
        self.log_cleared(cleared);
    }

    fn log_cleared(&self, cleared: Cleared) {
        match cleared {
            Cleared::Nothing => {}
            Cleared::Emptied => log::info!("the one-time code was taken off the clipboard"),
            Cleared::LeftAlone => log::info!("the clipboard changed since the one-time code was copied: left alone"),
            Cleared::Failed => log::warn!("taking the one-time code off the clipboard failed"),
        }
    }

    /// Sends a `notif.action` and tells the user what came of it. Never sent twice: with no `ack` in 5 seconds the user is told so and nothing is retried.
    async fn mirror_send_action(&self, device_id: &str, phone: &str, action: PressedAction) {
        let PressedAction { nid, i, label, text } = action;
        let is_reply = text.is_some();
        let now = self.clock.now_ms();
        // Only a paired phone ever gets an entry (a device ID that came from the arguments of a click is not trusted to make one).
        if !self.db.is_paired(device_id).unwrap_or(false) {
            log::warn!("an action on a phone notification was refused: the device is not a paired phone");
            self.mirror.count(|c| c.activations_refused += 1);
            return;
        }
        // From the first action on: a phone that has sent no `notif.status` yet has no entry, and gets one.
        let allowed = {
            let mut state = self.mirror.state();
            let short = short_of(device_id).unwrap_or_default();
            state.phones.entry(device_id.to_string()).or_insert_with(|| Phone::new(short)).actions.allow(now)
        };
        if !allowed {
            self.emit(Event::Toast("That is too fast. Wait a few seconds and try again.".to_string()));
            return;
        }
        // The phone may be a moment away (Kin was started by this very click).
        let sender = {
            let mut found = None;
            let deadline = tokio::time::Instant::now() + PHONE_WAIT;
            loop {
                if self.connections.has_capability(device_id, wire::CAPABILITY)
                    && let Some(tx) = self.connections.open_sender(device_id)
                {
                    found = Some(tx);
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            found
        };
        let Some(sender) = sender else {
            self.mirror.count(|c| c.actions_failed += 1);
            self.emit(Event::Toast(format!("{phone} is not connected. Nothing was sent.")));
            return;
        };
        let mut frame = json!({ "type": "notif.action", "id": encoding::uuid_v7(unix_ms()), "nid": nid, "i": i, "label": label });
        if let Some(t) = text {
            frame["text"] = json!(t);
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        if sender.send(ConnCommand::Notif { frame, reply: Some(reply_tx) }).is_err() {
            self.mirror.count(|c| c.actions_failed += 1);
            self.emit(Event::Toast(format!("{phone} is not connected. Nothing was sent.")));
            return;
        }
        // The connection answers within 5 seconds (or says it has no reply): the extra seconds are for the channel.
        let outcome = tokio::time::timeout(Duration::from_secs(8), reply_rx).await;
        let (ok, message) = match outcome {
            Ok(Ok(SendOutcome::Ack(_))) => (true, if is_reply { format!("Reply sent to {phone}.") } else { format!("Sent to {phone}.") }),
            Ok(Ok(SendOutcome::Failed(code))) => (false, failure_text(&code, phone)),
            Ok(Ok(SendOutcome::NotConnected)) => (false, format!("{phone} is not connected. Nothing was sent.")),
            _ => (false, format!("No answer from {phone}. It may not have received it.")),
        };
        self.mirror.count(|c| if ok { c.actions_sent += 1 } else { c.actions_failed += 1 });
        log::info!("an action on a phone notification: {}", if ok { "acked" } else { "not done" });
        self.emit(Event::Toast(message));
    }

    // ----- the settings -----------------------------------------------------------------------------------------------

    fn save_mirror_settings(&self, what: &str) {
        let settings = lock(&self.settings).clone();
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json ({what}) failed: {e}");
        }
    }

    /// "Show phone notifications". Turning it on starts the toast layer (and registers the keys); turning it off removes the toasts and the keys. The phone is
    /// told by a new `hello` either way.
    pub fn set_mirror_notifications(self: &Arc<Self>, on: bool) {
        let changed = {
            let mut s = lock(&self.settings);
            let changed = s.mirror_notifications != on;
            s.mirror_notifications = on;
            changed
        };
        if !changed {
            return;
        }
        log::info!("show phone notifications: {}", if on { "on" } else { "off" });
        self.save_mirror_settings("show phone notifications");
        if on {
            self.mirror_start();
        } else {
            self.mirror_stop(true);
        }
        // Only a phone that lists `notif` cares whether the PC does.
        self.connections.send_hello_to_capable(wire::CAPABILITY);
        self.emit(Event::StateChanged);
    }

    pub fn set_mirror_sound(&self, on: bool) {
        self.set_mirror_flag("mirrorSound", on, |s| &mut s.mirror_sound);
    }

    pub fn set_mirror_hide_content(&self, on: bool) {
        self.set_mirror_flag("mirrorHideContent", on, |s| &mut s.mirror_hide_content);
    }

    pub fn set_mirror_dismiss_to_phone(&self, on: bool) {
        self.set_mirror_flag("mirrorDismissToPhone", on, |s| &mut s.mirror_dismiss_to_phone);
    }

    /// The fallback switch of decision 2: only text on the toasts, no reply box and no buttons.
    pub fn set_mirror_text_only(&self, on: bool) {
        self.set_mirror_flag("mirrorTextOnly", on, |s| &mut s.mirror_text_only);
    }

    fn set_mirror_flag(&self, name: &str, on: bool, field: impl FnOnce(&mut crate::settings::Settings) -> &mut bool) {
        let changed = {
            let mut s = lock(&self.settings);
            let f = field(&mut s);
            let changed = *f != on;
            *f = on;
            changed
        };
        if changed {
            log::info!("{name}: {}", if on { "on" } else { "off" });
            self.save_mirror_settings(name);
            self.emit(Event::StateChanged);
        }
    }

    /// Starts the toast layer: the keys, the toast worker, the COM class. Does nothing if the mirror is off. A failure is kept (`unavailable`) and the PC
    /// then does not list `notif`.
    pub fn mirror_start(self: &Arc<Self>) {
        if !self.mirror_on() || self.mirror.has_surface() {
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                *lock(&self.mirror.unavailable) = Some(format!("the path of Kin.exe is not known ({e})"));
                return;
            }
        };
        let identity = registration::Identity::for_config(&self.config);
        // The installed Kin's own names are only ever written by a normal release run, never by a debug build, a developer setting or a build folder.
        if identity.kind == registration::Kind::Release && !(self.config.autostart_is_real() && !crate::integration::is_build_folder(&exe))
        {
            let why = "this run is not the installed Kin (a debug build, a developer setting or a build folder): it never writes the installed Kin's names".to_string();
            log::info!("phone notifications are not started: {why}");
            *lock(&self.mirror.unavailable) = Some(why);
            return;
        }
        *lock(&self.mirror.unavailable) = None;
        let icon = match self.write_app_icon() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("the app icon for phone notifications can't be written: {e}");
                self.config.data_dir.join("notif-app-icon.png")
            }
        };
        let registered = registration::register(&identity, &exe, &icon);
        if let Err(e) = &registered {
            log::warn!("registering phone notifications with Windows failed: {e}");
        } else {
            *lock(&self.mirror.identity) = Some(identity.clone());
        }
        // The toast layer: events of its toasts come back as MirrorEvents, on threads of Windows' own, so they are handed to the runtime.
        let Some(runtime) = self.mirror.runtime() else {
            *lock(&self.mirror.unavailable) = Some("no runtime to handle the toast events".to_string());
            return;
        };
        let core = self.clone();
        let events = move |e: MirrorEvent| {
            let core = core.clone();
            runtime.spawn(async move { core.mirror_event(e).await });
        };
        let on_surface_event = events.clone();
        let surface = surface::WinSurface::start(&identity.aumid, Arc::new(move |e| on_surface_event(MirrorEvent::Surface(e))));
        match surface {
            Ok(surface) => {
                let surface = Arc::new(surface);
                *lock(&self.mirror.win_surface) = Some(surface.clone());
                self.mirror.set_surface(Some(surface));
            }
            Err(e) => {
                log::warn!("the toast layer for phone notifications can't start: {e}");
                *lock(&self.mirror.unavailable) = Some(e);
                return;
            }
        }
        // The COM class. Without it (or without the keys) the toasts are text only: Windows would show buttons that nothing answers.
        let on_call = events;
        let class = registered
            .is_ok()
            .then(|| activator::ClassRegistration::start(identity.clsid_u128(), Arc::new(move |a| on_call(MirrorEvent::Activation(a)))))
            .transpose();
        match class {
            Ok(Some(class)) => {
                *lock(&self.mirror.class) = Some(class);
                self.mirror.set_activator_ok(true);
                log::info!("phone notifications are on: toasts under {}, activator class {} is registered", identity.aumid, identity.clsid);
            }
            Ok(None) | Err(_) => {
                if let Err(e) = class.map(|_| ()) {
                    log::warn!("{e}");
                }
                self.mirror.set_activator_ok(false);
                log::warn!("phone notifications are on in text-only form: the reply box and the buttons need the activator class");
            }
        }
        self.emit(Event::StateChanged);
    }

    /// Stops the toast layer. With `remove` the toasts and the keys go too (the switch was turned off, or a developer instance exits).
    pub fn mirror_stop(&self, remove: bool) {
        if let Some(mut class) = lock(&self.mirror.class).take() {
            class.stop();
        }
        self.mirror.set_activator_ok(false);
        if remove && let Some(surface) = self.mirror.surface() {
            surface.clear_all();
        }
        let win = lock(&self.mirror.win_surface).take();
        self.mirror.set_surface(None);
        if let Some(win) = win {
            win.stop();
        }
        self.mirror.forget_all();
        if remove && let Some(identity) = lock(&self.mirror.identity).take() {
            match registration::unregister(&identity) {
                Ok(n) => log::info!("phone notifications: {n} registry key(s) of {} removed", identity.aumid),
                Err(e) => log::warn!("removing the phone notification keys failed: {e}"),
            }
        }
    }

    /// The app's own icon as a PNG in the data directory: what Windows Settings shows next to "Kin: phone notifications".
    fn write_app_icon(&self) -> std::io::Result<std::path::PathBuf> {
        const ICON: &[u8] = include_bytes!("../../icons/128x128.png");
        let path = self.config.data_dir.join("notif-app-icon.png");
        if std::fs::read(&path).ok().as_deref() != Some(ICON) {
            std::fs::write(&path, ICON)?;
        }
        Ok(path)
    }

    // ----- what the windows show --------------------------------------------------------------------------------------

    /// What each phone's line in the settings window says (design 5.8).
    fn mirror_phone_lines(&self) -> Vec<Value> {
        self.db
            .list_devices()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let connected = self.connections.open_device_ids().contains(&d.device_id);
                let lists_notif = self.connections.has_capability(&d.device_id, wire::CAPABILITY);
                let status = self.mirror.status_of(&d.device_id);
                let text = match (connected, lists_notif, status) {
                    (false, _, _) => "offline".to_string(),
                    (true, false, _) => "Update Kin on your phone".to_string(),
                    (true, true, None) => "waiting for the phone".to_string(),
                    (true, true, Some(s)) if s.access == wire::Access::Missing => "notification access is off".to_string(),
                    (true, true, Some(s)) if !s.enabled => "mirroring is off on the phone".to_string(),
                    (true, true, Some(s)) => format!("mirroring {} app{}", s.apps, if s.apps == 1 { "" } else { "s" }),
                };
                json!({ "name": d.name, "status": text })
            })
            .collect()
    }

    /// The "Phone notifications" part of the settings window and of `status.json`.
    pub fn mirror_view(&self) -> Value {
        let s = lock(&self.settings).clone();
        json!({
            "enabled": s.mirror_notifications,
            "sound": s.mirror_sound,
            "hideContent": s.mirror_hide_content,
            "dismissToPhone": s.mirror_dismiss_to_phone,
            "textOnly": s.mirror_text_only,
            "running": self.mirror.has_surface(),
            "activator": self.mirror.activator_ok(),
            "unavailable": self.mirror.unavailable(),
            "phones": self.mirror_phone_lines(),
        })
    }

    // ----- for tests ------------------------------------------------------------------------------------------------------

    /// For tests: the switch is on and a toast layer of the test's own is in place (no Windows, no registry, no COM class).
    #[doc(hidden)]
    pub fn mirror_for_tests(&self, surface: Arc<dyn ToastSurface>) {
        lock(&self.settings).mirror_notifications = true;
        self.mirror.set_surface(Some(surface));
        self.mirror.set_activator_ok(true);
    }

    /// For tests: the numbers that `status.json` shows.
    #[doc(hidden)]
    pub fn mirror_counters(&self) -> Counters {
        self.mirror.counters()
    }

    /// For tests: how many toasts of a phone the ledger says are on screen.
    #[doc(hidden)]
    pub fn mirror_on_screen(&self, device_id: &str) -> usize {
        self.mirror.on_screen(device_id)
    }

    /// For tests: the folder of the icon cache.
    #[doc(hidden)]
    pub fn mirror_icon_dir(&self) -> std::path::PathBuf {
        self.mirror.icons().dir().to_path_buf()
    }

    /// For tests: presses a button for this device ID the way a click does, and says how many phones the ledger knows afterwards.
    #[doc(hidden)]
    pub async fn mirror_press_for_tests(&self, device_id: &str, nid: &str) -> usize {
        let action = PressedAction { nid: nid.to_string(), i: 0, label: "Open".to_string(), text: None };
        self.mirror_send_action(device_id, "a phone", action).await;
        self.mirror.state().phones.len()
    }

    /// For tests: forget the ledger (what a restart of Kin does), leaving the toasts that Windows holds.
    #[doc(hidden)]
    pub fn mirror_forget_ledger_for_tests(&self) {
        self.mirror.forget_all();
    }

    // ----- developer commands -----------------------------------------------------------------------------------------

    /// `--dev-toast reply|copy|button|dismiss <nid> ...`: the same functions a click and the X call, without Windows. Refused unless the app was started with a
    /// developer setting. The outcome is in the log (never the text).
    pub async fn mirror_dev_toast(self: &Arc<Self>, action: &str, nid: &str, arg: Option<String>) {
        if !self.config.dev_hooks_enabled() {
            log::warn!("refused --dev-toast: it works only when the app was started with a developer setting");
            return;
        }
        let Some((_, short, entry)) = self.mirror.find(nid) else {
            log::warn!("--dev-toast: no phone notification with that nid is on screen");
            return;
        };
        let activation = |action: ToastAction, index: Option<u8>, inputs: Vec<(String, String)>| Activation {
            source: Source::Dev,
            arguments: ToastArgs::new(&short, nid, action, index).to_arguments(),
            inputs,
        };
        match action {
            "reply" => {
                let Some((i, _)) = entry.offered.reply else {
                    log::warn!("--dev-toast reply: that notification offers no reply");
                    return;
                };
                self.mirror_activation(activation(ToastAction::Reply, Some(i), vec![("reply".to_string(), arg.unwrap_or_default())])).await;
            }
            "copy" => self.mirror_activation(activation(ToastAction::Copy, None, vec![])).await,
            "button" => {
                let i = arg.and_then(|a| a.parse::<u8>().ok()).unwrap_or(0);
                self.mirror_activation(activation(ToastAction::Button, Some(i), vec![])).await;
            }
            "dismiss" => {
                let group = args::group_of(&short);
                self.mirror_surface_event(SurfaceEvent::Dismissed {
                    tag: nid.to_string(),
                    group,
                    generation: entry.generation,
                    reason: DismissReason::UserCanceled,
                })
                .await;
            }
            other => log::warn!("--dev-toast: no action called {other:?} (reply, copy, button, dismiss)"),
        }
    }
}

/// The 8 characters of a device ID that a log line shows.
fn short_prefix(device_id: &str) -> &str {
    short(device_id)
}

/// The text a reply click carries, trimmed (empty when there is none).
fn reply_text(activation: &Activation) -> String {
    activation.inputs.iter().find(|(k, _)| k == "reply").map(|(_, v)| crate::links::trim(v).to_string()).unwrap_or_default()
}

/// What the user is told when the phone answered a `notif.action` with an `error`.
fn failure_text(code: &str, phone: &str) -> String {
    match code {
        "stale" => format!("The notification changed on {phone}. Nothing was sent."),
        "not_found" => format!("That notification is gone from {phone}."),
        "disabled" => format!("Notification mirroring is turned off on {phone}."),
        "action_failed" => format!("{phone} couldn't do that. The app may have closed the notification."),
        "rate_limited" => "That is too fast. Wait a few seconds and try again.".to_string(),
        _ => format!("{phone} refused it ({code})."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_short_id_of_a_device_is_the_base32_of_the_hash_its_id_is_the_base64url_of() {
        let hash = [7u8; 32];
        let id = encoding::device_id_from_hash(&hash);
        assert_eq!(short_of(&id), Some(encoding::short_id_from_hash(&hash)));
        assert_eq!(short_of("not a device id"), None);
        assert_eq!(short_of(&id[..40]), None);
    }
}
