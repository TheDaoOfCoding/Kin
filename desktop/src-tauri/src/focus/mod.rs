//! Focus and Do Not Disturb on the PC (milestone 2B.5, protocol.md "Do Not Disturb, Focus and alerts", design section 5): **tell the phone when this PC is in Focus** and
//! **hold Kin's own optional toasts while the phone is in Do Not Disturb**.
//!
//! * [`state`]: the rule "the PC is in Focus" and the 3 s stability filter. Pure. [`hold`]: which toasts are held. Pure. [`source`]: the real, **read-only** source
//!   (`FocusSessionManager` and `SHQueryUserNotificationState`).
//! * This file is the core's side: the settings, the source that exists **only while the feature is on and a phone that lists `presence` is connected** (no timer and no
//!   registered event otherwise), the read on the event, on every 15 s poll tick and when a phone connects, and what the presence pump does with a change.
//!
//! **Kin never changes Windows Focus or quiet time.** No code path calls the two calls that start or end a Focus session (a Limited Access Feature), and nothing writes a
//! Windows setting. Developer runs use `--dev-focus on|off|auto` (or `--dev-set devFocus`): a fake source, so a test never touches the user's real setting.
//!
//! **What is never stored or logged:** a history of Focus or Do Not Disturb. The log has the words `focus: on (focus session)` and `focus: off`; the phone's `dnd` is
//! memory only and forgotten when its connection closes.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub mod hold;
pub mod source;
pub mod state;

use crate::core::{Core, Event, lock};
use crate::features;
use crate::presence::wire::Dnd;
use crate::quiet::UserState;
use hold::ToastKind;
use state::{FakeQuiet, POLL_MS, QuietSource, StableFocus, is_focus, reason_word};

/// `--dev-focus on|off|auto`: what replaces the real source in a developer run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevFocus {
    /// The real, read-only source (the default).
    Auto,
    /// A fake that says "in Focus".
    On,
    /// A fake that says "not in Focus".
    Off,
}

impl DevFocus {
    pub fn parse(word: &str) -> Option<DevFocus> {
        match word {
            "auto" => Some(DevFocus::Auto),
            "on" => Some(DevFocus::On),
            "off" => Some(DevFocus::Off),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            DevFocus::Auto => "auto",
            DevFocus::On => "on",
            DevFocus::Off => "off",
        }
    }
}

#[derive(Default)]
struct Inner {
    filter: StableFocus,
    /// Exists only while the feature is on and a phone that lists `presence` is connected.
    source: Option<Arc<dyn QuietSource>>,
    next_poll_ms: Option<i64>,
    /// Why the last reading said Focus (a word, for the log when the stable value changes).
    reason: &'static str,
    /// What the source said about `FocusSessionManager` (`None`: never read).
    sessions_supported: Option<bool>,
}

/// The Focus state of this PC, in the presence module's `Presence`.
pub struct FocusRuntime {
    inner: Mutex<Inner>,
    dev: Mutex<Option<DevFocus>>,
    /// A test's own source (instead of the real or the dev fake).
    forced: Mutex<Option<Arc<dyn QuietSource>>>,
    /// Set by the Windows event: the pump reads at its next turn. (The handler decides nothing.)
    dirty: Arc<AtomicBool>,
    reads: AtomicU64,
    changes: AtomicU64,
    held_toasts: AtomicU64,
}

impl FocusRuntime {
    pub fn new() -> FocusRuntime {
        FocusRuntime {
            inner: Mutex::new(Inner::default()),
            dev: Mutex::new(None),
            forced: Mutex::new(None),
            dirty: Arc::new(AtomicBool::new(false)),
            reads: AtomicU64::new(0),
            changes: AtomicU64::new(0),
            held_toasts: AtomicU64::new(0),
        }
    }

    /// `--dev-focus` at start.
    pub fn set_dev(&self, mode: DevFocus) {
        *lock(&self.dev) = Some(mode);
    }

    /// The stable value (what goes into a snapshot, if the other conditions hold).
    pub fn stable(&self) -> bool {
        lock(&self.inner).filter.stable()
    }

    /// Whether the source exists now (the feature is on and a phone is connected): for the tests of "nothing is registered otherwise".
    pub fn has_source(&self) -> bool {
        lock(&self.inner).source.is_some()
    }

    pub fn held_toasts(&self) -> u64 {
        self.held_toasts.load(Ordering::Relaxed)
    }
}

impl Default for FocusRuntime {
    fn default() -> Self {
        FocusRuntime::new()
    }
}

impl Core {
    // ----- the switches --------------------------------------------------------------------------------------------------

