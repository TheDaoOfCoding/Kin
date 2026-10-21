//! `protocol/presence.json` (changelog 58) against the real code: the validation of a `presence` body, `pauseUntil[]` (with the clock changes of Berlin and New York,
//! read through a zone made of rules), the pause gate table, pause expiry, the low-battery machine, the battery maps, the sender and receiver limits of
//! `presenceRate[]`, the PC's rule for `origin`, and what the PC's ring client does with the phone's `ring[]` statements. (`features[]` is run in `features.rs`;
//! `shot[]` is the phone's, the PC only checks that it is there.)
mod common;

use common::load;
use kin_lib::arrival::{Origin, ToastKind, plan};
use kin_lib::messages::{self, Dedup, Frame, Receiver, Role};
use kin_lib::presence::battery::{map_android, map_windows};
use kin_lib::presence::limits::{PresenceBucket, PresenceSender, Taken};
use kin_lib::presence::nudge::LowBatteryNudge;
use kin_lib::presence::pause::{self, Gate, PauseChoice, PauseState, PeerPauseView, RuleZone, Side};
use kin_lib::presence::wire::{Battery, PeerPause};
use kin_lib::ring::{RingClient, RingState};
use serde_json::{Value, json};

fn presence() -> Value {
    load("presence.json")
}

fn arr<'a>(root: &'a Value, key: &str) -> &'a [Value] {
    root[key].as_array().unwrap_or_else(|| panic!("presence.json has no {key}[]"))
}

struct NoDedup;

impl Dedup for NoDedup {
    fn lookup(&mut self, _id: &str) -> Option<String> {
        None
    }
    fn remember(&mut self, _id: &str, _response: &str) {}
}

#[test]
fn the_limits_of_the_file_are_the_limits_of_the_code() {
    let l = &presence()["limits"];
    assert_eq!(l["frame"], kin_lib::presence::wire::MAX_FRAME_BYTES);
    assert_eq!(l["ringFrame"], kin_lib::presence::wire::MAX_RING_FRAME_BYTES);
    assert_eq!(l["maxPauseMs"], kin_lib::presence::wire::MAX_PAUSE_MS);
    assert_eq!(l["batteryMinStep"], kin_lib::presence::limits::BATTERY_MIN_STEP);
    assert_eq!(l["batteryCrossings"], json!(kin_lib::presence::limits::BATTERY_CROSSINGS));
    assert_eq!(l["batteryEveryMs"], kin_lib::presence::limits::BATTERY_EVERY_MS);
    assert_eq!(l["pauseTrailingMs"], kin_lib::presence::limits::PAUSE_TRAILING_MS);
    assert_eq!(l["receiverCapacity"], kin_lib::presence::limits::RECEIVER_CAPACITY);
    assert_eq!(l["receiverRefillMs"], kin_lib::presence::limits::RECEIVER_REFILL_MS);
    assert_eq!(l["receiverErrorSpacingMs"], kin_lib::presence::limits::RECEIVER_ERROR_SPACING_MS);
    assert_eq!(l["pcRingEveryMs"], kin_lib::ring::MIN_GAP_MS);
    assert_eq!(l["tomorrowHour"], pause::TOMORROW_HOUR);
    assert_eq!(l["tomorrowMinAheadMs"], pause::TOMORROW_MIN_AHEAD_MS);
    assert_eq!(l["nudgeAt"], kin_lib::presence::nudge::NUDGE_AT);
    assert_eq!(l["nudgeRearmAt"], kin_lib::presence::nudge::REARM_AT);
}

