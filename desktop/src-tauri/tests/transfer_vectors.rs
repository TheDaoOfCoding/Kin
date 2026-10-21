//! The vectors of milestone 1.3 (protocol/README.md, "Milestone 1.3"), run against the real code:
//!
//! * `file-names.json`: `cases[]` and `createNew[]` (both operations), with a fake that answers "already exists" and again with the
//!   real create and the real no-replace move in a temporary folder,
//! * `urls.json` and the line ends and size threshold of `text-rules.json`,
//! * `http.json`: every transfer exchange and sequence (`transfers-*`, `put-*`, `get-*`, `delete-*`, `internal-error`, `upload-*`,
//!   `download-*`, ...), in-process against the router with a core in the state the case's `context` describes.
//!
//! Nothing here touches the real Downloads folder or the real clipboard.
mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use common::*;
use futures_util::stream;
use kin_lib::clipboard::{self, text_hash};
use kin_lib::core::Core;
use kin_lib::filenames::{self, CreateError};
use kin_lib::server::{self, Peer};
use kin_lib::sync::{Kind, SendPlan, plan_send};
use serde_json::Value;
use tower::ServiceExt;

fn matches(name: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| p.strip_suffix('*').map_or(name == *p, |prefix| name.starts_with(prefix)))
}

fn names(cases: &Value) -> Vec<&str> {
    cases.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

// ----- file-names.json ---------------------------------------------------------------------------------------------------

#[test]
fn file_names_cases() {
    let root = load("file-names.json");
    assert_eq!(root["maxLength"], 200);
    assert_eq!(root["maxExtensionBytes"], 16);
    let mut n = 0;
    for case in root["cases"].as_array().unwrap() {
        let existing = strings(&case["existing"]);
        let got = filenames::clean_file_name(case["input"].as_str().unwrap(), &existing);
        assert_eq!(got, case["expected"].as_str().unwrap(), "{}: input {:?}", case["name"], case["input"]);
        n += 1;
    }
    assert!(n >= 100, "{n} cases");
}

/// The create-new loop with a fake create: a name in `onDisk` says "already exists" (compared case-insensitively, as NTFS does),
/// `createFails` is any other failure.
fn fake_create(case: &Value) -> Result<String, String> {
    let on_disk: HashSet<String> = strings(&case["onDisk"]).iter().map(|n| filenames::fold(n)).collect();
    let fails = case.get("createFails").and_then(Value::as_str).map(str::to_string);
    let mut existing: HashSet<String> = strings(&case["existing"]).iter().map(|n| filenames::fold(n)).collect();
    filenames::create_new(case["input"].as_str().unwrap(), &mut existing, |name| {
        if on_disk.contains(&filenames::fold(name)) {
            Err(CreateError::Exists)
        } else if let Some(reason) = &fails {
            Err(CreateError::Other(reason.clone()))
        } else {
            Ok(())
        }
    })
    .map(|(name, ())| name)
}

fn expected_of(case: &Value) -> Result<String, ()> {
    match &case["expected"] {
        Value::String(s) => Ok(s.clone()),
        other => {
            assert_eq!(other["fails"], "internal", "{}", case["name"]);
            Err(())
        }
    }
}

#[test]
fn file_names_create_new_with_a_fake_create() {
    let root = load("file-names.json");
    let mut moves = 0;
    for case in root["createNew"].as_array().unwrap() {
        let got = fake_create(case).map_err(|_| ());
        assert_eq!(got, expected_of(case), "{}", case["name"]);
        moves += usize::from(case["operation"] == "move_part");
    }
    assert!(moves >= 4, "{moves} move_part cases");
}

/// The same cases with the real thing, the way the app does it: the `.part` file is created with `CREATE_NEW` under the first free
/// name, and when the last byte is in it is moved to its final name with the no-replace move, which is where a file that appeared
/// in the meantime is noticed (a replacing rename would fail `move-part-case-folding-by-the-filesystem`). Whatever was in the
/// folder before is still there afterwards, untouched.
#[test]
fn file_names_create_new_in_a_real_folder() {
    let root = load("file-names.json");
    for case in root["createNew"].as_array().unwrap() {
        if case.get("createFails").is_some() {
            continue; // a failure that is not "already exists" can't be made to order in a folder
        }
        let dir = tempfile::tempdir().unwrap();
        for name in strings(&case["onDisk"]) {
            std::fs::write(dir.path().join(&name), format!("theirs:{name}")).unwrap();
        }
        let mut existing: HashSet<String> = strings(&case["existing"]).iter().map(|n| filenames::fold(n)).collect();
        let input = case["input"].as_str().unwrap();
        // All 100 candidates taken (a case of the limit): the `.part` file can't get a name, and nothing in the folder changes.
        if expected_of(case).is_err() {
            // (the app lists the folder first, so `existing` holds what is on disk)
            existing.extend(filenames::existing_names(dir.path()));
            let failed = filenames::create_new(input, &mut existing, |n| filenames::create_part(dir.path(), n));
            assert!(failed.is_err(), "{}: the limit of {} candidates fails the file", case["name"], filenames::MAX_CANDIDATES);
            let count = std::fs::read_dir(dir.path()).unwrap().count();
            assert_eq!(count, strings(&case["onDisk"]).len(), "{}: nothing was created", case["name"]);
            continue;
        }
        // `existing` can be stale: the files of `onDisk` are really there. The `.part` file takes the first name it likes ...
        let (first, handle) = filenames::create_new(input, &mut existing, |n| filenames::create_part(dir.path(), n)).unwrap();
        drop(handle);
        std::fs::write(filenames::part_path(dir.path(), &first), "mine").unwrap();
        // ... and the move to the final name is the real collision check.
        let got = filenames::finish_part(dir.path(), &first, &mut HashSet::new());
        assert_eq!(got.map_err(|_| ()), expected_of(case), "{}", case["name"]);
        let final_name = expected_of(case).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join(&final_name)).unwrap(), "mine", "{}", case["name"]);
        // Nothing that was there has been replaced.
        for name in strings(&case["onDisk"]) {
            assert_eq!(std::fs::read_to_string(dir.path().join(&name)).unwrap(), format!("theirs:{name}"), "{}: {name}", case["name"]);
        }
    }
}

