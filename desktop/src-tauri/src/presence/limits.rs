//! When a `presence` goes out and how many a side takes in (design 3.4). Pure, with the time given as an argument, so the vectors
//! (`protocol/presence.json` `presenceRate[]`) and the tests drive them with a fake clock.
//!
//! * **The sender** sends a whole snapshot (a `presence` is the full current state) right after both `hello`s, and again when something in it changes:
//!   a battery change when the level moved by at least 2 points from the last **sent** one, crossed 15, 20 or 100, or when `charging` or `saver` flipped,
//!   and **at most once per 30 s** (trailing); a pause change **within 1 s** (trailing), never held back by the battery limit; and a one-shot at the moment
//!   a pause ends. Never periodically.
//! * **The receiver** takes at most 10 in a burst and one more per 3 s, and answers the excess with one `rate_limited` per 10 s.
use super::pause::PauseState;
use super::wire::{Battery, PeerPause, PresenceBody};

pub const BATTERY_MIN_STEP: u8 = 2;
pub const BATTERY_CROSSINGS: [u8; 3] = [15, 20, 100];
pub const BATTERY_EVERY_MS: i64 = 30_000;
pub const PAUSE_TRAILING_MS: i64 = 1_000;

pub const RECEIVER_CAPACITY: i64 = 10;
pub const RECEIVER_REFILL_MS: i64 = 3_000;
pub const RECEIVER_ERROR_SPACING_MS: i64 = 10_000;

/// One snapshot that goes on the wire, at time `t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Send {
    pub t: i64,
    pub body: PresenceBody,
}

/// What identifies a pause for "did it change since the last send": its end, not the milliseconds left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PauseKey {
    None,
    UntilResumed,
    Until(i64),
}

fn key_of(pause: Option<PauseState>, now: i64) -> PauseKey {
    match pause {
        None => PauseKey::None,
        Some(PauseState::UntilResumed) => PauseKey::UntilResumed,
        Some(p @ PauseState::Until(until)) => {
            if p.is_on(now) {
                PauseKey::Until(until)
            } else {
                PauseKey::None
            }
        }
    }
}

fn crosses(a: u8, b: u8) -> bool {
    BATTERY_CROSSINGS.iter().any(|x| (a >= *x) != (b >= *x))
}

