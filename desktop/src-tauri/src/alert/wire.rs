//! The wire form of `alert` (protocol.md "Do Not Disturb, Focus and alerts", changelog 59) and the PC's decision when an alert item is about to go on the wire.
//! Pure: no sockets, no clock of its own. `protocol/alerts.json` (`alert[]`, `alertSend[]`) and the `alert-*` cases of `messages.json` run against it
//! (`tests/alerts_vectors.rs`).
//!
//! An alert is a text of the user's own script ("Build finished") that goes to the phone as a notification. It is the class of a `text`: acked, de-duplicated,
//! queued, re-sent. It never reaches the phone's clipboard.
use serde_json::{Map, Value, json};

use crate::json::{uint, valid_id};
use crate::links::clean_link_text;

/// The capability the phone lists in every `hello`. The PC lists nothing for it.
pub const CAPABILITY: &str = "alert";

/// A received `alert` frame over this many bytes is `error too_large`.
pub const MAX_FRAME_BYTES: usize = 4096;
/// The text is cut to this many code points (the CLI never cuts silently: over it, it fails).
pub const MAX_TEXT_CHARS: usize = 300;
pub const MAX_TITLE_CHARS: usize = 60;
/// `waitedMs` is cut to 7 days.
pub const MAX_WAITED_MS: u64 = 604_800_000;
/// A sender leaves `waitedMs` out when the item waited less than this.
pub const WAITED_OMIT_BELOW_MS: i64 = 1_000;
/// Expiry, as the sender chooses it (`--ttl`, `ttlSec`).
pub const TTL_DEFAULT_SEC: u64 = 86_400;
pub const TTL_MIN_SEC: u64 = 60;
pub const TTL_MAX_SEC: u64 = 604_800;
/// The extension's downloads wait this long (informational: the extension sends it as `ttlSec`).
pub const DOWNLOAD_TTL_SEC: u64 = 1_800;

/// What an `alert` frame says after cleaning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertMsg {
    pub text: String,
    pub title: Option<String>,
    pub waited_ms: Option<u64>,
}

impl AlertMsg {
    /// The normalized content of the vectors (`expect.alert`).
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "text": self.text });
        if let Some(t) = &self.title {
            v["title"] = json!(t);
        }
        if let Some(w) = self.waited_ms {
            v["waitedMs"] = json!(w);
        }
        v
    }
}

/// The fields of an `alert` (the `id` is checked by the caller): `text` is a string that is not empty after cleaning, `title` an optional string (`null` is absent, an
/// empty one after cleaning is absent), `waitedMs` an optional non-negative integer, cut to 7 days. `None` is `bad_request`.
pub fn parse(obj: &Map<String, Value>) -> Option<AlertMsg> {
    valid_id(obj.get("id"))?;
    let text = clean_link_text(obj.get("text")?.as_str()?, MAX_TEXT_CHARS)?;
    let title = match obj.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => clean_link_text(s, MAX_TITLE_CHARS),
        Some(_) => return None,
    };
    let waited_ms = match obj.get("waitedMs") {
        None | Some(Value::Null) => None,
        Some(v) => Some(uint(v)?.min(MAX_WAITED_MS)),
    };
    Some(AlertMsg { text, title, waited_ms })
}

/// The frame the PC sends. `waited_ms` is already decided by [`decide`].
pub fn alert_frame(id: &str, text: &str, title: Option<&str>, waited_ms: Option<u64>) -> Value {
    let mut v = json!({ "type": "alert", "id": id, "text": text });
    if let Some(t) = title {
        v["title"] = json!(t);
    }
    if let Some(w) = waited_ms {
        v["waitedMs"] = json!(w);
    }
    v
}

/// What the PC does with an alert item that is about to go on the wire (3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendDecision {
    /// Over its expiry: it fails locally as `expired` and is never sent. Decided first, connected or not.
    Expired,
    /// The phone is not connected: it stays queued.
    Queued,
    /// The phone is connected but its latest `hello` lacks `alert`: it fails locally as `peer_unsupported`, nothing is sent, and **never as a text** (a text would be
    /// copied to the phone's clipboard).
    PeerUnsupported,
    /// Goes now. `waited_ms`: how long it waited, cut to 7 days, left out under 1 s.
    Send { waited_ms: Option<u64> },
}

pub fn decide(now_ms: i64, created_ms: i64, expires_ms: i64, connected: bool, peer_has_alert: bool) -> SendDecision {
    if now_ms >= expires_ms {
        return SendDecision::Expired;
    }
    if !connected {
        return SendDecision::Queued;
    }
    if !peer_has_alert {
        return SendDecision::PeerUnsupported;
    }
    let waited = (now_ms - created_ms).max(0);
    SendDecision::Send { waited_ms: (waited >= WAITED_OMIT_BELOW_MS).then(|| (waited as u64).min(MAX_WAITED_MS)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(text: &str) -> Map<String, Value> {
        crate::json::parse_object(text).unwrap()
    }

    #[test]
    fn an_alert_is_cleaned_and_cut() {
        let frame = format!(r#"{{"id":"a","title":"{}Render  ","text":"line one\nline two","waitedMs":999999999999}}"#, '\u{202e}');
        let m = parse(&obj(&frame)).unwrap();
        assert_eq!(m.to_json(), json!({"title":"_Render","text":"line one line two","waitedMs":604_800_000u64}));
        assert!(parse(&obj(r#"{"id":"a","text":" \n "}"#)).is_none(), "empty after cleaning");
        assert!(parse(&obj(r#"{"id":"a","text":"x","title":5}"#)).is_none());
        assert!(parse(&obj(r#"{"id":"a","text":"x","waitedMs":-1}"#)).is_none());
        assert!(parse(&obj(r#"{"id":"a","text":"x","waitedMs":1.5}"#)).is_none());
        assert!(parse(&obj(r#"{"text":"x"}"#)).is_none(), "an id is required");
    }

    #[test]
    fn the_frame_has_a_title_and_waited_only_when_there_are_some() {
        assert_eq!(alert_frame("i", "t", None, None), json!({"type":"alert","id":"i","text":"t"}));
        assert_eq!(alert_frame("i", "t", Some("T"), Some(5)), json!({"type":"alert","id":"i","text":"t","title":"T","waitedMs":5}));
    }

    #[test]
    fn the_decision_is_expiry_then_connection_then_capability() {
        // expired wins over everything, at exactly the expiry
        assert_eq!(decide(100, 0, 100, true, false), SendDecision::Expired);
        assert_eq!(decide(99, 0, 100, false, false), SendDecision::Queued);
        assert_eq!(decide(99, 0, 100, true, false), SendDecision::PeerUnsupported);
        assert_eq!(decide(999, 0, 100_000, true, true), SendDecision::Send { waited_ms: None });
        assert_eq!(decide(1000, 0, 100_000, true, true), SendDecision::Send { waited_ms: Some(1000) });
        // a clock that jumped: cut to 7 days
        assert_eq!(decide(700_000_000, 0, i64::MAX, true, true), SendDecision::Send { waited_ms: Some(MAX_WAITED_MS) });
    }
}