// ----- urls.json ---------------------------------------------------------------------------------------------------------

#[test]
fn urls() {
    let root = load("urls.json");
    assert_eq!(root["maxBytes"], kin_lib::links::MAX_URL_BYTES);
    let mut accepted = 0;
    for case in root["cases"].as_array().unwrap() {
        let text = match case.get("synthesize") {
            Some(s) => {
                let text = format!(
                    "{}{}{}",
                    s["prefix"].as_str().unwrap(),
                    s["fill"].as_str().unwrap().repeat(s["fillCount"].as_u64().unwrap() as usize),
                    s["suffix"].as_str().unwrap_or("")
                );
                assert_eq!(kin_lib::links::trim(&text).len() as u64, s["trimmedBytes"].as_u64().unwrap(), "{}", case["name"]);
                text
            }
            None => case["input"].as_str().unwrap().to_string(),
        };
        let want = case["isUrl"].as_bool().unwrap();
        assert_eq!(kin_lib::links::is_openable_url(&text), want, "{}: {:?}", case["name"], text.chars().take(100).collect::<String>());
        // What a receiver does with it: opens only when the sender said so and the rule agrees.
        assert_eq!(kin_lib::links::openable_url(&text, true).is_some(), want, "{}", case["name"]);
        assert!(kin_lib::links::openable_url(&text, false).is_none(), "{}: isUrl false never opens", case["name"]);
        accepted += usize::from(want);
    }
    assert!(accepted >= 25 && root["cases"].as_array().unwrap().len() >= 100);
}

// ----- text-rules.json ---------------------------------------------------------------------------------------------------

#[test]
fn line_ends_on_the_wire_and_on_the_windows_clipboard() {
    let root = load("text-rules.json");
    for case in root["lineEndings"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let wire = case["wire"].as_str().unwrap();
        if let Some(text) = case.get("clipboardText").and_then(Value::as_str) {
            assert_eq!(clipboard::to_wire(text), wire, "{name}: toWire");
        }
        assert_eq!(clipboard::to_windows_newlines(wire), case["windowsClipboard"].as_str().unwrap(), "{name}: the Windows clipboard form");
        // The loop guard hashes the wire form, so both forms of one text are the same text to it.
        assert_eq!(hex(&text_hash(wire)), case["wireSha256Hex"].as_str().unwrap(), "{name}: hash of the wire form");
        assert_eq!(
            hex(&text_hash(&clipboard::to_wire(&clipboard::to_windows_newlines(wire)))),
            hex(&text_hash(&clipboard::to_wire(wire))),
            "{name}"
        );
    }
}

