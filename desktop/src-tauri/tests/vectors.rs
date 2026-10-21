//! The vectors in `protocol/` that are in scope for milestones 1.1 and 1.2 (see protocol/README.md, "What each milestone
//! needs"), run against the real code: the encodings, the certificate profile, the pairing URL, the error table, the WebSocket
//! receiver (1.2 adds the `text` and `clipboard` cases, 1.3 the `offer`, `complete` and `cancel` ones and the `sensitive` flag) and
//! the pairing endpoints (in-process, without TLS: the TLS cases are in `tls_vectors.rs`). The 1.3 transfers, file names, URLs and
//! text rules are in `transfer_vectors.rs`.
mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::Request;
use common::*;
use kin_lib::encoding::*;
use kin_lib::links;
use kin_lib::messages::{self, Dedup, Frame, OfferState, OfferedFile, Progress, Receiver, Role};
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

/// An offer as the receiver remembers it: which paired PC made it, how its download stands, and its files (`fileId`, size).
struct OfferRecord {
    device: String,
    progress: Progress,
    files: Vec<(String, u64)>,
}

fn file_key(files: &[OfferedFile]) -> Vec<(String, u64)> {
    let mut key: Vec<(String, u64)> = files.iter().map(|f| (f.file_id.clone(), f.size)).collect();
    key.sort();
    key
}

/// Dedup memory in RAM, for the receiver tests. Message IDs are remembered per paired PC (as the database does); an offer's ID is
/// remembered for all of them, with the PC that made it.
struct MemoryDedup {
    seen: HashMap<(String, String), String>,
    /// The offers by transfer ID (the `transfer_over` event of a sequence changes `progress`).
    offers: HashMap<String, OfferRecord>,
    /// The paired PC on the connection now (`{ "event": "reconnect", "device": "second" }` changes it).
    device: String,
}

impl Default for MemoryDedup {
    fn default() -> Self {
        MemoryDedup { seen: HashMap::new(), offers: HashMap::new(), device: "first".to_string() }
    }
}

impl Dedup for MemoryDedup {
    fn lookup(&mut self, id: &str) -> Option<String> {
        self.seen.get(&(self.device.clone(), id.to_string())).cloned()
    }

    fn remember(&mut self, id: &str, response: &str) {
        self.seen.insert((self.device.clone(), id.to_string()), response.to_string());
    }

    fn offer_state(&mut self, id: &str, files: &[OfferedFile]) -> OfferState {
        match self.offers.get(id) {
            None => OfferState::NotAnOffer,
            Some(o) if o.device != self.device => OfferState::OtherDevice,
            Some(o) => OfferState::Own { progress: o.progress, same_files: o.files == file_key(files) },
        }
    }

    fn remember_offer(&mut self, id: &str, files: &[OfferedFile]) {
        self.offers
            .insert(id.to_string(), OfferRecord { device: self.device.clone(), progress: Progress::Unfinished, files: file_key(files) });
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
        let id = base["id"].as_str().map(String::from);
        (Input::Text(text.into_bytes()), id)
    } else {
        panic!("case without an input: {}", case["name"]);
    }
}

fn open_receiver(role: Role, dedup: &mut MemoryDedup) -> Receiver {
    open_receiver_with(role, dedup, None, None)
}

/// A receiver after the peer's `hello`. `own` is what this side's own `hello` lists, `peer` what the peer's lists (changelog 56; a case or sequence of
/// `messages.json` that has `own` and `peer` keys).
fn open_receiver_with(role: Role, dedup: &mut MemoryDedup, own: Option<&Value>, peer: Option<&Value>) -> Receiver {
    let mut rx = Receiver::new(role);
    let mut hello =
        json!({ "type": "hello", "id": "setup-hello", "protocol": 1, "name": "Peer", "platform": "android", "appVersion": "0.1.0" });
    if let Some(peer) = peer {
        hello["capabilities"] = peer.clone();
    }
    rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), dedup);
    assert!(rx.is_open());
    set_own(&mut rx, own);
    rx
}

