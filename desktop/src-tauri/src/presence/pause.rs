//! Pause Kin (design 3.5, protocol.md "Pause: what stops, what still arrives"): this PC's own persisted state, how a choice becomes a time, what the peer's
//! pause means here, and the gate table. All pure; the zone of the clock is a trait, so a test can use any zone (`protocol/presence.json`: `pauseUntil[]`,
//! `pauseGate[]`, `pauseExpiry[]`).
use serde_json::{Map, Value, json};

use super::wire::{MAX_PAUSE_MS, PeerPause};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 86_400_000;
/// "Until tomorrow" is the next 08:00 of the local wall clock that is at least two hours away. One constant, one place.
pub const TOMORROW_HOUR: i64 = 8;
pub const TOMORROW_MIN_AHEAD_MS: i64 = 2 * HOUR_MS;

/// What the user picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseChoice {
    OneHour,
    Tomorrow,
    UntilResumed,
}

impl PauseChoice {
    /// `1h`, `tomorrow`, `resumed` (the words of `Kin.exe --pause` and the UI).
    pub fn parse(s: &str) -> Option<PauseChoice> {
        Some(match s {
            "1h" => PauseChoice::OneHour,
            "tomorrow" => PauseChoice::Tomorrow,
            "resumed" | "until-resumed" => PauseChoice::UntilResumed,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            PauseChoice::OneHour => "1h",
            PauseChoice::Tomorrow => "tomorrow",
            PauseChoice::UntilResumed => "resumed",
        }
    }
}

/// This device's own pause: a time (Unix ms) or "until resumed". It is what `settings.json` keeps (`"pause": {"until": ms}` or `{"untilResumed": true}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseState {
    Until(i64),
    UntilResumed,
}

impl PauseState {
    /// The gate: a wall-clock comparison on each use, no timer. A `pausedUntil` more than 8 days ahead of the current clock is expired (a clock set
    /// backwards must not freeze Kin for a year).
    pub fn is_on(&self, now_ms: i64) -> bool {
        match *self {
            PauseState::UntilResumed => true,
            PauseState::Until(until) => now_ms < until && until - now_ms <= MAX_PAUSE_MS as i64,
        }
    }

    /// How a `presence` tells the peer: the time left (at least 1 ms), or "until resumed". `None` once it has ended.
    pub fn to_wire(&self, now_ms: i64) -> Option<PeerPause> {
        match *self {
            PauseState::UntilResumed => Some(PeerPause::UntilResumed),
            PauseState::Until(_) if !self.is_on(now_ms) => None,
            PauseState::Until(until) => Some(PeerPause::Remaining(((until - now_ms) as u64).clamp(1, MAX_PAUSE_MS))),
        }
    }

    pub fn to_json(&self) -> Value {
        match *self {
            PauseState::Until(ms) => json!({ "until": ms }),
            PauseState::UntilResumed => json!({ "untilResumed": true }),
        }
    }

    /// From `settings.json`: `{"until": 1760000000000}` or `{"untilResumed": true}`. Anything else is no pause.
    pub fn from_json(v: &Value) -> Option<PauseState> {
        let m: &Map<String, Value> = v.as_object()?;
        if m.get("untilResumed").and_then(Value::as_bool) == Some(true) {
            return Some(PauseState::UntilResumed);
        }
        m.get("until").and_then(Value::as_i64).filter(|n| *n > 0).map(PauseState::Until)
    }

    /// "until 14:30" for the tooltip and the windows (`local` is the local time of the end), "until resumed" for the open-ended one.
    pub fn until_text(&self, local_hhmm: Option<&str>) -> String {
        match (self, local_hhmm) {
            (PauseState::UntilResumed, _) => "until resumed".to_string(),
            (PauseState::Until(_), Some(t)) => format!("until {t}"),
            (PauseState::Until(_), None) => "for a while".to_string(),
        }
    }
}

