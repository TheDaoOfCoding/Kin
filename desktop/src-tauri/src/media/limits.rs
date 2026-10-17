//! The rates of protocol.md "Rates, limits, ordering" for `media.*`: five limiters (a token bucket or a sliding window, with an error answer at most once per
//! 10 s for some) and the 250 ms trailing throttle of the source. Pure: time is an argument (milliseconds on a monotonic clock) and the bucket counts in
//! thousandths of a token, so `protocol/media.json` (`rate[]`, `throttle[]`) pins them exactly and a test needs no clock.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `capacity` tokens, refilled `per_second` per second.
    Bucket { capacity: u64, per_second: u64 },
    /// At most `max` in any `window_ms`.
    Window { max: usize, window_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub shape: Shape,
    /// At most one error answer in this time (for the limiters that answer with `rate_limited`).
    pub error_spacing_ms: Option<u64>,
}

/// The source's `media.state`: a bucket of 10, one token per 250 ms.
pub const SOURCE_STATE: Spec = Spec { shape: Shape::Bucket { capacity: 10, per_second: 4 }, error_spacing_ms: None };
/// The viewer's receiving of `media.state`: a bucket of 20, 5 per second, one `rate_limited` per 10 s.
pub const VIEWER_STATE: Spec = Spec { shape: Shape::Bucket { capacity: 20, per_second: 5 }, error_spacing_ms: Some(10_000) };
/// A source's `media.cmd` budget: a bucket of 20, 2 per second, one `rate_limited` per 10 s.
pub const SOURCE_CMD: Spec = Spec { shape: Shape::Bucket { capacity: 20, per_second: 2 }, error_spacing_ms: Some(10_000) };
/// A viewer's `media.cmd`: 20 per 10 s.
pub const VIEWER_CMD: Spec = Spec { shape: Shape::Window { max: 20, window_ms: 10_000 }, error_spacing_ms: None };
/// A source's `media.image`: 20 per 60 s.
pub const IMAGE: Spec = Spec { shape: Shape::Window { max: 20, window_ms: 60_000 }, error_spacing_ms: None };

/// The limiter a vector names.
pub fn spec_named(name: &str) -> Option<Spec> {
    Some(match name {
        "sourceState" => SOURCE_STATE,
        "viewerState" => VIEWER_STATE,
        "sourceCmd" => SOURCE_CMD,
        "viewerCmd" => VIEWER_CMD,
        "image" => IMAGE,
        _ => return None,
    })
}

/// What the limiter says about one event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub allowed: bool,
    /// Over budget, and this is the one error answer of the spacing (the caller sends `rate_limited`).
    pub send_error: bool,
}

#[derive(Debug)]
pub struct Limiter {
    spec: Spec,
    /// Thousandths of a token.
    tokens: u64,
    last_ms: u64,
    times: VecDeque<u64>,
    last_error_ms: Option<u64>,
}

impl Limiter {
    pub fn new(spec: Spec) -> Limiter {
        let tokens = match spec.shape {
            Shape::Bucket { capacity, .. } => capacity * 1000,
            Shape::Window { .. } => 0,
        };
        Limiter { spec, tokens, last_ms: 0, times: VecDeque::new(), last_error_ms: None }
    }

    /// Judges one event at `now_ms`, and uses a token (or a slot of the window) when it is allowed.
    pub fn check(&mut self, now_ms: u64) -> Verdict {
        let allowed = match self.spec.shape {
            Shape::Bucket { capacity, per_second } => {
                // One token per second is one thousandth per millisecond.
                self.tokens = (self.tokens + now_ms.saturating_sub(self.last_ms) * per_second).min(capacity * 1000);
                self.last_ms = self.last_ms.max(now_ms);
                let ok = self.tokens >= 1000;
                if ok {
                    self.tokens -= 1000;
                }
                ok
            }
            Shape::Window { max, window_ms } => {
                while self.times.front().is_some_and(|t| now_ms.saturating_sub(*t) >= window_ms) {
                    self.times.pop_front();
                }
                let ok = self.times.len() < max;
                if ok {
                    self.times.push_back(now_ms);
                }
                ok
            }
        };
        let mut send_error = false;
        if !allowed && let Some(spacing) = self.spec.error_spacing_ms {
            send_error = self.last_error_ms.is_none_or(|at| now_ms.saturating_sub(at) >= spacing);
            if send_error {
                self.last_error_ms = Some(now_ms);
            }
        }
        Verdict { allowed, send_error }
    }
}