fn set_own(rx: &mut Receiver, own: Option<&Value>) {
    let own: Vec<String> =
        own.and_then(Value::as_array).map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
    rx.set_own_capabilities(&own.iter().map(String::as_str).collect::<Vec<_>>());
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
    // A link that is delivered says what the receiver shows and opens: `expect.link` is the URL as sent and the cleaned title and sourceApp.
    if let messages::Action::DeliverLink { url, title, source_app, .. } = &out.action {
        let mut got = json!({ "url": url });
        if let Some(t) = title {
            got["title"] = json!(t);
        }
        if let Some(a) = source_app {
            got["sourceApp"] = json!(a);
        }
        assert_eq!(&got, &expect["link"], "{name}: the link");
    }
    // A `notif.*` message says what the receiver ended up with (changelog 56): `expect.notif`, the cleaned and cut fields.
    if let Some(want) = expect.get("notif") {
        match &out.action {
            messages::Action::Notif(m) => assert_eq!(&m.to_json(), want, "{name}: what the receiver ends up with"),
            other => panic!("{name}: expected a notification, got {other:?}"),
        }
    }
    // A `media.*` message says what the receiver ended up with (changelog 57): `expect.media`.
    if let Some(want) = expect.get("media") {
        match &out.action {
            messages::Action::Media(m) => assert_eq!(&m.msg.to_json(), want, "{name}: what the receiver ends up with"),
            other => panic!("{name}: expected a media message, got {other:?}"),
        }
    }
    // A `presence` or `ring.status` says what the receiver ended up with (changelog 58): `expect.presence`.
    if let Some(want) = expect.get("presence") {
        match &out.action {
            messages::Action::Presence(m) => {
                let got = match m {
                    kin_lib::presence::wire::PresenceMsg::Presence(b) => b.to_json(),
                    kin_lib::presence::wire::PresenceMsg::RingStatus(s) => s.to_json(),
                };
                assert_eq!(&got, want, "{name}: what the receiver ends up with");
            }
            other => panic!("{name}: expected a presence message, got {other:?}"),
        }
    }
    // An `alert` says what the receiver ended up with (changelog 59): `expect.alert`, the cleaned text and title and the cut `waitedMs`.
    if let Some(want) = expect.get("alert") {
        match &out.action {
            messages::Action::DeliverAlert { alert, .. } => assert_eq!(&alert.to_json(), want, "{name}: what the receiver ends up with"),
            other => panic!("{name}: expected an alert, got {other:?}"),
        }
    }
    // A text that is delivered says whether it is sensitive: `expect.sensitive`, false when the case doesn't say.
    if let messages::Action::DeliverText { sensitive, .. } = &out.action {
        assert_eq!(*sensitive, expect["sensitive"].as_bool().unwrap_or(false), "{name}: sensitive");
    }
}

fn run_message_case(case: &Value) {
    let name = case["name"].as_str().unwrap();
    let role = role_of(&case["to"]);
    let mut dedup = MemoryDedup::default();
    let (own, peer) = (case.get("own"), case.get("peer"));
    let mut rx = if case["state"] == "open" {
        open_receiver_with(role, &mut dedup, own, peer)
    } else {
        let mut rx = Receiver::new(role);
        set_own(&mut rx, own);
        rx
    };
    // A case may say that the phone's ring budget is spent (changelog 58): a `ring.start` is then `rate_limited`.
    rx.set_ring_budget_spent(case["ringBudgetSpent"] == true);
    // ...and that its alert bucket is empty (changelog 59): an `alert` is then `rate_limited`.
    rx.set_alert_budget_spent(case["alertBudgetSpent"] == true);
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

/// Milestone 1.2: the `text` and `clipboard` cases. (`text-frame-exactly-1-MiB`, `text-frame-multibyte-under-limit` and
/// `text-valid-surrogate-pair-escape` are among the `text-*` ones.) The `offer`, `complete` and `cancel` cases and
/// `duplicate-offer-is-acked-not-restarted` are 1.3.
const MESSAGES_VALID_SCOPE_1_2: &[&str] = &["text-*", "clipboard-*", "duplicate-keys-*"];
/// Milestone 1.3: the transfer messages. (The `text-sensitive-*` cases are among the `text-*` ones above.)
const MESSAGES_VALID_SCOPE_1_3: &[&str] = &["offer-*", "complete-*", "cancel-*"];
const MESSAGES_INVALID_SCOPE_1_3: &[&str] = &["offer-*", "complete-*", "cancel-*"];
const MESSAGES_SEQUENCE_SCOPE_1_3: &[&str] = &["duplicate-offer-is-acked-not-restarted"];
/// 1.4: the offer-reuse rule and the queue's re-sends (the sequences have `reconnect` and `transfer_over` events).
const MESSAGES_SEQUENCE_SCOPE_1_4: &[&str] = &[
    "offer-reusing-*",
    "offer-from-another-device-is-bad-request",
    "offer-repeated-while-unfinished-is-acked-after-a-reconnect",
    "same-pc-repeat-*",
    "complete-re-sent-*",
    "cancel-re-sent-*",
    "queue-*",
];
const MESSAGES_INVALID_SCOPE_1_2: &[&str] = &["text-*", "clipboard-*"];
/// 2B.1: the `link` message (changelog 54). The checks of what the cases are made of are in `links_scope_is_what_the_readme_says`.
const MESSAGES_VALID_SCOPE_2B1: &[&str] = &["link-*"];
const MESSAGES_INVALID_SCOPE_2B1: &[&str] = &["link-*"];
const MESSAGES_SEQUENCE_SCOPE_2B1: &[&str] = &["link-*"];
/// 2B.2 (notifications, changelog 56): the `notif-*` cases.
const MESSAGES_VALID_SCOPE_2B2: &[&str] = &["notif-*"];
const MESSAGES_INVALID_SCOPE_2B2: &[&str] = &["notif-*"];
const MESSAGES_SEQUENCE_SCOPE_2B2: &[&str] = &[
    "notif-dropped-when-feature-off-is-silent",
    "notif-capability-changes-with-a-new-hello",
    "notif-not-sent-without-capability-receiver-side",
    "notif-is-not-in-the-dedup-memory",
    "notif-action-is-acked-and-a-repeat-is-not-run-twice",
    "notif-action-is-not-resent-after-reconnect",
    "notif-post-after-reconnect-needs-the-hellos-again",
    "notif-sync-removes-the-rest",
    "notif-invalid-post-keeps-the-connection-open",
    "notif-status-then-icon-then-post",
];
/// 2B.4 (presence, pause and ring, changelog 58): the `presence-*` and `ring-*` cases. The rules that are not one frame are in `presence.json`: `presence_vectors.rs`.
const MESSAGES_VALID_SCOPE_2B4: &[&str] = &["presence-*", "ring-*"];
const MESSAGES_INVALID_SCOPE_2B4: &[&str] = &["presence-*", "ring-*"];
const MESSAGES_SEQUENCE_SCOPE_2B4: &[&str] = &[
    "presence-not-sent-without-capability",
    "presence-dropped-when-feature-off-is-silent",
    "presence-is-not-acked-or-deduplicated",
    "presence-snapshot-replaces-pause",
    "presence-forgotten-on-close",
    "ring-not-sent-without-capability",
    "ring-capability-follows-the-switch",
    "ring-start-is-not-resent-after-reconnect",
    "ring-start-is-acked-once",
    "ring-status-is-ephemeral",
    "ring-disabled-error-is-not-remembered",
];
/// 2B.5 (Do Not Disturb, Focus and alerts, changelog 59): the `alert-*` cases. The rules that are not one frame are in `alerts.json`: `alerts_vectors.rs`.
const MESSAGES_VALID_SCOPE_2B5: &[&str] = &["alert-*"];
const MESSAGES_INVALID_SCOPE_2B5: &[&str] = &["alert-*"];
const MESSAGES_SEQUENCE_SCOPE_2B5: &[&str] = &[
    "alert-is-queued-and-acked",
    "alert-repeat-gets-the-first-answer",
    "alert-rate-limited-fails-the-item",
    "alert-quiet-when-paused",
    "alert-capability-is-the-senders-rule",
    "alert-expires-locally",
];
/// 2B.3 (media controls, changelog 57): the `media-*` cases. The policy ones (who may send what, what a source answers) are in `media.json`: `media_vectors.rs`.
const MESSAGES_VALID_SCOPE_2B3: &[&str] = &["media-*"];
const MESSAGES_INVALID_SCOPE_2B3: &[&str] = &["media-*"];
const MESSAGES_SEQUENCE_SCOPE_2B3: &[&str] = &[
    "media-is-not-in-the-dedup-memory",
    "media-status-change-needs-no-new-hello",
    "media-capability-changes-with-a-new-hello",
    "media-cmd-is-not-resent-after-reconnect",
    "media-image-before-state",
    "media-invalid-state-keeps-the-connection-open",
];
const MESSAGES_SEQUENCE_SCOPE_1_2: &[&str] =
    &["handshake-then-text-then-duplicate", "invalid-message-keeps-connection-open", "rejected-message-id-is-not-remembered"];

fn with(a: &[&'static str], b: &[&'static str]) -> Vec<&'static str> {
    a.iter().chain(b).copied().collect()
}