/// The offset of a time zone from UTC at an instant: local time = UTC + offset.
pub trait Zone: Send + Sync {
    fn offset_ms(&self, utc_ms: i64) -> i64;
}

/// The wall clock of this Windows user (`SystemTimeToTzSpecificLocalTime`): the zone and daylight saving the user's Windows says.
pub struct WindowsZone;

impl Zone for WindowsZone {
    fn offset_ms(&self, utc_ms: i64) -> i64 {
        crate::localtime::local_offset_ms(utc_ms).unwrap_or(0)
    }
}

// ----- calendar arithmetic (proleptic Gregorian, days since 1970-01-01) --------------------------------------------------------

/// Days since 1970-01-01 of a calendar day.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The calendar day of a number of days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = Sunday .. 6 = Saturday
fn weekday_of_days(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// The local wall-clock time `hour:minute` of a calendar day, as a UTC instant. A time that does not exist (the spring change) is read with the offset
/// from before the change, one that exists twice (the autumn change) with the first. `08:00` is never either in the zones that matter.
fn local_to_utc(zone: &dyn Zone, days: i64, hour: i64, minute: i64) -> i64 {
    let local = days * DAY_MS + hour * HOUR_MS + minute * 60_000;
    let mut t = local - zone.offset_ms(local);
    t = local - zone.offset_ms(t);
    t
}

/// The end of a pause chosen at `now_ms`. `1h` is one hour of elapsed time. `tomorrow` is **08:00 of the zone's wall clock**, the first one **at least
/// two hours away** (at 06:00 sharp it is today 08:00, a second later tomorrow 08:00), so across a daylight-saving change it is not 24 hours.
pub fn pause_for(choice: PauseChoice, now_ms: i64, zone: &dyn Zone) -> PauseState {
    match choice {
        PauseChoice::UntilResumed => PauseState::UntilResumed,
        PauseChoice::OneHour => PauseState::Until(now_ms + HOUR_MS),
        PauseChoice::Tomorrow => {
            let local_days = (now_ms + zone.offset_ms(now_ms)).div_euclid(DAY_MS);
            let today = local_to_utc(zone, local_days, TOMORROW_HOUR, 0);
            let at = if today - now_ms >= TOMORROW_MIN_AHEAD_MS { today } else { local_to_utc(zone, local_days + 1, TOMORROW_HOUR, 0) };
            PauseState::Until(at)
        }
    }
}

// ----- the peer's pause, as this side knows it --------------------------------------------------------------------------------

/// The pause of the peer as told by its last `presence` on this connection: `expiry = arrival (monotonic) + remainingMs`. This side resumes sending
/// automatic things at the expiry, or when a `presence` without `pause` arrives (that replaces this value by `None`), whichever is first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerPauseView {
    arrival_ms: i64,
    pause: PeerPause,
}

impl PeerPauseView {
    pub fn new(pause: PeerPause, arrival_ms: i64) -> PeerPauseView {
        PeerPauseView { arrival_ms, pause }
    }

    pub fn is_paused(&self, now_ms: i64) -> bool {
        match self.pause {
            PeerPause::UntilResumed => true,
            PeerPause::Remaining(ms) => now_ms < self.arrival_ms + ms.min(MAX_PAUSE_MS) as i64,
        }
    }

    /// The time left, for "paused until 14:30" (the viewer adds it to its own clock).
    pub fn remaining_ms(&self, now_ms: i64) -> Option<i64> {
        match self.pause {
            PeerPause::UntilResumed => None,
            PeerPause::Remaining(ms) => Some((self.arrival_ms + ms.min(MAX_PAUSE_MS) as i64 - now_ms).max(0)),
        }
    }

    pub fn until_resumed(&self) -> bool {
        self.pause == PeerPause::UntilResumed
    }
}

// ----- the gate table ----------------------------------------------------------------------------------------------------------

/// Where the gate is asked: when a message is about to go out, or when it has arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Send,
    Receive,
}

