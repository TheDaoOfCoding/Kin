//! What the debug window shows (docs/targets.md): the first-try success rate of sends (target #1), the latency numbers (target #2: tap
//! to `ack`, and the ping round trip), recent errors with their codes, and the connection history.
//!
//! **What counts as a send.** One row of `sends` per `text`, per automatic `clipboard`, per file offer (PC to phone) and per file
//! upload (phone to PC); a `ping` has a row for its latency and its errors but is not a send. **A send is a first-try success** when it
//! went through at its first attempt: it is `ok` with `attempts` = 1. A send that had to be tried again (the phone was offline, no `ack`
//! in 5 s, the transfer had to resume) is `ok` but not a first-try success. A send that ended with an error is `failed`. A send that the
//! user canceled, and one that is still waiting, are not counted either way. The rate is `first-try successes / settled sends` (the sends
//! that are `ok` or `failed`) over the last 30 days. A second rate leaves out the sends that were asked for while the phone was not
//! connected (`offline_at_tap`): those can't succeed at the first try whatever the network does, and targets.md asks for the rate "on a
//! normal home network".
//!
//! No text and no file name is in the send log or in what this file builds.
use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use crate::core::Core;
use crate::encoding::unix_ms;
use crate::queuedb::{ConnectionEvent, SendRow};

/// The success rate covers this many days.
pub const WINDOW_DAYS: i64 = 30;
const DAY_MS: i64 = 24 * 3_600_000;
/// The kinds that are sends. (`ping` is not one.)
const SEND_KINDS: [&str; 4] = ["text", "clipboard", "offer", "upload"];
/// How many recent errors and connection events the window lists.
pub const ERRORS_SHOWN: usize = 20;
pub const CONNECTIONS_SHOWN: usize = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rate {
    /// Sends that ended `ok` or `failed`.
    pub settled: u32,
    /// Of those, the ones that went through at the first attempt.
    pub first_try_ok: u32,
}

impl Rate {
    /// 0 to 100, one decimal. `None` when nothing has settled yet.
    pub fn percent(&self) -> Option<f64> {
        (self.settled > 0).then(|| (self.first_try_ok as f64 * 1000.0 / self.settled as f64).round() / 10.0)
    }

    fn add(&mut self, row: &SendRow) {
        if row.result == "ok" || row.result == "failed" {
            self.settled += 1;
            if row.result == "ok" && row.attempts <= 1 {
                self.first_try_ok += 1;
            }
        }
    }

    fn to_json(self) -> Value {
        json!({ "settled": self.settled, "firstTryOk": self.first_try_ok, "percent": self.percent() })
    }
}

fn is_send(row: &SendRow) -> bool {
    SEND_KINDS.contains(&row.kind.as_str())
}

/// The first-try success rate over `rows`. `connected_only` leaves out the sends that were asked for while the phone was not connected.
pub fn first_try_rate<'a>(rows: impl IntoIterator<Item = &'a SendRow>, connected_only: bool) -> Rate {
    let mut rate = Rate::default();
    for row in rows.into_iter().filter(|r| is_send(r) && !(connected_only && r.offline_at_tap)) {
        rate.add(row);
    }
    rate
}

/// The same, for each kind of send.
pub fn rate_by_kind<'a>(rows: impl IntoIterator<Item = &'a SendRow>, connected_only: bool) -> BTreeMap<String, Rate> {
    let mut by_kind: BTreeMap<String, Rate> = BTreeMap::new();
    for row in rows.into_iter().filter(|r| is_send(r) && !(connected_only && r.offline_at_tap)) {
        by_kind.entry(row.kind.clone()).or_default().add(row);
    }
    by_kind
}

/// Latency numbers over a list of measurements (milliseconds).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Latency {
    pub n: usize,
    pub last_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
}