/// The 1.3 message cases are in the runs above. This pins how many there are and that the sensitive flag is covered, so a dropped
/// vector is noticed here.
#[test]
fn messages_1_3_scope_is_what_the_readme_says() {
    let root = load("messages.json");
    for (key, scope, at_least) in [
        ("valid", MESSAGES_VALID_SCOPE_1_3, 12),
        ("invalid", MESSAGES_INVALID_SCOPE_1_3, 27),
        ("sequences", MESSAGES_SEQUENCE_SCOPE_1_3, 1),
    ] {
        assert_patterns_match(key, scope, &names(&root[key]));
        let n = root[key].as_array().unwrap().iter().filter(|c| matches(c["name"].as_str().unwrap(), scope)).count();
        assert!(n >= at_least, "messages.json {key}[]: {n} cases in the 1.3 scope, expected at least {at_least}");
    }
    let sensitive = names(&root["valid"]).into_iter().filter(|n| n.starts_with("text-sensitive-")).count();
    assert!(sensitive >= 6, "the sensitive flag has {sensitive} valid cases");
    let bad = names(&root["invalid"]).into_iter().filter(|n| n.starts_with("text-sensitive-")).count();
    assert!(bad >= 5, "the sensitive flag has {bad} invalid cases");
}

fn names(cases: &Value) -> Vec<&str> {
    cases.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect()
}

