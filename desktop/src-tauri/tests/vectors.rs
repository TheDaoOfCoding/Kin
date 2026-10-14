//! The vectors in `protocol/` that are in scope for milestone 1.1 (see protocol/README.md, "What each milestone needs"),
//! run against the real code: the encodings, the certificate profile, the pairing URL, the error table, the WebSocket
//! receiver and the HTTP endpoints (in-process, without TLS: the TLS cases are in `tls_vectors.rs`).
mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::Request;
use common::*;
use kin_lib::encoding::*;
use kin_lib::messages::{self, Dedup, Frame, Receiver, Role};
use kin_lib::pairing::{SeedState, pair_url};
use kin_lib::server::{self, ApiError, Peer};
use kin_lib::tls::certificate_profile_violations;
use serde_json::{Value, json};
use tower::ServiceExt;

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// A name matches a scope pattern exactly, or by prefix when the pattern ends in `*` (as in protocol/README.md).
fn matches(name: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| p.strip_suffix('*').map_or(name == *p, |prefix| name.starts_with(prefix)))
}

/// Every pattern of a scope list has to match a case, so the list and the files can't drift apart.
fn assert_patterns_match(what: &str, patterns: &[&str], names: &[&str]) {
    for p in patterns {
        assert!(names.iter().any(|n| matches(n, &[p])), "{what}: the scope pattern {p:?} matches no case");
    }
}

// ----- device-id.json ---------------------------------------------------------------------------------------------

#[test]
fn device_id_keys() {
    let root = load("device-id.json");
    for key in root["keys"].as_array().unwrap() {
        let name = key["name"].as_str().unwrap();
        let cert = common::vector_keys().into_iter().find(|k| k.device_id == key["deviceId"].as_str().unwrap()).unwrap();
        let spki_hex = key["spkiDerHex"].as_str().unwrap();
        // The key itself: the SPKI derived from the PKCS#8 private key equals the vector.
        let pair = rcgen::KeyPair::try_from(cert.pkcs8.as_slice()).unwrap();
        let spki = rcgen::PublicKeyData::subject_public_key_info(&pair);
        assert_eq!(spki, hex_to_bytes(spki_hex), "{name}: SPKI from the key");
        assert_eq!(spki.len() as u64, key["spkiLength"].as_u64().unwrap());
        // The certificate over it gives the same SPKI, the same hash and the same IDs.
        let from_cert = spki_of_certificate(&cert.cert).expect("SPKI from the certificate");
        assert_eq!(from_cert, spki.as_slice(), "{name}: SPKI from the certificate");
        assert_eq!(sha256(from_cert).to_vec(), hex_to_bytes(key["sha256Hex"].as_str().unwrap()));
        assert_eq!(device_id_from_spki(from_cert), key["deviceId"].as_str().unwrap());
        assert_eq!(short_id_from_spki(from_cert), key["shortId"].as_str().unwrap());
        // The certificate profile.
        assert_eq!(certificate_profile_violations(&cert.cert), Vec::<String>::new(), "{name}");
    }
}

#[test]
fn device_id_hashes_tokens_and_shapes() {
    let root = load("device-id.json");
    for v in root["hashVectors"].as_array().unwrap() {
        let hash: [u8; 32] = hex_to_bytes(v["sha256Hex"].as_str().unwrap()).try_into().unwrap();
        assert_eq!(device_id_from_hash(&hash), v["deviceId"].as_str().unwrap(), "{}", v["name"]);
        assert_eq!(short_id_from_hash(&hash), v["shortId"].as_str().unwrap(), "{}", v["name"]);
    }
    for v in root["tokenVectors"].as_array().unwrap() {
        assert_eq!(base64url(&hex_to_bytes(v["bytesHex"].as_str().unwrap())), v["token"].as_str().unwrap(), "{}", v["name"]);
    }
    for v in root["shapeVectors"].as_array().unwrap() {
        let value = v["value"].as_str().unwrap();
        let got = match v["kind"].as_str().unwrap() {
            "deviceId" => is_valid_device_id(value),
            "token" => is_valid_token(value),
            other => panic!("unknown shape kind {other}"),
        };
        assert_eq!(got, v["valid"].as_bool().unwrap(), "{value:?} ({})", v["reason"]);
    }
}

// ----- pairing-urls.json ------------------------------------------------------------------------------------------