/// The source's trailing throttle: the first value goes at once if the last send was `interval_ms` ago or more, else the latest value is held and goes when the
/// interval is over; a value equal to the one last sent is never sent (and cancels a held one).
#[derive(Debug)]
pub struct Throttle<T> {
    interval_ms: u64,
    last_sent_at: Option<u64>,
    last_sent: Option<T>,
    pending: Option<T>,
}

impl<T: Clone + PartialEq> Throttle<T> {
    pub fn new(interval_ms: u64) -> Throttle<T> {
        Throttle { interval_ms, last_sent_at: None, last_sent: None, pending: None }
    }

    /// The value that was held goes now if its time has come: `(the time it goes at, the value)`.
    pub fn flush(&mut self, now_ms: u64) -> Option<(u64, T)> {
        let at = self.last_sent_at? + self.interval_ms;
        if self.pending.is_some() && at <= now_ms {
            let v = self.pending.take()?;
            self.last_sent_at = Some(at);
            self.last_sent = Some(v.clone());
            return Some((at, v));
        }
        None
    }

    /// A new value at `now_ms`. Returns what goes out because of it (a held value whose time came first, and the value itself if it can go at once).
    pub fn push(&mut self, now_ms: u64, value: T) -> Vec<(u64, T)> {
        let mut out: Vec<(u64, T)> = self.flush(now_ms).into_iter().collect();
        if self.last_sent.as_ref() == Some(&value) {
            self.pending = None;
            return out;
        }
        if self.last_sent_at.is_none_or(|t| now_ms.saturating_sub(t) >= self.interval_ms) {
            self.last_sent_at = Some(now_ms);
            self.last_sent = Some(value.clone());
            self.pending = None;
            out.push((now_ms, value));
        } else {
            self.pending = Some(value);
        }
        out
    }

    /// When a held value goes out, if there is one.
    pub fn next_due(&self) -> Option<u64> {
        self.pending.as_ref().and(self.last_sent_at).map(|t| t + self.interval_ms)
    }

    /// Forgets what was sent (a new connection: the viewer has nothing yet).
    pub fn reset(&mut self) {
        self.last_sent_at = None;
        self.last_sent = None;
        self.pending = None;
    }
}

/// The viewer's coalescing of a command that is a stream (a seek bar or a volume slider being dragged): the first goes at once, and while the interval is still
/// running the **last** value is held and goes when it is over. Unlike [`Throttle`] an equal value is not skipped (a command is not a state).
#[derive(Debug)]
pub struct Coalesce<T> {
    interval_ms: u64,
    last_sent_at: Option<u64>,
    pending: Option<T>,
}

impl<T> Coalesce<T> {
    pub fn new(interval_ms: u64) -> Coalesce<T> {
        Coalesce { interval_ms, last_sent_at: None, pending: None }
    }

    /// Returns the value to send now, if any (a held value whose time came goes first, and the new one is then held for the next interval).
    pub fn push(&mut self, now_ms: u64, value: T) -> Vec<T> {
        let mut out: Vec<T> = self.flush(now_ms).into_iter().collect();
        if self.last_sent_at.is_none_or(|t| now_ms.saturating_sub(t) >= self.interval_ms) {
            self.last_sent_at = Some(now_ms);
            self.pending = None;
            out.push(value);
        } else {
            self.pending = Some(value);
        }
        out
    }

    pub fn flush(&mut self, now_ms: u64) -> Option<T> {
        let at = self.last_sent_at? + self.interval_ms;
        if self.pending.is_some() && at <= now_ms {
            self.last_sent_at = Some(at);
            return self.pending.take();
        }
        None
    }