#[test]
fn messages_valid_and_invalid() {
    let root = load("messages.json");
    let valid = with(
        &with(
            &with(&with(&with(MESSAGES_VALID_SCOPE, MESSAGES_VALID_SCOPE_1_2), MESSAGES_VALID_SCOPE_1_3), MESSAGES_VALID_SCOPE_2B1),
            MESSAGES_VALID_SCOPE_2B2,
        ),
        &with(&with(MESSAGES_VALID_SCOPE_2B3, MESSAGES_VALID_SCOPE_2B4), MESSAGES_VALID_SCOPE_2B5),
    );
    let invalid = with(
        &with(
            &with(&with(&with(MESSAGES_INVALID_SCOPE, MESSAGES_INVALID_SCOPE_1_2), MESSAGES_INVALID_SCOPE_1_3), MESSAGES_INVALID_SCOPE_2B1),
            MESSAGES_INVALID_SCOPE_2B2,
        ),
        &with(&with(MESSAGES_INVALID_SCOPE_2B3, MESSAGES_INVALID_SCOPE_2B4), MESSAGES_INVALID_SCOPE_2B5),
    );
    for (key, scope) in [("valid", valid.as_slice()), ("invalid", invalid.as_slice())] {
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

/// The 1.2 cases are in the run above. This pins which ones they are, so a renamed or dropped vector is noticed here and not
/// by a count that quietly goes down.
#[test]
fn messages_1_2_scope_is_what_the_readme_says() {
    let root = load("messages.json");
    for (key, scope, at_least) in [
        ("valid", MESSAGES_VALID_SCOPE_1_2, 14),
        ("invalid", MESSAGES_INVALID_SCOPE_1_2, 24),
        ("sequences", MESSAGES_SEQUENCE_SCOPE_1_2, 3),
    ] {
        assert_patterns_match(key, scope, &names(&root[key]));
        let n = root[key].as_array().unwrap().iter().filter(|c| matches(c["name"].as_str().unwrap(), scope)).count();
        assert!(n >= at_least, "messages.json {key}[]: {n} cases in the 1.2 scope, expected at least {at_least}");
    }
    // None of the 1.3 cases (offer, complete, cancel) is in the 1.2 scope.
    for key in ["valid", "invalid"] {
        for case in root[key].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            if name.starts_with("offer-") || name.starts_with("complete-") || name.starts_with("cancel-") {
                assert!(!matches(name, MESSAGES_VALID_SCOPE_1_2) && !matches(name, MESSAGES_INVALID_SCOPE_1_2), "{name} is a 1.3 case");
            }
        }
    }
}

#[test]
fn messages_sequences() {
    let root = load("messages.json");
    let scope = with(
        &with(
            &with(
                &with(
                    &with(&with(MESSAGES_SEQUENCE_SCOPE, MESSAGES_SEQUENCE_SCOPE_1_2), MESSAGES_SEQUENCE_SCOPE_1_3),
                    MESSAGES_SEQUENCE_SCOPE_1_4,
                ),
                MESSAGES_SEQUENCE_SCOPE_2B1,
            ),
            MESSAGES_SEQUENCE_SCOPE_2B2,
        ),
        &with(&with(MESSAGES_SEQUENCE_SCOPE_2B3, MESSAGES_SEQUENCE_SCOPE_2B4), MESSAGES_SEQUENCE_SCOPE_2B5),
    );
    assert_patterns_match("sequences", &scope, &names(&root["sequences"]));
    let mut with_events = 0;
    for seq in root["sequences"].as_array().unwrap() {
        let seq_name = seq["name"].as_str().unwrap();
        if !matches(seq_name, &scope) {
            continue;
        }
        // Each role has its own receiver, with its own state and memory of IDs. `from` is the sender.
        let mut receivers: HashMap<&str, (Receiver, MemoryDedup)> = HashMap::new();
        for (i, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
            // An event is something that happens to the receiving app, not a frame. A new connection starts every receiver again (before
            // the hello) and keeps the memory of IDs, which is in the database. The end of a transfer uses its ID up.
            if let Some(event) = step.get("event").and_then(Value::as_str) {
                with_events += 1;
                for (role, (rx, dedup)) in receivers.iter_mut() {
                    match event {
                        "reconnect" => {
                            *rx = Receiver::new(role_of(&json!(*role)));
                            // `"device": "second"`: the new connection is from another paired PC.
                            dedup.device = step.get("device").and_then(Value::as_str).unwrap_or("first").to_string();
                        }
                        "transfer_over" => {
                            let id = step["transferId"].as_str().expect("transfer_over names a transfer");
                            let how = match step["how"].as_str().expect("transfer_over says how") {
                                "done" => Progress::Done,
                                "canceled" => Progress::Canceled,
                                "failed" => Progress::Failed,
                                other => panic!("{seq_name} step {}: unknown way to end a transfer: {other}", i + 1),
                            };
                            if let Some(o) = dedup.offers.get_mut(id) {
                                o.progress = how;
                            }
                        }
                        other => panic!("{seq_name} step {}: unknown event {other}", i + 1),
                    }
                }
                continue;
            }
            let receiver_role = if step["from"] == "phone" { "pc" } else { "phone" };
            let (rx, dedup) =
                receivers.entry(receiver_role).or_insert_with(|| (Receiver::new(role_of(&json!(receiver_role))), MemoryDedup::default()));
            // A step may say whether the receiver's alert bucket is empty at that moment (changelog 59).
            if let Some(spent) = step.get("alertBudgetSpent").and_then(Value::as_bool) {
                rx.set_alert_budget_spent(spent);
            }
            let (input, id) = input_of(step);
            let out = match &input {
                Input::Text(bytes) => rx.handle_frame(Frame::Text(bytes), dedup),
                Input::Binary => rx.handle_frame(Frame::Binary, dedup),
            };
            // A `hello` that was accepted also tells what its sender lists when that side is the receiver (changelog 56): the other receiver's own list.
            if step["frame"]["type"] == "hello" && matches!(out.action, messages::Action::HandshakeOk) {
                let sender_role = if step["from"] == "phone" { "phone" } else { "pc" };
                let (sender_rx, _) =
                    receivers.entry(sender_role).or_insert_with(|| (Receiver::new(role_of(&json!(sender_role))), MemoryDedup::default()));
                set_own(sender_rx, step["frame"].get("capabilities"));
            }
            check_outcome(&format!("{seq_name} step {}", i + 1), &out, &step["expect"], id.as_deref());
        }
    }
    assert!(with_events >= 8, "the 1.4 sequences have events: {with_events}");
}

/// 2B.1: the `link-*` cases are in the runs above. This pins how many there are, that both directions are covered, and that every invalid one is
/// `bad_request` with the connection left open.
#[test]
fn links_scope_is_what_the_readme_says() {
    let root = load("messages.json");
    for (key, scope, at_least) in [
        ("valid", MESSAGES_VALID_SCOPE_2B1, 18),
        ("invalid", MESSAGES_INVALID_SCOPE_2B1, 28),
        ("sequences", MESSAGES_SEQUENCE_SCOPE_2B1, 4),
    ] {
        assert_patterns_match(key, scope, &names(&root[key]));
        let n = root[key].as_array().unwrap().iter().filter(|c| matches(c["name"].as_str().unwrap(), scope)).count();
        assert!(n >= at_least, "messages.json {key}[]: {n} link cases, expected at least {at_least}");
    }
    let valid: Vec<&Value> =
        root["valid"].as_array().unwrap().iter().filter(|c| c["name"].as_str().unwrap().starts_with("link-")).collect();
    assert!(valid.iter().any(|c| c["from"] == "pc") && valid.iter().any(|c| c["from"] == "phone"), "links go both ways");
    for c in valid {
        assert_eq!(c["expect"]["action"], "deliver_link", "{}", c["name"]);
    }
    for c in root["invalid"].as_array().unwrap().iter().filter(|c| c["name"].as_str().unwrap().starts_with("link-")) {
        assert_eq!(c["expect"]["code"], "bad_request", "{}", c["name"]);
        let close = if c["state"] == "before_hello" { json!(1002) } else { json!(null) };
        assert_eq!(c["expect"]["close"], close, "{}: only a link before the hello closes the connection", c["name"]);
    }
}

// ----- links.json -------------------------------------------------------------------------------------------------

fn synthesized(c: &Value) -> String {
    let s = &c["synthesize"];
    format!(
        "{}{}{}",
        s["prefix"].as_str().unwrap(),
        s["fill"].as_str().unwrap().repeat(s["fillCount"].as_u64().unwrap() as usize),
        s["suffix"].as_str().unwrap_or("")
    )
}

#[test]
fn links_json_urls() {
    let root = load("links.json");
    assert_eq!(root["maxUrlBytes"], links::MAX_URL_BYTES);
    let mut checked = 0;
    for c in root["urls"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let text = if c.get("synthesize").is_some() { synthesized(c) } else { c["input"].as_str().unwrap().to_string() };
        if c.get("synthesize").is_some() {
            assert_eq!(text.len() as u64, c["synthesize"]["bytes"].as_u64().unwrap(), "{name}: the synthesized size");
        }
        assert_eq!(links::is_link_url(&text), c["valid"].as_bool().unwrap(), "{name}");
        // The same URL in a `link` frame: delivered as it is (an unchanged URL is the same string), or answered with bad_request.
        let mut dedup = MemoryDedup::default();
        let mut rx = open_receiver(Role::Phone, &mut dedup);
        let frame = json!({ "type": "link", "id": "u1", "url": text });
        let out = rx.handle_frame(Frame::Text(frame.to_string().as_bytes()), &mut dedup);
        if c["valid"] == true {
            match out.action {
                messages::Action::DeliverLink { url, .. } => assert_eq!(url, text, "{name}: delivered unchanged"),
                other => panic!("{name}: expected a link, got {other:?}"),
            }
        } else {
            assert_eq!(out.reply.as_ref().map(|r| r["code"].clone()), Some(json!("bad_request")), "{name}");
            assert_eq!(out.reply.as_ref().and_then(|r| r.get("ref")), Some(&json!("u1")), "{name}");
        }
        checked += 1;
    }
    assert!(checked >= 40, "{checked} URLs");
}

/// `display[]` (changelog 55): the address the PC shows and whether it has a warning (a warning means it is never opened automatically).
#[test]
fn links_json_display() {
    let root = load("links.json");
    let cases = root["display"].as_array().unwrap();
    assert!(cases.len() >= 40, "{} display cases", cases.len());
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let url = c["url"].as_str().unwrap();
        assert!(links::is_link_url(url), "{name}: the URL is valid");
        let got = links::link_address(url);
        assert_eq!(got.display, c["display"].as_str().unwrap(), "{name}: the address");
        assert_eq!(got.warn, c["warn"].as_bool().unwrap(), "{name}: the warning");
        // The toast says it: a warning puts the sign in front of the address, and decide() then never opens the link by itself.
        assert_eq!(got.shown().starts_with('\u{26a0}'), got.warn, "{name}");
        let decision = kin_lib::phonelinks::decide(true, kin_lib::quiet::UserState::AcceptsNotifications, got.warn);
        assert_eq!(decision == kin_lib::phonelinks::LinkDecision::Open, !got.warn, "{name}: opened by itself only without a warning");
    }
}

