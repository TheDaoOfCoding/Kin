// The host's loop and relay logic, in process: framed requests in memory -> host::run -> a real named pipe -> a real pipe server (the mock
// Kin). The Kin-starting tests start the real mock-kin.exe through the host's own start code.
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use kin_native_host::framing::MAX_REQUEST;
use kin_native_host::host::Ended;
use kin_native_host::mock::server::{Handler, Reply, Server};
use serde_json::{Value, json};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

// ---- forwarding -----------------------------------------------------------------------------------------------------------------------

#[test]
fn status_is_forwarded_and_gets_origin_and_host_version() {
    let pipe = unique_pipe();
    let (h, seen) = recording(|v| {
        respond(json!({ "v": 1, "ref": v["id"], "type": "status", "ok": true, "running": true, "kinVersion": "9.9.9",
            "devices": [{ "id": "d1", "name": "Pixel 8", "online": true, "queuedCount": 0 }], "defaultDeviceId": "d1" }))
    });
    let _srv = Server::start(&pipe, h).unwrap();
    let cfg = config(&pipe);
    let a = ask(&cfg, &req("status"));
    assert_eq!(a["ref"], "id-status");
    assert_eq!(a["running"], true);
    assert_eq!(a["devices"][0]["name"], "Pixel 8");
    assert_eq!(a["hostVersion"], kin_native_host::HOST_VERSION, "Kin left it out: the host fills it in");

    let got: Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(got["type"], "status");
    assert_eq!(got["origin"], "chrome-extension://testid/");
    assert_eq!(got["hostVersion"], kin_native_host::HOST_VERSION);
}

#[test]
fn kins_host_version_is_kept_and_other_fields_pass_through_unchanged() {
    let pipe = unique_pipe();
    let (h, _) = recording(|v| {
        respond(
            json!({ "v": 1, "ref": v["id"], "type": "start", "ok": true, "running": true, "hostVersion": "kin-says", "future": { "x": [1, 2] }, "devices": [] }),
        )
    });
    let _srv = Server::start(&pipe, h).unwrap();
    let a = ask(&config(&pipe), &req("start"));
    assert_eq!(a["hostVersion"], "kin-says");
    assert_eq!(a["future"], json!({ "x": [1, 2] }));
}

#[test]
fn the_request_reaches_kin_byte_for_byte_with_only_the_two_fields_added() {
    let pipe = unique_pipe();
    let (h, seen) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let original = "{ \"v\":1,\"id\":\"abc\", \"type\":\"send\",\"kind\":\"text\", \"text\":\"h\\u00e9llo\\n\\ud83d\\ude00 \\\"q\\\"\", \"unknown\":{\"a\":[1,2.50,null]} }";
    let (ended, r) = run_host(&config(&pipe), "chrome-extension://abc/", frame_bytes(original.as_bytes()));
    assert_eq!(ended, Ended::InputClosed);
    assert_eq!(r[0]["ok"], true);
    let got = String::from_utf8(seen.lock().unwrap()[0].clone()).unwrap();
    let prefix = &original[..original.rfind('}').unwrap()];
    assert!(got.starts_with(prefix), "the extension's bytes are unchanged:\n{got}");
    assert_eq!(
        &got[prefix.len()..],
        ",\"origin\":\"chrome-extension://abc/\",\"hostVersion\":\"0.1.0\"}".replace("0.1.0", kin_native_host::HOST_VERSION)
    );
}