#[test]
fn the_sender_threshold() {
    let root = load("text-rules.json");
    assert_eq!(root["senderThresholdBytes"], kin_lib::sync::SENDER_THRESHOLD_BYTES);
    for case in root["senderSize"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut wire_bytes = case["wireBytes"].as_u64().unwrap() as usize;
        if let Some(s) = case.get("synthesize") {
            let text = format!(
                "{}{}",
                s["fill"].as_str().unwrap().repeat(s["fillCount"].as_u64().unwrap() as usize),
                s["suffix"].as_str().unwrap_or("")
            );
            assert_eq!(text.len() as u64, s["rawBytes"].as_u64().unwrap(), "{name}: raw size");
            wire_bytes = clipboard::to_wire(&text).len();
            assert_eq!(wire_bytes as u64, case["wireBytes"].as_u64().unwrap(), "{name}: the size is measured after the conversion");
        }
        let kind = if case["kind"] == "clipboard" { Kind::Clipboard } else { Kind::Text };
        let sensitive = case.get("sensitive").and_then(Value::as_bool).unwrap_or(false);
        let got = match plan_send(kind, sensitive, wire_bytes) {
            SendPlan::Text => "text",
            SendPlan::Clipboard => "clipboard",
            SendPlan::File => "file",
            SendPlan::TooLarge => "too_large",
            SendPlan::NotSent => "not_sent",
        };
        assert_eq!(got, case["expected"].as_str().unwrap(), "{name}");
    }
}

// ----- http.json: the transfer exchanges and sequences -----------------------------------------------------------------

const HTTP_EXCHANGE_SCOPE_1_3: &[&str] = &["transfers-*", "put-*", "get-*", "delete-*", "internal-error"];
const HTTP_SEQUENCE_SCOPE_1_3: &[&str] =
    &["upload-*", "download-*", "repeat-with-different-files-keeps-the-stored-list", "delete-*", "transfer-id-*"];

/// A core in the state a case's `context` describes: the free space, the size limit, the transfer or offer that exists, and the
/// device that asks. The PC is vector key 1.
struct Setup {
    env: TestEnv,
    client: String,
    #[allow(dead_code)]
    sources: tempfile::TempDir,
}