#[test]
fn links_json_titles_and_source_apps() {
    let root = load("links.json");
    for (key, max) in [("titles", links::MAX_LINK_TITLE_CHARS), ("sourceApps", links::MAX_LINK_SOURCE_CHARS)] {
        assert_eq!(root[if key == "titles" { "maxTitleCodePoints" } else { "maxSourceAppCodePoints" }], max);
        for c in root[key].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let got = links::clean_link_text(c["input"].as_str().unwrap(), max);
            assert_eq!(got.as_deref(), c["expected"].as_str(), "{key} {name}");
        }
    }
}

/// What this PC does with a `link` for a peer whose `hello`s were these: the connection's `wire_form` after the receiver took the hellos.
#[test]
fn links_json_send_as_and_text_fallback() {
    let root = load("links.json");
    for c in root["sendAs"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut dedup = MemoryDedup::default();
        let mut rx = Receiver::new(Role::Phone);
        for h in c["hellos"].as_array().unwrap() {
            let mut hello = json!({ "type": "hello", "id": "h", "protocol": 1, "name": "X", "platform": "android", "appVersion": "0.2.0" });
            for (k, v) in h.as_object().unwrap() {
                hello[k] = v.clone();
            }
            let out = rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), &mut dedup);
            assert!(matches!(out.action, messages::Action::HandshakeOk), "{name}: a bad capabilities never fails a hello");
        }
        let link = json!({ "type": "link", "id": "L1", "url": "https://example.com/a?t=1", "title": "A page", "sourceApp": "Browser" });
        let wire = kin_lib::ws::wire_form(link.clone(), rx.peer());
        match c["expected"].as_str().unwrap() {
            "link" => assert_eq!(wire, link, "{name}"),
            "text" => assert_eq!(wire, json!({ "type": "text", "id": "L1", "text": "https://example.com/a?t=1", "isUrl": true }), "{name}"),
            other => panic!("{name}: {other}"),
        }
        // Everything that is not a link goes as it is.
        let text = json!({ "type": "text", "id": "T1", "text": "x", "isUrl": false });
        assert_eq!(kin_lib::ws::wire_form(text.clone(), rx.peer()), text, "{name}");
    }
    for c in root["textFallback"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let link = json!({ "type": "link", "id": "L1", "url": c["link"]["url"], "title": c["link"]["title"], "sourceApp": c["link"]["sourceApp"] });
        let wire = kin_lib::ws::wire_form(link, None);
        let mut expected = c["expected"].clone();
        expected["id"] = json!("L1");
        assert_eq!(wire, expected, "{name}");
    }
}

