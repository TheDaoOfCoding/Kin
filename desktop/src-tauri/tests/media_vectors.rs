//! `protocol/media.json` (milestone 2B.3, changelog 57) against the real code: `sid`, field cleaning, the image rules (through the real receiver), the position
//! rule on both sides, which session is shown, the throttle and the five limiters, the Windows maps, the volume steps, and who may send what and what a source
//! answers to a command. The `media-*` frames themselves run in `vectors.rs`. The arrays that only the phone runs (`androidMap[]`, `toAndroid[]`, the Android
//! parts of `sourcePosition[]`, `volumeLevel[]` and `volumeSet[]`) are skipped here.
mod common;

use common::*;
use kin_lib::encoding::base64url;
use kin_lib::links::clean_link_text;
use kin_lib::media::limits::{self, Limiter, Throttle};
use kin_lib::media::map::{Controls, SmtcStatus, windows_to_wire, wire_to_smtc};
use kin_lib::media::pick::{Candidate, ClearDebounce, pick};
use kin_lib::media::policy::{self, Kind, PeerStatus, Verdict};
use kin_lib::media::position::{shown, windows_source_position};
use kin_lib::media::volume::{level_of_scalar, target_level};
use kin_lib::media::wire::{self, Action, Cmd, CmdKind, PlayState, VolumeOp, sid_of};
use kin_lib::messages::{self, Dedup, Frame, Receiver, Role};
use serde_json::{Value, json};

struct NoDedup;

impl Dedup for NoDedup {
    fn lookup(&mut self, _: &str) -> Option<String> {
        None
    }

    fn remember(&mut self, _: &str, _: &str) {}
}

fn arr<'a>(root: &'a Value, key: &str) -> &'a Vec<Value> {
    let a = root[key].as_array().unwrap_or_else(|| panic!("media.json {key}[]"));
    assert!(!a.is_empty(), "media.json {key}[] is empty");
    a
}

fn name(c: &Value) -> &str {
    c["name"].as_str().unwrap()
}

fn state_of(s: &str) -> PlayState {
    PlayState::parse(s).unwrap_or_else(|| panic!("a state: {s}"))
}

fn actions_of(v: &Value) -> Vec<Action> {
    v.as_array().unwrap().iter().map(|a| Action::parse(a.as_str().unwrap()).unwrap()).collect()
}

fn action_names(a: &[Action]) -> Vec<&'static str> {
    a.iter().map(|x| x.as_str()).collect()
}

fn parts(list: &Value) -> String {
    list.as_array()
        .unwrap()
        .iter()
        .map(|p| match p {
            Value::String(s) => s.clone(),
            Value::Array(a) => a[0].as_str().unwrap().repeat(a[1].as_u64().unwrap() as usize),
            other => panic!("a part: {other}"),
        })
        .collect()
}

/// A receiver of the PC that has seen the phone's `hello` (both list `media`).
fn pc_receiver() -> (Receiver, NoDedup) {
    let mut dedup = NoDedup;
    let mut rx = Receiver::new(Role::Pc);
    let hello = json!({ "type": "hello", "id": "h", "protocol": 1, "name": "Phone", "platform": "android", "appVersion": "0.2.0", "capabilities": ["media"] });
    rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), &mut dedup);
    rx.set_own_capabilities(&["media"]);
    (rx, dedup)
}