    fn set_focus_switch(self: &Arc<Self>, name: &str, on: bool, field: impl FnOnce(&mut crate::settings::Settings) -> &mut bool) {
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
        // The pump looks at the new setting at once: a source is made or dropped, and a snapshot that no longer (or now does) carry `focus` goes out.
        self.ensure_presence_pump();
        self.presence.wake();
        self.emit(Event::StateChanged);
    }

    /// "Silence my phone while this PC is in Focus" (off by default). Turning it on only starts reading; turning it off drops the source and takes `focus` out of the
    /// snapshot at once.
    pub fn set_focus_share(self: &Arc<Self>, on: bool) {
        self.set_focus_switch("focusShare", on, |s| &mut s.focus_share);
        if !on {
            self.focus_tick(crate::encoding::unix_ms() as i64);
            // The sender armed a send within a second: the pump comes back for it.
            self.presence.wake();
        }
    }

    /// "Also when I'm presenting or playing full screen" (off by default).
    pub fn set_focus_also_presenting(self: &Arc<Self>, on: bool) {
        self.set_focus_switch("focusAlsoPresenting", on, |s| &mut s.focus_also_presenting);
    }

    /// "Hold Kin's own toasts while my phone is in Do Not Disturb" (on by default).
    pub fn set_hold_toasts_on_phone_dnd(self: &Arc<Self>, on: bool) {
        self.set_focus_switch("holdToastsOnPhoneDnd", on, |s| &mut s.hold_toasts_on_phone_dnd);
    }

    /// `--dev-focus` / `--dev-set devFocus`: replaces the real source with a fake (`on`, `off`) or puts the real, read-only one back (`auto`).
    pub fn dev_set_focus(self: &Arc<Self>, mode: DevFocus) {
        *lock(&self.presence.focus.dev) = Some(mode);
        lock(&self.presence.focus.inner).source = None;
        log::info!("developer setting: the Focus source is {}", mode.word());
        self.ensure_presence_pump();
        self.presence.wake();
        self.emit(Event::StateChanged);
    }

    /// For tests: this source instead of any other (the next read builds nothing).
    #[doc(hidden)]
    pub fn set_quiet_source(&self, source: Arc<dyn QuietSource>) {
        *lock(&self.presence.focus.forced) = Some(source);
        lock(&self.presence.focus.inner).source = None;
    }

    /// Whether the Focus source exists now: only while the feature is on and a phone that lists `presence` is connected (nothing is registered or read otherwise).
    pub fn focus_source_active(&self) -> bool {
        self.presence.focus.has_source()
    }

    /// What the Windows event does: the pump reads at its next turn instead of waiting for the 15 s tick. (It decides nothing: the reading, the 3 s filter and the send
    /// are the pump's.) A test calls it after changing its fake source.
    #[doc(hidden)]
    pub fn focus_event(&self) {
        self.presence.focus.dirty.store(true, Ordering::SeqCst);
        self.presence.wake();
    }

    fn focus_make_source(&self) -> Arc<dyn QuietSource> {
        if let Some(s) = lock(&self.presence.focus.forced).clone() {
            return s;
        }
        match *lock(&self.presence.focus.dev) {
            Some(DevFocus::On) => Arc::new(FakeQuiet::new(true, UserState::AcceptsNotifications)),
            Some(DevFocus::Off) => Arc::new(FakeQuiet::new(false, UserState::AcceptsNotifications)),
            Some(DevFocus::Auto) | None => {
                let dirty = self.presence.focus.dirty.clone();
                let wake = self.presence.wake_handle();
                Arc::new(source::WindowsQuiet::new(Arc::new(move || {
                    dirty.store(true, Ordering::SeqCst);
                    wake.notify_one();
                })))
            }
        }
    }

    // ----- the pump's part -----------------------------------------------------------------------------------------------