/// Whether the battery of a snapshot differs enough from the last sent one to be worth a message.
fn significant(last: Option<Battery>, now: Option<Battery>) -> bool {
    match (last, now) {
        (None, None) => false,
        (Some(_), None) | (None, Some(_)) => true,
        (Some(a), Some(b)) => {
            a.level.abs_diff(b.level) >= BATTERY_MIN_STEP
                || crosses(a.level, b.level)
                || a.charging != b.charging
                || a.saver.unwrap_or(false) != b.saver.unwrap_or(false)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    Expire,
    Pause,
    Battery,
}

#[derive(Debug, Default)]
pub struct PresenceSender {
    connected: bool,
    battery: Option<Battery>,
    pause: Option<PauseState>,
    /// Changelog 59: whether this PC is in Focus (the stable value of the 3 s filter). What goes on the wire is this **and not paused** (a paused PC asks not to be
    /// acted on, so it puts no `focus` in its snapshot).
    focus: bool,
    last_battery: Option<Battery>,
    last_pause: Option<PauseKey>,
    last_focus: bool,
    last_sent: Option<i64>,
    due_battery: Option<i64>,
    due_pause: Option<i64>,
    due_expire: Option<i64>,
}

impl PresenceSender {
    pub fn new() -> PresenceSender {
        PresenceSender::default()
    }

    /// `focus` as it would be sent at `now`: only while not paused.
    fn effective_focus(&self, now: i64) -> bool {
        self.focus && !self.pause.is_some_and(|p| p.is_on(now))
    }

    /// Whether what a pause or a focus change affects differs from what was last sent.
    fn pause_or_focus_changed(&self, at: i64) -> bool {
        Some(key_of(self.pause, at)) != self.last_pause || self.effective_focus(at) != self.last_focus
    }

    fn body(&self, now: i64) -> PresenceBody {
        PresenceBody { battery: self.battery, pause: self.pause.and_then(|p| p.to_wire(now)), dnd: None, focus: self.effective_focus(now) }
    }

    fn send(&mut self, now: i64) -> Send {
        let body = self.body(now);
        self.last_battery = self.battery;
        self.last_pause = Some(key_of(self.pause, now));
        self.last_focus = body.focus;
        self.last_sent = Some(now);
        self.due_battery = None;
        self.due_pause = None;
        Send { t: now, body }
    }

    /// When the next timer is due (a trailing send, or the end of a pause), if one is armed.
    pub fn next_due(&self) -> Option<i64> {
        [self.due_expire, self.due_pause, self.due_battery].into_iter().flatten().min()
    }

    /// Fires every timer that is due at `now` or earlier, in order.
    pub fn advance(&mut self, now: i64) -> Vec<Send> {
        let mut out = Vec::new();
        loop {
            let mut dues: Vec<(i64, Due)> = Vec::new();
            for (at, kind) in [(self.due_expire, Due::Expire), (self.due_pause, Due::Pause), (self.due_battery, Due::Battery)] {
                if let Some(at) = at.filter(|at| *at <= now) {
                    dues.push((at, kind));
                }
            }
            let Some((at, kind)) = dues.into_iter().min_by_key(|(at, kind)| (*at, *kind as u8)) else {
                return out;
            };
            match kind {
                Due::Expire => {
                    self.due_expire = None;
                    if self.connected && self.pause_or_focus_changed(at) {
                        out.push(self.send(at));
                    }
                }
                Due::Pause => {
                    self.due_pause = None;
                    if self.connected && self.pause_or_focus_changed(at) {
                        out.push(self.send(at));
                    }
                }
                Due::Battery => {
                    self.due_battery = None;
                    if self.connected && significant(self.last_battery, self.battery) {
                        out.push(self.send(at));
                    }
                }
            }
        }
    }

    /// Both `hello`s are accepted: the snapshot goes at once (an empty one tells the peer "not paused").
    pub fn connected(&mut self, now: i64) -> Vec<Send> {
        let mut out = self.advance(now);
        self.connected = true;
        out.push(self.send(now));
        out
    }

    /// The connection ended: nothing is armed any more, and the next one starts again with a snapshot.
    pub fn disconnected(&mut self) {
        self.connected = false;
        self.due_battery = None;
        self.due_pause = None;
        self.last_battery = None;
        self.last_pause = None;
        self.last_focus = false;
        self.last_sent = None;
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// The battery changed (`None`: it is not shared, or this PC has none).
    pub fn set_battery(&mut self, now: i64, battery: Option<Battery>) -> Vec<Send> {
        let mut out = self.advance(now);
        self.battery = battery;
        if self.connected && significant(self.last_battery, self.battery) {
            let ready = self.last_sent.is_none_or(|t| now - t >= BATTERY_EVERY_MS);
            if ready {
                out.push(self.send(now));
            } else if self.due_battery.is_none() {
                self.due_battery = self.last_sent.map(|t| t + BATTERY_EVERY_MS);
            }
        }
        out
    }

    /// The user switched sharing on or off: that is the user's own act, not a stream, so it goes **at once** (a pending trailing send is used up by it) instead of
    /// waiting out the 30 s: a PC that has just been told to stop sharing its battery must not go on showing it for half a minute.
    pub fn set_battery_now(&mut self, now: i64, battery: Option<Battery>) -> Vec<Send> {
        let mut out = self.advance(now);
        self.battery = battery;
        if self.connected && significant(self.last_battery, self.battery) {
            out.push(self.send(now));
        }
        out
    }

    /// This PC's Focus changed (the stable value of the 3 s filter). Like a pause it goes **within 1 s** (trailing) and is never held back by the battery limit.
    pub fn set_focus(&mut self, now: i64, focus: bool) -> Vec<Send> {
        let out = self.advance(now);
        self.focus = focus;
        if self.connected && self.due_pause.is_none() && self.pause_or_focus_changed(now) {
            self.due_pause = Some(now + PAUSE_TRAILING_MS);
        }
        out
    }

    /// A clean quit: the last snapshot, at once, with **no `focus`** (the phone lets go of its silence without waiting for its 60 s). `None` if nothing is connected.
    pub fn quit_snapshot(&mut self, now: i64) -> Option<Send> {
        if !self.connected {
            return None;
        }
        self.focus = false;
        Some(self.send(now))
    }

    /// The pause changed (`None`: resumed). It goes within 1 s, and a one-shot is armed for the moment it ends.
    pub fn set_pause(&mut self, now: i64, pause: Option<PauseState>) -> Vec<Send> {
        let out = self.advance(now);
        self.pause = pause;
        self.due_expire = match pause {
            Some(PauseState::Until(until)) => Some(until),
            _ => None,
        };
        if self.connected && self.due_pause.is_none() {
            self.due_pause = Some(now + PAUSE_TRAILING_MS);
        }
        out
    }
}

/// The receiver's bucket per connection: capacity 10, one token back per 3 s. Counted in 1/3000 of a token (one unit per millisecond), so nothing rounds.
#[derive(Debug)]
pub struct PresenceBucket {
    tokens: i64,
    last: Option<i64>,
    last_error: Option<i64>,
}

/// What the bucket says about one arriving `presence`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    Accepted,
    /// Dropped and counted; `error`: answer with one `rate_limited` (no `ref`).
    Dropped {
        error: bool,
    },
}

impl Default for PresenceBucket {
    fn default() -> Self {
        PresenceBucket::new()
    }
}

impl PresenceBucket {
    pub fn new() -> PresenceBucket {
        PresenceBucket { tokens: RECEIVER_CAPACITY * RECEIVER_REFILL_MS, last: None, last_error: None }
    }

    pub fn take(&mut self, now: i64) -> Taken {
        let cap = RECEIVER_CAPACITY * RECEIVER_REFILL_MS;
        if let Some(last) = self.last {
            self.tokens = (self.tokens + (now - last).max(0)).min(cap);
        }
        self.last = Some(now);
        if self.tokens >= RECEIVER_REFILL_MS {
            self.tokens -= RECEIVER_REFILL_MS;
            return Taken::Accepted;
        }
        let error = self.last_error.is_none_or(|e| now - e >= RECEIVER_ERROR_SPACING_MS);
        if error {
            self.last_error = Some(now);
        }
        Taken::Dropped { error }
    }
}

/// What a received `presence` says about the pause, as a view stamped with its arrival (the receiver's monotonic clock).
pub fn peer_pause_of(body: &PresenceBody, arrival_ms: i64) -> Option<super::pause::PeerPauseView> {
    body.pause.map(|p: PeerPause| super::pause::PeerPauseView::new(p, arrival_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(level: u8, charging: bool) -> Option<Battery> {
        Some(Battery { level, charging, saver: None })
    }

    #[test]
    fn a_snapshot_goes_after_the_hello_even_with_nothing_in_it() {
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(62, true));
        let sent = s.connected(100);
        assert_eq!(sent, [Send { t: 100, body: PresenceBody { battery: bat(62, true), ..Default::default() } }]);
        let mut s = PresenceSender::new();
        assert_eq!(s.connected(0), [Send { t: 0, body: PresenceBody::default() }]);
    }

    #[test]
    fn the_battery_goes_at_most_every_30_s_and_the_latest_value_wins() {
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(80, false));
        s.connected(0);
        assert!(s.set_battery(1000, bat(78, false)).is_empty());
        assert!(s.set_battery(5000, bat(76, false)).is_empty());
        assert_eq!(s.next_due(), Some(30_000));
        let sent = s.advance(40_000);
        assert_eq!(sent, [Send { t: 30_000, body: PresenceBody { battery: bat(76, false), ..Default::default() } }]);
        // Back to the sent value inside the window: nothing goes at all.
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(80, false));
        s.connected(0);
        s.set_battery(1000, bat(78, false));
        s.set_battery(2000, bat(80, false));
        assert!(s.advance(40_000).is_empty());
    }

    #[test]
    fn at_exactly_30_s_it_goes_at_once_and_at_29999_it_waits() {
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(80, false));
        s.connected(0);
        assert_eq!(s.set_battery(30_000, bat(78, false)).len(), 1);
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(80, false));
        s.connected(0);
        assert!(s.set_battery(29_999, bat(78, false)).is_empty());
        assert_eq!(s.advance(30_000)[0].t, 30_000);
    }

    #[test]
    fn crossing_15_20_or_100_counts_but_16_to_15_does_not() {
        let go = |from: u8, to: u8| {
            let mut s = PresenceSender::new();
            s.set_battery(0, bat(from, false));
            s.connected(0);
            s.set_battery(31_000, bat(to, false)).len() == 1
        };
        assert!(go(15, 14) && go(19, 20) && go(99, 100));
        assert!(!go(16, 15) && !go(50, 49));
        assert!(go(50, 48), "two points");
    }

    #[test]
    fn a_pause_goes_within_a_second_ends_with_a_one_shot_and_is_not_held_by_the_battery_limit() {
        let mut s = PresenceSender::new();
        s.connected(0);
        assert!(s.set_pause(5000, Some(PauseState::Until(3_605_000))).is_empty());
        let sent = s.advance(7000);
        assert_eq!(sent, [Send { t: 6000, body: PresenceBody { pause: Some(PeerPause::Remaining(3_599_000)), ..Default::default() } }]);
        assert_eq!(s.next_due(), Some(3_605_000), "the one-shot at the end of the pause");
        let sent = s.advance(3_606_000);
        assert_eq!(sent, [Send { t: 3_605_000, body: PresenceBody::default() }]);
        // A pause that came and went inside the second sends nothing.
        let mut s = PresenceSender::new();
        s.connected(0);
        s.set_pause(5000, Some(PauseState::Until(3_605_000)));
        s.set_pause(5400, None);
        assert!(s.advance(9000).is_empty());
        // A held battery change rides along on a pause send.
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(80, false));
        s.connected(0);
        s.set_battery(1000, bat(78, false));
        s.set_pause(2000, Some(PauseState::UntilResumed));
        let sent = s.advance(40_000);
        assert_eq!(
            sent,
            [Send { t: 3000, body: PresenceBody { battery: bat(78, false), pause: Some(PeerPause::UntilResumed), ..Default::default() } }]
        );
    }

    #[test]
    fn nothing_goes_before_the_hello_and_nothing_is_armed_after_the_connection_ends() {
        let mut s = PresenceSender::new();
        assert!(s.set_battery(0, bat(50, false)).is_empty());
        assert!(s.set_pause(10, Some(PauseState::UntilResumed)).is_empty());
        assert_eq!(s.next_due(), None, "no timer runs for a connection that isn't there");
        let sent = s.connected(1000);
        assert_eq!(sent.len(), 1);
        s.disconnected();
        assert!(!s.is_connected());
        assert!(s.set_battery(5000, bat(10, false)).is_empty());
    }

    /// 2B.5: `focus` is in the first snapshot, a change goes within 1 s (never held back by the battery rule) and a paused PC puts none in.
    #[test]
    fn focus_goes_in_the_snapshot_within_a_second_and_never_while_paused() {
        let mut s = PresenceSender::new();
        s.set_focus(0, true);
        let first = s.connected(100);
        assert_eq!(first.len(), 1);
        assert!(first[0].body.focus, "a phone that connects while the PC is in Focus hears it at once");
        // Focus ends: one snapshot within 1 s, without focus.
        assert!(s.set_focus(10_000, false).is_empty());
        assert_eq!(s.next_due(), Some(11_000));
        let sent = s.advance(11_000);
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].body.focus && sent[0].t == 11_000);
        // Back in Focus, then the PC pauses: the pause goes out without focus, resuming brings it back.
        s.set_focus(20_000, true);
        assert!(s.advance(21_000)[0].body.focus);
        s.set_pause(30_000, Some(PauseState::UntilResumed));
        let paused = s.advance(31_000);
        assert_eq!(paused.len(), 1);
        assert!(!paused[0].body.focus && paused[0].body.pause.is_some(), "a paused PC asks not to be acted on: no focus");
        s.set_pause(40_000, None);
        let resumed = s.advance(41_000);
        assert!(resumed[0].body.focus && resumed[0].body.pause.is_none());
        // Focus flipping while paused changes nothing on the wire.
        s.set_pause(50_000, Some(PauseState::UntilResumed));
        s.advance(51_000);
        assert!(s.set_focus(60_000, false).is_empty());
        assert_eq!(s.next_due(), None, "nothing to send: the snapshot would be the same");
    }

    /// Round 3: the pump tells every sender the stable value on every turn. An unchanged value does nothing, and a sender that was created with a stale `true` (a reconnect that
    /// raced the pump) is armed to say the truth within a second.
    #[test]
    fn telling_a_sender_the_same_focus_again_does_nothing_and_a_stale_one_heals() {
        let mut s = PresenceSender::new();
        s.set_focus(0, false);
        s.connected(0);
        for t in [1_000, 2_000, 3_000] {
            assert!(s.set_focus(t, false).is_empty());
            assert_eq!(s.next_due(), None, "the same value again arms nothing");
        }
        // The connection was created with `focus: true` while the stable value is false.
        let mut stale = PresenceSender::new();
        stale.set_focus(0, true);
        assert!(stale.connected(0)[0].body.focus);
        assert!(stale.set_focus(5_000, false).is_empty());
        assert_eq!(stale.next_due(), Some(6_000));
        let healed = stale.advance(6_000);
        assert_eq!(healed.len(), 1);
        assert!(!healed[0].body.focus, "the phone hears that Focus is over");
        // And once healed, telling it again is a no-op.
        assert!(stale.set_focus(7_000, false).is_empty());
        assert_eq!(stale.next_due(), None);
    }

    #[test]
    fn a_clean_quit_sends_one_snapshot_without_focus_at_once() {
        let mut s = PresenceSender::new();
        assert_eq!(s.quit_snapshot(0), None, "nothing is connected");
        s.set_battery(0, bat(70, false));
        s.set_focus(0, true);
        assert!(s.connected(100)[0].body.focus);
        let last = s.quit_snapshot(200).expect("a snapshot");
        assert!(!last.body.focus && last.body.battery == bat(70, false) && last.t == 200, "{last:?}");
        assert_eq!(s.next_due(), None);
    }

    #[test]
    fn a_focus_change_is_not_held_back_by_the_battery_limit() {
        let mut s = PresenceSender::new();
        s.set_battery(0, bat(60, false));
        s.connected(0);
        // A battery change inside the 30 s is held; a focus change right after it is not.
        s.set_battery(1_000, bat(50, false));
        assert_eq!(s.next_due(), Some(30_000));
        s.set_focus(2_000, true);
        let sent = s.advance(3_000);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].body.focus);
    }

    #[test]
    fn the_receiver_takes_ten_then_one_per_3_s_and_answers_one_error_per_10_s() {
        let mut b = PresenceBucket::new();
        let results: Vec<Taken> = (0..12).map(|_| b.take(0)).collect();
        assert_eq!(results.iter().filter(|t| **t == Taken::Accepted).count(), 10);
        assert_eq!(results[10], Taken::Dropped { error: true });
        assert_eq!(results[11], Taken::Dropped { error: false }, "one error per 10 s");
        assert_eq!(b.take(2999), Taken::Dropped { error: false });
        assert_eq!(b.take(3000), Taken::Accepted);
        let mut b = PresenceBucket::new();
        for _ in 0..20 {
            b.take(0);
        }
        let at_9999: Vec<Taken> = (0..5).map(|_| b.take(9999)).collect();
        assert_eq!(at_9999.iter().filter(|t| **t == Taken::Accepted).count(), 3);
        assert!(at_9999.iter().all(|t| *t != Taken::Dropped { error: true }));
        assert_eq!(b.take(10_000), Taken::Dropped { error: true }, "the next error after 10 s");
    }
}