#[test]
fn presence_bodies() {
    let root = presence();
    let mut checked = 0;
    for c in arr(&root, "presence") {
        let name = c["name"].as_str().unwrap();
        let text = match (c.get("frame"), c.get("raw")) {
            (Some(f), _) => f.to_string(),
            (None, Some(r)) => r.as_str().unwrap().to_string(),
            _ => panic!("{name}: no input"),
        };
        let mut rx = Receiver::new(Role::Pc);
        rx.set_own_capabilities(&["presence"]);
        let hello = json!({ "type": "hello", "id": "h", "protocol": 1, "name": "Peer", "platform": "android", "appVersion": "0.2.0", "capabilities": ["presence"] });
        rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), &mut NoDedup);
        let out = rx.handle_frame(Frame::Text(text.as_bytes()), &mut NoDedup);
        match c["expect"].as_str().unwrap() {
            "accepted" => match out.action {
                messages::Action::Presence(kin_lib::presence::wire::PresenceMsg::Presence(body)) => {
                    assert_eq!(body.to_json(), c["content"], "{name}");
                    assert!(out.reply.is_none(), "{name}: a presence is never acked");
                }
                other => panic!("{name}: {other:?}"),
            },
            "bad_request" => {
                let reply = out.reply.unwrap_or_else(|| panic!("{name}: expected an error"));
                assert_eq!(reply["code"], "bad_request", "{name}");
            }
            other => panic!("{name}: unknown expectation {other}"),
        }
        checked += 1;
    }
    assert!(checked >= 40, "{checked}");
}

/// `now` is an ISO time with an offset (`2026-03-29T01:30:00+01:00`): the instant, and a way to write an instant back in the zone.
fn parse_iso(s: &str) -> i64 {
    let (date_time, offset) = s.split_at(s.len() - 6);
    let (date, time) = date_time.split_once('T').unwrap();
    let d: Vec<i64> = date.split('-').map(|x| x.parse().unwrap()).collect();
    let t: Vec<i64> = time.split(':').map(|x| x.parse().unwrap()).collect();
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let (oh, om): (i64, i64) = (offset[1..3].parse().unwrap(), offset[4..6].parse().unwrap());
    let local = pause::days_from_civil(d[0], d[1], d[2]) * pause::DAY_MS + t[0] * pause::HOUR_MS + t[1] * 60_000 + t[2] * 1000;
    local - sign * (oh * pause::HOUR_MS + om * 60_000)
}

fn iso_in(zone: &RuleZone, instant_ms: i64) -> String {
    use kin_lib::presence::pause::Zone;
    let off = zone.offset_ms(instant_ms);
    let local = instant_ms + off;
    let (y, m, d) = pause::civil_from_days(local.div_euclid(pause::DAY_MS));
    let rem = local.rem_euclid(pause::DAY_MS);
    let (hh, mm, ss) = (rem / pause::HOUR_MS, rem % pause::HOUR_MS / 60_000, rem % 60_000 / 1000);
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.abs() / 60_000;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}{sign}{:02}:{:02}", a / 60, a % 60)
}