#[test]
fn limits_and_limiters_match_the_spec() {
    let d = load("media.json");
    let l = &d["limits"];
    let want = |k: &str| l[k].as_u64().unwrap_or_else(|| panic!("limit {k}"));
    assert_eq!(want("app") as usize, wire::MAX_APP);
    assert_eq!(want("title") as usize, wire::MAX_TITLE);
    assert_eq!(want("artist") as usize, wire::MAX_ARTIST);
    assert_eq!(want("album") as usize, wire::MAX_ALBUM);
    assert_eq!(want("actions") as usize, wire::MAX_ACTIONS_LOOKED_AT);
    assert_eq!(want("durationMs"), wire::MAX_DURATION_MS);
    assert_eq!(want("positionMs"), wire::MAX_POSITION_MS);
    assert_eq!(want("rateMilli"), wire::MAX_RATE_MILLI);
    assert_eq!(want("seekMs"), wire::MAX_SEEK_MS);
    assert_eq!(want("stateFrameSend") as usize, wire::MAX_STATE_SEND_BYTES);
    assert_eq!(want("stateFrameRecv") as usize, wire::MAX_FRAME_BYTES);
    assert_eq!(want("imageFrame") as usize, wire::MAX_IMAGE_FRAME_BYTES);
    assert_eq!(want("imageBytes") as usize, wire::MAX_IMAGE_BYTES);
    assert_eq!(want("artPixels") as u32, wire::ART_MAX_PIXELS);
    assert_eq!(want("iconPixels") as u32, wire::ICON_MAX_PIXELS);
    assert_eq!(want("imageMinPixels") as u32, wire::IMAGE_MIN_PIXELS);
    assert_eq!(want("pausedExpiryMs"), kin_lib::media::pick::PAUSED_EXPIRY_MS);
    assert_eq!(want("clearDebounceMs"), kin_lib::media::pick::CLEAR_DEBOUNCE_MS);
    assert_eq!(want("positionJumpMs"), kin_lib::media::position::JUMP_MS);
    assert_eq!(want("endGraceMs"), kin_lib::media::position::END_GRACE_MS);
    assert_eq!(want("throttleMs"), kin_lib::media::source::THROTTLE_MS);
    for (key, spec) in d["limiters"].as_object().unwrap() {
        let mine = limits::spec_named(key).unwrap_or_else(|| panic!("limiter {key}"));
        match (spec["type"].as_str().unwrap(), mine.shape) {
            ("bucket", limits::Shape::Bucket { capacity, per_second }) => {
                assert_eq!((spec["capacity"].as_u64(), spec["perSecond"].as_u64()), (Some(capacity), Some(per_second)), "{key}");
            }
            ("window", limits::Shape::Window { max, window_ms }) => {
                assert_eq!((spec["max"].as_u64(), spec["windowMs"].as_u64()), (Some(max as u64), Some(window_ms)), "{key}");
            }
            other => panic!("{key}: the shape differs: {other:?}"),
        }
        assert_eq!(spec["errorSpacingMs"].as_u64(), mine.error_spacing_ms, "{key}");
    }
}

#[test]
fn sid_and_clean() {
    let d = load("media.json");
    for c in arr(&d, "sid") {
        assert_eq!(sid_of(c["identity"].as_str().unwrap()), c["sid"].as_str().unwrap(), "{}", name(c));
    }
    for c in arr(&d, "clean") {
        let max = d["limits"][c["field"].as_str().unwrap()].as_u64().unwrap() as usize;
        let input = if c.get("inputParts").is_some() { parts(&c["inputParts"]) } else { c["input"].as_str().unwrap().to_string() };
        let expected =
            if c.get("expectedParts").is_some() { Some(parts(&c["expectedParts"])) } else { c["expected"].as_str().map(str::to_string) };
        let got = clean_link_text(&input, max);
        assert_eq!(got, expected, "{}", name(c));
        if let Some(g) = got {
            assert_eq!(clean_link_text(&g, max).as_deref(), Some(g.as_str()), "{}: stable", name(c));
        }
    }
}

#[test]
fn images_through_the_real_receiver() {
    let d = load("media.json");
    for c in arr(&d, "image") {
        let bytes = std::fs::read(protocol_dir().join(c["dataFile"].as_str().unwrap())).unwrap();
        let hash = c.get("hash").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| wire::hash_of(&bytes));
        let frame = json!({ "type": "media.image", "id": "image-case", "hash": hash, "kind": c["kind"], "data": base64url(&bytes) });
        let (mut rx, mut dedup) = pc_receiver();
        let out = rx.handle_frame(Frame::Text(frame.to_string().as_bytes()), &mut dedup);
        let got = match (&out.reply, &out.action) {
            (Some(r), _) if r["code"] == "bad_request" => "bad_request".to_string(),
            (Some(r), _) => r["code"].as_str().unwrap().to_string(),
            (None, messages::Action::Media(_)) => "accepted".to_string(),
            (None, messages::Action::MediaDropped) => "dropped".to_string(),
            other => panic!("{}: {other:?}", name(c)),
        };
        assert_eq!(got, c["expect"].as_str().unwrap(), "{}", name(c));
    }
    assert_eq!(std::fs::read(protocol_dir().join("fixtures/media-art-24576.png")).unwrap().len(), 24576);
    assert_eq!(std::fs::read(protocol_dir().join("fixtures/media-art-24577.png")).unwrap().len(), 24577);
}

