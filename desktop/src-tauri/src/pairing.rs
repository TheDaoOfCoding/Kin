//! Pairing tokens and the pairing URL (protocol.md, "Pairing").
//!
//! One token is active at a time: the one on screen. It is single use, valid for 5 minutes, compared in constant
//! time, and invalidated after 5 wrong guesses within 60 seconds. Tokens that are no longer active are remembered
//! for an hour, so they answer `token_expired` or `token_used` instead of `bad_token`.
use std::collections::VecDeque;
use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use crate::encoding::{self, percent_encode, truncate_chars};

pub const TOKEN_TTL: Duration = Duration::from_secs(300);
const HISTORY_KEPT_FOR: Duration = Duration::from_secs(3600);
const HISTORY_MAX: usize = 64;
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const MAX_NAME_CHARS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairError {
    BadToken,
    TokenExpired,
    TokenUsed,
}

impl PairError {
    /// The `error` code of the response.
    pub fn code(self) -> &'static str {
        match self {
            PairError::BadToken => "bad_token",
            PairError::TokenExpired => "token_expired",
            PairError::TokenUsed => "token_used",
        }
    }
}

struct Token {
    value: String,
    expires_at: Instant,
    used: bool,
    invalidated: bool,
    /// When it stopped being the active token. Only set in the history.
    ended_at: Instant,
}

#[derive(Default)]
pub struct Pairing {
    active: Option<Token>,
    history: VecDeque<Token>,
    failures: VecDeque<Instant>,
}