/// What the gate says: `Send` (a sender may send), `Drop`, `Quiet` (a receiver stores it and shows nothing) or `Flows` (nothing changes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Send,
    Drop,
    Quiet,
    Flows,
}

impl Gate {
    pub fn word(self) -> &'static str {
        match self {
            Gate::Send => "send",
            Gate::Drop => "drop",
            Gate::Quiet => "quiet",
            Gate::Flows => "flows",
        }
    }
}

/// The table of protocol.md ("Pause: what stops, what still arrives"). `kind` is the name of the vectors: automatic things (`clipboard`, `notif.post`,
/// `media.state`, `media.image`, `media.volume`, `screenshot`) stop while this side or the peer is paused (a receiver looks only at its own pause);
/// manual things (`text`, `link`, `offer`, `file`) are always sent and arrive quietly at a paused receiver; everything else (cleanup, `presence`,
/// `ring.*`, `unpair`, `ping`, transfers in flight) is not touched.
pub fn gate(side: Side, kind: &str, self_paused: bool, peer_paused: bool) -> Gate {
    const AUTO: [&str; 6] = ["clipboard", "notif.post", "media.state", "media.image", "media.volume", "screenshot"];
    const MANUAL: [&str; 4] = ["text", "link", "offer", "file"];
    match side {
        Side::Send if AUTO.contains(&kind) => {
            if self_paused || peer_paused {
                Gate::Drop
            } else {
                Gate::Send
            }
        }
        Side::Send if MANUAL.contains(&kind) => Gate::Send,
        // An auto-sent screenshot is an upload: a paused PC saves it, quietly.
        Side::Receive if kind == "screenshot" => {
            if self_paused {
                Gate::Quiet
            } else {
                Gate::Flows
            }
        }
        Side::Receive if AUTO.contains(&kind) => {
            if self_paused {
                Gate::Drop
            } else {
                Gate::Flows
            }
        }
        Side::Receive if MANUAL.contains(&kind) => {
            if self_paused {
                Gate::Quiet
            } else {
                Gate::Flows
            }
        }
        _ => Gate::Flows,
    }
}

// ----- a zone made of rules, for tests and vectors ----------------------------------------------------------------------------

/// How daylight saving starts and ends in a rule zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DstRule {
    None,
    /// Last Sunday of March 01:00 UTC to last Sunday of October 01:00 UTC.
    Europe,
    /// Second Sunday of March 02:00 local standard time to first Sunday of November 02:00 local daylight time.
    UnitedStates,
}

/// A zone with a standard offset and a daylight-saving rule: enough to test the pause across a clock change without a time zone database. (The real
/// clock is Windows'.)
#[derive(Clone, Copy, Debug)]
pub struct RuleZone {
    pub standard_offset_ms: i64,
    pub dst: DstRule,
}

impl RuleZone {
    pub const UTC: RuleZone = RuleZone { standard_offset_ms: 0, dst: DstRule::None };

    /// The zones of the vectors: `UTC`, `Europe/Berlin`, `America/New_York`, `Asia/Kolkata`.
    pub fn named(name: &str) -> Option<RuleZone> {
        Some(match name {
            "UTC" => RuleZone::UTC,
            "Europe/Berlin" => RuleZone { standard_offset_ms: HOUR_MS, dst: DstRule::Europe },
            "America/New_York" => RuleZone { standard_offset_ms: -5 * HOUR_MS, dst: DstRule::UnitedStates },
            "Asia/Kolkata" => RuleZone { standard_offset_ms: 5 * HOUR_MS + 30 * 60_000, dst: DstRule::None },
            _ => return None,
        })
    }
}

fn nth_sunday(year: i64, month: i64, n: i64) -> i64 {
    // The first Sunday of the month, then n - 1 weeks on (n = 1 for the first, 2 for the second).
    let first = days_from_civil(year, month, 1);
    first + (7 - weekday_of_days(first)) % 7 + (n - 1) * 7
}

fn last_sunday(year: i64, month: i64) -> i64 {
    let next_first = if month == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, month + 1, 1) };
    let last = next_first - 1;
    last - weekday_of_days(last)
}