#[test]
fn position_and_source_position() {
    let d = load("media.json");
    for c in arr(&d, "position") {
        let got = shown(
            state_of(c["state"].as_str().unwrap()),
            c["positionMs"].as_u64(),
            c["rateMilli"].as_u64(),
            c["durationMs"].as_u64(),
            c["elapsedMs"].as_u64().unwrap(),
        );
        assert_eq!(got.position_ms, c["expect"]["positionMs"].as_u64(), "{}", name(c));
        assert_eq!(got.state.as_str(), c["expect"]["state"].as_str().unwrap(), "{}", name(c));
    }
    let mut windows = 0;
    for c in arr(&d, "sourcePosition") {
        if c["platform"] != "windows" {
            continue;
        }
        windows += 1;
        let ticks = |k: &str| c[k].as_str().unwrap().parse::<i64>().unwrap();
        let got = windows_source_position(
            ticks("positionTicks"),
            ticks("lastUpdatedTicks"),
            ticks("nowTicks"),
            c["rateMilli"].as_u64(),
            c["state"] == "playing",
            c["durationMs"].as_u64(),
        );
        assert_eq!(Some(got), c["expect"].as_u64(), "{}", name(c));
    }
    assert!(windows >= 6, "the Windows cases of sourcePosition[]: {windows}");
}

#[test]
fn pick_clear_debounce_throttle_and_rates() {
    let d = load("media.json");
    for c in arr(&d, "pick") {
        let sessions: Vec<Candidate> = c["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Candidate {
                sid: s["sid"].as_str().unwrap().to_string(),
                own: s["own"].as_bool().unwrap_or(false),
                has_title: s["title"].as_bool().unwrap(),
                state: state_of(s["state"].as_str().unwrap()),
                paused_ms: s["pausedMs"].as_u64(),
            })
            .collect();
        let got = pick(&sessions, c["current"].as_str()).map(|s| s.sid.clone());
        assert_eq!(got.as_deref(), c["expect"].as_str(), "{}", name(c));
    }
    for c in arr(&d, "clearDebounce") {
        let mut deb = ClearDebounce::default();
        let got: Vec<u64> = c["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| deb.observe(e["t"].as_u64().unwrap(), e["present"].as_bool().unwrap()))
            .collect();
        let want: Vec<u64> = c["expect"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
        assert_eq!(got, want, "{}", name(c));
    }
    for c in arr(&d, "throttle") {
        let mut t: Throttle<String> = Throttle::new(250);
        let mut got: Vec<(u64, String)> = Vec::new();
        for e in c["events"].as_array().unwrap() {
            got.extend(t.push(e["t"].as_u64().unwrap(), e["value"].as_str().unwrap().to_string()));
        }
        got.extend(t.flush(u64::MAX));
        let want: Vec<(u64, String)> =
            c["expect"].as_array().unwrap().iter().map(|e| (e["t"].as_u64().unwrap(), e["value"].as_str().unwrap().to_string())).collect();
        assert_eq!(got, want, "{}", name(c));
    }
    for c in arr(&d, "rate") {
        let mut l = Limiter::new(limits::spec_named(c["limiter"].as_str().unwrap()).unwrap());
        let (mut runs, mut errors): (Vec<(String, u64)>, Vec<u64>) = (Vec::new(), Vec::new());
        for e in c["events"].as_array().unwrap() {
            let (t, n) = (e[0].as_u64().unwrap(), e[1].as_u64().unwrap());
            for _ in 0..n {
                let v = l.check(t);
                let outcome = if v.allowed { "accepted" } else { "dropped" };
                match runs.last_mut() {
                    Some((o, count)) if o == outcome => *count += 1,
                    _ => runs.push((outcome.to_string(), 1)),
                }
                if v.send_error {
                    errors.push(t);
                }
            }
        }
        let want: Vec<(String, u64)> =
            c["expect"].as_array().unwrap().iter().map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_u64().unwrap())).collect();
        assert_eq!(runs, want, "{}", name(c));
        let want_errors: Vec<u64> = c["errorsAt"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
        assert_eq!(errors, want_errors, "{}: error answers", name(c));
    }
}