/// The PC only writes the URL, it never parses one, so the canonical cases are the ones that apply to it.
#[test]
fn pairing_urls_canonical_writing() {
    let root = load("pairing-urls.json");
    let mut written = 0;
    for case in root["valid"].as_array().unwrap() {
        if case["canonical"].as_bool() != Some(true) {
            continue;
        }
        let parsed = &case["parsed"];
        let addresses: Vec<_> = parsed["addresses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| format!("{}:{}", a["host"].as_str().unwrap(), a["port"]).parse().unwrap())
            .collect();
        let url = pair_url(&addresses, parsed["id"].as_str().unwrap(), parsed["token"].as_str().unwrap(), parsed["name"].as_str());
        assert_eq!(url, case["url"].as_str().unwrap(), "{}", case["name"]);
        written += 1;
    }
    assert!(written >= 10, "only {written} canonical cases found");
}

// ----- http.json: errors[] ----------------------------------------------------------------------------------------

#[test]
fn http_error_table() {
    let all = [
        ApiError::BadRequest,
        ApiError::Unauthorized,
        ApiError::BadToken,
        ApiError::TokenExpired,
        ApiError::TokenUsed,
        ApiError::UnsupportedVersion,
        ApiError::NotFound,
        ApiError::OffsetMismatch,
        ApiError::SourceChanged,
        ApiError::TooLarge,
        ApiError::DiskFull,
        ApiError::Internal,
    ];
    let root = load("http.json");
    let mut seen = 0;
    for e in root["errors"].as_array().unwrap() {
        let code = e["code"].as_str().unwrap();
        let Some(status) = e["status"].as_u64() else {
            assert_eq!(code, "unsupported_type", "only the WebSocket-only code has no HTTP status");
            continue;
        };
        let api = all.iter().find(|a| a.code() == code).unwrap_or_else(|| panic!("no ApiError for {code}"));
        assert_eq!(api.status().as_u16() as u64, status, "{code}");
        seen += 1;
    }
    assert_eq!(seen, all.len(), "every error code of the table is covered");
}

// ----- messages.json ----------------------------------------------------------------------------------------------

/// Dedup memory in RAM, for the receiver tests.
#[derive(Default)]
struct MemoryDedup(HashMap<String, String>);

impl Dedup for MemoryDedup {
    fn lookup(&mut self, id: &str) -> Option<String> {
        self.0.get(id).cloned()
    }

    fn remember(&mut self, id: &str, response: &str) {
        self.0.insert(id.to_string(), response.to_string());
    }
}

enum Input {
    Text(Vec<u8>),
    Binary,
}

fn input_of(case: &Value) -> (Input, Option<String>) {
    let id = |frame: &Value| frame["id"].as_str().map(String::from);
    if let Some(frame) = case.get("frame") {
        (Input::Text(frame.to_string().into_bytes()), id(frame))
    } else if let Some(raw) = case.get("raw") {
        (Input::Text(raw.as_str().unwrap().as_bytes().to_vec()), None)
    } else if let Some(hex) = case.get("rawHex") {
        (Input::Text(hex_to_bytes(hex.as_str().unwrap())), None)
    } else if case.get("binaryHex").is_some() {
        (Input::Binary, None)
    } else if let Some(s) = case.get("synthesize") {
        let mut base = s["base"].clone();
        let filler = s["fill"].as_str().unwrap().repeat(s["fillCount"].as_u64().unwrap() as usize);
        base[s["field"].as_str().unwrap()] = Value::String(filler);
        let text = base.to_string();
        assert_eq!(text.len() as u64, s["totalBytes"].as_u64().unwrap(), "the synthesized frame has the stated size");
        (Input::Text(text.into_bytes()), None)
    } else {
        panic!("case without an input: {}", case["name"]);
    }
}

fn open_receiver(role: Role, dedup: &mut MemoryDedup) -> Receiver {
    let mut rx = Receiver::new(role);
    let hello =
        json!({ "type": "hello", "id": "setup-hello", "protocol": 1, "name": "Peer", "platform": "android", "appVersion": "0.1.0" });
    rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), dedup);
    assert!(rx.is_open());
    rx
}

fn role_of(v: &Value) -> Role {
    match v.as_str().unwrap() {
        "pc" => Role::Pc,
        "phone" => Role::Phone,
        other => panic!("unknown role {other}"),
    }
}

