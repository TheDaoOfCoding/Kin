//! The wire form of `presence`, `ring.status`, `ring.start` and `ring.stop` (protocol.md "Presence, pause and ring", changelog 58): what a frame has to look
//! like and what the receiver ends up with. Pure: no sockets, no Windows. `protocol/presence.json` (`presence[]`) and the `presence-*` and `ring-*` cases of
//! `messages.json` run against it (`tests/vectors.rs`, `tests/presence_vectors.rs`).
//!
//! Nothing here keeps anything: a battery level or a pause time is shown, or dropped, never stored.
use serde_json::{Map, Value, json};

use crate::json::{uint, valid_id};

/// The capability of `hello.capabilities` that turns `presence` on. Every 0.2.0 build lists it.
pub const CAPABILITY: &str = "presence";
/// The capability a phone lists while "Let my PC ring this phone" is on. The PC lists nothing for it.
pub const RING_CAPABILITY: &str = "ring";

/// A received `presence` frame over this many bytes is `error too_large`.
pub const MAX_FRAME_BYTES: usize = 4096;
/// Every `ring.*` frame (`ring.status` too) is limited to this.
pub const MAX_RING_FRAME_BYTES: usize = 2048;
/// A pause is at most 8 days; a longer `remainingMs` is cut to it.
pub const MAX_PAUSE_MS: u64 = 691_200_000;

pub fn is_presence_type(ty: &str) -> bool {
    matches!(ty, "presence" | "ring.status")
}

pub fn is_ring_command(ty: &str) -> bool {
    matches!(ty, "ring.start" | "ring.stop")
}

pub fn frame_limit(ty: &str) -> usize {
    if ty == "presence" { MAX_FRAME_BYTES } else { MAX_RING_FRAME_BYTES }
}

/// The battery of a device as it goes on the wire: `level` 0 to 100, `charging` = plugged in, `saver` = the power-saving mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub level: u8,
    pub charging: bool,
    pub saver: Option<bool>,
}

impl Battery {
    pub fn to_json(self) -> Value {
        let mut v = json!({ "level": self.level, "charging": self.charging });
        if let Some(s) = self.saver {
            v["saver"] = json!(s);
        }
        v
    }
}

/// What a `presence` says about the sender's pause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerPause {
    /// The pause ends this many milliseconds after the message was sent (1 to 8 days' worth).
    Remaining(u64),
    UntilResumed,
}

impl PeerPause {
    pub fn to_json(self) -> Value {
        match self {
            PeerPause::Remaining(ms) => json!({ "remainingMs": ms }),
            PeerPause::UntilResumed => json!({ "untilResumed": true }),
        }
    }
}

/// The phone's Do Not Disturb filter while it is on (changelog 59). Off is not a value: it is the absence of the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dnd {
    Priority,
    Alarms,
    /// "Total silence".
    Total,
}

impl Dnd {
    pub fn word(self) -> &'static str {
        match self {
            Dnd::Priority => "priority",
            Dnd::Alarms => "alarms",
            Dnd::Total => "total",
        }
    }

    /// The word of the wire; any other string is unknown (a later Android filter) and is `None`, not an error.
    pub fn from_word(word: &str) -> Option<Dnd> {
        match word {
            "priority" => Some(Dnd::Priority),
            "alarms" => Some(Dnd::Alarms),
            "total" => Some(Dnd::Total),
            _ => None,
        }
    }
}

/// A whole snapshot: what is true about the sender right now. A field that is absent means "not shared / not applicable" and "not paused".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PresenceBody {
    pub battery: Option<Battery>,
    pub pause: Option<PeerPause>,
    /// Changelog 59, phone to PC: the phone's Do Not Disturb filter while it is on.
    pub dnd: Option<Dnd>,
    /// Changelog 59, PC to phone: `true` only while the PC is in Focus. (`false` is the absence of the field.)
    pub focus: bool,
}

impl PresenceBody {
    /// The normalized content of the vectors (`expect.presence`).
    pub fn to_json(&self) -> Value {
        let mut v = json!({});
        if let Some(b) = self.battery {
            v["battery"] = b.to_json();
        }
        if let Some(p) = self.pause {
            v["pause"] = p.to_json();
        }
        if let Some(d) = self.dnd {
            v["dnd"] = json!(d.word());
        }
        if self.focus {
            v["focus"] = json!(true);
        }
        v
    }
}

