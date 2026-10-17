//! The PC's defence against a flood of `notif.post` (protocol.md "Ordering, limits and offline"): a token bucket per phone (it survives a reconnect), a window in which an
//! identical post of one `nid` is ignored, and at most one `rate_limited` error per 10 seconds. It also holds the limit of the actions the user
//! presses (5 per 10 s).
//!
//! Time is an argument (milliseconds on a monotonic clock) and the arithmetic is in thousandths of a token, so the vectors (`notif.json`, `rate[]`)
//! pin it exactly and a test needs no clock.
use std::collections::{HashMap, VecDeque};

/// Posts the bucket holds: 20, refilled 1 per second.
const BUCKET_TOKENS: u64 = 20;
const MILLI: u64 = 1000;
/// An identical payload for one `nid` within this long of the last accepted one is ignored.
pub const DUPLICATE_WINDOW_MS: u64 = 2000;
/// At most one `rate_limited` error in this time.
pub const ERROR_SPACING_MS: u64 = 10_000;
/// The user's actions toward the phone: at most 5 in 10 s.
pub const ACTIONS_PER_WINDOW: usize = 5;
pub const ACTION_WINDOW_MS: u64 = 10_000;

/// What the receiver does with a post.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    /// The same payload for the same `nid` within 2 s of the last accepted one: no new popup, no token used.
    IgnoredDuplicate,
    /// Over the budget: dropped and counted. `send_error`: this is the one `error` `rate_limited` of the 10 seconds.
    DroppedRate {
        send_error: bool,
    },
}

pub struct PostLimiter {
    /// Thousandths of a token.
    tokens: u64,
    last_ms: u64,
    last_accepted: HashMap<String, (u64, u64)>,
    last_error_ms: Option<u64>,
}

impl Default for PostLimiter {
    fn default() -> Self {
        PostLimiter { tokens: BUCKET_TOKENS * MILLI, last_ms: 0, last_accepted: HashMap::new(), last_error_ms: None }
    }
}

impl PostLimiter {
    /// Judges a post of `nid` with this payload `content_hash` (any stable hash of what the toast would show) at `now_ms`.
    pub fn post(&mut self, now_ms: u64, nid: &str, content_hash: u64) -> Verdict {
        // One token per second is one thousandth per millisecond.
        self.tokens = (self.tokens + now_ms.saturating_sub(self.last_ms)).min(BUCKET_TOKENS * MILLI);
        self.last_ms = self.last_ms.max(now_ms);
        if let Some((at, hash)) = self.last_accepted.get(nid)
            && *hash == content_hash
            && now_ms.saturating_sub(*at) < DUPLICATE_WINDOW_MS
        {
            return Verdict::IgnoredDuplicate;
        }
        if self.tokens >= MILLI {
            self.tokens -= MILLI;
            // The memory of payloads is only as long as a window needs it to be.
            self.last_accepted.retain(|_, (at, _)| now_ms.saturating_sub(*at) < DUPLICATE_WINDOW_MS);
            self.last_accepted.insert(nid.to_string(), (now_ms, content_hash));
            return Verdict::Accepted;
        }
        let send_error = self.last_error_ms.is_none_or(|at| now_ms.saturating_sub(at) >= ERROR_SPACING_MS);
        if send_error {
            self.last_error_ms = Some(now_ms);
        }
        Verdict::DroppedRate { send_error }
    }
}

/// The user pressed a button: at most 5 per 10 seconds, over that the press is refused (and says so).
#[derive(Default)]
pub struct ActionLimiter {
    times: VecDeque<u64>,
}