fn check_outcome(name: &str, out: &messages::Outcome, expect: &Value, frame_id: Option<&str>) {
    match expect["reply"].as_str().unwrap() {
        "none" => assert!(out.reply.is_none(), "{name}: expected no reply, got {:?}", out.reply),
        "ack" => {
            let want = expect.get("ref").and_then(Value::as_str).or(frame_id).expect("an ack needs a ref");
            assert_eq!(out.reply, Some(json!({ "type": "ack", "ref": want })), "{name}");
        }
        "error" => {
            let reply = out.reply.as_ref().unwrap_or_else(|| panic!("{name}: expected an error reply"));
            assert_eq!(reply["type"], "error", "{name}");
            assert_eq!(reply["code"], expect["code"], "{name}: error code");
            // `ref` is left out when the vector says null.
            assert_eq!(reply.get("ref"), expect.get("ref").filter(|r| !r.is_null()), "{name}: error ref");
            assert!(reply["message"].as_str().is_some_and(|m| !m.is_empty()), "{name}: error message");
        }
        other => panic!("unknown reply kind {other}"),
    }
    assert_eq!(out.close.map(u64::from), expect["close"].as_u64(), "{name}: close code");
    if let Some(action) = expect.get("action").and_then(Value::as_str) {
        assert_eq!(out.action.vector_name(), Some(action), "{name}: action");
    }
}

fn run_message_case(case: &Value) {
    let name = case["name"].as_str().unwrap();
    let role = role_of(&case["to"]);
    let mut dedup = MemoryDedup::default();
    let mut rx = if case["state"] == "open" { open_receiver(role, &mut dedup) } else { Receiver::new(role) };
    let (input, id) = input_of(case);
    let out = match &input {
        Input::Text(bytes) => rx.handle_frame(Frame::Text(bytes), &mut dedup),
        Input::Binary => rx.handle_frame(Frame::Binary, &mut dedup),
    };
    check_outcome(name, &out, &case["expect"], id.as_deref());
}

const MESSAGES_VALID_SCOPE: &[&str] = &[
    "hello-*",
    "ack",
    "ack-from-pc",
    "error-*",
    "ping-*",
    "unpair-*",
    "nesting-depth-64-*",
    "brackets-inside-strings-do-not-count-as-nesting",
];

const MESSAGES_INVALID_SCOPE: &[&str] = &[
    "first-message-*",
    "hello-*",
    "ack-*",
    "error-*",
    "unpair-missing-id",
    "ping-*",
    "type-*",
    "unknown-type-is-checked-before-fields",
    "frame-*",
    "binary-frame*",
    "text-frame-1-MiB-plus-1-byte",
    "text-frame-multibyte-over-limit",
    "text-frame-invalid-utf8",
    "text-lone-*",
    "text-high-surrogate-followed-by-non-low-escape",
    "text-reversed-surrogate-pair-escape",
    "lone-surrogate-escape-*",
    "strict-*",
    "nesting-*",
];

const MESSAGES_SEQUENCE_SCOPE: &[&str] = &[
    "second-hello-after-handshake-is-a-normal-message",
    "invalid-hello-does-not-count-as-accepted",
    "garbage-before-hello-keeps-the-connection-open",
    "future-hello-gets-unsupported-version-not-a-timeout",
    "duplicate-ping-shows-one-toast",
];

fn names(cases: &Value) -> Vec<&str> {
    cases.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect()
}

#[test]
fn messages_valid_and_invalid() {
    let root = load("messages.json");
    for (key, scope) in [("valid", MESSAGES_VALID_SCOPE), ("invalid", MESSAGES_INVALID_SCOPE)] {
        assert_patterns_match(key, scope, &names(&root[key]));
        let mut run = 0;
        for case in root[key].as_array().unwrap() {
            if matches(case["name"].as_str().unwrap(), scope) {
                run_message_case(case);
                run += 1;
            }
        }
        eprintln!("messages.json {key}[]: {run} cases");
        assert!(run > 0);
    }
}