    /// Called by the presence pump on every turn. While the feature is on **and** a phone that lists `presence` is connected: reads on the event, on each 15 s tick and
    /// the first time, runs the 3 s filter and tells the senders when the stable value changed. Otherwise nothing is registered, read or polled, and a value that was
    /// on is taken out of the snapshots. Returns when it wants to be called next.
    ///
    /// **The two rules that keep a phone from being left silent (2B.5 review, round 3):**
    /// * the phones are counted **while the Focus lock is held**, so a `hello` that has just seeded the filter can't be undone by a count made before it;
    /// * every turn tells every connection's sender the stable value, not only a change (`PresenceSender::set_focus` ignores an unchanged value), so a connection that was
    ///   created with a value this side no longer holds (a reconnect racing the pump) hears the truth within a second.
    ///
    /// Windows is read (up to 2 s) and a dropped source is dropped (its worker stops) **outside** the lock, so the status window and a `hello` never wait for either.
    pub(crate) fn focus_tick(self: &Arc<Self>, now: i64) -> Option<i64> {
        let share = self.feature_enabled(features::FOCUS_SHARE, None);
        let also = lock(&self.settings).focus_also_presenting;
        let rt = &self.presence.focus;
        let mut change: Option<(bool, &'static str)> = None;
        let mut dropped: Option<Arc<dyn QuietSource>> = None;
        let mut dropped_source = false;
        // 1. Under the lock, without touching Windows: is this turn active, which source, is a reading due.
        let (current, read_due) = {
            let mut inner = lock(&rt.inner);
            if !(share && self.presence_connection_count() > 0) {
                let was = inner.filter.stable();
                dropped = inner.source.take();
                dropped_source = dropped.is_some();
                inner.filter.reset();
                inner.next_poll_ms = None;
                if was {
                    change = Some((false, "off"));
                }
                (None, false)
            } else {
                let dirty = rt.dirty.swap(false, Ordering::SeqCst);
                let poll_due = inner.next_poll_ms.unwrap_or(now);
                (Some(inner.source.clone()), dirty || now >= poll_due)
            }
        };
        drop(dropped.take());
        let mut due = None;
        let mut stable = false;
        if let Some(existing) = current {
            // 2. Windows, with no lock held.
            let made = existing.is_none();
            let source = existing.unwrap_or_else(|| self.focus_make_source());
            let reading = (made || read_due).then(|| {
                let session = source.focus_session();
                let state = source.quiet_state();
                (session, state, source.focus_sessions_supported())
            });
            // 3. Under the lock again: the phones are counted here (a phone may have gone or come meanwhile), and the reading goes into the filter.
            let mut spare: Option<Arc<dyn QuietSource>> = None;
            {
                let mut inner = lock(&rt.inner);
                if self.presence_connection_count() == 0 {
                    let was = inner.filter.stable();
                    spare = inner.source.take();
                    dropped_source = dropped_source || spare.is_some();
                    inner.filter.reset();
                    inner.next_poll_ms = None;
                    if was {
                        change = Some((false, "off"));
                    }
                    if made {
                        // The source built for this turn is not kept: nothing is read while no phone is connected.
                        spare = Some(source);
                    }
                } else {
                    let installed = made && inner.source.is_none();
                    if installed {
                        inner.source = Some(source.clone());
                    } else if made {
                        // A `hello` made one first: use that one, drop this one (below, outside the lock).
                        spare = Some(source.clone());
                    }
                    let usable = inner.source.is_some();
                    match reading {
                        Some((session, state, supported)) if usable => {
                            inner.sessions_supported = Some(supported);
                            rt.reads.fetch_add(1, Ordering::Relaxed);
                            inner.next_poll_ms = Some(now + POLL_MS);
                            inner.reason = reason_word(session, state, also);
                            let raw = is_focus(session, state, also);
                            let before = inner.filter.stable();
                            if installed {
                                // The first reading after nothing was being read: there is no flapping history to filter (it is what a connecting phone gets too).
                                inner.filter.seed(raw);
                                if raw != before {
                                    change = Some((raw, if raw { inner.reason } else { "off" }));
                                }
                            } else if let Some((_, value)) = inner.filter.feed(now, raw) {
                                change = Some((value, if value { inner.reason } else { "off" }));
                            }
                        }
                        _ => {
                            if let Some((_, value)) = inner.filter.advance(now) {
                                change = Some((value, if value { inner.reason } else { "off" }));
                            }
                        }
                    }
                    due = if inner.source.is_none() {
                        // Replaced (a test's or a developer's source) while Windows was being read: the next turn makes the new one.
                        Some(now + 1)
                    } else {
                        [inner.next_poll_ms, inner.filter.next_due()].into_iter().flatten().min()
                    };
                    stable = inner.filter.stable();
                }
            }
            drop(spare);
        }
        if let Some((on, why)) = change {
            rt.changes.fetch_add(1, Ordering::Relaxed);
            // The word only: never a time or a history.
            log::info!("focus: {}", if on { format!("on ({why})") } else { "off".to_string() });
            self.emit(Event::StateChanged);
        } else if dropped_source {
            // `status.json` says the source is gone.
            self.emit(Event::StateChanged);
        }
        // Every turn, change or not: a sender that already has this value does nothing, one that has another one is armed to send within a second.
        self.presence_sync_focus(now, stable);
        due
    }

    /// Whether this PC is in Focus as the senders should tell a phone that just connected, only while the feature is on. When nothing was being read (this is the first phone
    /// since the source was dropped) it is read **now** and is the stable value at once (no 3 s to wait: there is no flapping history yet), so the first snapshot of a connection
    /// tells the truth and a reconnect after a Wi-Fi blip does not look to the phone like "left Focus and came back" (its Do Not Disturb would flap).
    ///
    /// Windows is read outside the lock. The value can still be stale by the time the connection exists (the pump may count no phone in between and reset): the pump's next
    /// turn (the `hello` wakes it) tells the connection's sender the stable value, so that never lasts.
    pub(crate) fn focus_for_new_connection(&self, now: i64) -> bool {
        if !self.feature_enabled(features::FOCUS_SHARE, None) {
            return false;
        }
        let also = lock(&self.settings).focus_also_presenting;
        let rt = &self.presence.focus;
        {
            let inner = lock(&rt.inner);
            if inner.source.is_some() {
                return inner.filter.stable();
            }
        }
        let source = self.focus_make_source();
        let supported = source.focus_sessions_supported();
        let (session, state) = (source.focus_session(), source.quiet_state());
        let on = is_focus(session, state, also);
        let mut inner = lock(&rt.inner);
        if inner.source.is_some() {
            // Another turn installed one while Windows was being read: its value is the truth.
            let stable = inner.filter.stable();
            drop(inner);
            drop(source);
            return stable;
        }
        rt.reads.fetch_add(1, Ordering::Relaxed);
        inner.sessions_supported = Some(supported);
        inner.reason = reason_word(session, state, also);
        inner.filter.seed(on);
        inner.next_poll_ms = Some(now + POLL_MS);
        inner.source = Some(source);
        if on {
            log::info!("focus: on ({})", inner.reason);
        }
        inner.filter.stable()
    }

    // ----- Do Not Disturb of the phone: what the PC does with it ------------------------------------------------------------

    /// Whether one of Kin's own optional toasts is held now (design 3.6): the phone is in Do Not Disturb and the setting is on. Counted, never logged with a name.
    pub fn hold_own_toast(&self, kind: ToastKind) -> bool {
        let held = hold::is_held(self.presence_phone_dnd(), self.feature_enabled(features::DND_HOLD_TOASTS, None), kind);
        if held {
            self.presence.focus.held_toasts.fetch_add(1, Ordering::Relaxed);
            log::info!("a toast of Kin's own was held while the phone is in Do Not Disturb");
        }
        held
    }

    // ----- what the windows and status.json show --------------------------------------------------------------------------

    /// The Settings window's part: the switches, and what Windows allows here.
    pub fn focus_panel(&self) -> Value {
        // The settings first, then the Focus state: the two locks are never held together.
        let (share, also, hold) = {
            let s = lock(&self.settings);
            (s.focus_share, s.focus_also_presenting, s.hold_toasts_on_phone_dnd)
        };
        let inner = lock(&self.presence.focus.inner);
        json!({
            "share": share,
            "alsoPresenting": also,
            "holdToasts": hold,
            // `null` until a source has been made (the feature is off, or no phone is connected): the row says nothing about Windows 10 then.
            "sessionsSupported": inner.sessions_supported,
            "on": inner.filter.stable(),
        })
    }

    /// For `status.json`: words and numbers only, no history.
    pub fn focus_status_json(&self) -> Value {
        let rt = &self.presence.focus;
        let (share, also, hold) = {
            let s = lock(&self.settings);
            (s.focus_share, s.focus_also_presenting, s.hold_toasts_on_phone_dnd)
        };
        json!({
            "share": share,
            "alsoPresenting": also,
            "holdToasts": hold,
            "on": rt.stable(),
            "sourceActive": rt.has_source(),
            "dev": lock(&rt.dev).map(|d| d.word()),
            "counts": {
                "reads": rt.reads.load(Ordering::Relaxed),
                "changes": rt.changes.load(Ordering::Relaxed),
                "heldToasts": rt.held_toasts(),
            },
        })
    }
}

/// The word of the phone's filter for the tray and the drop window ("Phone: Do Not Disturb"): the three filters read the same to the person at the PC, except total silence.
pub fn dnd_text(dnd: Dnd) -> &'static str {
    match dnd {
        Dnd::Priority | Dnd::Alarms => "Do Not Disturb",
        Dnd::Total => "Do Not Disturb (total silence)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_words_parse() {
        assert_eq!(DevFocus::parse("on"), Some(DevFocus::On));
        assert_eq!(DevFocus::parse("off"), Some(DevFocus::Off));
        assert_eq!(DevFocus::parse("auto"), Some(DevFocus::Auto));
        assert_eq!(DevFocus::parse("yes"), None);
    }

    #[test]
    fn the_chip_says_total_silence_only_for_total_silence() {
        assert_eq!(dnd_text(Dnd::Priority), "Do Not Disturb");
        assert_eq!(dnd_text(Dnd::Alarms), "Do Not Disturb");
        assert!(dnd_text(Dnd::Total).contains("total silence"));
    }

    #[test]
    fn toast_kinds_are_the_words_of_the_vectors() {
        assert_eq!(ToastKind::from_word("low_battery"), Some(ToastKind::LowBattery));
        assert_eq!(ToastKind::from_word("something_new"), None);
    }
}