/// What the phone says about its ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingStatus {
    pub ringing: bool,
    /// With `stopped`: why (`timeout`, `user`, `pc`, `limit`, `error`, or anything else, which is shown as plain "stopped").
    pub reason: Option<String>,
    /// With `ringing`: whether Android will play something.
    pub audible: bool,
}

impl RingStatus {
    pub fn to_json(&self) -> Value {
        if self.ringing {
            json!({ "state": "ringing", "audible": self.audible })
        } else {
            let mut v = json!({ "state": "stopped" });
            if let Some(r) = &self.reason {
                v["reason"] = json!(r);
            }
            v
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresenceMsg {
    Presence(PresenceBody),
    RingStatus(RingStatus),
}

/// What a `presence` or `ring.status` frame that passed the size rule turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    /// A wrong type or a missing field (`bad_request` with `ref`).
    Bad,
    Ok(PresenceMsg),
}

/// An optional field: absent and `null` are the same.
fn present<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    map.get(key).filter(|v| !v.is_null())
}

/// An optional boolean: absent and `null` are `Ok(None)`, any other type is an error.
fn opt_bool(map: &Map<String, Value>, key: &str) -> Result<Option<bool>, ()> {
    match present(map, key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(()),
    }
}

fn parse_battery(v: &Value) -> Option<Battery> {
    let b = v.as_object()?;
    let level = uint(b.get("level")?).filter(|l| *l <= 100)? as u8;
    // `charging` is required: a missing, null or non-boolean value is invalid.
    let charging = b.get("charging")?.as_bool()?;
    let saver = opt_bool(b, "saver").ok()?;
    Some(Battery { level, charging, saver })
}

fn parse_pause(v: &Value) -> Option<PeerPause> {
    let p = v.as_object()?;
    let remaining = present(p, "remainingMs");
    let until = present(p, "untilResumed");
    match (remaining, until) {
        (Some(ms), None) => {
            let ms = uint(ms).filter(|n| *n >= 1)?;
            Some(PeerPause::Remaining(ms.min(MAX_PAUSE_MS)))
        }
        // Only `true` means "until resumed": `false`, "true" and 1 are invalid.
        (None, Some(Value::Bool(true))) => Some(PeerPause::UntilResumed),
        _ => None,
    }
}

/// The fields of `presence` (`battery` and `pause`, each optional: absent and `null` are the same, anything else of the wrong shape is `Bad`) or of
/// `ring.status` (`state` required, `reason` and `audible` optional). Unknown fields are ignored.
pub fn parse(ty: &str, obj: &Map<String, Value>) -> Parsed {
    if valid_id(obj.get("id")).is_none() {
        return Parsed::Bad;
    }
    match ty {
        "presence" => {
            let battery = match present(obj, "battery") {
                None => None,
                Some(v) => match parse_battery(v) {
                    Some(b) => Some(b),
                    None => return Parsed::Bad,
                },
            };
            let pause = match present(obj, "pause") {
                None => None,
                Some(v) => match parse_pause(v) {
                    Some(p) => Some(p),
                    None => return Parsed::Bad,
                },
            };
            // Changelog 59. `dnd` is a string (a word this table knows, else absent; a wrong type is `Bad`), `focus` a boolean (`true` counts, `false` is absent).
            let dnd = match present(obj, "dnd") {
                None => None,
                Some(Value::String(s)) => Dnd::from_word(s),
                Some(_) => return Parsed::Bad,
            };
            let focus = match opt_bool(obj, "focus") {
                Ok(f) => f == Some(true),
                Err(()) => return Parsed::Bad,
            };
            Parsed::Ok(PresenceMsg::Presence(PresenceBody { battery, pause, dnd, focus }))
        }
        "ring.status" => {
            let ringing = match obj.get("state").and_then(Value::as_str) {
                Some("ringing") => true,
                Some("stopped") => false,
                _ => return Parsed::Bad,
            };
            let reason = match present(obj, "reason") {
                None => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => return Parsed::Bad,
            };
            let audible = match opt_bool(obj, "audible") {
                Ok(a) => a.unwrap_or(true),
                Err(()) => return Parsed::Bad,
            };
            Parsed::Ok(PresenceMsg::RingStatus(RingStatus { ringing, reason: if ringing { None } else { reason }, audible }))
        }
        _ => Parsed::Bad,
    }
}

pub fn presence_frame(id: &str, body: &PresenceBody) -> Value {
    let mut v = body.to_json();
    v["type"] = json!("presence");
    v["id"] = json!(id);
    v
}

pub fn ring_start_frame(id: &str) -> Value {
    json!({ "type": "ring.start", "id": id })
}

pub fn ring_stop_frame(id: &str) -> Value {
    json!({ "type": "ring.stop", "id": id })
}

pub fn vector_name(ty: &str) -> &'static str {
    match ty {
        "presence" => "presence",
        "ring.start" => "ring_start",
        "ring.stop" => "ring_stop",
        _ => "ring_status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(text: &str) -> Map<String, Value> {
        crate::json::parse_object(text).unwrap()
    }