#[test]
fn a_forged_origin_from_the_extension_is_overridden() {
    let pipe = unique_pipe();
    let (h, seen) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let (_, r) = run_host(
        &config(&pipe),
        "chrome-extension://real/",
        frame_bytes(br#"{"v":1,"id":"a","type":"status","origin":"evil","hostVersion":"0"}"#),
    );
    assert_eq!(r[0]["ok"], true);
    let got: Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap(); // the last of two equal keys wins, as in Kin
    assert_eq!(got["origin"], "chrome-extension://real/");
    assert_ne!(got["hostVersion"], "0");
}

#[test]
fn a_five_mib_request_goes_through_unchanged() {
    let pipe = unique_pipe();
    let (h, seen) = recording(|v| {
        respond(
            json!({ "v": 1, "ref": v["id"], "type": "send", "ok": true, "queued": false, "len": v["data"].as_str().map_or(0, str::len) }),
        )
    });
    let _srv = Server::start(&pipe, h).unwrap();
    let data = "QUJD".repeat(5 * 1024 * 1024 / 4);
    let big = json!({ "v": 1, "id": "big", "type": "send", "kind": "image", "data": data, "mime": "image/png", "name": "a.png" });
    let a = ask(&config(&pipe), &big);
    assert_eq!(a["ok"], true);
    assert_eq!(a["len"], data.len());
    assert!(seen.lock().unwrap()[0].len() > data.len());
}

#[test]
fn several_requests_in_one_session_each_get_one_answer_in_order() {
    let pipe = unique_pipe();
    let (h, _) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let mut input = Vec::new();
    for i in 0..5 {
        input.extend(frame(&json!({ "v": 1, "id": format!("r{i}"), "type": "status" })));
    }
    let (ended, r) = run_host(&config(&pipe), "", input);
    assert_eq!(ended, Ended::InputClosed);
    assert_eq!(r.iter().map(|v| v["ref"].as_str().unwrap().to_string()).collect::<Vec<_>>(), ["r0", "r1", "r2", "r3", "r4"]);
}

// ---- the host's own answers --------------------------------------------------------------------------------------------------------------

#[test]
fn bad_json_is_bad_request_and_the_session_goes_on() {
    let pipe = unique_pipe();
    let (h, _) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let mut input = frame_bytes(b"{ not json");
    input.extend(frame_bytes(b"[1,2,3]"));
    input.extend(frame(&json!({ "v": 1, "type": "status" }))); // no id
    input.extend(frame(&req("status")));
    let (ended, r) = run_host(&config(&pipe), "", input);
    assert_eq!(ended, Ended::InputClosed);
    assert_eq!(r.len(), 4);
    for bad in &r[..3] {
        assert_eq!(
            (code(bad), bad["type"].as_str(), bad["ok"].clone(), bad.get("ref")),
            ("BAD_REQUEST", Some("error"), json!(false), None)
        );
    }
    assert_eq!(r[3]["ok"], true);
}

#[test]
fn wrong_version_and_unknown_type_are_answered_without_asking_kin() {
    // No server, and a Kin that couldn't be started: these two must not need one.
    let cfg = config(&unique_pipe());
    let a = ask(&cfg, &json!({ "v": 2, "id": "x1", "type": "status" }));
    assert_eq!((code(&a), a["ref"].as_str(), a["supported"].clone()), ("UNSUPPORTED_VERSION", Some("x1"), json!([1])));
    let a = ask(&cfg, &json!({ "v": 1, "id": "x2", "type": "frobnicate" }));
    assert_eq!((code(&a), a["ref"].as_str(), a["type"].as_str()), ("UNSUPPORTED_TYPE", Some("x2"), Some("frobnicate")));
    assert_eq!(a["ok"], false);
}

#[test]
fn an_oversize_prefix_is_too_large_and_stops_the_session_without_reading_the_body() {
    let cfg = config(&unique_pipe());
    let mut input = ((MAX_REQUEST + 1) as u32).to_le_bytes().to_vec(); // the header only: the body never comes
    input.extend(frame(&req("status"))); // would be swallowed as body if the host tried to read it
    let (ended, r) = run_host(&cfg, "", input);
    assert_eq!(ended, Ended::TooLarge);
    assert_eq!(r.len(), 1);
    assert_eq!((code(&r[0]), r[0]["type"].as_str()), ("TOO_LARGE", Some("error")));
}

#[test]
fn a_request_of_exactly_64_mib_is_not_too_large() {
    // Only the size rule is checked here (the body is a valid object padded to the limit); Kin isn't running, which is a normal answer.
    let cfg = config(&unique_pipe());
    let mut body = br#"{"v":1,"id":"a","type":"status","pad":""#.to_vec();
    let tail = b"\"}";
    body.resize(MAX_REQUEST - tail.len(), b'x');
    body.extend_from_slice(tail);
    assert_eq!(body.len(), MAX_REQUEST);
    let (ended, r) = run_host(&cfg, "", frame_bytes(&body));
    assert_eq!(ended, Ended::InputClosed);
    assert_eq!((r[0]["type"].as_str(), r[0]["running"].clone()), (Some("status"), json!(false)));
}

#[test]
fn stdin_closing_ends_the_session_quietly() {
    let cfg = config(&unique_pipe());
    assert_eq!(run_host(&cfg, "", Vec::new()), (Ended::InputClosed, vec![]));
    // Inside the header, and inside the body: nothing is answered.
    assert_eq!(run_host(&cfg, "", vec![5, 0]), (Ended::InputBroken, vec![]));
    let mut cut = frame(&req("status"));
    cut.truncate(cut.len() - 3);
    assert_eq!(run_host(&cfg, "", cut), (Ended::InputBroken, vec![]));
}

// ---- Kin not running -----------------------------------------------------------------------------------------------------------------------

#[test]
fn status_without_kin_is_running_false_and_never_starts_kin() {
    let pipe = unique_pipe();
    let state = State::new();
    state.scenario(json!({ "devices": [] }));
    let cfg = config_with_kin(&pipe, &state); // a Kin that WOULD be started by start or send
    let a = ask(&cfg, &req("status"));
    assert_eq!(
        (a["ok"].clone(), a["running"].clone(), a["type"].as_str(), a["devices"].clone()),
        (json!(true), json!(false), Some("status"), json!([]))
    );
    assert_eq!(a["hostVersion"], kin_native_host::HOST_VERSION);
    assert_eq!(a["ref"], "id-status");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(state.starts(), 0, "status must never start Kin");
}

#[test]
fn open_without_kin_is_kin_not_running_and_never_starts_kin() {
    let pipe = unique_pipe();
    let state = State::new();
    let cfg = config_with_kin(&pipe, &state);
    let a = ask(&cfg, &json!({ "v": 1, "id": "o1", "type": "open", "target": "main" }));
    assert_eq!((code(&a), a["ok"].clone(), a["ref"].as_str()), ("KIN_NOT_RUNNING", json!(false), Some("o1")));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(state.starts(), 0);
}

#[test]
fn send_and_start_without_a_kin_exe_are_kin_not_running_at_once() {
    let cfg = config(&unique_pipe()); // kin_exe: None
    for ty in ["send", "start"] {
        let t = Instant::now();
        let a = ask(&cfg, &json!({ "v": 1, "id": "s1", "type": ty, "kind": "text", "text": "x" }));
        assert_eq!((code(&a), a["type"].as_str()), ("KIN_NOT_RUNNING", Some(ty)));
        assert!(t.elapsed() < Duration::from_secs(2), "no waiting when there is nothing to start");
    }
    let mut cfg = config(&unique_pipe());
    cfg.kin_exe = Some(r"C:\definitely\not\here\Kin.exe".into());
    assert_eq!(code(&ask(&cfg, &req("start"))), "KIN_NOT_RUNNING");
}

#[test]
fn start_starts_kin_waits_for_the_pipe_and_answers_like_status() {
    let pipe = unique_pipe();
    let state = State::new();
    state.scenario(json!({ "startDelayMs": 600, "devices": [{ "id": "d1", "name": "Pixel", "online": true, "queuedCount": 0 }], "defaultDeviceId": "d1" }));
    let cfg = config_with_kin(&pipe, &state);
    let t = Instant::now();
    let a = ask(&cfg, &req("start"));
    assert_eq!((a["ok"].clone(), a["type"].as_str(), a["running"].clone()), (json!(true), Some("start"), json!(true)), "{a}");
    assert_eq!(a["devices"][0]["name"], "Pixel");
    assert!(t.elapsed() >= Duration::from_millis(500), "it waited for the pipe");
    assert_eq!(state.starts(), 1);
    // A second start finds it running and starts nothing.
    let a = ask(&cfg, &req("start"));
    assert_eq!(a["running"], true);
    assert_eq!(state.starts(), 1, "nothing is started twice");
}

#[test]
fn send_starts_kin_and_then_goes_through() {
    let pipe = unique_pipe();
    let state = State::new();
    state.scenario(json!({ "startDelayMs": 300, "devices": [{ "id": "d1", "name": "Pixel", "online": false, "queuedCount": 0 }], "defaultDeviceId": "d1" }));
    let cfg = config_with_kin(&pipe, &state);
    let a = ask(&cfg, &json!({ "v": 1, "id": "s1", "type": "send", "kind": "link", "url": "https://example.com/a?t=5s", "title": "T" }));
    assert_eq!((a["ok"].clone(), a["queued"].clone(), code(&a)), (json!(true), json!(true), "DEVICE_OFFLINE_QUEUED"), "{a}");
    assert_eq!(state.starts(), 1);
    let logged = state.log();
    let sent = logged.iter().find_map(|e| e.get("req")).expect("Kin saw the request");
    assert_eq!(sent["url"], "https://example.com/a?t=5s");
    assert_eq!(logged[0]["origin"], "chrome-extension://testid/");
}

#[test]
fn an_automatic_alert_never_starts_kin_but_a_manual_one_does() {
    let pipe = unique_pipe();
    let state = State::new();
    state.scenario(json!({ "devices": [{ "id": "d1", "name": "Pixel", "online": true, "queuedCount": 0 }], "defaultDeviceId": "d1" }));
    let cfg = config_with_kin(&pipe, &state); // a Kin that WOULD be started by a send
    let t = Instant::now();
    let a = ask(
        &cfg,
        &json!({ "v": 1, "id": "al1", "type": "send", "kind": "alert", "title": "Download finished", "text": "a.zip", "source": "downloads", "auto": true, "ttlSec": 1800 }),
    );
    assert_eq!(
        (code(&a), a["ok"].clone(), a["ref"].as_str(), a["type"].as_str()),
        ("KIN_NOT_RUNNING", json!(false), Some("al1"), Some("send")),
        "{a}"
    );
    assert!(t.elapsed() < Duration::from_secs(2), "no waiting for a Kin nobody starts");
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(state.starts(), 0, "a finished download must not start Kin");
    // The same alert without `auto` is the user's own request and may start Kin (the mock Kin doesn't know alerts and refuses it, but it was started to be asked).
    let a = ask(&cfg, &json!({ "v": 1, "id": "al2", "type": "send", "kind": "alert", "title": "Hi", "text": "x" }));
    assert_ne!(code(&a), "KIN_NOT_RUNNING", "{a}");
    assert_eq!(state.starts(), 1);
}

#[test]
fn a_kin_that_cannot_start_gives_kin_not_running_after_the_wait() {
    let pipe = unique_pipe();
    let state = State::new();
    state.scenario(json!({ "startable": false }));
    let mut cfg = config_with_kin(&pipe, &state);
    cfg.start_wait = Duration::from_millis(1200);
    let t = Instant::now();
    let a = ask(&cfg, &json!({ "v": 1, "id": "s1", "type": "send", "kind": "text", "text": "x" }));
    assert_eq!((code(&a), a["ok"].clone()), ("KIN_NOT_RUNNING", json!(false)));
    assert!(t.elapsed() >= Duration::from_millis(1100) && t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    assert_eq!(state.starts(), 1);
}

#[test]
fn hosts_started_together_start_kin_once() {
    let pipe = unique_pipe();
    let state = State::new();
    state.scenario(json!({ "startDelayMs": 800, "devices": [{ "id": "d1", "name": "Pixel", "online": true, "queuedCount": 0 }], "defaultDeviceId": "d1" }));
    let cfg = Arc::new(config_with_kin(&pipe, &state));
    let threads: Vec<_> = (0..4)
        .map(|i| {
            let cfg = cfg.clone();
            std::thread::spawn(move || ask(&cfg, &json!({ "v": 1, "id": format!("p{i}"), "type": "send", "kind": "text", "text": "hi" })))
        })
        .collect();
    for t in threads {
        let a = t.join().unwrap();
        assert_eq!(a["ok"], true, "{a}");
    }
    assert_eq!(state.starts(), 1, "the per-user start lock makes it one start");
}

// ---- failures on the pipe ----------------------------------------------------------------------------------------------------------------

#[test]
fn a_kin_that_does_not_answer_in_time_is_a_timeout_and_the_session_recovers() {
    let pipe = unique_pipe();
    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let c2 = calls.clone();
    let h: Handler = Arc::new(move |body| {
        let v: Value = serde_json::from_slice(&body).unwrap();
        if c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(1500));
        }
        ok_for(&v)
    });
    let _srv = Server::start(&pipe, h).unwrap();
    let mut cfg = config(&pipe);
    cfg.send_timeout = Duration::from_millis(400);
    let mut input = frame(&json!({ "v": 1, "id": "slow", "type": "send", "kind": "text", "text": "x" }));
    input.extend(frame(&json!({ "v": 1, "id": "fast", "type": "send", "kind": "text", "text": "x" })));
    let t = Instant::now();
    let (ended, r) = run_host(&cfg, "", input);
    assert_eq!(ended, Ended::InputClosed);
    assert_eq!((code(&r[0]), r[0]["ref"].as_str(), r[0]["type"].as_str()), ("TIMEOUT", Some("slow"), Some("send")));
    assert_eq!((r[1]["ok"].clone(), r[1]["ref"].as_str()), (json!(true), Some("fast")), "the next request still works");
    assert!(t.elapsed() < Duration::from_millis(1400), "the host gave up at its own limit, not at Kin's pace: {:?}", t.elapsed());
}

