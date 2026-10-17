//! Ring my phone, the PC's side (design 6.4, protocol.md "Ring"): the client state of one phone, the 5 s wait for the `ack`, the 1-start-per-10-s
//! limit, the 35 s local watchdog and the words the user reads. Pure (the time is an argument), so a test drives it with a fake clock. The PC makes **no
//! sound** at any point: a ring is a message to the phone, and the phone does the ringing.
use serde_json::{Value, json};

/// The phone has to answer a `ring.start` with an `ack` within this long, else the user is told "no answer" and nothing is sent again.
pub const ACK_WAIT_MS: i64 = 5_000;
/// The PC sends at most one `ring.start` per this long.
pub const MIN_GAP_MS: i64 = 10_000;
/// A ring is 30 s on the phone; if the phone never says it stopped (it died, the link did) the PC stops showing "ringing" after this long.
pub const WATCHDOG_MS: i64 = 35_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RingState {
    Idle,
    /// A `ring.start` is on its way (waiting for the `ack` and for `ring.status ringing`).
    Requested {
        since: i64,
    },
    Ringing {
        since: i64,
        audible: bool,
    },
    /// The phone said it stopped (`reason`: `timeout`, `user`, `pc`, `limit`, `error`, or anything else, shown as "stopped").
    Stopped {
        reason: Option<String>,
    },
}