/// `values` is newest first, so `last` is the first one. The median and the 95th percentile are nearest-rank.
pub fn latency(values: &[f64]) -> Option<Latency> {
    let last = *values.first()?;
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = |p: f64| sorted[((p / 100.0 * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1];
    let round = |v: f64| (v * 10.0).round() / 10.0;
    Some(Latency { n: values.len(), last_ms: round(last), median_ms: round(rank(50.0)), p95_ms: round(rank(95.0)) })
}

fn latencies(rows: &[SendRow], kinds: &[&str]) -> Vec<f64> {
    rows.iter().filter(|r| kinds.contains(&r.kind.as_str())).filter_map(|r| r.latency_ms).collect()
}

impl Core {
    /// Starts the record of a send (when it is first tried, or when it is queued because the phone is offline).
    pub fn record_send_start(&self, id: &str, kind: &str, attempts: i64, offline_at_tap: bool) {
        let row = SendRow {
            id: id.to_string(),
            at: unix_ms() as i64,
            kind: kind.to_string(),
            attempts,
            result: "pending".into(),
            error: None,
            latency_ms: None,
            offline_at_tap,
        };
        if let Err(e) = self.db.upsert_send(&row) {
            log::warn!("saving the send record failed: {e}");
        }
    }

    /// One more attempt at a send that is queued.
    pub fn record_send_attempt(&self, id: &str, kind: &str) {
        let mut row = self.db.get_send(id).ok().flatten().unwrap_or_else(|| SendRow {
            id: id.to_string(),
            at: unix_ms() as i64,
            kind: kind.to_string(),
            attempts: 0,
            result: "pending".into(),
            error: None,
            latency_ms: None,
            offline_at_tap: false,
        });
        row.attempts += 1;
        if let Err(e) = self.db.upsert_send(&row) {
            log::warn!("saving the send record failed: {e}");
        }
    }

    /// Ends the record of a send: `ok`, `failed` (with the error code) or `canceled`, and for a `text` or `clipboard` the tap to `ack`.
    /// `attempts` is only used when there is no record yet (a send that was never started in this database, such as an upload).
    #[allow(clippy::too_many_arguments)]
    pub fn record_send_end(
        &self,
        id: &str,
        kind: &str,
        started_ms: i64,
        attempts: i64,
        result: &str,
        error: Option<&str>,
        latency_ms: Option<f64>,
    ) {
        let mut row = self.db.get_send(id).ok().flatten().unwrap_or_else(|| SendRow {
            id: id.to_string(),
            at: started_ms,
            kind: kind.to_string(),
            attempts,
            result: "pending".into(),
            error: None,
            latency_ms: None,
            offline_at_tap: false,
        });
        if row.result != "pending" {
            return; // a send ends once
        }
        row.result = result.to_string();
        row.error = error.map(str::to_string);
        if latency_ms.is_some() {
            row.latency_ms = latency_ms;
        }
        if let Err(e) = self.db.upsert_send(&row) {
            log::warn!("saving the send record failed: {e}");
        }
    }

    /// An attempt that got no `ack` (the send stays pending): the code is kept for the list of recent errors.
    pub fn record_send_error(&self, id: &str, code: &str) {
        if let Ok(Some(mut row)) = self.db.get_send(id) {
            row.error = Some(code.to_string());
            if let Err(e) = self.db.upsert_send(&row) {
                log::warn!("saving the send record failed: {e}");
            }
        }
    }

    /// Notes a connection event (`connected`, `disconnected`) for the debug window.
    pub fn log_connection(&self, event: &str, detail: &str) {
        if let Err(e) = self.db.log_connection(unix_ms() as i64, event, detail) {
            log::warn!("saving the connection history failed: {e}");
        }
    }

    /// Everything the debug window shows. `network` is the profile of the connected networks (asked of Windows by the caller, on the
    /// blocking pool, because it is a COM call).
    pub fn debug_view(&self, network: Value) -> Value {
        let now = unix_ms() as i64;
        let rows = self.db.sends_since(now - WINDOW_DAYS * DAY_MS).unwrap_or_default();
        let overall = first_try_rate(&rows, false);
        let connected = first_try_rate(&rows, true);
        let by_kind: BTreeMap<String, Value> = rate_by_kind(&rows, false).into_iter().map(|(k, r)| (k, r.to_json())).collect();
        let errors: Vec<Value> = self
            .db
            .recent_send_errors(ERRORS_SHOWN)
            .unwrap_or_default()
            .iter()
            .map(|r| json!({ "at": r.at, "kind": r.kind, "code": r.error, "id": crate::core::short_transfer(&r.id), "result": r.result, "attempts": r.attempts }))
            .collect();
        let events: Vec<ConnectionEvent> = self.db.connection_events(CONNECTIONS_SHOWN).unwrap_or_default();
        let queue = self.queue_status();
        let snap = self.snapshot();
        json!({
            "windowDays": WINDOW_DAYS,
            "successRate": {
                "firstTry": overall.to_json(),
                "firstTryWhileConnected": connected.to_json(),
                "byKind": by_kind,
                "target": 99.0,
            },
            "latency": {
                "tapToAck": latency(&latencies(&rows, &["text", "clipboard"])),
                "ping": latency(&latencies(&rows, &["ping"])),
            },
            "errors": errors,
            "connections": events,
            "network": network,
            "queue": queue,
            "state": snap.state.as_str(),
            "version": crate::core::APP_VERSION,
            "port": self.port(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(kind: &str, attempts: i64, result: &str, offline: bool) -> SendRow {
        SendRow {
            id: "x".into(),
            at: 1,
            kind: kind.into(),
            attempts,
            result: result.into(),
            error: None,
            latency_ms: None,
            offline_at_tap: offline,
        }
    }

    #[test]
    fn first_try_success_is_ok_at_the_first_attempt() {
        let rows = vec![
            send("text", 1, "ok", false),
            send("text", 1, "ok", false),
            send("text", 2, "ok", false), // went through, but not at the first try
            send("offer", 1, "failed", false),
        ];
        let rate = first_try_rate(&rows, false);
        assert_eq!((rate.settled, rate.first_try_ok), (4, 2));
        assert_eq!(rate.percent(), Some(50.0));
    }

    #[test]
    fn pending_canceled_and_pings_are_not_counted() {
        let rows = vec![
            send("text", 1, "ok", false),
            send("text", 0, "pending", false),
            send("offer", 1, "canceled", false),
            send("ping", 1, "ok", false),
            send("ping", 1, "failed", false),
        ];
        assert_eq!(first_try_rate(&rows, false), Rate { settled: 1, first_try_ok: 1 });
        assert_eq!(first_try_rate(&[], false).percent(), None, "no sends yet: no percentage, not 0 or 100");
    }

    #[test]
    fn the_connected_only_rate_leaves_out_sends_asked_for_while_offline() {
        let rows = vec![send("text", 1, "ok", false), send("text", 2, "ok", true), send("offer", 3, "failed", true)];
        assert_eq!(first_try_rate(&rows, false), Rate { settled: 3, first_try_ok: 1 });
        assert_eq!(first_try_rate(&rows, true), Rate { settled: 1, first_try_ok: 1 });
        assert_eq!(first_try_rate(&rows, true).percent(), Some(100.0));
    }

    #[test]
    fn percentages_round_to_one_decimal_and_by_kind_splits_the_rows() {
        let mut rows: Vec<SendRow> = (0..2).map(|_| send("text", 1, "ok", false)).collect();
        rows.push(send("text", 2, "ok", false));
        assert_eq!(first_try_rate(&rows, false).percent(), Some(66.7));
        rows.push(send("upload", 1, "ok", false));
        let by = rate_by_kind(&rows, false);
        assert_eq!(by["text"], Rate { settled: 3, first_try_ok: 2 });
        assert_eq!(by["upload"], Rate { settled: 1, first_try_ok: 1 });
        assert!(!by.contains_key("ping"));
    }

    #[test]
    fn a_hundred_sends_with_one_retry_is_99_percent() {
        let mut rows: Vec<SendRow> = (0..99).map(|_| send("clipboard", 1, "ok", false)).collect();
        rows.push(send("text", 2, "ok", false));
        assert_eq!(first_try_rate(&rows, false).percent(), Some(99.0), "the target of targets.md #1 is met exactly");
    }

    #[test]
    fn latency_numbers_are_nearest_rank() {
        assert_eq!(latency(&[]), None);
        let l = latency(&[40.0, 10.0, 20.0, 30.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0]).unwrap();
        assert_eq!((l.n, l.last_ms, l.median_ms, l.p95_ms), (10, 40.0, 50.0, 100.0));
        let one = latency(&[12.34]).unwrap();
        assert_eq!((one.last_ms, one.median_ms, one.p95_ms), (12.3, 12.3, 12.3));
    }
}
