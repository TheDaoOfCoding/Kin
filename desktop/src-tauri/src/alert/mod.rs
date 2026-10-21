//! Alerts on the PC (milestone 2B.5, protocol.md "Do Not Disturb, Focus and alerts", design sections 3.2, 3.4 and 5.3): "tell me when it's done".
//!
//! An alert is a short text from the user's own script (`Kin.exe --notify "Build finished"`) or from the browser extension ("Download finished") that goes to the
//! phone as a notification. The PC makes it a **text-like item of the queue** (`queue.rs`, `TextBody::Alert`): stored as `queued` with its own expiry before the
//! first attempt, sent in creation order after the `hello`s, re-queued without an ack, and failed locally as `expired` or `peer_unsupported`. It never reaches the
//! phone's clipboard.
//!
//! * [`wire`]: the frame, the phone's side of the rules (for the vectors) and the PC's send decision. Pure.
//! * This file: what a pipe `send` of `kind: "alert"` may carry (`parse_pipe_request`). Pure.
//! * `cli_notify.rs`: the `Kin.exe --notify` client.
//!
//! **What is never logged:** a text, a title or a file name. A log line has the word of the outcome and the expiry (`alert: sent`, `alert: queued, ttl 24h`).
use serde_json::{Map, Value};

use crate::links::clean_link_text;

pub mod wire;

/// Who made an alert, for the wording of the log and the history only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Cli,
    Downloads,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Cli => "cli",
            Source::Downloads => "downloads",
        }
    }
}

/// A `send` of `kind: "alert"` after validation (docs/extension.md, "send"): the text and title are cleaned the way the phone cleans them, so the history of the PC
/// shows what the phone gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeAlert {
    pub text: String,
    pub title: Option<String>,
    pub ttl_sec: u64,
    pub source: Source,
    /// An event-driven alert (the extension's downloads): skipped while the PC is paused.
    pub auto: bool,
}

/// The fields of a pipe `send` of kind `alert`. `Err` is the `BAD_REQUEST` message (nothing is cut silently: a text over 300 characters is refused).
pub fn parse_pipe_request(req: &Map<String, Value>) -> Result<PipeAlert, String> {
    let Some(text) = req.get("text").and_then(Value::as_str) else {
        return Err("There is no text to send.".into());
    };
    let trimmed = text.trim();
    let chars = trimmed.chars().count();
    if chars == 0 {
        return Err("There is no text to send.".into());
    }
    if chars > wire::MAX_TEXT_CHARS {
        return Err(format!("An alert is at most {} characters; this one has {chars}.", wire::MAX_TEXT_CHARS));
    }
    let Some(text) = clean_link_text(trimmed, wire::MAX_TEXT_CHARS) else {
        return Err("There is no text to send.".into());
    };
    let title = match req.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => {
            let t = t.trim();
            let n = t.chars().count();
            if n > wire::MAX_TITLE_CHARS {
                return Err(format!("A title is at most {} characters; this one has {n}.", wire::MAX_TITLE_CHARS));
            }
            clean_link_text(t, wire::MAX_TITLE_CHARS)
        }
        Some(_) => return Err("The title must be a string.".into()),
    };
    let ttl_sec = match req.get("ttlSec") {
        None | Some(Value::Null) => wire::TTL_DEFAULT_SEC,
        Some(v) => match v.as_u64() {
            Some(s) if (wire::TTL_MIN_SEC..=wire::TTL_MAX_SEC).contains(&s) => s,
            _ => return Err("ttlSec must be a whole number from 60 to 604800.".into()),
        },
    };
    let source = match req.get("source").and_then(Value::as_str) {
        Some("downloads") => Source::Downloads,
        _ => Source::Cli,
    };
    let auto = match req.get("auto") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("auto must be true or false.".into()),
    };
    Ok(PipeAlert { text, title, ttl_sec, source, auto })
}

/// "24h", "30m", "3d", "90s": how long an alert may wait, for the log and the CLI's line. Whole hours up to 48 h are hours (the default is "24h", as the design says), days beyond.
pub fn ttl_word(ttl_sec: u64) -> String {
    if ttl_sec.is_multiple_of(86_400) && ttl_sec > 48 * 3_600 {
        format!("{}d", ttl_sec / 86_400)
    } else if ttl_sec.is_multiple_of(3_600) {
        format!("{}h", ttl_sec / 3_600)
    } else if ttl_sec.is_multiple_of(60) {
        format!("{}m", ttl_sec / 60)
    } else {
        format!("{ttl_sec}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(text: &str) -> Map<String, Value> {
        crate::json::parse_object(text).unwrap()
    }

    #[test]
    fn a_pipe_alert_has_defaults_and_is_cleaned() {
        let a = parse_pipe_request(&obj(r#"{"text":"  Build\nfinished  ","title":" CI "}"#)).unwrap();
        assert_eq!(
            a,
            PipeAlert { text: "Build finished".into(), title: Some("CI".into()), ttl_sec: 86_400, source: Source::Cli, auto: false }
        );
        let d = parse_pipe_request(&obj(r#"{"text":"x","ttlSec":1800,"source":"downloads","auto":true}"#)).unwrap();
        assert_eq!((d.ttl_sec, d.source, d.auto), (1800, Source::Downloads, true));
        let unknown_source = parse_pipe_request(&obj(r#"{"text":"x","source":"toaster"}"#)).unwrap();
        assert_eq!(unknown_source.source, Source::Cli, "another value is ignored as absent");
    }

    #[test]
    fn nothing_is_cut_silently() {
        let long = "a".repeat(301);
        let e = parse_pipe_request(&obj(&format!(r#"{{"text":"{long}"}}"#))).unwrap_err();
        assert!(e.contains("301"), "{e}");
        assert!(parse_pipe_request(&obj(&format!(r#"{{"text":"{}"}}"#, "a".repeat(300)))).is_ok());
        let e = parse_pipe_request(&obj(&format!(r#"{{"text":"x","title":"{}"}}"#, "t".repeat(61)))).unwrap_err();
        assert!(e.contains("61"), "{e}");
    }

    #[test]
    fn bad_fields_are_refused() {
        for bad in [
            r#"{}"#,
            r#"{"text":""}"#,
            r#"{"text":"   "}"#,
            r#"{"text":5}"#,
            r#"{"text":"x","title":5}"#,
            r#"{"text":"x","ttlSec":59}"#,
            r#"{"text":"x","ttlSec":604801}"#,
            r#"{"text":"x","ttlSec":1.5}"#,
            r#"{"text":"x","ttlSec":"60"}"#,
            r#"{"text":"x","ttlSec":-60}"#,
            r#"{"text":"x","auto":"yes"}"#,
        ] {
            assert!(parse_pipe_request(&obj(bad)).is_err(), "{bad}");
        }
        for good in
            [r#"{"text":"x","ttlSec":60}"#, r#"{"text":"x","ttlSec":604800}"#, r#"{"text":"x","title":null,"ttlSec":null,"auto":null}"#]
        {
            assert!(parse_pipe_request(&obj(good)).is_ok(), "{good}");
        }
    }

    #[test]
    fn the_ttl_is_written_in_the_biggest_whole_unit() {
        assert_eq!(ttl_word(86_400), "24h");
        assert_eq!(ttl_word(172_800), "48h");
        assert_eq!(ttl_word(172_800 + 3_600), "49h");
        assert_eq!(ttl_word(259_200), "3d");
        assert_eq!(ttl_word(7_200), "2h");
        assert_eq!(ttl_word(1_800), "30m");
        assert_eq!(ttl_word(90), "90s");
        assert_eq!(ttl_word(604_800), "7d");
    }
}