#[test]
fn status_has_its_own_shorter_limit() {
    let pipe = unique_pipe();
    let h: Handler = Arc::new(|body| {
        std::thread::sleep(Duration::from_millis(1500));
        ok_for(&serde_json::from_slice(&body).unwrap())
    });
    let _srv = Server::start(&pipe, h).unwrap();
    let mut cfg = config(&pipe);
    cfg.status_timeout = Duration::from_millis(300);
    let t = Instant::now();
    assert_eq!(code(&ask(&cfg, &req("status"))), "TIMEOUT");
    assert!(t.elapsed() < Duration::from_millis(1200));
}

#[test]
fn kin_hanging_up_without_an_answer_is_internal() {
    let pipe = unique_pipe();
    let h: Handler = Arc::new(|_| Reply::Close);
    let _srv = Server::start(&pipe, h).unwrap();
    let a = ask(&config(&pipe), &req("send"));
    assert_eq!((code(&a), a["ref"].as_str()), ("INTERNAL", Some("id-send")));
}

#[test]
fn an_answer_that_is_not_json_or_is_over_1_mib_is_internal_and_never_forwarded() {
    for (name, answer) in [
        ("garbage", b"this is not json".to_vec()),
        ("array", b"[1,2,3]".to_vec()),
        (
            "too big",
            format!("{{\"v\":1,\"ref\":\"id-send\",\"type\":\"send\",\"ok\":true,\"pad\":\"{}\"}}", "x".repeat(1024 * 1024)).into_bytes(),
        ),
    ] {
        let pipe = unique_pipe();
        let h: Handler = Arc::new(move |_| Reply::Respond(answer.clone()));
        let _srv = Server::start(&pipe, h).unwrap();
        let a = ask(&config(&pipe), &req("send"));
        assert_eq!((code(&a), a["ref"].as_str()), ("INTERNAL", Some("id-send")), "{name}");
    }
}

