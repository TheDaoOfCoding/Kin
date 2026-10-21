//! `protocol/alerts.json` (milestone 2B.5, changelog 59) against the real code: the cleaning of an `alert` (through the real receiver), the PC's send decision, what
//! "the PC is in Focus" means with its 3 s filter, which toasts are held, the `Kin.exe --notify` command line and exit codes, and `presenceDnd[]` of `presence.json`.
//! The `alert-*` frames themselves run in `vectors.rs`. The arrays that only the phone runs (`alertRate[]`, `dndMap[]`, `dndRequest[]`, `dndFollow[]`) and the
//! browser's (`downloadAlert[]`) are only checked to be there.
mod common;

use common::load;
use kin_lib::alert::wire::{self as alert, SendDecision};
use kin_lib::cli_notify::{self, DeviceMatch};
use kin_lib::focus::hold::{ToastKind, is_held};
use kin_lib::focus::state::{self, StableFocus, is_focus, snapshot_focus};
use kin_lib::messages::{self, Dedup, Frame, Receiver, Role};
use kin_lib::presence::wire::Dnd;
use kin_lib::quiet::UserState;
use serde_json::{Value, json};

fn alerts() -> Value {
    load("alerts.json")
}

fn arr<'a>(root: &'a Value, key: &str) -> &'a [Value] {
    root[key].as_array().unwrap_or_else(|| panic!("alerts.json has no {key}[]"))
}

/// `[[string, count], ...]` as one string.
fn parts(v: &Value) -> String {
    v.as_array().unwrap().iter().map(|p| p[0].as_str().unwrap().repeat(p[1].as_u64().unwrap() as usize)).collect()
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
    let l = &alerts()["limits"];
    assert_eq!(l["frame"], alert::MAX_FRAME_BYTES);
    assert_eq!(l["textMax"], alert::MAX_TEXT_CHARS);
    assert_eq!(l["titleMax"], alert::MAX_TITLE_CHARS);
    assert_eq!(l["waitedMaxMs"], alert::MAX_WAITED_MS);
    assert_eq!(l["waitedOmitBelowMs"], alert::WAITED_OMIT_BELOW_MS);
    assert_eq!(l["ttlDefaultSec"], alert::TTL_DEFAULT_SEC);
    assert_eq!(l["ttlMinSec"], alert::TTL_MIN_SEC);
    assert_eq!(l["ttlMaxSec"], alert::TTL_MAX_SEC);
    assert_eq!(l["focusStableMs"], state::STABLE_MS);
    assert_eq!(l["focusPollMs"], state::POLL_MS);
    assert_eq!(l["pipeSendsPerWindow"], kin_lib::nativehost::api::SENDS_PER_WINDOW);
    assert_eq!(l["pipeWindowMs"], kin_lib::nativehost::api::SEND_WINDOW.as_millis() as u64);
    assert_eq!(l["downloadTtlSec"], alert::DOWNLOAD_TTL_SEC);
}