#[test]
fn windows_maps_and_volume() {
    let d = load("media.json");
    for c in arr(&d, "windowsMap") {
        let k = &c["controls"];
        let b = |f: &str| k[f].as_bool().unwrap();
        let controls =
            Controls { play: b("play"), pause: b("pause"), toggle: b("toggle"), next: b("next"), prev: b("prev"), position: b("position") };
        let got = windows_to_wire(c["status"].as_i64().unwrap() as i32, controls, c["durationMs"].as_u64().unwrap());
        assert_eq!(got.state.is_some(), c["expect"]["candidate"].as_bool().unwrap(), "{}", name(c));
        assert_eq!(got.state.map(PlayState::as_str), c["expect"]["state"].as_str(), "{}", name(c));
        let want: Vec<&str> = c["expect"]["actions"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert_eq!(action_names(&got.actions), want, "{}", name(c));
    }
    for c in arr(&d, "toSmtc") {
        let got = wire_to_smtc(state_of(c["state"].as_str().unwrap()), &actions_of(&c["actions"]), c["testMode"].as_bool().unwrap());
        let status = match got.status {
            SmtcStatus::Changing => "Changing",
            SmtcStatus::Stopped => "Stopped",
            SmtcStatus::Playing => "Playing",
            SmtcStatus::Paused => "Paused",
        };
        let e = &c["expect"];
        assert_eq!(status, e["status"].as_str().unwrap(), "{}", name(c));
        assert_eq!(
            (got.play, got.pause, got.next, got.prev),
            (
                e["playEnabled"].as_bool().unwrap(),
                e["pauseEnabled"].as_bool().unwrap(),
                e["nextEnabled"].as_bool().unwrap(),
                e["prevEnabled"].as_bool().unwrap()
            ),
            "{}",
            name(c)
        );
    }
    for c in arr(&d, "volumeLevel") {
        if c["platform"] == "windows" {
            assert_eq!(u64::from(level_of_scalar(c["scalar"].as_f64().unwrap() as f32)), c["level"].as_u64().unwrap(), "{}", name(c));
        }
    }
    let mut checked = 0;
    for c in arr(&d, "volumeSet") {
        if c["platform"] != "windows" {
            continue;
        }
        checked += 1;
        let op = &c["op"];
        if let Some(m) = op["mute"].as_bool() {
            assert_eq!(
                target_level(c["steps"].as_u64().unwrap() as u32, c["level"].as_u64().unwrap() as u8, VolumeOp::Mute(m)),
                None,
                "{}",
                name(c)
            );
            assert_eq!(c["expect"]["muted"].as_bool(), Some(m), "{}", name(c));
            continue;
        }
        let op = match (op["level"].as_u64(), op["delta"].as_i64()) {
            (Some(l), None) => VolumeOp::Level(l as u8),
            (None, Some(delta)) => VolumeOp::Delta(delta),
            other => panic!("{}: {other:?}", name(c)),
        };
        let got = target_level(c["steps"].as_u64().unwrap() as u32, c["level"].as_u64().unwrap() as u8, op);
        assert_eq!(got.map(u64::from), c["expect"]["level"].as_u64(), "{}", name(c));
    }
    assert!(checked >= 10, "the Windows cases of volumeSet[]: {checked}");
}

#[test]
fn who_may_send_what_and_what_a_source_answers() {
    let d = load("media.json");
    for c in arr(&d, "sendRule") {
        let kind = Kind::parse(c["kind"].as_str().unwrap()).unwrap_or_else(|| panic!("{}: a kind", name(c)));
        let hellos = c["hellos"].as_array().unwrap();
        let lists = hellos.last().is_some_and(|h| h.as_array().unwrap().iter().any(|x| x == "media"));
        let peer = c["status"].as_object().map(|s| PeerStatus {
            show: s.get("show").and_then(Value::as_bool).unwrap_or(false),
            share: s.get("share").and_then(Value::as_bool).unwrap_or(false),
        });
        let got = policy::may_send(kind, c["ownShare"].as_bool().unwrap(), c["shareJustTurnedOff"].as_bool().unwrap_or(false), lists, peer);
        assert_eq!(if got { "send" } else { "nothing" }, c["expected"].as_str().unwrap(), "{}", name(c));
    }
    for c in arr(&d, "viewerRule") {
        let got = policy::viewer_accepts(Kind::parse(c["kind"].as_str().unwrap()).unwrap(), c["ownShow"].as_bool().unwrap());
        assert_eq!(if got { "accept" } else { "drop" }, c["expected"].as_str().unwrap(), "{}", name(c));
    }
    for c in arr(&d, "cmdResult") {
        let k = c["cmd"]["cmd"].as_str().unwrap();
        let kind = match k {
            "play" => CmdKind::Play,
            "pause" => CmdKind::Pause,
            "toggle" => CmdKind::Toggle,
            "next" => CmdKind::Next,
            "prev" => CmdKind::Prev,
            "seek" => CmdKind::Seek(0),
            "volume" => CmdKind::Volume(VolumeOp::Level(10)),
            other => panic!("{other}"),
        };
        let cmd = Cmd { sid: c["cmd"]["sid"].as_str().map(str::to_string), kind };
        let active = c["active"].as_object().map(|a| (a["sid"].as_str().unwrap().to_string(), actions_of(&a["actions"])));
        let got = policy::cmd_result(
            c["ownShare"].as_bool().unwrap(),
            c["rateOk"].as_bool().unwrap(),
            active.as_ref().map(|(s, a)| (s.as_str(), a.as_slice())),
            &cmd,
            c["systemOk"].as_bool().unwrap(),
        );
        assert_eq!(got, Verdict::parse(c["expected"].as_str().unwrap()).unwrap(), "{}", name(c));
    }
    // The design's own names are in there.
    assert!(d["sendRule"].as_array().unwrap().iter().any(|c| name(c) == "media-not-sent-before-peer-status"));
    assert!(d["viewerRule"].as_array().unwrap().iter().any(|c| name(c).starts_with("media-dropped-when-viewer-off-is-silent")));
    for n in ["media-cmd-to-non-sharing-source-is-disabled", "media-cmd-stale-sid-is-stale"] {
        assert!(d["cmdResult"].as_array().unwrap().iter().any(|c| name(c) == n), "{n}");
    }
}

/// The scope numbers of `messages.json`: a dropped vector is noticed here.
#[test]
fn media_scope_is_what_the_readme_says() {
    let root = load("messages.json");
    let count = |key: &str| root[key].as_array().unwrap().iter().filter(|c| name(c).starts_with("media-")).count();
    assert!(count("valid") >= 35, "valid media cases: {}", count("valid"));
    assert!(count("invalid") >= 50, "invalid media cases: {}", count("invalid"));
    assert!(count("sequences") >= 6, "media sequences: {}", count("sequences"));
    for ty in ["media.status", "media.state", "media.image", "media.clear", "media.volume", "media.cmd"] {
        let valid =
            root["valid"].as_array().unwrap().iter().filter(|c| c["frame"]["type"] == ty && c["expect"]["action"] != "media_dropped");
        assert!(valid.count() > 0, "{ty}: a valid example");
        assert!(
            root["valid"].as_array().unwrap().iter().filter(|c| c["frame"]["type"] == ty).all(|c| c["expect"]["reply"] == "none"),
            "{ty} is never acked"
        );
    }
}