/// This PC says in its own `hello` that it understands `link` (the phone sends a `link` only to a peer that says so), `media` (always: its switches travel in
/// `media.status`, changelog 57) and `presence` (always, changelog 58). It lists nothing for `ring`: it only ever receives `ring.status`.
#[test]
fn the_pcs_hello_lists_the_link_media_and_presence_capabilities() {
    assert_eq!(messages::CAPABILITIES, ["link", "media", "presence"]);
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

// ----- notif.json and the notif-* cases (2B.2, changelog 56) ------------------------------------------------------------------

use kin_lib::mirror::{args as toast_args, limits as mirror_limits, wire};

/// The `notif-*` cases are in the runs above. This pins how many there are, so a dropped vector is noticed here.
#[test]
fn notif_scope_is_what_the_readme_says() {
    let root = load("messages.json");
    for (key, scope, at_least) in [
        ("valid", MESSAGES_VALID_SCOPE_2B2, 30),
        ("invalid", MESSAGES_INVALID_SCOPE_2B2, 55),
        ("sequences", MESSAGES_SEQUENCE_SCOPE_2B2, 10),
    ] {
        assert_patterns_match(key, scope, &names(&root[key]));
        let n = root[key].as_array().unwrap().iter().filter(|c| matches(c["name"].as_str().unwrap(), scope)).count();
        assert!(n >= at_least, "messages.json {key}[]: {n} notif cases, expected at least {at_least}");
    }
    // Only notif.action is acked; the other six are ephemeral and never get a reply.
    for c in root["valid"].as_array().unwrap().iter().filter(|c| c["name"].as_str().unwrap().starts_with("notif-")) {
        let ty = c["frame"]["type"].as_str().unwrap();
        let acked = c["expect"]["action"] != "notif_dropped" && ty == "notif.action";
        assert_eq!(c["expect"]["reply"], if acked { "ack" } else { "none" }, "{}", c["name"]);
    }
    let notif = load("notif.json");
    for key in [
        "nid",
        "iconHash",
        "icon",
        "clean",
        "post",
        "code",
        "toastIds",
        "toastArguments",
        "rate",
        "coalesce",
        "sendNotif",
        "syncDiff",
        "toastLabel",
    ] {
        assert!(!notif[key].as_array().unwrap().is_empty(), "notif.json {key}[]");
    }
}

fn limits_match(notif: &Value) {
    let l = &notif["limits"];
    let pairs: [(&str, usize); 17] = [
        ("app", wire::MAX_APP),
        ("title", wire::MAX_TITLE),
        ("text", wire::MAX_TEXT),
        ("bigText", wire::MAX_BIG_TEXT),
        ("subText", wire::MAX_SUB_TEXT),
        ("conversation", wire::MAX_CONVERSATION),
        ("messageSender", wire::MAX_MESSAGE_SENDER),
        ("messageText", wire::MAX_MESSAGE_TEXT),
        ("line", wire::MAX_LINE),
        ("hint", wire::MAX_HINT),
        ("label", wire::MAX_LABEL),
        ("messages", wire::MAX_MESSAGES),
        ("lines", wire::MAX_LINES),
        ("actions", wire::MAX_ACTIONS),
        ("nids", wire::MAX_SYNC_NIDS),
        ("actionText", wire::MAX_ACTION_TEXT),
        ("iconBytes", wire::MAX_ICON_BYTES),
    ];
    for (key, value) in pairs {
        assert_eq!(l[key], value, "limits.{key}");
    }
    assert_eq!(l["frameBytes"], wire::MAX_FRAME_BYTES);
    assert_eq!(l["iconFrameBytes"], wire::MAX_ICON_FRAME_BYTES);
    assert_eq!((l["iconMin"].as_u64(), l["iconMax"].as_u64()), (Some(24), Some(128)));
}

fn base64_url_decode(text: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).expect("valid base64url")
}