/// Compares in constant time for equal lengths. The length itself isn't secret (it is always 22).
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Pairing {
    /// Starts a pairing: a new token, and the previous active one (if any) is no longer valid.
    pub fn begin(&mut self, now: Instant) -> String {
        self.retire(now, false);
        let value = encoding::new_token();
        self.active = Some(Token { value: value.clone(), expires_at: now + TOKEN_TTL, used: false, invalidated: false, ended_at: now });
        self.failures.clear();
        value
    }

    /// Ends the pairing without a result (the window was closed): the token stops working.
    pub fn end(&mut self, now: Instant) {
        self.retire(now, false);
    }

    /// When the active token runs out, if there is one that can still be used.
    pub fn active_expiry(&self, now: Instant) -> Option<Instant> {
        self.active.as_ref().filter(|t| !t.invalidated && now < t.expires_at).map(|t| t.expires_at)
    }

    /// `true` if the pairing in progress was shut down by too many wrong guesses (see `MAX_FAILURES`).
    pub fn is_invalidated(&self) -> bool {
        self.active.as_ref().is_some_and(|t| t.invalidated)
    }

    /// Checks a presented token. This doesn't use it up: call [`Pairing::consume`] once the device is saved, with
    /// the same lock still held, so two requests can't both succeed.
    pub fn check(&mut self, now: Instant, presented: &str) -> Result<(), PairError> {
        // Look at every token, without stopping at the first match.
        let matches_active = self.active.as_ref().is_some_and(|t| ct_eq(&t.value, presented));
        let remembered = self.history.iter().fold(None, |found, t| if ct_eq(&t.value, presented) { Some(t) } else { found });

        if matches_active {
            let t = self.active.as_ref().expect("matched above");
            return if t.invalidated || now >= t.expires_at { Err(PairError::TokenExpired) } else { Ok(()) };
        }
        if let Some(t) = remembered {
            return Err(if t.used { PairError::TokenUsed } else { PairError::TokenExpired });
        }
        self.record_failure(now);
        Err(PairError::BadToken)
    }

    /// Uses the active token up. It answers `token_used` from now on, and no pairing is in progress.
    pub fn consume(&mut self, now: Instant) {
        self.retire(now, true);
    }

    fn record_failure(&mut self, now: Instant) {
        let Some(active) = self.active.as_mut().filter(|t| !t.invalidated) else {
            return;
        };
        self.failures.push_back(now);
        while self.failures.front().is_some_and(|f| now.duration_since(*f) > FAILURE_WINDOW) {
            self.failures.pop_front();
        }
        if self.failures.len() >= MAX_FAILURES {
            active.invalidated = true;
        }
    }

    fn retire(&mut self, now: Instant, used: bool) {
        if let Some(mut t) = self.active.take() {
            t.used = used;
            t.invalidated = !used;
            t.ended_at = now;
            self.history.push_back(t);
        }
        self.history.retain(|t| now.saturating_duration_since(t.ended_at.max(t.expires_at)) < HISTORY_KEPT_FOR);
        while self.history.len() > HISTORY_MAX {
            self.history.pop_front();
        }
    }

    /// For tests: puts a token in a given state.
    #[doc(hidden)]
    pub fn seed(&mut self, value: &str, state: SeedState, now: Instant) {
        self.active = None;
        self.history.clear();
        self.failures.clear();
        let token = Token { value: value.to_string(), expires_at: now + TOKEN_TTL, used: false, invalidated: false, ended_at: now };
        match state {
            SeedState::None => {}
            SeedState::Active => self.active = Some(token),
            SeedState::Expired => self.active = Some(Token { expires_at: now, ..token }),
            SeedState::Invalidated => self.active = Some(Token { invalidated: true, ..token }),
            SeedState::Used => self.history.push_back(Token { used: true, ..token }),
            SeedState::UsedAndExpired => self.history.push_back(Token { used: true, expires_at: now, ..token }),
        }
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum SeedState {
    None,
    Active,
    Expired,
    Invalidated,
    Used,
    UsedAndExpired,
}

/// The canonical pairing URL, as the PC writes it into the QR code: parameters in the order `v`, `a`, `id`, `tk`,
/// `n`. The name is cut to 64 code points and percent-encoded; `:` and `,` stay as they are in `a`.
pub fn pair_url(addresses: &[SocketAddrV4], device_id: &str, token: &str, name: Option<&str>) -> String {
    let a = addresses.iter().map(|s| format!("{}:{}", s.ip(), s.port())).collect::<Vec<_>>().join(",");
    let mut url = format!("kinrelay://pair?v=1&a={a}&id={device_id}&tk={token}");
    if let Some(name) = name.map(|n| truncate_chars(n, MAX_NAME_CHARS)).filter(|n| !n.is_empty()) {
        url.push_str("&n=");
        url.push_str(&percent_encode(name));
    }
    url
}

/// The QR code as an SVG string, black on white with the quiet zone, at least 256 px wide.
pub fn qr_svg(url: &str) -> String {
    use qrcode::{QrCode, render::svg};
    match QrCode::new(url.as_bytes()) {
        Ok(code) => code.render::<svg::Color>().min_dimensions(256, 256).build(),
        Err(e) => {
            log::error!("QR code failed: {e}");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "AAECAwQFBgcICQoLDA0ODw";
    fn wrong(i: usize) -> String {
        format!("ZZZZZZZZZZZZZZZZZZZZZ{i}")
    }

    fn active(now: Instant) -> Pairing {
        let mut p = Pairing::default();
        p.seed(GOOD, SeedState::Active, now);
        p
    }

    #[test]
    fn a_good_token_works_once() {
        let now = Instant::now();
        let mut p = active(now);
        assert_eq!(p.check(now, GOOD), Ok(()));
        p.consume(now);
        assert_eq!(p.check(now, GOOD), Err(PairError::TokenUsed));
        // And after it has expired too: "used" is checked first.
        assert_eq!(p.check(now + TOKEN_TTL * 2, GOOD), Err(PairError::TokenUsed));
    }

    #[test]
    fn checking_does_not_use_the_token_up() {
        let now = Instant::now();
        let mut p = active(now);
        assert_eq!(p.check(now, GOOD), Ok(()));
        assert_eq!(p.check(now, GOOD), Ok(()));
    }

    #[test]
    fn a_token_expires_after_300_seconds() {
        let now = Instant::now();
        let mut p = active(now);
        assert_eq!(p.check(now + Duration::from_secs(299), GOOD), Ok(()));
        assert_eq!(p.check(now + Duration::from_secs(300), GOOD), Err(PairError::TokenExpired));
    }

    #[test]
    fn five_wrong_guesses_invalidate_the_token() {
        let now = Instant::now();
        let mut p = active(now);
        for i in 1..=5 {
            assert_eq!(p.check(now, &wrong(i)), Err(PairError::BadToken), "guess {i} still answers bad_token");
        }
        assert_eq!(p.check(now, GOOD), Err(PairError::TokenExpired));
        // A new token starts with a clean count.
        let fresh = p.begin(now);
        assert_eq!(p.check(now, &fresh), Ok(()));
    }

    #[test]
    fn failures_older_than_60_seconds_do_not_count() {
        let now = Instant::now();
        let mut p = active(now);
        for i in 1..=4 {
            let _ = p.check(now, &wrong(i));
        }
        assert_eq!(p.check(now + Duration::from_secs(61), &wrong(5)), Err(PairError::BadToken));
        assert_eq!(p.check(now + Duration::from_secs(61), GOOD), Ok(()));
    }

    #[test]
    fn other_answers_are_not_counted() {
        let now = Instant::now();
        let mut p = active(now);
        for _ in 0..10 {
            assert_eq!(p.check(now, GOOD), Ok(()));
        }
        assert_eq!(p.check(now, GOOD), Ok(()));
    }

    #[test]
    fn no_pairing_in_progress_is_bad_token() {
        let now = Instant::now();
        let mut p = Pairing::default();
        assert_eq!(p.check(now, GOOD), Err(PairError::BadToken));
    }

    #[test]
    fn closing_the_window_or_starting_again_invalidates_the_old_token() {
        let now = Instant::now();
        let mut p = Pairing::default();
        let first = p.begin(now);
        let second = p.begin(now);
        assert_eq!(p.check(now, &first), Err(PairError::TokenExpired));
        p.end(now);
        assert_eq!(p.check(now, &second), Err(PairError::TokenExpired));
        assert!(p.active_expiry(now).is_none());
    }

    #[test]
    fn old_tokens_are_forgotten_after_an_hour() {
        let now = Instant::now();
        let mut p = Pairing::default();
        let first = p.begin(now);
        p.end(now);
        assert_eq!(p.check(now + Duration::from_secs(3000), &first), Err(PairError::TokenExpired));
        let later = now + TOKEN_TTL + HISTORY_KEPT_FOR + Duration::from_secs(1);
        p.begin(later);
        assert_eq!(p.check(later, &first), Err(PairError::BadToken));
    }

    #[test]
    fn the_url_is_written_in_canonical_form() {
        let id = "yetOwJHO-3liWauefLv2Zs8_NNIkRYvy7_i5RBG-fP8";
        let addrs = ["192.168.1.20:47821".parse().unwrap(), "10.0.0.5:50000".parse().unwrap()];
        assert_eq!(
            pair_url(&addrs, id, GOOD, Some("Desk PC")),
            format!("kinrelay://pair?v=1&a=192.168.1.20:47821,10.0.0.5:50000&id={id}&tk={GOOD}&n=Desk%20PC")
        );
        assert!(!pair_url(&addrs[..1], id, GOOD, None).contains("&n="));
        assert!(!pair_url(&addrs[..1], id, GOOD, Some("")).contains("&n="));
    }
}