#[test]
fn alert_frames_are_cleaned_by_the_real_receiver() {
    let root = alerts();
    let mut checked = 0;
    for c in arr(&root, "alert") {
        let name = c["name"].as_str().unwrap();
        let text = match (c.get("frame"), c.get("raw")) {
            (Some(f), _) => {
                let mut f = f.clone();
                if let Some(p) = c.get("frameParts") {
                    for (field, pieces) in p.as_object().unwrap() {
                        f[field] = Value::String(parts(pieces));
                    }
                }
                f.to_string()
            }
            (None, Some(r)) => r.as_str().unwrap().to_string(),
            _ => panic!("{name}: no input"),
        };
        let mut rx = Receiver::new(Role::Phone);
        rx.set_own_capabilities(&["alert"]);
        let hello = json!({ "type": "hello", "id": "h", "protocol": 1, "name": "Desk PC", "platform": "windows", "appVersion": "0.2.0" });
        rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), &mut NoDedup);
        let out = rx.handle_frame(Frame::Text(text.as_bytes()), &mut NoDedup);
        match c["expect"].as_str().unwrap() {
            "accepted" => match &out.action {
                messages::Action::DeliverAlert { alert, .. } => {
                    let mut want = c.get("content").cloned().unwrap_or_else(|| json!({}));
                    if let Some(p) = c.get("contentParts") {
                        for (field, pieces) in p.as_object().unwrap() {
                            want[field] = Value::String(parts(pieces));
                        }
                    }
                    assert_eq!(alert.to_json(), want, "{name}");
                    assert_eq!(out.reply.as_ref().map(|r| r["type"].clone()), Some(json!("ack")), "{name}: acked");
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
    assert!(checked >= 38, "{checked}");
}

#[test]
fn the_send_decision() {
    let root = alerts();
    for c in arr(&root, "alertSend") {
        let name = c["name"].as_str().unwrap();
        let caps = c["peerCaps"].as_array().unwrap();
        let has = caps.iter().any(|x| x == alert::CAPABILITY);
        let created = c["createdMs"].as_i64().unwrap();
        let expires = created + c["ttlMs"].as_i64().unwrap();
        let got = alert::decide(c["nowMs"].as_i64().unwrap(), created, expires, c["connected"].as_bool().unwrap(), has);
        let want = &c["expect"];
        match (want["result"].as_str().unwrap(), got) {
            ("expired", SendDecision::Expired) | ("queued", SendDecision::Queued) | ("peer_unsupported", SendDecision::PeerUnsupported) => {
            }
            ("sent", SendDecision::Send { waited_ms }) => assert_eq!(waited_ms, want["waitedMs"].as_u64(), "{name}"),
            (w, g) => panic!("{name}: expected {w}, got {g:?}"),
        }
    }
}

fn quiet_of(word: &str) -> UserState {
    match word {
        "accepts" => UserState::AcceptsNotifications,
        "not_present" => UserState::NotPresent,
        "busy" => UserState::Busy,
        "fullscreen" => UserState::RunningD3dFullScreen,
        "presentation" => UserState::PresentationMode,
        "quiet_time" => UserState::QuietTime,
        "app" => UserState::RunningWindowsStoreApp,
        other => panic!("unknown quiet state {other}"),
    }
}

#[test]
fn what_it_means_for_the_pc_to_be_in_focus() {
    let root = alerts();
    for c in arr(&root, "focusState") {
        let got =
            is_focus(c["focusSession"].as_bool().unwrap(), quiet_of(c["quiet"].as_str().unwrap()), c["alsoPresenting"].as_bool().unwrap());
        assert_eq!(got, c["expect"].as_bool().unwrap(), "{}", c["name"]);
    }
    for c in arr(&root, "focusSnapshot") {
        let got = snapshot_focus(
            c["shareOn"].as_bool().unwrap(),
            c["stable"].as_bool().unwrap(),
            c["paused"].as_bool().unwrap(),
            c["peerHasPresence"].as_bool().unwrap(),
        );
        assert_eq!(got, c["expect"].as_bool().unwrap(), "{}", c["name"]);
    }
}

#[test]
fn the_three_second_filter() {
    let root = alerts();
    for c in arr(&root, "focusTimeline") {
        let name = c["name"].as_str().unwrap();
        let also = c["alsoPresenting"].as_bool().unwrap();
        let mut filter = StableFocus::new();
        let mut sent: Vec<Value> = Vec::new();
        for s in c["samples"].as_array().unwrap() {
            let raw = is_focus(s["focusSession"].as_bool().unwrap(), quiet_of(s["quiet"].as_str().unwrap()), also);
            if let Some((t, focus)) = filter.feed(s["t"].as_i64().unwrap(), raw) {
                sent.push(json!({ "t": t, "focus": focus }));
            }
        }
        if let Some((t, focus)) = filter.advance(c["endAt"].as_i64().unwrap()) {
            sent.push(json!({ "t": t, "focus": focus }));
        }
        assert_eq!(Value::Array(sent), c["expect"], "{name}");
    }
}

#[test]
fn which_toasts_are_held() {
    let root = alerts();
    for c in arr(&root, "holdToasts") {
        let name = c["name"].as_str().unwrap();
        let dnd = c["dnd"].as_str().map(|w| Dnd::from_word(w).unwrap_or_else(|| panic!("{name}: unknown dnd word")));
        // A kind this build does not know is shown (held is for the two named kinds only).
        let held = match ToastKind::from_word(c["kind"].as_str().unwrap()) {
            Some(kind) => is_held(dnd, c["setting"].as_bool().unwrap(), kind),
            None => false,
        };
        assert_eq!(if held { "held" } else { "shown" }, c["expect"], "{name}");
    }
}

fn args_of(c: &Value) -> Vec<String> {
    c["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| match a.as_str() {
            Some(s) => s.to_string(),
            None => parts(&a["parts"]),
        })
        .collect()
}

#[test]
fn the_notify_command_line() {
    let root = alerts();
    for c in arr(&root, "notifyCli") {
        let name = c["name"].as_str().unwrap();
        let stdin: Vec<u8> = match (c.get("stdinParts"), c.get("stdinHex").and_then(Value::as_str)) {
            (Some(p), _) => parts(p).into_bytes(),
            (None, Some(hex)) => (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect(),
            _ => Vec::new(),
        };
        let got = cli_notify::parse_args(&args_of(c), || stdin);
        let want = &c["expect"];
        match (want["kind"].as_str().unwrap(), got) {
            ("send", Ok(a)) => {
                if let Some(text) = want.get("text") {
                    assert_eq!(&json!(a.text), text, "{name}");
                }
                if let Some(n) = want.get("textLength") {
                    assert_eq!(json!(a.text.chars().count()), *n, "{name}");
                }
                if let Some(n) = want.get("titleLength") {
                    assert_eq!(json!(a.title.as_deref().unwrap_or("").chars().count()), *n, "{name}");
                } else {
                    assert_eq!(a.title.as_deref(), want.get("title").and_then(Value::as_str), "{name}: title");
                }
                assert_eq!(json!(a.ttl_sec), want["ttlSec"], "{name}: ttl");
                assert_eq!(a.device.as_deref(), want.get("device").and_then(Value::as_str), "{name}: device");
                assert_eq!(a.no_start, want.get("noStart").and_then(Value::as_bool).unwrap_or(false), "{name}: --no-start");
            }
            ("usage", Err(u)) => {
                assert_eq!(json!(u.exit), want["exit"], "{name}: exit code");
                assert_eq!(json!(u.reason), want["reason"], "{name}: reason");
                if let Some(n) = want.get("count") {
                    assert_eq!(json!(u.count), *n, "{name}: count");
                }
                assert!(!u.message().is_empty(), "{name}: a message");
            }
            (kind, got) => panic!("{name}: expected {kind}, got {got:?}"),
        }
    }
}

#[test]
fn the_device_selector() {
    let root = alerts();
    for c in arr(&root, "notifyDevice") {
        let devices: Vec<(String, String)> = c["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["id"].as_str().unwrap().to_string(), d["name"].as_str().unwrap().to_string()))
            .collect();
        let got = cli_notify::match_device(c["selector"].as_str().unwrap(), &devices);
        let want = &c["expect"];
        match (want["result"].as_str().unwrap(), got) {
            ("match", DeviceMatch::Match(id)) => assert_eq!(json!(id), want["id"], "{}", c["name"]),
            ("ambiguous", DeviceMatch::Ambiguous) | ("none", DeviceMatch::None) => {}
            (w, g) => panic!("{}: expected {w}, got {g:?}", c["name"]),
        }
    }
}

#[test]
fn the_exit_code_of_an_answer() {
    let root = alerts();
    for c in arr(&root, "notifyExit") {
        let response = if c["response"].is_null() { None } else { Some(&c["response"]) };
        let (exit, word) = cli_notify::exit_for(response);
        assert_eq!(json!(exit), c["expect"]["exit"], "{}", c["name"]);
        assert_eq!(json!(word.vector_name()), c["expect"]["word"], "{}", c["name"]);
    }
}

#[test]
fn presence_with_dnd_and_focus() {
    let root = load("presence.json");
    let mut checked = 0;
    for c in arr(&root, "presenceDnd") {
        let name = c["name"].as_str().unwrap();
        let text = c["frame"].to_string();
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
            "bad_request" => assert_eq!(out.reply.unwrap_or_else(|| panic!("{name}: expected an error"))["code"], "bad_request", "{name}"),
            other => panic!("{name}: unknown expectation {other}"),
        }
        checked += 1;
    }
    assert!(checked >= 22, "{checked}");
}

/// The arrays only the phone (or the browser) runs are in the file, so a dropped vector is noticed here.
#[test]
fn the_arrays_for_the_other_sides_are_there() {
    let root = alerts();
    for (key, at_least) in [("alertRate", 7), ("dndMap", 7), ("dndRequest", 4), ("dndFollow", 25), ("downloadAlert", 20)] {
        assert!(arr(&root, key).len() >= at_least, "{key}[] has fewer than {at_least} cases");
    }
}