#[test]
fn an_answer_of_exactly_1_mib_is_forwarded() {
    let pipe = unique_pipe();
    let prefix = r#"{"v":1,"ref":"id-send","type":"send","ok":true,"pad":""#;
    let mut answer = prefix.as_bytes().to_vec();
    answer.resize(1024 * 1024 - 2, b'x');
    answer.extend_from_slice(b"\"}");
    assert_eq!(answer.len(), 1024 * 1024);
    let h: Handler = Arc::new(move |_| Reply::Respond(answer.clone()));
    let _srv = Server::start(&pipe, h).unwrap();
    assert_eq!(ask(&config(&pipe), &req("send"))["ok"], true);
}

#[test]
fn a_busy_pipe_is_not_running_for_status_and_a_timeout_for_send() {
    let pipe = unique_pipe();
    // One instance only, and a Kin that holds it for a while: every other client gets "pipe busy".
    let h: Handler = Arc::new(|body| {
        std::thread::sleep(Duration::from_millis(2500));
        ok_for(&serde_json::from_slice(&body).unwrap())
    });
    let _srv = Server::start_with(&pipe, h, 1).unwrap();
    let cfg = Arc::new(config(&pipe));
    let c2 = cfg.clone();
    let holder = std::thread::spawn(move || ask(&c2, &req("send")));
    std::thread::sleep(Duration::from_millis(300));

    let t = Instant::now();
    let a = ask(&cfg, &req("status"));
    assert_eq!((a["ok"].clone(), a["running"].clone()), (json!(true), json!(false)), "{a}");
    assert!(
        t.elapsed() >= Duration::from_millis(250) && t.elapsed() < Duration::from_secs(2),
        "waited the status limit: {:?}",
        t.elapsed()
    );
    let a = ask(&cfg, &req("open"));
    assert_eq!(code(&a), "TIMEOUT", "{a}"); // alive, but no free instance within the wait
    assert_eq!(holder.join().unwrap()["ok"], true);
}