    #[test]
    fn a_presence_with_a_hundred_kilobyte_field_is_too_large_and_a_string_level_is_bad() {
        // The size rule is the receiver's (messages.rs); here is the field rule: a string level is `Bad`.
        let bad = obj(r#"{"type":"presence","id":"a","battery":{"level":"62","charging":true}}"#);
        assert_eq!(parse("presence", &bad), Parsed::Bad);
        const { assert!(MAX_FRAME_BYTES < 100 * 1024) };
    }

    #[test]
    fn a_pause_is_exactly_one_of_remaining_or_until_resumed() {
        let ok = |t: &str| match parse("presence", &obj(t)) {
            Parsed::Ok(PresenceMsg::Presence(b)) => Some(b.pause),
            _ => None,
        };
        assert_eq!(ok(r#"{"id":"a","pause":{"remainingMs":5}}"#), Some(Some(PeerPause::Remaining(5))));
        assert_eq!(ok(r#"{"id":"a","pause":{"untilResumed":true}}"#), Some(Some(PeerPause::UntilResumed)));
        assert_eq!(ok(r#"{"id":"a","pause":{"remainingMs":999999999999}}"#), Some(Some(PeerPause::Remaining(MAX_PAUSE_MS))));
        for bad in [
            r#"{"id":"a","pause":{}}"#,
            r#"{"id":"a","pause":{"remainingMs":1,"untilResumed":true}}"#,
            r#"{"id":"a","pause":{"untilResumed":false}}"#,
            r#"{"id":"a","pause":{"remainingMs":0}}"#,
            r#"{"id":"a","pause":{"remainingMs":1.5}}"#,
            r#"{"id":"a","pause":{"remainingMs":-1}}"#,
            r#"{"id":"a","pause":"1h"}"#,
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_ring_status_keeps_what_belongs_to_its_state() {
        let parse_status = |t: &str| match parse("ring.status", &obj(t)) {
            Parsed::Ok(PresenceMsg::RingStatus(s)) => Some(s.to_json()),
            _ => None,
        };
        assert_eq!(parse_status(r#"{"id":"a","state":"ringing","reason":"user"}"#), Some(json!({"state":"ringing","audible":true})));
        assert_eq!(
            parse_status(r#"{"id":"a","state":"stopped","reason":"timeout","audible":false}"#),
            Some(json!({"state":"stopped","reason":"timeout"}))
        );
        assert_eq!(parse_status(r#"{"id":"a","state":"paused"}"#), None);
        assert_eq!(parse_status(r#"{"id":"a","state":"stopped","reason":3}"#), None);
    }

    #[test]
    fn frames_are_built_the_way_the_vectors_read_them() {
        let body = PresenceBody {
            battery: Some(Battery { level: 62, charging: true, saver: None }),
            pause: Some(PeerPause::Remaining(1000)),
            ..Default::default()
        };
        assert_eq!(
            presence_frame("x", &body),
            json!({"type":"presence","id":"x","battery":{"level":62,"charging":true},"pause":{"remainingMs":1000}})
        );
        assert_eq!(presence_frame("x", &PresenceBody::default()), json!({"type":"presence","id":"x"}));
        assert_eq!(ring_start_frame("r"), json!({"type":"ring.start","id":"r"}));
    }
}