fn setup(context: &Value) -> Setup {
    let keys = vector_keys();
    let client = context["client"]["deviceId"].as_str().unwrap().to_string();
    let server = &context["server"];
    let has_transfer = context["transfer"].is_object();
    let sources = tempfile::tempdir().unwrap();
    // `createFails` is a real failure: with no transfer yet the folder for received files can't be created (a file is in its
    // way); with one under way the folder is replaced by a file after the `.part` files were made.
    let blocker = sources.path().join("blocker");
    let create_fails = server["createFails"] == true;
    let env = test_core_with(identity_of(&keys[0]), "Desk PC", |config| {
        if create_fails && !has_transfer {
            std::fs::write(&blocker, "a file where a folder is needed").unwrap();
            config.dev_received_dir = Some(blocker.join("received"));
        }
    });
    let core = &env.core;
    if context["client"]["paired"] == true {
        let device =
            kin_lib::storage::NewDevice { device_id: &client, name: "Test Phone", platform: "android", model: None, app_version: None };
        core.db.upsert_device(&device, 1).unwrap();
    }
    if server["diskFull"] == true {
        core.set_fake_free_bytes(Some(0));
    }
    if let Some(n) = server["freeBytes"].as_u64() {
        core.set_fake_free_bytes(Some(n));
    }
    if server["tooLarge"] == true {
        core.hooks.max_file_bytes.store(1 << 20, std::sync::atomic::Ordering::Relaxed);
    }
    if server["bug"] == true {
        core.hooks.fail_internal.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(t) = context["transfer"].as_object() {
        let owner = t.get("owner").and_then(Value::as_str).unwrap_or(&client);
        let files: Vec<(String, u64, u64)> = match t.get("files").and_then(Value::as_object) {
            Some(f) => f.iter().map(|(id, v)| (id.clone(), v["size"].as_u64().unwrap(), v["stored"].as_u64().unwrap_or(0))).collect(),
            None => vec![("f1".to_string(), 1, 0)],
        };
        let refs: Vec<(&str, u64, u64)> = files.iter().map(|(i, s, d)| (i.as_str(), *s, *d)).collect();
        core.seed_upload(owner, t["id"].as_str().unwrap(), &refs);
        if create_fails {
            let dir = core.received_dir();
            std::fs::remove_dir_all(&dir).unwrap();
            std::fs::write(&dir, "a file where the folder was").unwrap();
        }
    }
    if let Some(o) = context.get("offer").filter(|o| o.is_object()) {
        let owner = o.get("owner").and_then(Value::as_str).unwrap_or(&client);
        let mut made: Vec<(String, PathBuf)> = Vec::new();
        for (id, f) in o["files"].as_object().unwrap() {
            let path = sources.path().join(format!("{id}.bin"));
            std::fs::File::create(&path).unwrap().set_len(f["size"].as_u64().unwrap()).unwrap();
            made.push((id.clone(), path));
        }
        let refs: Vec<(&str, &std::path::Path)> = made.iter().map(|(i, p)| (i.as_str(), p.as_path())).collect();
        core.seed_offer(owner, o["id"].as_str().unwrap(), &refs, o["expired"] == true);
        if o["sourceChanged"] == true {
            // The modified time moves on (the size stays what the offer said).
            for (_, path) in &made {
                let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)).unwrap();
            }
        }
    }
    Setup { env, client, sources }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Sends one request through the router with the peer attached, as the accept loop does. `cut` ends a body early (an upload whose
/// connection broke), and for a download it stops reading after that many bytes.
async fn send(core: &Arc<Core>, client: &str, request: &Value, cut: Option<u64>) -> Reply {
    let method = request["method"].as_str().unwrap();
    let mut builder = Request::builder().method(method).uri(request["path"].as_str().unwrap());
    for (n, v) in request["headers"].as_object().unwrap() {
        builder = builder.header(n.as_str(), v.as_str().unwrap());
    }
    let body = if method == "PUT" {
        // Raw bytes, as many as the case says, in pieces of 64 KiB; a cut ends the stream with an error.
        let total = request["bodyBytes"].as_u64().unwrap_or(0);
        let send_n = cut.map_or(total, |c| c.min(total));
        let mut items: Vec<Result<axum::body::Bytes, std::io::Error>> = Vec::new();
        let mut left = send_n;
        while left > 0 {
            let n = left.min(65536) as usize;
            items.push(Ok(axum::body::Bytes::from(vec![0u8; n])));
            left -= n as u64;
        }
        if cut.is_some_and(|c| c < total) {
            items.push(Err(std::io::Error::other("the connection broke")));
        }
        Body::from_stream(stream::iter(items))
    } else if let Some(raw) = request.get("bodyRaw") {
        Body::from(raw.as_str().unwrap().as_bytes().to_vec())
    } else if let Some(n) = request.get("bodyBytes").and_then(Value::as_u64) {
        let overhead = r#"{"pad":""}"#.len();
        Body::from(format!(r#"{{"pad":"{}"}}"#, "a".repeat(n as usize - overhead)))
    } else {
        match request.get("body") {
            Some(Value::String(s)) => Body::from(s.clone()),
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        }
    };
    let mut req = builder.body(body).unwrap();
    req.extensions_mut().insert(Peer { device_id: client.to_string(), remote: "127.0.0.1:1".parse().unwrap() });
    let response = server::router(core.clone()).oneshot(req).await.unwrap();
    let status = response.status().as_u16();
    let headers: Vec<(String, String)> =
        response.headers().iter().map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap().to_string())).collect();
    let bytes = if let (true, Some(limit)) = (method == "GET", cut) {
        // A download that is cut off: read the first bytes and drop the rest.
        use http_body_util::BodyExt;
        let mut body = response.into_body();
        let mut got = Vec::new();
        while (got.len() as u64) < limit {
            match body.frame().await {
                Some(Ok(f)) => {
                    if let Some(d) = f.data_ref() {
                        got.extend_from_slice(d);
                    }
                }
                _ => break,
            }
        }
        got.truncate(limit as usize);
        got
    } else {
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    };
    Reply { status, headers, body: bytes }
}

fn check_reply(name: &str, got: &Reply, want: &Value) {
    assert_eq!(got.status as u64, want["status"].as_u64().unwrap(), "{name}: status (body {:?})", String::from_utf8_lossy(&got.body));
    for (header, value) in want["headers"].as_object().into_iter().flatten() {
        let value = value.as_str().unwrap();
        assert_eq!(got.header(header), Some(value), "{name}: header {header}");
    }
    if let Some(n) = want.get("bodyBytes").and_then(Value::as_u64) {
        assert_eq!(got.body.len() as u64, n, "{name}: body length");
        return;
    }
    if want["status"] == 204 {
        assert!(got.body.is_empty(), "{name}: a 204 has no body");
        return;
    }
    // A JSON body: the message of an error is only an example, so it has to be there but is not compared.
    let mut body: Value =
        serde_json::from_slice(&got.body).unwrap_or_else(|e| panic!("{name}: the body is not JSON ({e}): {:?}", got.body));
    let mut want_body = want["body"].clone();
    if want_body.get("message").is_some() {
        assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()), "{name}: no message in {body}");
        body.as_object_mut().unwrap().remove("message");
        want_body.as_object_mut().unwrap().remove("message");
    }
    assert_eq!(body, want_body, "{name}: body");
}