    pub fn next_due(&self) -> Option<u64> {
        self.pending.as_ref().and(self.last_sent_at).map(|t| t + self.interval_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_of_commands_goes_first_at_once_then_the_last_one_per_interval() {
        let mut c = Coalesce::new(150);
        assert_eq!(c.push(0, 10), [10]);
        assert!(c.push(50, 11).is_empty() && c.push(100, 12).is_empty());
        assert_eq!(c.next_due(), Some(150));
        assert_eq!(c.flush(149), None);
        assert_eq!(c.flush(150), Some(12));
        assert_eq!(c.push(160, 12), Vec::<i32>::new(), "an equal value is still a command");
        assert_eq!(c.flush(300), Some(12));
        assert_eq!(c.push(1000, 5), [5]);
    }

    fn run(spec: Spec, events: &[(u64, usize)]) -> (Vec<bool>, Vec<u64>) {
        let mut l = Limiter::new(spec);
        let (mut out, mut errors) = (Vec::new(), Vec::new());
        for (t, n) in events {
            for _ in 0..*n {
                let v = l.check(*t);
                out.push(v.allowed);
                if v.send_error {
                    errors.push(*t);
                }
            }
        }
        (out, errors)
    }

    #[test]
    fn a_burst_uses_the_tokens_and_a_token_comes_back_on_time() {
        let (out, _) = run(SOURCE_STATE, &[(0, 12)]);
        assert_eq!(out.iter().filter(|a| **a).count(), 10);
        let (out, _) = run(SOURCE_STATE, &[(0, 10), (249, 1), (250, 1), (250, 1)]);
        assert_eq!(&out[10..], [false, true, false]);
    }

    #[test]
    fn errors_are_ten_seconds_apart_and_only_for_the_limiters_that_answer() {
        let (_, errors) = run(VIEWER_STATE, &[(0, 21), (5000, 1), (9999, 25), (10_000, 25)]);
        assert_eq!(errors, [0, 10_000]);
        let (_, errors) = run(SOURCE_CMD, &[(0, 25)]);
        assert_eq!(errors, [0]);
        let (_, errors) = run(SOURCE_STATE, &[(0, 50)]);
        assert!(errors.is_empty());
    }

    #[test]
    fn a_window_slides() {
        let (out, _) = run(VIEWER_CMD, &[(0, 10), (5000, 10), (9999, 1), (10_000, 10), (10_000, 1)]);
        assert_eq!(out.iter().filter(|a| **a).count(), 30);
        assert!(!out[20] && out[21..31].iter().all(|a| *a) && !out[31]);
        let (out, _) = run(IMAGE, &[(0, 20), (59_999, 1), (60_000, 21)]);
        assert_eq!(out.iter().filter(|a| **a).count(), 40);
    }

    fn throttled(events: &[(u64, &str)]) -> Vec<(u64, String)> {
        let mut t = Throttle::new(250);
        let mut out: Vec<(u64, String)> = Vec::new();
        for (at, v) in events {
            out.extend(t.push(*at, v.to_string()));
        }
        out.extend(t.flush(u64::MAX));
        out
    }

    #[test]
    fn the_throttle_sends_the_first_at_once_and_the_last_of_a_burst_later() {
        assert_eq!(throttled(&[(0, "A"), (100, "B"), (200, "C")]), [(0, "A".to_string()), (250, "C".to_string())]);
        assert_eq!(throttled(&[(0, "A"), (1000, "A")]), [(0, "A".to_string())], "identical is not sent");
        assert_eq!(throttled(&[(0, "A"), (100, "B"), (200, "A")]), [(0, "A".to_string())], "back to the sent value cancels");
        assert_eq!(throttled(&[(0, "A"), (250, "B")]), [(0, "A".to_string()), (250, "B".to_string())]);
        assert_eq!(throttled(&[(0, "A"), (249, "B")]), [(0, "A".to_string()), (250, "B".to_string())]);
        assert_eq!(throttled(&[(0, "A"), (100, "B"), (260, "C")]), [(0, "A".to_string()), (250, "B".to_string()), (500, "C".to_string())]);
    }

    #[test]
    fn next_due_says_when_to_wake_up() {
        let mut t = Throttle::new(250);
        assert_eq!(t.next_due(), None);
        t.push(0, 1);
        assert_eq!(t.next_due(), None);
        t.push(100, 2);
        assert_eq!(t.next_due(), Some(250));
    }
}