// ---- who the server is -----------------------------------------------------------------------------------------------------------

#[test]
fn the_server_must_be_the_expected_program() {
    let pipe = unique_pipe();
    let (h, seen) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap(); // served by THIS test process

    // Expected: this very program -> fine.
    let mut cfg = config(&pipe);
    cfg.expected_server_image = Some(std::env::current_exe().unwrap());
    assert_eq!(ask(&cfg, &req("status"))["running"], true);
    assert_eq!(seen.lock().unwrap().len(), 1);

    // Expected: another program (an impostor squatting on the name) -> KIN_NOT_RUNNING, and nothing is sent to it, not even for a status.
    cfg.expected_server_image = Some(r"C:\Program Files\Kin\Kin.exe".into());
    for ty in ["status", "send", "start", "open"] {
        let a = ask(&cfg, &req(ty));
        assert_eq!((code(&a), a["ok"].clone()), ("KIN_NOT_RUNNING", json!(false)), "{ty}");
    }
    assert_eq!(seen.lock().unwrap().len(), 1, "the impostor received nothing");
}

#[test]
fn an_impostor_pipe_does_not_make_the_host_start_kin() {
    let pipe = unique_pipe();
    let state = State::new();
    let (h, _) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let cfg = config_with_kin(&pipe, &state); // expects mock-kin.exe, but the server is this test
    assert_eq!(code(&ask(&cfg, &req("send"))), "KIN_NOT_RUNNING");
    assert_eq!(state.starts(), 0, "the pipe is there, so there is nothing to start");
}