#[test]
fn pause_until() {
    let root = presence();
    for c in arr(&root, "pauseUntil") {
        let name = c["name"].as_str().unwrap();
        let zone = RuleZone::named(c["zone"].as_str().unwrap()).unwrap_or_else(|| panic!("{name}: an unknown zone"));
        let choice = PauseChoice::parse(c["choice"].as_str().unwrap()).unwrap();
        let now = parse_iso(c["now"].as_str().unwrap());
        let PauseState::Until(end) = pause::pause_for(choice, now, &zone) else { panic!("{name}: a timed choice gives a time") };
        assert_eq!(iso_in(&zone, end), c["expect"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn pause_gate_table() {
    let root = presence();
    for c in arr(&root, "pauseGate") {
        let name = c["name"].as_str().unwrap();
        let side = if c["at"] == "send" { Side::Send } else { Side::Receive };
        let got = pause::gate(side, c["kind"].as_str().unwrap(), c["selfPaused"].as_bool().unwrap(), c["peerPaused"].as_bool().unwrap());
        assert_eq!(got.word(), c["expected"].as_str().unwrap(), "{name}");
        assert!(matches!(got, Gate::Send | Gate::Drop | Gate::Quiet | Gate::Flows));
    }
}

#[test]
fn pause_expiry() {
    let root = presence();
    for c in arr(&root, "pauseExpiry") {
        let name = c["name"].as_str().unwrap();
        let expected = c["expected"].as_bool().unwrap();
        if c["side"] == "peer" {
            let arrival = c["arrivalAt"].as_i64().unwrap();
            let pause =
                if c["untilResumed"] == true { PeerPause::UntilResumed } else { PeerPause::Remaining(c["remainingMs"].as_u64().unwrap()) };
            let view = PeerPauseView::new(pause, arrival);
            let now = c["nowAt"].as_i64().unwrap();
            // A `presence` without `pause` that arrived by `now` replaces the view by nothing.
            let resumed_by_presence = c.get("laterPresenceWithoutPauseAt").and_then(Value::as_i64).is_some_and(|t| t <= now);
            assert_eq!(!resumed_by_presence && view.is_paused(now), expected, "{name}");
        } else {
            let now = c["now"].as_i64().unwrap();
            let state = if c["untilResumed"] == true {
                Some(PauseState::UntilResumed)
            } else {
                c.get("pausedUntil").and_then(Value::as_i64).map(PauseState::Until)
            };
            assert_eq!(state.is_some_and(|s| s.is_on(now)), expected, "{name}");
        }
    }
}

#[test]
fn low_battery_machine() {
    let root = presence();
    for c in arr(&root, "lowBattery") {
        let name = c["name"].as_str().unwrap();
        let mut machine = LowBatteryNudge::new();
        let mut nudges = Vec::new();
        for s in c["steps"].as_array().unwrap() {
            if s["none"] == true {
                continue;
            }
            if machine.snapshot(s["level"].as_u64().unwrap() as u8, s["charging"].as_bool().unwrap(), s["paused"] == true) {
                nudges.push(s["t"].as_i64().unwrap());
            }
        }
        assert_eq!(json!(nudges), c["expect"], "{name}");
    }
}

#[test]
fn battery_maps() {
    let root = presence();
    for c in arr(&root, "batteryMap") {
        let name = c["name"].as_str().unwrap();
        let got = if c["platform"] == "android" {
            map_android(
                c["level"].as_i64().unwrap() as i32,
                c["scale"].as_i64().unwrap() as i32,
                c["status"].as_i64().unwrap() as i32,
                c["plugged"].as_i64().unwrap() as i32,
            )
        } else {
            map_windows(
                c["ACLineStatus"].as_u64().unwrap() as u8,
                c["BatteryFlag"].as_u64().unwrap() as u8,
                c["BatteryLifePercent"].as_u64().unwrap() as u8,
            )
        };
        let want = if c["expect"].is_null() {
            None
        } else {
            Some(Battery {
                level: c["expect"]["level"].as_u64().unwrap() as u8,
                charging: c["expect"]["charging"].as_bool().unwrap(),
                saver: None,
            })
        };
        assert_eq!(got, want, "{name}");
    }
}

fn battery_of(v: &Value) -> Option<Battery> {
    v.is_object().then(|| Battery {
        level: v["level"].as_u64().unwrap() as u8,
        charging: v["charging"].as_bool().unwrap(),
        saver: v.get("saver").and_then(Value::as_bool),
    })
}

#[test]
fn presence_rate_senders_and_receivers() {
    let root = presence();
    for c in arr(&root, "presenceRate") {
        let name = c["name"].as_str().unwrap();
        if c["side"] == "sender" {
            let mut sender = PresenceSender::new();
            let mut sent = Vec::new();
            for e in c["events"].as_array().unwrap() {
                let t = e["t"].as_i64().unwrap();
                let out = match e["op"].as_str().unwrap() {
                    "connected" => sender.connected(t),
                    "battery" => sender.set_battery(t, if e["none"] == true { None } else { battery_of(e) }),
                    "pause" => {
                        let state = if e["untilResumed"] == true {
                            Some(PauseState::UntilResumed)
                        } else {
                            e["until"].as_i64().map(PauseState::Until)
                        };
                        sender.set_pause(t, state)
                    }
                    "end" => sender.advance(t),
                    other => panic!("{name}: unknown op {other}"),
                };
                sent.extend(out);
            }
            let got: Vec<Value> = sent
                .iter()
                .map(|s| {
                    let mut v = json!({ "t": s.t });
                    if let Some(b) = s.body.battery {
                        v["battery"] = b.to_json();
                    }
                    if let Some(p) = s.body.pause {
                        v["pause"] = p.to_json();
                    }
                    v
                })
                .collect();
            assert_eq!(json!(got), c["expect"], "{name}");
        } else {
            let mut bucket = PresenceBucket::new();
            let mut runs: Vec<(String, i64)> = Vec::new();
            let mut errors = Vec::new();
            for pair in c["events"].as_array().unwrap() {
                let (t, n) = (pair[0].as_i64().unwrap(), pair[1].as_i64().unwrap());
                for _ in 0..n {
                    let (word, error) = match bucket.take(t) {
                        Taken::Accepted => ("accepted", false),
                        Taken::Dropped { error } => ("dropped", error),
                    };
                    if error {
                        errors.push(t);
                    }
                    match runs.last_mut() {
                        Some((w, count)) if w == word => *count += 1,
                        _ => runs.push((word.to_string(), 1)),
                    }
                }
            }
            let got: Vec<Value> = runs.iter().map(|(w, n)| json!([w, n])).collect();
            assert_eq!(json!(got), c["expect"], "{name}");
            assert_eq!(json!(errors), c["errorsAt"], "{name}");
        }
    }
}

/// The phone's `ring[]` timelines, replayed on the PC's ring client: whatever the phone said last is what the PC shows.
#[test]
fn the_ring_client_follows_what_the_phone_says() {
    let root = presence();
    for c in arr(&root, "ring") {
        let name = c["name"].as_str().unwrap();
        let mut client = RingClient::new();
        let mut last: Option<RingState> = None;
        let mut t = 0i64;
        for out in c["expect"].as_array().unwrap() {
            let at = out[0].as_i64().unwrap();
            t = t.max(at);
            if out[1] == "status" {
                if out[2] == "ringing" {
                    client.on_status(true, out[3].as_bool().unwrap(), None, at, "Pixel 8");
                    last = Some(RingState::Ringing { since: at, audible: out[3].as_bool().unwrap() });
                } else {
                    let reason = out[3].as_str().map(str::to_string);
                    client.on_status(false, true, reason.as_deref(), at, "Pixel 8");
                    last = Some(RingState::Stopped { reason });
                }
                assert_eq!(Some(client.state().clone()), last, "{name}");
            }
        }
        // The vectors' stops always say why (`timeout`, `user`, `pc`, `limit`).
        if let Some(RingState::Stopped { reason }) = last {
            assert!(matches!(reason.as_deref(), Some("timeout" | "user" | "pc" | "limit")), "{name}: {reason:?}");
        }
    }
}

#[test]
fn origin_rules() {
    let root = presence();
    for c in arr(&root, "origin") {
        let name = c["name"].as_str().unwrap();
        let origin = Origin::parse(&c["origin"]);
        let p = plan(origin, c["imagesToClipboard"].as_bool().unwrap(), c["paused"].as_bool().unwrap());
        assert_eq!(p.toast.word(), c["expect"]["toast"].as_str().unwrap(), "{name}");
        assert_eq!(p.clipboard, c["expect"]["clipboard"].as_bool().unwrap(), "{name}");
        assert_eq!(p.sensitive_formats, c["expect"]["sensitiveFormats"].as_bool().unwrap(), "{name}");
    }
    assert_eq!(ToastKind::None.word(), "none");
}

#[test]
fn the_phones_arrays_are_there() {
    let root = presence();
    assert!(arr(&root, "shot").len() >= 20);
    assert!(arr(&root, "features").len() >= 10);
}