#[test]
fn notif_json_nid_icons_codes_and_cleaning() {
    let notif = load("notif.json");
    limits_match(&notif);
    for c in notif["nid"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        assert_eq!(wire::nid_of_key(c["key"].as_str().unwrap()), c["nid"].as_str().unwrap(), "nid {name}");
    }
    for c in notif["iconHash"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let bytes = base64_url_decode(c["png"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, c["bytes"].as_u64().unwrap(), "iconHash {name}");
        assert_eq!(wire::hex32(&sha256(&bytes)), c["hash"].as_str().unwrap(), "iconHash {name}");
        assert_eq!(base64url(&bytes), c["png"].as_str().unwrap(), "iconHash {name}: written without padding");
    }
    for c in notif["code"].as_array().unwrap() {
        assert_eq!(wire::valid_code(c["input"].as_str().unwrap()), c["valid"].as_bool().unwrap(), "code {}", c["name"]);
    }
    let max_of = |field: &str| match field {
        "app" => wire::MAX_APP,
        "title" => wire::MAX_TITLE,
        "text" => wire::MAX_TEXT,
        "bigText" => wire::MAX_BIG_TEXT,
        "subText" => wire::MAX_SUB_TEXT,
        "conversation" => wire::MAX_CONVERSATION,
        "messageSender" => wire::MAX_MESSAGE_SENDER,
        "messageText" => wire::MAX_MESSAGE_TEXT,
        "line" => wire::MAX_LINE,
        "hint" => wire::MAX_HINT,
        "label" => wire::MAX_LABEL,
        other => panic!("unknown field {other}"),
    };
    for c in notif["clean"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let got = links::clean_link_text(c["input"].as_str().unwrap(), max_of(c["field"].as_str().unwrap()));
        assert_eq!(got.as_deref(), c["expected"].as_str(), "clean {name}");
    }
}

/// `icon[]`: a `notif.icon` frame through the real receiver.
#[test]
fn notif_json_icons_through_the_receiver() {
    let notif = load("notif.json");
    for c in notif["icon"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut frame = json!({ "type": "notif.icon", "id": "icon-case" });
        if let Some(h) = c.get("hash") {
            frame["hash"] = h.clone();
        }
        if let Some(file) = c.get("pngFile").and_then(Value::as_str) {
            frame["png"] = json!(base64url(&std::fs::read(protocol_dir().join(file)).unwrap()));
        } else if let Some(p) = c.get("png") {
            frame["png"] = p.clone();
        }
        let mut dedup = MemoryDedup::default();
        let mut rx = open_receiver_with(Role::Pc, &mut dedup, Some(&json!(["notif"])), Some(&json!(["notif"])));
        let out = rx.handle_frame(Frame::Text(frame.to_string().as_bytes()), &mut dedup);
        let got = match (&out.action, out.reply.as_ref().and_then(|r| r["code"].as_str())) {
            (messages::Action::Notif(wire::NotifMsg::Icon { .. }), None) => "accepted",
            (messages::Action::NotifDropped(messages::NotifDrop::BadIcon), None) => "dropped",
            (_, Some("bad_request")) => "bad_request",
            (other, code) => panic!("{name}: {other:?} {code:?}"),
        };
        assert_eq!(got, c["expect"].as_str().unwrap(), "icon {name}");
        assert!(out.reply.is_none() || got == "bad_request", "{name}: an icon is never acked");
    }
}

/// `post[]`: cases in the format of `messages.json`, run as the others are.
#[test]
fn notif_json_posts_through_the_receiver() {
    let notif = load("notif.json");
    let cases = notif["post"].as_array().unwrap();
    assert!(cases.len() >= 70, "{} post cases", cases.len());
    for case in cases {
        run_message_case(case);
    }
}

#[test]
fn notif_json_toast_names_and_arguments() {
    use toast_args::{ToastAction, ToastArgs};
    let notif = load("notif.json");
    for c in notif["toastIds"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let (short, nid) = (c["shortId"].as_str().unwrap(), c["nid"].as_str().unwrap());
        assert_eq!(toast_args::tag_of(nid), c["tag"].as_str().unwrap(), "{name}");
        assert_eq!(toast_args::group_of(short), c["group"].as_str().unwrap(), "{name}");
        assert_eq!(ToastArgs::new(short, nid, ToastAction::Tap, None).to_arguments(), c["launch"].as_str().unwrap(), "{name}");
        assert_eq!(ToastArgs::new(short, nid, ToastAction::Copy, None).to_arguments(), c["copy"].as_str().unwrap(), "{name}");
        let reply_i = c["reply"]["i"].as_u64().unwrap() as u8;
        assert_eq!(
            ToastArgs::new(short, nid, ToastAction::Reply, Some(reply_i)).to_arguments(),
            c["reply"]["arguments"].as_str().unwrap(),
            "{name}"
        );
        let button_i = c["button"]["i"].as_u64().unwrap() as u8;
        assert_eq!(
            ToastArgs::new(short, nid, ToastAction::Button, Some(button_i)).to_arguments(),
            c["button"]["arguments"].as_str().unwrap(),
            "{name}"
        );
    }
    for c in notif["toastArguments"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let got = toast_args::parse(c["input"].as_str().unwrap());
        match c["valid"].as_bool().unwrap() {
            false => assert_eq!(got, None, "toastArguments {name}"),
            true => {
                let got = got.unwrap_or_else(|| panic!("toastArguments {name}: refused"));
                let want = &c["parsed"];
                assert_eq!(got.device, want["d"].as_str().unwrap(), "{name}");
                assert_eq!(got.nid, want["n"].as_str().unwrap(), "{name}");
                assert_eq!(got.index.map(u64::from), want["i"].as_u64(), "{name}");
                let word = match got.action {
                    ToastAction::Tap => "tap",
                    ToastAction::Reply => "reply",
                    ToastAction::Copy => "copy",
                    ToastAction::Button => "btn",
                };
                assert_eq!(word, want["a"].as_str().unwrap(), "{name}");
            }
        }
    }
}

/// `rate[]` for the PC's limiter (the phone's budget is the phone's own).
#[test]
fn notif_json_the_pcs_rate_limits() {
    let notif = load("notif.json");
    let mut ran = 0;
    for c in notif["rate"].as_array().unwrap() {
        if c["limiter"] != "pc" {
            continue;
        }
        let name = c["name"].as_str().unwrap();
        let mut limiter = mirror_limits::PostLimiter::default();
        let (mut got, mut errors) = (Vec::new(), Vec::new());
        for e in c["events"].as_array().unwrap() {
            let t = e["t"].as_u64().unwrap();
            // A stand-in for the payload: any stable hash of the case's content word.
            let content =
                e["content"].as_str().unwrap().bytes().fold(14695981039346656037u64, |h, b| (h ^ u64::from(b)).wrapping_mul(1099511628211));
            got.push(match limiter.post(t, e["nid"].as_str().unwrap(), content) {
                mirror_limits::Verdict::Accepted => "accepted",
                mirror_limits::Verdict::IgnoredDuplicate => "ignored_duplicate",
                mirror_limits::Verdict::DroppedRate { send_error } => {
                    if send_error {
                        errors.push(t);
                    }
                    "dropped_rate"
                }
            });
        }
        let want: Vec<&str> = c["expect"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(got, want, "rate {name}");
        let want_errors: Vec<u64> = c["errorsAt"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(errors, want_errors, "rate {name}: when the rate_limited errors go");
        ran += 1;
    }
    assert!(ran >= 6, "{ran} PC rate cases");
}

/// `sendNotif[]` and `syncDiff[]`.
#[test]
fn notif_json_when_to_send_and_what_a_sync_removes() {
    let notif = load("notif.json");
    for c in notif["sendNotif"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut dedup = MemoryDedup::default();
        let mut rx = Receiver::new(Role::Phone);
        for h in c["hellos"].as_array().unwrap() {
            let mut hello = json!({ "type": "hello", "id": "h", "protocol": 1, "name": "X", "platform": "windows", "appVersion": "0.2.0" });
            for (k, v) in h.as_object().unwrap() {
                hello[k] = v.clone();
            }
            let out = rx.handle_frame(Frame::Text(hello.to_string().as_bytes()), &mut dedup);
            assert!(matches!(out.action, messages::Action::HandshakeOk), "{name}");
        }
        let may = rx.peer().is_some_and(|p| p.has(wire::CAPABILITY));
        assert_eq!(if may { "send" } else { "nothing" }, c["expected"].as_str().unwrap(), "{name}");
    }
    for c in notif["syncDiff"].as_array().unwrap() {
        let strings = |k: &str| c[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(kin_lib::mirror::sync_remove(&strings("shown"), &strings("nids")), strings("remove"), "syncDiff {}", c["name"]);
    }
}

/// `toastLabel[]`: a hint or a button label as it is on the toast and in `notif.action.label`.
#[test]
fn notif_json_a_label_in_the_form_of_the_toast() {
    let notif = load("notif.json");
    let cases = notif["toastLabel"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for c in cases {
        let got = kin_lib::mirror::toast_xml::toast_label(c["input"].as_str().unwrap());
        assert_eq!(got, c["expected"].as_str().unwrap(), "toastLabel {}", c["name"]);
        assert_eq!(kin_lib::mirror::toast_xml::toast_label(&got), got, "idempotent: {}", c["name"]);
    }
}