impl Zone for RuleZone {
    fn offset_ms(&self, utc_ms: i64) -> i64 {
        let (year, _, _) = civil_from_days((utc_ms + self.standard_offset_ms).div_euclid(DAY_MS));
        let dst = match self.dst {
            DstRule::None => false,
            DstRule::Europe => {
                let start = last_sunday(year, 3) * DAY_MS + HOUR_MS;
                let end = last_sunday(year, 10) * DAY_MS + HOUR_MS;
                (start..end).contains(&utc_ms)
            }
            DstRule::UnitedStates => {
                let start = nth_sunday(year, 3, 2) * DAY_MS + 2 * HOUR_MS - self.standard_offset_ms;
                let end = nth_sunday(year, 11, 1) * DAY_MS + 2 * HOUR_MS - (self.standard_offset_ms + HOUR_MS);
                (start..end).contains(&utc_ms)
            }
        };
        self.standard_offset_ms + if dst { HOUR_MS } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * DAY_MS + h * HOUR_MS + min * 60_000
    }

    #[test]
    fn the_calendar_functions_round_trip_and_know_the_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for d in [-1000, 0, 1, 365, 10_000, 20_000, 24_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29), "a leap day");
        assert_eq!(weekday_of_days(days_from_civil(2026, 10, 25)), 0, "2026-10-25 is a Sunday");
    }

    #[test]
    fn the_rule_zones_change_their_clocks_on_the_right_days() {
        let berlin = RuleZone::named("Europe/Berlin").unwrap();
        assert_eq!(berlin.offset_ms(utc(2026, 3, 29, 0, 59)), HOUR_MS, "still winter time");
        assert_eq!(berlin.offset_ms(utc(2026, 3, 29, 1, 0)), 2 * HOUR_MS, "summer time from 01:00 UTC");
        assert_eq!(berlin.offset_ms(utc(2026, 10, 25, 0, 59)), 2 * HOUR_MS);
        assert_eq!(berlin.offset_ms(utc(2026, 10, 25, 1, 0)), HOUR_MS, "winter time again from 01:00 UTC");
        let ny = RuleZone::named("America/New_York").unwrap();
        assert_eq!(ny.offset_ms(utc(2026, 3, 8, 6, 59)), -5 * HOUR_MS);
        assert_eq!(ny.offset_ms(utc(2026, 3, 8, 7, 0)), -4 * HOUR_MS, "02:00 EST is 07:00 UTC");
        assert_eq!(ny.offset_ms(utc(2026, 11, 1, 5, 59)), -4 * HOUR_MS);
        assert_eq!(ny.offset_ms(utc(2026, 11, 1, 6, 0)), -5 * HOUR_MS, "02:00 EDT is 06:00 UTC");
    }

    #[test]
    fn tomorrow_is_the_next_0800_at_least_two_hours_away() {
        let z = RuleZone::UTC;
        let at = |h: i64, m: i64, s: i64| utc(2026, 6, 1, h, m) + s * 1000;
        let day = |d: i64| utc(2026, 6, d, 8, 0);
        for (now, want) in [
            (at(0, 0, 0), day(1)),
            (at(5, 0, 0), day(1)),
            (at(6, 0, 0), day(1)), // exactly two hours: today
            (at(6, 0, 1), day(2)),
            (at(7, 30, 0), day(2)),
            (at(8, 0, 0), day(2)),
            (at(23, 59, 0), day(2)),
        ] {
            assert_eq!(pause_for(PauseChoice::Tomorrow, now, &z), PauseState::Until(want), "{now}");
        }
    }

    #[test]
    fn tomorrow_is_a_wall_clock_time_across_the_clock_changes() {
        let berlin = RuleZone::named("Europe/Berlin").unwrap();
        // The evening before the spring change: 08:00 CEST (06:00 UTC) is only 7 hours away, not 9.
        let now = utc(2026, 3, 28, 22, 59); // 23:59 CET
        assert_eq!(pause_for(PauseChoice::Tomorrow, now, &berlin), PauseState::Until(utc(2026, 3, 29, 6, 0)));
        // The evening before the autumn change: 08:00 CET (07:00 UTC).
        let now = utc(2026, 10, 24, 21, 59); // 23:59 CEST
        assert_eq!(pause_for(PauseChoice::Tomorrow, now, &berlin), PauseState::Until(utc(2026, 10, 25, 7, 0)));
        // One hour is one hour of elapsed time: 01:30 CET + 1 h is 03:30 CEST.
        let one = pause_for(PauseChoice::OneHour, utc(2026, 3, 29, 0, 30), &berlin);
        assert_eq!(one, PauseState::Until(utc(2026, 3, 29, 1, 30)));
        let ny = RuleZone::named("America/New_York").unwrap();
        assert_eq!(
            pause_for(PauseChoice::Tomorrow, utc(2026, 3, 8, 3, 0), &ny),
            PauseState::Until(utc(2026, 3, 8, 12, 0)),
            "22:00 EST the evening before: 08:00 EDT"
        );
        let kolkata = RuleZone::named("Asia/Kolkata").unwrap();
        assert_eq!(
            pause_for(PauseChoice::Tomorrow, utc(2026, 6, 1, 1, 30), &kolkata),
            PauseState::Until(utc(2026, 6, 2, 2, 30)),
            "07:00 IST -> tomorrow 08:00 IST"
        );
    }

    #[test]
    fn the_gate_is_a_wall_clock_comparison_and_an_old_far_future_pause_is_expired() {
        let now = 1_760_000_000_000;
        assert!(PauseState::Until(now + 1).is_on(now));
        assert!(!PauseState::Until(now).is_on(now));
        assert!(PauseState::Until(now + MAX_PAUSE_MS as i64).is_on(now), "exactly 8 days still counts");
        assert!(!PauseState::Until(now + MAX_PAUSE_MS as i64 + 1).is_on(now), "a clock set backwards must not freeze Kin");
        assert!(PauseState::UntilResumed.is_on(i64::MAX));
        assert_eq!(PauseState::Until(now + 5000).to_wire(now), Some(PeerPause::Remaining(5000)));
        assert_eq!(PauseState::Until(now).to_wire(now), None);
    }

    #[test]
    fn the_state_is_kept_in_settings_json_and_read_back() {
        for s in [PauseState::Until(1_760_000_000_000), PauseState::UntilResumed] {
            assert_eq!(PauseState::from_json(&s.to_json()), Some(s));
        }
        for bad in [json!({}), json!({"until": "x"}), json!({"until": -5}), json!({"untilResumed": false}), json!(5), json!(null)] {
            assert_eq!(PauseState::from_json(&bad), None, "{bad}");
        }
    }

    #[test]
    fn the_peers_pause_ends_at_its_expiry() {
        let v = PeerPauseView::new(PeerPause::Remaining(60_000), 1000);
        assert!(v.is_paused(60_999) && !v.is_paused(61_000));
        assert_eq!(v.remaining_ms(31_000), Some(30_000));
        assert!(PeerPauseView::new(PeerPause::UntilResumed, 0).is_paused(1 << 40));
    }

    #[test]
    fn the_gate_table_has_a_word_for_every_kind() {
        assert_eq!(gate(Side::Send, "clipboard", false, true), Gate::Drop);
        assert_eq!(gate(Side::Receive, "clipboard", false, true), Gate::Flows);
        assert_eq!(gate(Side::Receive, "text", true, false), Gate::Quiet);
        assert_eq!(gate(Side::Send, "text", true, true), Gate::Send);
        assert_eq!(gate(Side::Receive, "screenshot", true, false), Gate::Quiet);
        assert_eq!(gate(Side::Send, "ring.start", true, true), Gate::Flows);
        assert_eq!(gate(Side::Receive, "notif.remove", true, false), Gate::Flows);
    }
}