impl RingState {
    pub fn word(&self) -> &'static str {
        match self {
            RingState::Idle => "idle",
            RingState::Requested { .. } => "requested",
            RingState::Ringing { .. } => "ringing",
            RingState::Stopped { .. } => "stopped",
        }
    }

    /// A ring is on its way or going: the tray offers "Stop ringing phone".
    pub fn is_active(&self) -> bool {
        matches!(self, RingState::Requested { .. } | RingState::Ringing { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingCounts {
    pub started: u32,
    pub ringing: u32,
    pub stopped: u32,
    pub no_answer: u32,
    pub refused: u32,
}

/// Why a press did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The last `ring.start` was less than 10 s ago.
    TooSoon { wait_ms: i64 },
    /// A ring is already on its way or going.
    AlreadyRinging,
}

#[derive(Debug)]
pub struct RingClient {
    state: RingState,
    last_start: Option<i64>,
    counts: RingCounts,
}

impl Default for RingClient {
    fn default() -> Self {
        RingClient::new()
    }
}

impl RingClient {
    pub fn new() -> RingClient {
        RingClient { state: RingState::Idle, last_start: None, counts: RingCounts::default() }
    }

    pub fn state(&self) -> &RingState {
        &self.state
    }

    pub fn counts(&self) -> RingCounts {
        self.counts
    }

    /// "Ring my phone" was pressed: if it may go, the client is `Requested` and the caller sends the `ring.start`.
    pub fn press(&mut self, now: i64) -> Result<(), Refusal> {
        if self.state.is_active() {
            return Err(Refusal::AlreadyRinging);
        }
        if let Some(last) = self.last_start.filter(|l| now - l < MIN_GAP_MS) {
            self.counts.refused += 1;
            return Err(Refusal::TooSoon { wait_ms: MIN_GAP_MS - (now - last) });
        }
        self.last_start = Some(now);
        self.counts.started += 1;
        self.state = RingState::Requested { since: now };
        Ok(())
    }

    /// The phone answered with an `error` instead of an `ack`: the user is told why, and the state goes back to idle.
    pub fn on_error(&mut self, code: &str, phone: &str) -> String {
        self.state = RingState::Idle;
        match code {
            "disabled" => format!("Ringing is turned off on {phone}"),
            "rate_limited" => "Wait a minute, then try again".to_string(),
            _ => format!("{phone} couldn't ring ({code})"),
        }
    }

    /// No `ack` in 5 seconds: "no answer", nothing is sent again.
    pub fn on_no_answer(&mut self, phone: &str) -> String {
        self.state = RingState::Idle;
        self.counts.no_answer += 1;
        format!("No answer from {phone}. It may be asleep or off Wi-Fi.")
    }

    /// There was no connection, or the phone doesn't list `ring`.
    pub fn on_not_connected(&mut self, phone: &str) -> String {
        self.state = RingState::Idle;
        format!("{phone} isn't connected, so Kin can't ring it.")
    }

    /// `ring.status` from the phone. The returned text is a notice to show, if the status calls for one.
    pub fn on_status(&mut self, ringing: bool, audible: bool, reason: Option<&str>, now: i64, phone: &str) -> Option<String> {
        if ringing {
            self.state = RingState::Ringing { since: now, audible };
            self.counts.ringing += 1;
            (!audible).then(|| "Ringing, but Do Not Disturb is on total silence: it won't make a sound".to_string())
        } else {
            self.state = RingState::Stopped { reason: reason.map(str::to_string) };
            self.counts.stopped += 1;
            let _ = phone;
            None
        }
    }

    /// The user pressed "Stop ringing phone" (the caller sends the `ring.stop`, which is always acked). The state is idle until the phone says otherwise.
    pub fn stop_pressed(&mut self) {
        if self.state.is_active() {
            self.state = RingState::Idle;
        }
    }

    /// The connection ended: nothing is ringing as far as this side knows.
    pub fn connection_lost(&mut self) {
        self.state = RingState::Idle;
    }

    /// The local watchdog: a ring that no `ring.status` ended is not shown for ever.
    pub fn tick(&mut self, now: i64) {
        let since = match self.state {
            RingState::Requested { since } | RingState::Ringing { since, .. } => since,
            _ => return,
        };
        if now - since >= WATCHDOG_MS {
            self.state = RingState::Idle;
        }
    }

    /// The line the tray and the drop window show: "Pixel 8 is ringing", else `None`.
    pub fn text(&self, phone: &str) -> Option<String> {
        match &self.state {
            RingState::Requested { .. } => Some(format!("Ringing {phone}\u{2026}")),
            RingState::Ringing { audible: true, .. } => Some(format!("{phone} is ringing")),
            RingState::Ringing { audible: false, .. } => Some(format!("{phone} is ringing, but silent (Do Not Disturb)")),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "state": self.state.word(),
            "audible": matches!(self.state, RingState::Ringing { audible: true, .. }),
            "counts": {
                "started": self.counts.started, "ringing": self.counts.ringing, "stopped": self.counts.stopped,
                "noAnswer": self.counts.no_answer, "refused": self.counts.refused,
            },
        })
    }
}

/// Why "Ring my phone" is not available right now, as the status line shows it. `None`: it is.
pub fn unavailable_reason(phone: &str, connected: bool, lists_presence: bool, lists_ring: bool) -> Option<String> {
    if !connected {
        return Some(format!("{phone} isn't connected, so Kin can't ring it."));
    }
    if !lists_ring {
        return Some(if lists_presence {
            format!("Ringing isn't available: turn it on in Kin on {phone}")
        } else {
            format!("Ringing isn't available: {phone} needs Kin 0.2.0, with ringing turned on")
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_asks_waits_for_the_status_and_the_ring_ends_with_the_phones_word() {
        let mut c = RingClient::new();
        assert_eq!(c.press(1000), Ok(()));
        assert_eq!(c.state(), &RingState::Requested { since: 1000 });
        assert_eq!(c.on_status(true, true, None, 1200, "Pixel 8"), None);
        assert_eq!(c.text("Pixel 8").as_deref(), Some("Pixel 8 is ringing"));
        assert!(c.state().is_active());
        c.on_status(false, true, Some("timeout"), 31_200, "Pixel 8");
        assert_eq!(c.state(), &RingState::Stopped { reason: Some("timeout".into()) });
        assert!(!c.state().is_active());
        assert_eq!(c.text("Pixel 8"), None);
    }

    #[test]
    fn at_most_one_start_per_10_s_and_no_second_ring_while_one_goes() {
        let mut c = RingClient::new();
        c.press(0).unwrap();
        assert_eq!(c.press(100), Err(Refusal::AlreadyRinging));
        c.stop_pressed();
        assert_eq!(c.press(9_999), Err(Refusal::TooSoon { wait_ms: 1 }));
        assert_eq!(c.press(10_000), Ok(()));
        assert_eq!(c.counts().refused, 1);
    }

    #[test]
    fn the_texts_of_the_errors_say_what_to_do() {
        let mut c = RingClient::new();
        c.press(0).unwrap();
        assert_eq!(c.on_error("disabled", "Pixel 8"), "Ringing is turned off on Pixel 8");
        assert_eq!(c.state(), &RingState::Idle);
        c.press(20_000).unwrap();
        assert_eq!(c.on_error("rate_limited", "Pixel 8"), "Wait a minute, then try again");
        c.press(40_000).unwrap();
        assert_eq!(c.on_no_answer("Pixel 8"), "No answer from Pixel 8. It may be asleep or off Wi-Fi.");
        assert_eq!(c.counts().no_answer, 1);
        assert_eq!(c.on_not_connected("Pixel 8"), "Pixel 8 isn't connected, so Kin can't ring it.");
    }

    #[test]
    fn total_silence_says_so_and_the_watchdog_ends_a_ring_nobody_ended() {
        let mut c = RingClient::new();
        c.press(0).unwrap();
        let notice = c.on_status(true, false, None, 100, "Pixel 8");
        assert_eq!(notice.as_deref(), Some("Ringing, but Do Not Disturb is on total silence: it won't make a sound"));
        assert!(c.text("Pixel 8").unwrap().contains("silent"));
        c.tick(34_999);
        assert!(c.state().is_active());
        c.tick(35_100);
        assert_eq!(c.state(), &RingState::Idle, "35 s after the ring began, the local watchdog lets go");
        c.press(50_000).unwrap();
        c.connection_lost();
        assert_eq!(c.state(), &RingState::Idle);
    }

    #[test]
    fn the_reason_ring_is_not_available_is_the_one_the_user_can_act_on() {
        assert_eq!(unavailable_reason("Pixel 8", false, true, true).as_deref(), Some("Pixel 8 isn't connected, so Kin can't ring it."));
        assert!(unavailable_reason("Pixel 8", true, true, false).unwrap().contains("turn it on in Kin on Pixel 8"));
        assert!(unavailable_reason("Pixel 8", true, false, false).unwrap().contains("needs Kin 0.2.0"));
        assert_eq!(unavailable_reason("Pixel 8", true, true, true), None);
    }
}