#[tokio::test]
async fn transfer_exchanges() {
    let root = load("http.json");
    let mut run = 0;
    for case in root["exchanges"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !matches(name, HTTP_EXCHANGE_SCOPE_1_3) {
            continue;
        }
        let setup = setup(&case["context"]);
        let got = send(&setup.env.core, &setup.client, &case["request"], None).await;
        check_reply(name, &got, &case["response"]);
        run += 1;
    }
    eprintln!("http.json exchanges[], 1.3: {run} cases");
    assert!(run >= 100, "only {run} exchanges ran");
    for p in HTTP_EXCHANGE_SCOPE_1_3 {
        assert!(names(&root["exchanges"]).iter().any(|n| matches(n, &[p])), "the pattern {p} matches no case");
    }
}

#[tokio::test]
async fn transfer_sequences() {
    let root = load("http.json");
    let mut run = 0;
    for seq in root["sequences"].as_array().unwrap() {
        let seq_name = seq["name"].as_str().unwrap();
        if !matches(seq_name, HTTP_SEQUENCE_SCOPE_1_3) {
            continue;
        }
        let setup = setup(&seq["context"]);
        for (i, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
            let label = format!("{seq_name} step {}", i + 1);
            if let Some(clock) = step.get("clock") {
                // Time passes on the PC: a finished upload is then older by that many hours.
                let hours = clock["advanceHours"].as_u64().unwrap();
                for entry in setup.env.core.transfers.all() {
                    if let Some(finished) = entry.meta.lock().unwrap().finished_ms.as_mut() {
                        *finished -= hours * 3_600_000;
                    }
                }
                continue;
            }
            let cut = step.get("interruptedAfterBytes").and_then(Value::as_u64);
            let got = send(&setup.env.core, &setup.client, &step["request"], cut).await;
            if step["response"].is_null() {
                assert!(cut.is_some(), "{label}: a step with no response is cut off");
            } else {
                check_reply(&label, &got, &step["response"]);
            }
        }
        run += 1;
    }
    assert!(run >= 6, "only {run} sequences ran");
}

/// What the vectors can't see: a `PUT` that completes a file moves it to its final name, with the bytes the body carried.
#[tokio::test]
async fn a_completed_upload_is_a_file_with_its_name_and_its_bytes() {
    let keys = vector_keys();
    let env = test_core(identity_of(&keys[0]), "Desk PC");
    let phone = keys[1].device_id.clone();
    let device = kin_lib::storage::NewDevice { device_id: &phone, name: "Test Phone", platform: "android", model: None, app_version: None };
    env.core.db.upsert_device(&device, 1).unwrap();
    let post = serde_json::json!({
        "method": "POST", "path": "/v1/transfers", "headers": { "Content-Type": "application/json" },
        "body": { "id": "t1", "files": [{ "fileId": "f1", "name": "..\\..\\evil:name?.txt", "size": 5, "mime": "text/plain" }] }
    });
    let reply = send(&env.core, &phone, &post, None).await;
    assert_eq!(reply.status, 200);
    let put = serde_json::json!({
        "method": "PUT", "path": "/v1/transfers/t1/files/f1?offset=0", "headers": { "Content-Length": "5" }, "bodyBytes": 5
    });
    let reply = send(&env.core, &phone, &put, None).await;
    assert_eq!(reply.status, 200, "{:?}", String::from_utf8_lossy(&reply.body));
    let dir = env.core.received_dir();
    let listing: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(listing, ["evil_name_.txt"], "the name was cleaned, and the .part file is gone");
    assert_eq!(std::fs::read(dir.join("evil_name_.txt")).unwrap(), vec![0u8; 5]);
}