impl ActionLimiter {
    pub fn allow(&mut self, now_ms: u64) -> bool {
        while self.times.front().is_some_and(|t| now_ms.saturating_sub(*t) >= ACTION_WINDOW_MS) {
            self.times.pop_front();
        }
        if self.times.len() >= ACTIONS_PER_WINDOW {
            return false;
        }
        self.times.push_back(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(events: &[(u64, &str, u64)]) -> (Vec<&'static str>, Vec<u64>) {
        let mut l = PostLimiter::default();
        let (mut out, mut errors) = (Vec::new(), Vec::new());
        for (t, nid, content) in events {
            out.push(match l.post(*t, nid, *content) {
                Verdict::Accepted => "accepted",
                Verdict::IgnoredDuplicate => "ignored_duplicate",
                Verdict::DroppedRate { send_error } => {
                    if send_error {
                        errors.push(*t);
                    }
                    "dropped_rate"
                }
            });
        }
        (out, errors)
    }

    #[test]
    fn a_burst_uses_the_twenty_tokens_and_the_first_drop_sends_one_error() {
        let ids: Vec<String> = (0..25).map(|k| format!("n{k}")).collect();
        let events: Vec<(u64, &str, u64)> = ids.iter().map(|n| (0, n.as_str(), 1)).collect();
        let (out, errors) = run(&events);
        assert_eq!(out.iter().filter(|o| **o == "accepted").count(), 20);
        assert_eq!(out.iter().filter(|o| **o == "dropped_rate").count(), 5);
        assert_eq!(errors, [0]);
    }

    #[test]
    fn a_token_comes_back_every_second_and_not_before() {
        let ids: Vec<String> = (0..20).map(|k| format!("n{k}")).collect();
        let mut events: Vec<(u64, &str, u64)> = ids.iter().map(|n| (0, n.as_str(), 1)).collect();
        events.extend([(999, "a", 1), (1000, "b", 1), (1000, "c", 1)]);
        let (out, _) = run(&events);
        assert_eq!(&out[20..], ["dropped_rate", "accepted", "dropped_rate"]);
    }

    #[test]
    fn an_identical_post_within_two_seconds_is_ignored_and_costs_nothing() {
        let (out, _) = run(&[(0, "n", 1), (1999, "n", 1), (2000, "n", 1), (2500, "n", 2), (2600, "n", 2)]);
        assert_eq!(out, ["accepted", "ignored_duplicate", "accepted", "accepted", "ignored_duplicate"]);
        let mut events = vec![(0u64, "x", 9u64)];
        events.extend(std::iter::repeat_n((100, "x", 9), 30));
        let ids: Vec<String> = (0..19).map(|k| format!("n{k}")).collect();
        events.extend(ids.iter().map(|n| (200, n.as_str(), 1)));
        let (out, _) = run(&events);
        assert!(out[1..31].iter().all(|o| *o == "ignored_duplicate"));
        assert!(out[31..].iter().all(|o| *o == "accepted"), "the repeats used no token: 1 + 19 still fit");
    }

    #[test]
    fn errors_are_at_least_ten_seconds_apart() {
        let ids: Vec<String> = (0..300).map(|k| format!("n{k}")).collect();
        let events: Vec<(u64, &str, u64)> = ids.iter().enumerate().map(|(k, n)| (k as u64 * 100, n.as_str(), 1)).collect();
        let (_, errors) = run(&events);
        assert!(errors.len() >= 2);
        assert!(errors.windows(2).all(|w| w[1] - w[0] >= ERROR_SPACING_MS), "{errors:?}");
    }

    #[test]
    fn a_dropped_post_is_not_remembered_as_the_last_accepted_one() {
        let ids: Vec<String> = (0..20).map(|k| format!("n{k}")).collect();
        let mut events: Vec<(u64, &str, u64)> = ids.iter().map(|n| (0, n.as_str(), 1)).collect();
        events.extend([(500, "x", 7), (1000, "x", 7)]);
        let (out, _) = run(&events);
        assert_eq!(&out[20..], ["dropped_rate", "accepted"]);
    }

    #[test]
    fn five_actions_in_ten_seconds() {
        let mut l = ActionLimiter::default();
        assert!((0..5).all(|k| l.allow(k * 100)));
        assert!(!l.allow(900));
        assert!(!l.allow(9999));
        assert!(l.allow(10_000), "the first press has left the window");
    }
}