#[test]
fn messages_sequences() {
    let root = load("messages.json");
    assert_patterns_match("sequences", MESSAGES_SEQUENCE_SCOPE, &names(&root["sequences"]));
    for seq in root["sequences"].as_array().unwrap() {
        let seq_name = seq["name"].as_str().unwrap();
        if !matches(seq_name, MESSAGES_SEQUENCE_SCOPE) {
            continue;
        }
        // Each role has its own receiver, with its own state and memory of IDs. `from` is the sender.
        let mut receivers: HashMap<&str, (Receiver, MemoryDedup)> = HashMap::new();
        for (i, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
            let receiver_role = if step["from"] == "phone" { "pc" } else { "phone" };
            let (rx, dedup) =
                receivers.entry(receiver_role).or_insert_with(|| (Receiver::new(role_of(&json!(receiver_role))), MemoryDedup::default()));
            let (input, id) = input_of(step);
            let out = match &input {
                Input::Text(bytes) => rx.handle_frame(Frame::Text(bytes), dedup),
                Input::Binary => rx.handle_frame(Frame::Binary, dedup),
            };
            check_outcome(&format!("{seq_name} step {}", i + 1), &out, &step["expect"], id.as_deref());
        }
    }
}

#[test]
fn messages_timers() {
    let root = load("messages.json");
    for t in root["timers"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let mut dedup = MemoryDedup::default();
        let rx = if t["state"] == "open" { open_receiver(Role::Pc, &mut dedup) } else { Receiver::new(Role::Pc) };
        let got = rx.hello_timeout(t["elapsedSinceOpenMs"].as_u64().unwrap());
        assert_eq!(got.map(u64::from), t["expect"]["close"].as_u64(), "{name}");
    }
}

#[test]
fn messages_close_codes() {
    let root = load("messages.json");
    for c in root["closeCodes"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let code = c["closeCode"].as_u64().map(|n| n as u16);
        let status = c["upgradeStatus"].as_u64().map(|n| n as u16);
        assert_eq!(messages::client_action(code, status).vector_name(), c["action"].as_str().unwrap(), "{name}");
        if let Some(resets) = c["resetsBackoff"].as_bool() {
            assert_eq!(messages::resets_backoff(c["helloAcceptedForMs"].as_u64()), resets, "{name}");
        }
    }
}

// ----- http.json: exchanges and sequences -------------------------------------------------------------------------

const HTTP_EXCHANGE_SCOPE: &[&str] = &["pair-*", "ws-upgrade-*", "unknown-route*", "unknown-version-path", "wrong-method-on-known-route*"];

const HTTP_SEQUENCE_SCOPE: &[&str] = &["pair-rate-limit-per-token", "pair-token-is-single-use"];

/// A core in the state the case's `context` describes. The PC is vector key 1 and is called "Desk PC".
fn core_for(context: &Value) -> (TestEnv, String) {
    let keys = vector_keys();
    let env = test_core(identity_of(&keys[0]), "Desk PC");
    let client_id = context["client"]["deviceId"].as_str().unwrap().to_string();
    let db = &env.core.db;
    let device = |name| kin_lib::storage::NewDevice { device_id: &client_id, name, platform: "android", model: None, app_version: None };
    if context["client"]["paired"] == true || context["phoneAlreadyPaired"] == true {
        db.upsert_device(&device("Old name"), 1).unwrap();
    }
    if let Some(token) = context.get("pairingToken") {
        let state = match (token["state"].as_str().unwrap(), token["expired"].as_bool().unwrap_or(false)) {
            ("active", _) => SeedState::Active,
            ("none", _) => SeedState::None,
            ("expired", _) => SeedState::Expired,
            ("invalidated", _) => SeedState::Invalidated,
            ("used", true) => SeedState::UsedAndExpired,
            ("used", false) => SeedState::Used,
            (other, _) => panic!("unknown token state {other}"),
        };
        env.core.seed_pairing_token(token["value"].as_str().unwrap(), state);
    }
    (env, client_id)
}

/// Sends a request through the router with the peer attached, as the accept loop does.
async fn send_in_process(env: &TestEnv, client_id: &str, request: &Value) -> (u16, Option<String>, Value) {
    let method = request["method"].as_str().unwrap();
    let mut builder = Request::builder().method(method).uri(request["path"].as_str().unwrap());
    for (n, v) in request["headers"].as_object().unwrap() {
        builder = builder.header(n.as_str(), v.as_str().unwrap());
    }
    let body: Vec<u8> = if let Some(raw) = request.get("bodyRaw") {
        raw.as_str().unwrap().as_bytes().to_vec()
    } else if let Some(n) = request.get("bodyBytes").and_then(Value::as_u64) {
        // `{"pad":"aaa…"}` of exactly n bytes.
        let overhead = r#"{"pad":""}"#.len();
        format!(r#"{{"pad":"{}"}}"#, "a".repeat(n as usize - overhead)).into_bytes()
    } else {
        match request.get("body") {
            Some(Value::String(s)) => s.as_bytes().to_vec(),
            Some(v) => v.to_string().into_bytes(),
            None => Vec::new(),
        }
    };
    let mut req = builder.body(Body::from(body)).unwrap();
    req.extensions_mut().insert(Peer { device_id: client_id.to_string(), remote: "127.0.0.1:1".parse().unwrap() });
    let response = server::router(env.core.clone()).oneshot(req).await.unwrap();
    let status = response.status().as_u16();
    let content_type = response.headers().get("content-type").map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, content_type, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn check_response(name: &str, got: (u16, Option<String>, Value), want: &Value) {
    let (status, content_type, body) = got;
    assert_eq!(status as u64, want["status"].as_u64().unwrap(), "{name}: status (body {body})");
    if let Some(expected) = want["headers"]["Content-Type"].as_str() {
        assert_eq!(content_type.as_deref(), Some(expected), "{name}: Content-Type");
    }
    // The message text is only an example: it must be there for an error, but it is never compared.
    let (mut got_body, mut want_body) = (body.clone(), want["body"].clone());
    if want_body.get("message").is_some() {
        assert!(got_body["message"].as_str().is_some_and(|m| !m.is_empty()), "{name}: error message missing in {body}");
        got_body.as_object_mut().unwrap().remove("message");
        want_body.as_object_mut().unwrap().remove("message");
    }
    assert_eq!(got_body, want_body, "{name}: body");
}

#[tokio::test]
async fn http_exchanges() {
    let root = load("http.json");
    assert_patterns_match("exchanges", HTTP_EXCHANGE_SCOPE, &names(&root["exchanges"]));
    let mut run = 0;
    for case in root["exchanges"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !matches(name, HTTP_EXCHANGE_SCOPE) {
            continue;
        }
        let (env, client_id) = core_for(&case["context"]);
        let got = send_in_process(&env, &client_id, &case["request"]).await;
        check_response(name, got, &case["response"]);
        if let Some(effect) = case["context"].get("effect") {
            let saved = env.core.db.get_device(effect["savedDeviceId"].as_str().unwrap_or(&client_id)).unwrap();
            let saved = saved.unwrap_or_else(|| panic!("{name}: the device was not saved"));
            if let Some(n) = effect["savedName"].as_str() {
                assert_eq!(saved.name, n, "{name}: saved name");
            }
            if effect["recordUpdated"] == true {
                assert_eq!(saved.name, case["request"]["body"]["name"].as_str().unwrap(), "{name}: record updated");
            }
        }
        run += 1;
    }
    eprintln!("http.json exchanges[]: {run} cases");
    assert!(run >= 60);
}

#[tokio::test]
async fn http_sequences() {
    let root = load("http.json");
    assert_patterns_match("sequences", HTTP_SEQUENCE_SCOPE, &names(&root["sequences"]));
    for seq in root["sequences"].as_array().unwrap() {
        let seq_name = seq["name"].as_str().unwrap();
        if !matches(seq_name, HTTP_SEQUENCE_SCOPE) {
            continue;
        }
        let (env, client_id) = core_for(&seq["context"]);
        for (i, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
            let got = send_in_process(&env, &client_id, &step["request"]).await;
            check_response(&format!("{seq_name} step {}", i + 1), got, &step["response"]);
        }
    }
}

/// The 413 answer is given after the whole body was read, and the Content-Type check comes after the size check.
#[tokio::test]
async fn pair_oversize_body_wins_over_content_type() {
    let (env, client_id) = core_for(&json!({ "client": { "deviceId": "x".repeat(43) } }));
    let request = json!({ "method": "POST", "path": "/v1/pair", "headers": { "Content-Type": "text/plain" }, "bodyBytes": 1_048_577 });
    let (status, _, body) = send_in_process(&env, &client_id, &request).await;
    assert_eq!((status, body["error"].as_str()), (413, Some("too_large")));
    // Exactly 1 MiB is not too large (it is bad here only because the Content-Type is wrong).
    let request = json!({ "method": "POST", "path": "/v1/pair", "headers": { "Content-Type": "text/plain" }, "bodyBytes": 1_048_576 });
    let (status, _, body) = send_in_process(&env, &client_id, &request).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("bad_request")));
}
