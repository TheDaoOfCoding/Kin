//! Milestone 2B.1 against the real core in the test process, with a fake phone: the `link` message in both directions, the browser extension's
//! requests over the real pipe (`status`, `send` of a text, a link, a page and an image, every limit and error of docs/extension.md), and what the
//! PC does with a link from the phone (opens it, or offers to; never over a fullscreen game).
//!
//! Everything listens on 127.0.0.1 or on a pipe with a test name (`app.kinrelay.dev.kintest-...`, never the release one); the clipboard is in
//! memory; the "browser" only records which links it was asked to open (nothing is ever opened); images come from a fake downloader.
mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use common::*;
use kin_lib::core::Event;
use kin_lib::nativehost::api::{Api, OpenHook, OpenTarget};
use kin_lib::nativehost::framing;
use kin_lib::nativehost::imagefetch::{FetchError, FetchFuture, Fetched, ImageFetcher};
use kin_lib::nativehost::pipe::{Handler, PipeServer, Reply};
use kin_lib::quiet::{FixedUserState, UserState};
use serde_json::{Value, json};

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 1, 2, 3, 4];
const YOUTUBE: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s&list=PLabc123#frag";

// ----- a fake image downloader --------------------------------------------------------------------------------------------

#[derive(Default)]
struct FakeFetcher {
    answers: Mutex<VecDeque<Result<Vec<u8>, FetchError>>>,
    asked: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeFetcher {
    fn will_answer(&self, answer: Result<Vec<u8>, FetchError>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn asked(&self) -> Vec<(String, Option<String>)> {
        self.asked.lock().unwrap().clone()
    }
}

impl ImageFetcher for FakeFetcher {
    fn fetch<'a>(&'a self, url: &'a str, referer: Option<&'a str>) -> FetchFuture<'a> {
        self.asked.lock().unwrap().push((url.to_string(), referer.map(str::to_string)));
        let next = self.answers.lock().unwrap().pop_front().unwrap_or(Err(FetchError::NeedsBytes("nothing scripted".into())));
        Box::pin(async move { next.map(|bytes| Fetched { bytes, final_url: url.to_string() }) })
    }
}

// ----- the extension side: a pipe and an Api in front of the world's core ---------------------------------------------------------

static NEXT_PIPE: AtomicU32 = AtomicU32::new(0);

struct Ext {
    w: World,
    pipe: String,
    _server: PipeServer,
    fetcher: Arc<FakeFetcher>,
    opened: Arc<Mutex<Vec<OpenTarget>>>,
}

impl Ext {
    async fn new() -> Ext {
        let w = World::new().await;
        let fetcher = Arc::new(FakeFetcher::default());
        let opened = Arc::new(Mutex::new(Vec::new()));
        let hook: OpenHook = {
            let opened = opened.clone();
            Arc::new(move |t| opened.lock().unwrap().push(t))
        };
        let api = Arc::new(Api::new(w.env.core.clone(), fetcher.clone(), Some(hook)));
        let handler: Handler = {
            let api = api.clone();
            Arc::new(move |body| {
                let api = api.clone();
                Box::pin(async move { Reply::Respond(api.handle(&body).await) })
            })
        };
        let pipe = format!(r"\\.\pipe\app.kinrelay.dev.kintest-links-{}-{}", std::process::id(), NEXT_PIPE.fetch_add(1, Ordering::SeqCst));
        let _server = PipeServer::start(&pipe, handler, tokio::runtime::Handle::current()).unwrap();
        Ext { w, pipe, _server, fetcher, opened }
    }

    /// One request over the pipe, the way the host does it.
    async fn ask(&self, request: Value) -> Value {
        pipe_ask(self.pipe.clone(), request.to_string()).await
    }

    /// A `send` request that waits for the answer while the test plays the phone: returns the answer and the frame the PC sent to the phone,
    /// which the test acks (or not).
    async fn send_and_answer(&self, ws: &mut Ws, request: Value, answer_with_ack: bool) -> (Value, Value) {
        let pending = tokio::spawn(pipe_ask(self.pipe.clone(), request.to_string()));
        let frame = next_json(ws).await;
        if answer_with_ack {
            send_json(ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
        }
        (pending.await.unwrap(), frame)
    }
}

/// A request over the real pipe, waiting for a busy pipe the way the host does (`WaitNamedPipe`).
async fn pipe_ask(name: String, body: String) -> Value {
    tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let mut client = loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&name) {
                Err(e) if e.raw_os_error() == Some(231) && start.elapsed() < Duration::from_secs(3) => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                other => break other.unwrap(),
            }
        };
        framing::write_frame(&mut client, body.as_bytes()).unwrap();
        let answer =
            framing::read_frame(&mut client, framing::MAX_RESPONSE).expect("an answer").expect("an answer, not the end of the stream");
        serde_json::from_slice(&answer).unwrap()
    })
    .await
    .unwrap()
}

fn req(extra: Value) -> Value {
    let mut r = json!({ "v": 1, "id": uuid(), "type": "send" });
    for (k, v) in extra.as_object().unwrap() {
        r[k] = v.clone();
    }
    r
}

fn uuid() -> String {
    kin_lib::encoding::uuid_v7(kin_lib::encoding::unix_ms())
}

fn phone_hello_with(capabilities: Option<Value>) -> Value {
    let mut h = phone_hello();
    if let Some(c) = capabilities {
        h["capabilities"] = c;
    }
    h
}

/// Opens the WebSocket and does the `hello` exchange, with the capabilities the phone says it has.
async fn open_ws_with(w: &World, capabilities: Option<Value>) -> (Ws, Value) {
    let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
    let hello = next_json(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    send_json(&mut ws, &phone_hello_with(capabilities)).await;
    w.wait_for_state(kin_lib::core::LinkState::PhoneConnected).await;
    (ws, hello)
}

async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn assert_no_frame(ws: &mut Ws, ms: u64) {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        match tokio::time::timeout_at(until, ws.next()).await {
            Err(_) => return,
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(other) => panic!("the PC sent something it should not have: {other:?}"),
        }
    }
}

fn code(answer: &Value) -> &str {
    answer["code"].as_str().unwrap_or("")
}

// ===== the PC's hello ==========================================================================================================

#[tokio::test]
async fn the_pcs_hello_says_it_understands_links() {
    let w = World::new().await;
    w.pair().await;
    let (_ws, hello) = open_ws_with(&w, None).await;
    assert_eq!(hello["capabilities"], json!(["link"]));
}

// ===== the extension: status, open, protocol errors ========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn status_lists_the_devices_their_state_and_how_many_items_wait() {
    let e = Ext::new().await;
    let none = e.ask(json!({ "v": 1, "id": "s1", "type": "status" })).await;
    assert_eq!((none["ok"].clone(), none["ref"].clone(), none["running"].clone()), (json!(true), json!("s1"), json!(true)));
    assert_eq!(none["devices"], json!([]));
    assert!(none.get("defaultDeviceId").is_none(), "no phone, no default");
    assert!(none["kinVersion"].is_string());

    e.w.pair().await;
    let offline = e.ask(json!({ "v": 1, "id": "s2", "type": "status" })).await;
    let d = &offline["devices"][0];
    assert_eq!((d["name"].as_str(), d["online"].as_bool(), d["queuedCount"].as_u64()), (Some("Test Phone"), Some(false), Some(0)));
    assert_eq!(offline["defaultDeviceId"], d["id"]);
    // Nothing that identifies the network or the keys is in it.
    let text = offline.to_string();
    assert!(!text.contains("127.0.0.1") && !text.contains("tk"), "{text}");

    // One item waits in the queue.
    let queued = e.ask(req(json!({ "kind": "link", "url": "https://example.com/queued" }))).await;
    assert_eq!((queued["ok"].clone(), queued["queued"].clone()), (json!(true), json!(true)));
    assert_eq!(e.ask(json!({ "v": 1, "id": "s3", "type": "status" })).await["devices"][0]["queuedCount"], 1);

    let (_ws, _) = open_ws_with(&e.w, None).await;
    let online = e.ask(json!({ "v": 1, "id": "s4", "type": "status" })).await;
    assert_eq!(online["devices"][0]["online"], true);
    // `start` is answered like `status` (the host does the starting).
    let started = e.ask(json!({ "v": 1, "id": "s5", "type": "start" })).await;
    assert_eq!((started["type"].clone(), started["running"].clone()), (json!("start"), json!(true)));
}

#[tokio::test(flavor = "multi_thread")]
async fn open_asks_for_a_window_and_a_bad_target_is_refused() {
    let e = Ext::new().await;
    assert_eq!(e.ask(json!({ "v": 1, "id": "o1", "type": "open", "target": "main" })).await["ok"], true);
    assert_eq!(e.ask(json!({ "v": 1, "id": "o2", "type": "open", "target": "pairing" })).await["ok"], true);
    assert_eq!(e.ask(json!({ "v": 1, "id": "o3", "type": "open" })).await["ok"], true, "no target: main");
    assert_eq!(*e.opened.lock().unwrap(), [OpenTarget::Main, OpenTarget::Pairing, OpenTarget::Main]);
    let bad = e.ask(json!({ "v": 1, "id": "o4", "type": "open", "target": "settings" })).await;
    assert_eq!((bad["ok"].clone(), code(&bad)), (json!(false), "BAD_REQUEST"));
    assert_eq!(e.opened.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn a_request_that_is_not_one_gets_the_error_of_the_contract() {
    let w = World::new().await;
    let api = Api::new(w.env.core.clone(), Arc::new(FakeFetcher::default()), None);
    let ask = |body: &str| {
        let body = body.to_string();
        let api = &api;
        async move { serde_json::from_slice::<Value>(&api.handle(body.as_bytes()).await).unwrap() }
    };
    for bad in [
        "not json",
        "[1]",
        "null",
        "{}",
        r#"{"v":1}"#,
        r#"{"v":1,"id":"a"}"#,
        r#"{"v":1,"id":"","type":"status"}"#,
        r#"{"v":1,"id":5,"type":"status"}"#,
    ] {
        let a = ask(bad).await;
        assert_eq!((a["ok"].clone(), code(&a), a["type"].clone()), (json!(false), "BAD_REQUEST", json!("error")), "{bad}");
        assert!(a.get("ref").is_none(), "{bad}: no usable id, no ref");
    }
    let long_id = format!(r#"{{"v":1,"id":"{}","type":"status"}}"#, "x".repeat(65));
    assert_eq!(code(&ask(&long_id).await), "BAD_REQUEST");
    let ok_id = format!(r#"{{"v":1,"id":"{}","type":"status"}}"#, "x".repeat(64));
    assert_eq!(ask(&ok_id).await["ok"], true);
    // A version it doesn't know: UNSUPPORTED_VERSION with the list, and the ref.
    let v2 = ask(r#"{"v":2,"id":"a","type":"status"}"#).await;
    assert_eq!(
        (code(&v2), v2["supported"].clone(), v2["ref"].clone(), v2["type"].clone()),
        ("UNSUPPORTED_VERSION", json!([1]), json!("a"), json!("error"))
    );
    for no_v in [r#"{"id":"a","type":"status"}"#, r#"{"v":"1","id":"a","type":"status"}"#, r#"{"v":1.0,"id":"a","type":"status"}"#] {
        // 1.0 is a float in serde_json, so it is not the integer 1.
        let a = ask(no_v).await;
        assert!(a["ok"] == false || no_v.contains("1.0"), "{no_v}: {a}");
    }
    let unknown = ask(r#"{"v":1,"id":"a","type":"unpair"}"#).await;
    assert_eq!((code(&unknown), unknown["ref"].clone()), ("UNSUPPORTED_TYPE", json!("a")));
    // No unpair, no settings, no history over the pipe.
    for ty in ["unpair", "settings", "history", "pair", "ping"] {
        assert_eq!(code(&ask(&format!(r#"{{"v":1,"id":"a","type":"{ty}"}}"#)).await), "UNSUPPORTED_TYPE", "{ty}");
    }
    // A repeated key: the last one wins (the JSON rules of protocol.md).
    assert_eq!(code(&ask(r#"{"v":1,"id":"a","type":"status","type":"nope"}"#).await), "UNSUPPORTED_TYPE");
    assert_eq!(ask(r#"{"v":1,"id":"a","type":"nope","type":"status"}"#).await["ok"], true);
    // Fields the host adds, and unknown ones, are ignored.
    assert_eq!(
        ask(r#"{"v":1,"id":"a","type":"status","origin":"chrome-extension://x/","hostVersion":"0.1.0","future":{"a":[1]}}"#).await["ok"],
        true
    );
    // Pairing nothing: not paired.
    let send = ask(r#"{"v":1,"id":"a","type":"send","kind":"text","text":"hi"}"#).await;
    assert_eq!((code(&send), send["ref"].clone()), ("NOT_PAIRED", json!("a")));
}

#[tokio::test]
async fn at_most_thirty_sends_in_ten_seconds_and_the_rest_are_told_to_slow_down() {
    let w = World::new().await;
    let api = Api::new(w.env.core.clone(), Arc::new(FakeFetcher::default()), None);
    let mut answers = Vec::new();
    for i in 0..32 {
        let body = json!({ "v": 1, "id": format!("r{i}"), "type": "send", "kind": "bogus" }).to_string();
        answers.push(serde_json::from_slice::<Value>(&api.handle(body.as_bytes()).await).unwrap());
    }
    // Every one reaches `send` (a bogus kind or no phone is an answer of its own), and the 31st is over the limit.
    let limited: Vec<usize> =
        answers.iter().enumerate().filter(|(_, a)| a["message"] == "Too many requests, slow down").map(|(i, _)| i).collect();
    assert_eq!(limited, [30, 31]);
    assert_eq!(code(&answers[30]), "INTERNAL");
    // `status` is not a send and is never limited.
    assert_eq!(serde_json::from_slice::<Value>(&api.handle(br#"{"v":1,"id":"s","type":"status"}"#).await).unwrap()["ok"], true);
}

// ===== the extension: text =========================================================================================================

#[tokio::test(flavor = "multi_thread")]
async fn a_text_goes_to_the_phone_as_a_text_and_is_acked() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    let (answer, frame) =
        e.send_and_answer(&mut ws, req(json!({ "kind": "text", "text": "line one\r\nline two https://example.com" })), true).await;
    // LF on the wire, never `sensitive` (a browser can't know), isUrl is Kin's own decision (the text is more than a URL).
    assert_eq!(
        (frame["type"].as_str(), frame["text"].as_str(), frame["isUrl"].clone()),
        (Some("text"), Some("line one\nline two https://example.com"), json!(false))
    );
    assert!(frame.get("sensitive").is_none());
    assert_eq!(
        (answer["ok"].clone(), answer["queued"].clone(), answer["deviceName"].clone()),
        (json!(true), json!(false), json!("Test Phone"))
    );
    assert_eq!(answer["deviceId"], e.w.phone.device_id);
    // A text that is only a URL is marked as one.
    let (_, frame) = e.send_and_answer(&mut ws, req(json!({ "kind": "text", "text": "https://example.com/a" })), true).await;
    assert_eq!(frame["isUrl"], true);
    // It is in the history like any explicit text, and in no clipboard.
    let items = e.w.core().history_view()["items"].as_array().unwrap().clone();
    assert!(items.iter().any(|i| i["direction"] == "out" && i["kind"] == "text" && i["status"] == "done"));
    assert!(e.w.env.clipboard.app_writes().is_empty(), "sending never touches the PC's clipboard");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_text_for_a_phone_that_is_away_is_queued_and_arrives_when_it_is_back() {
    let e = Ext::new().await;
    e.w.pair().await;
    let answer = e.ask(req(json!({ "kind": "text", "text": "later" }))).await;
    assert_eq!((answer["ok"].clone(), answer["queued"].clone(), code(&answer)), (json!(true), json!(true), "DEVICE_OFFLINE_QUEUED"));
    assert_eq!(answer["message"], "Test Phone is offline. It will be sent when the phone is back.");
    let (mut ws, _) = open_ws_with(&e.w, None).await;
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].as_str(), frame["text"].as_str()), (Some("text"), Some("later")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_text_over_128_kib_goes_as_a_txt_file_and_over_4_mib_is_refused() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    let big = "x".repeat(200 * 1024);
    let (answer, frame) = e.send_and_answer(&mut ws, req(json!({ "kind": "text", "text": big })), true).await;
    assert_eq!(frame["type"], "offer");
    assert_eq!(
        (frame["files"][0]["name"].as_str(), frame["files"][0]["size"].as_u64(), frame["files"][0]["mime"].as_str()),
        (Some("Text.txt"), Some(200 * 1024), Some("text/plain"))
    );
    assert_eq!((answer["ok"].clone(), answer["queued"].clone()), (json!(true), json!(false)));
    assert!(answer["transferId"].is_string());
    // Exactly 128 KiB is still a text message.
    let (_, frame) = e.send_and_answer(&mut ws, req(json!({ "kind": "text", "text": "y".repeat(128 * 1024) })), true).await;
    assert_eq!(frame["type"], "text");
    // Over 4 MiB: TOO_LARGE, nothing sent. Empty and missing: BAD_REQUEST.
    let too_big = e.ask(req(json!({ "kind": "text", "text": "z".repeat(4 * 1024 * 1024 + 1) }))).await;
    assert_eq!(code(&too_big), "TOO_LARGE");
    assert_eq!(code(&e.ask(req(json!({ "kind": "text", "text": "" }))).await), "BAD_REQUEST");
    assert_eq!(code(&e.ask(req(json!({ "kind": "text" }))).await), "BAD_REQUEST");
    assert_eq!(code(&e.ask(req(json!({ "kind": "text", "text": 5 }))).await), "BAD_REQUEST");
    assert_no_frame(&mut ws, 300).await;
}

// ===== the extension: links and pages ====================================================================================================

#[tokio::test(flavor = "multi_thread")]
async fn a_link_goes_to_a_phone_that_knows_links_as_a_link_with_the_url_unchanged() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    let title = "A video\u{202e}\ngpj  with   spaces";
    let (answer, frame) =
        e.send_and_answer(&mut ws, req(json!({ "kind": "link", "url": YOUTUBE, "title": title, "timestampSec": 83 })), true).await;
    assert_eq!(frame["type"], "link");
    assert_eq!(frame["url"], YOUTUBE, "t=, list= and the fragment are not touched");
    assert_eq!(frame["title"], "A video_ gpj with spaces", "cleaned like a file name");
    assert_eq!(frame["sourceApp"], "Browser");
    assert_eq!(frame["id"].as_str().unwrap().len(), 36, "a message id of its own, not the request's");
    assert_eq!((answer["ok"].clone(), answer["queued"].clone()), (json!(true), json!(false)));
    // A page is the same message. A title of more than 300 characters is cut; an empty one is left out.
    let (_, frame) =
        e.send_and_answer(&mut ws, req(json!({ "kind": "page", "url": "https://example.com/p", "title": "t".repeat(400) })), true).await;
    assert_eq!((frame["type"].as_str(), frame["title"].as_str().map(|t| t.chars().count())), (Some("link"), Some(300)));
    let (_, frame) = e.send_and_answer(&mut ws, req(json!({ "kind": "link", "url": "https://example.com/q", "title": "  " })), true).await;
    assert!(frame.get("title").is_none());
    // The history says it is a link, with its title.
    let items = e.w.core().history_view()["items"].as_array().unwrap().clone();
    assert!(items.iter().any(|i| i["isLink"] == true && i["title"] == "A video_ gpj with spaces" && i["text"] == YOUTUBE));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_for_a_phone_without_the_capability_goes_as_a_text_under_the_same_id() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, None).await; // a 0.1.x phone: no capabilities
    let (answer, frame) = e.send_and_answer(&mut ws, req(json!({ "kind": "link", "url": YOUTUBE, "title": "A video" })), true).await;
    assert_eq!(
        frame,
        json!({ "type": "text", "id": frame["id"], "text": YOUTUBE, "isUrl": true }),
        "no title, no sourceApp: an old phone shows Open"
    );
    assert_eq!(answer["ok"], true);
    // A phone that sends a capabilities value that is not a list counts as having none (and its hello was accepted).
    drop(ws);
    let (mut ws, _) = open_ws_with(&e.w, Some(json!("link"))).await;
    let (_, frame) = e.send_and_answer(&mut ws, req(json!({ "kind": "link", "url": "https://example.com/" })), true).await;
    assert_eq!(frame["type"], "text");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_link_goes_out_in_the_form_the_phone_understands_when_it_is_back() {
    let e = Ext::new().await;
    e.w.pair().await;
    // Two links wait; the phone comes back as a 0.2.0 one for the first, as a 0.1.x one for the second connection.
    for u in ["https://example.com/1", "https://example.com/2"] {
        let a = e.ask(req(json!({ "kind": "link", "url": u, "title": "T" }))).await;
        assert_eq!(a["queued"], true);
    }
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    let first = next_json(&mut ws).await;
    assert_eq!(
        (first["type"].as_str(), first["url"].as_str(), first["title"].as_str()),
        (Some("link"), Some("https://example.com/1"), Some("T"))
    );
    // The phone's ack never comes: the second link stays queued, the connection is closed as dead after 5 s. Close it ourselves and come back as an old phone.
    drop(ws);
    wait_until("the PC to see the phone gone", || e.w.core().snapshot().state == kin_lib::core::LinkState::PhoneOffline).await;
    let (mut ws, _) = open_ws_with(&e.w, None).await;
    let again = next_json(&mut ws).await;
    assert_eq!(again["id"], first["id"], "the unacked first link is sent again, with its id");
    assert_eq!(
        (again["type"].as_str(), again["text"].as_str(), again["isUrl"].clone()),
        (Some("text"), Some("https://example.com/1"), json!(true))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_that_is_not_a_web_link_is_refused_and_nothing_is_sent_or_queued() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    let long = format!("https://e.com/{}", "a".repeat(8180));
    for (kind, bad) in [
        ("link", "javascript:alert(1)"),
        ("link", "file:///C:/Windows/notepad.exe"),
        ("link", "data:text/html,hi"),
        ("link", "ftp://example.com/a"),
        ("link", "chrome://settings"),
        ("link", "about:blank"),
        ("link", "blob:https://example.com/1"),
        ("link", "intent://x#Intent;end"),
        ("page", "mailto:a@b.c"),
        ("link", " https://example.com/"),
        ("link", "https://example.com/ "),
        ("link", "https://example.com/a b"),
        ("link", "https://example.com/\n"),
        ("link", "https://"),
        ("link", r"https://evil.example\@google.com"),
        ("page", "https://google.com@evil.example/"),
        ("link", "https://user:pw@example.com/"),
        ("link", "https://goog%6Ce.com/"),
        ("link", long.as_str()),
    ] {
        let a = e.ask(req(json!({ "kind": kind, "url": bad }))).await;
        assert_eq!((a["ok"].clone(), code(&a)), (json!(false), "BAD_URL"), "{bad:?}");
    }
    for bad in
        [json!({ "kind": "link" }), json!({ "kind": "link", "url": 5 }), json!({ "kind": "link", "url": "https://e.com/", "title": 5 })]
    {
        assert_eq!(code(&e.ask(req(bad)).await), "BAD_REQUEST");
    }
    assert_eq!(code(&e.ask(req(json!({ "kind": "bogus" }))).await), "BAD_REQUEST");
    assert_eq!(code(&e.ask(req(json!({}))).await), "BAD_REQUEST");
    assert_no_frame(&mut ws, 300).await;
    assert_eq!(e.w.core().snapshot().queued, 0, "nothing was queued");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_to_a_phone_that_is_not_paired_is_not_paired_and_a_named_phone_is_used() {
    let e = Ext::new().await;
    let none = e.ask(req(json!({ "kind": "text", "text": "x" }))).await;
    assert_eq!((code(&none), none["message"].clone()), ("NOT_PAIRED", json!("Pair your phone with Kin first.")));
    e.w.pair().await;
    let unknown = e.ask(req(json!({ "kind": "text", "text": "x", "deviceId": "not-a-device" }))).await;
    assert_eq!((code(&unknown), unknown["message"].clone()), ("NOT_PAIRED", json!("That phone isn't paired.")));
    assert_eq!(code(&e.ask(req(json!({ "kind": "text", "text": "x", "deviceId": 5 }))).await), "BAD_REQUEST");
    let named = e.ask(req(json!({ "kind": "text", "text": "x", "deviceId": e.w.phone.device_id }))).await;
    assert_eq!((named["ok"].clone(), named["queued"].clone()), (json!(true), json!(true)));
}

// ===== the extension: images ===============================================================================================================

#[tokio::test(flavor = "multi_thread")]
async fn an_image_by_url_is_downloaded_by_kin_and_sent_as_a_file() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    e.fetcher.will_answer(Ok(PNG.to_vec()));
    let (answer, frame) = e
        .send_and_answer(&mut ws, req(json!({ "kind": "image", "url": "https://img.example.com/dir/cat%20pic.jpg?size=big", "pageUrl": "https://example.com/post", "name": "cat.jpg" })), true)
        .await;
    // The bytes say PNG: the name gets the right extension, whatever the page called it.
    assert_eq!(frame["type"], "offer");
    assert_eq!((frame["files"][0]["name"].as_str(), frame["files"][0]["size"].as_u64()), (Some("cat.png"), Some(PNG.len() as u64)));
    assert_eq!((answer["ok"].clone(), answer["queued"].clone()), (json!(true), json!(false)));
    assert!(answer["transferId"].is_string());
    assert_eq!(
        e.fetcher.asked(),
        [("https://img.example.com/dir/cat%20pic.jpg?size=big".to_string(), Some("https://example.com/post".to_string()))]
    );
    // Without a name: the last segment of the URL.
    e.fetcher.will_answer(Ok(PNG.to_vec()));
    let (_, frame) =
        e.send_and_answer(&mut ws, req(json!({ "kind": "image", "url": "https://img.example.com/dir/cat%20pic.png" })), true).await;
    assert_eq!(frame["files"][0]["name"], "cat pic.png");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_that_kin_cannot_download_says_so_in_the_words_of_the_contract() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, None).await;
    let image = |extra: Value| {
        let mut r = req(json!({ "kind": "image", "url": "https://img.example.com/a.png" }));
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };
    // A sign-in wall, a timeout, an error: the browser may try.
    e.fetcher.will_answer(Err(FetchError::NeedsBytes("the server answered 403".into())));
    let a = e.ask(image(json!({}))).await;
    assert_eq!(
        (a["ok"].clone(), code(&a), a["message"].clone()),
        (json!(false), "IMAGE_NEEDS_BYTES", json!("The site wouldn't give Kin the picture."))
    );
    // An HTML page where the picture should be: the same.
    e.fetcher.will_answer(Ok(b"<!DOCTYPE html><html>sign in</html>".to_vec()));
    assert_eq!(code(&e.ask(image(json!({}))).await), "IMAGE_NEEDS_BYTES");
    // Over 64 MiB: too large, not "needs bytes".
    e.fetcher.will_answer(Err(FetchError::TooLarge));
    assert_eq!(code(&e.ask(image(json!({}))).await), "TOO_LARGE");
    // Not a web address: no download is even tried.
    let before = e.fetcher.asked().len();
    for bad in ["data:image/png;base64,AAAA", "blob:https://example.com/1", "file:///C:/a.png", "ftp://example.com/a.png"] {
        assert_eq!(code(&e.ask(image(json!({ "url": bad }))).await), "BAD_URL", "{bad}");
    }
    assert_eq!(e.fetcher.asked().len(), before);
    // Exactly one of url and data.
    assert_eq!(code(&e.ask(req(json!({ "kind": "image" }))).await), "BAD_REQUEST");
    assert_eq!(code(&e.ask(image(json!({ "data": "AAAA", "mime": "image/png", "name": "a.png" }))).await), "BAD_REQUEST");
    assert_no_frame(&mut ws, 300).await;
    assert_eq!(e.w.core().snapshot().queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_sent_as_bytes_is_checked_by_its_content_and_limited_to_32_mib() {
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, None).await;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    // A picture, whatever the mime says; the name gets the extension of what it is.
    let (answer, frame) = e
        .send_and_answer(
            &mut ws,
            req(json!({ "kind": "image", "data": b64(PNG), "mime": "text/html", "name": "photo.jpg", "retryOf": "earlier" })),
            true,
        )
        .await;
    assert_eq!((frame["type"].as_str(), frame["files"][0]["name"].as_str()), (Some("offer"), Some("photo.png")));
    assert_eq!(answer["ok"], true);
    // Not a picture: NOT_AN_IMAGE (the fallback is not tried again), even with an image mime.
    let a = e.ask(req(json!({ "kind": "image", "data": b64(b"<html>not a picture</html>"), "mime": "image/png", "name": "x.png" }))).await;
    assert_eq!((code(&a), a["message"].clone()), ("NOT_AN_IMAGE", json!("That doesn't look like an image.")));
    // Broken base64, a missing mime, and a missing name are the extension's bug.
    assert_eq!(
        code(&e.ask(req(json!({ "kind": "image", "data": "!!!not base64", "mime": "image/png", "name": "x.png" }))).await),
        "BAD_REQUEST"
    );
    assert_eq!(code(&e.ask(req(json!({ "kind": "image", "data": b64(PNG), "name": "x.png" }))).await), "BAD_REQUEST");
    // One byte over 32 MiB of raw bytes.
    let mut big = PNG.to_vec();
    big.resize(32 * 1024 * 1024 + 1, 0);
    let a = e.ask(req(json!({ "kind": "image", "data": b64(&big), "mime": "image/png", "name": "big.png" }))).await;
    assert_eq!(code(&a), "TOO_LARGE");
    assert_no_frame(&mut ws, 300).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_for_a_phone_that_is_away_is_queued() {
    let e = Ext::new().await;
    e.w.pair().await;
    e.fetcher.will_answer(Ok(PNG.to_vec()));
    let a = e.ask(req(json!({ "kind": "image", "url": "https://img.example.com/a.png" }))).await;
    assert_eq!((a["ok"].clone(), a["queued"].clone(), code(&a)), (json!(true), json!(true), "DEVICE_OFFLINE_QUEUED"));
    assert_eq!(e.w.core().snapshot().queued, 1);
    let (mut ws, _) = open_ws_with(&e.w, None).await;
    let offer = next_json(&mut ws).await;
    assert_eq!((offer["type"].as_str(), offer["files"][0]["name"].as_str()), (Some("offer"), Some("a.png")));
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_the_user_sent_is_in_the_log() {
    capture_logs();
    let e = Ext::new().await;
    e.w.pair().await;
    let (mut ws, _) = open_ws_with(&e.w, Some(json!(["link"]))).await;
    e.fetcher.will_answer(Ok(PNG.to_vec()));
    let _ = e
        .send_and_answer(
            &mut ws,
            req(json!({ "kind": "link", "url": "https://secret-host.example/path?token=SECRETTOKEN", "title": "SECRETTITLE" })),
            true,
        )
        .await;
    let _ = e.send_and_answer(&mut ws, req(json!({ "kind": "text", "text": "SECRETTEXT" })), true).await;
    let _ = e
        .send_and_answer(
            &mut ws,
            req(json!({ "kind": "image", "url": "https://secret-host.example/SECRETNAME.png", "name": "SECRETFILE.png" })),
            true,
        )
        .await;
    let _ = e.ask(req(json!({ "kind": "link", "url": "javascript:SECRETSCRIPT" }))).await;
    let log = captured_lines().join("\n");
    for secret in ["SECRETTOKEN", "SECRETTITLE", "SECRETTEXT", "SECRETNAME", "SECRETFILE", "SECRETSCRIPT", "secret-host"] {
        assert!(!log.contains(secret), "{secret} reached the log:\n{log}");
    }
    assert!(log.contains("extension: send"), "the sends are logged, by count and outcome");
}

// ===== a link from the phone ================================================================================================================

async fn phone_sends_link(ws: &mut Ws, id: &str, url: &str, title: Option<&str>) -> Value {
    let mut frame = json!({ "type": "link", "id": id, "url": url, "sourceApp": "Chrome" });
    if let Some(t) = title {
        frame["title"] = json!(t);
    }
    send_json(ws, &frame).await;
    next_json(ws).await
}

fn link_toasts(events: &[Event]) -> Vec<(u32, String, String, bool)> {
    events
        .iter()
        .filter_map(|e| {
            if let Event::LinkToast { id, title, body, opened } = e { Some((*id, title.clone(), body.clone(), *opened)) } else { None }
        })
        .collect()
}

/// The toast of the next link: it is announced after the link was opened (or not), so it is waited for.
async fn next_link_toast(w: &mut World) -> (u32, String, String, bool) {
    let mut toasts = Vec::new();
    wait_until("the toast of the link", || {
        toasts.extend(link_toasts(&drain(&mut w.env.events)));
        !toasts.is_empty()
    })
    .await;
    assert_eq!(toasts.len(), 1, "one toast per link");
    toasts.remove(0)
}

async fn world_with_phone() -> (World, Ws) {
    let w = World::new().await;
    w.pair().await;
    let (ws, _) = open_ws_with(&w, Some(json!(["link"]))).await;
    // Whatever the machine of the test is doing: nothing special.
    w.core().set_user_state_probe(Arc::new(FixedUserState::new(UserState::AcceptsNotifications)));
    (w, ws)
}

#[tokio::test]
async fn a_link_from_the_phone_opens_in_the_browser_and_the_toast_has_copy() {
    let (mut w, mut ws) = world_with_phone().await;
    drain(&mut w.env.events);
    let reply = phone_sends_link(&mut ws, "L1", YOUTUBE, Some("A video")).await;
    assert_eq!(reply, json!({ "type": "ack", "ref": "L1" }), "acked as soon as it is valid, not when it is opened");
    let (id, title, body, opened) = next_link_toast(&mut w).await;
    assert_eq!(w.env.opener.opened(), [YOUTUBE], "opened exactly as sent");
    assert_eq!((title.as_str(), body.as_str(), opened), ("Opened link from phone", "A video\nwww.youtube.com", true));
    // Never on the clipboard by itself.
    assert!(w.env.clipboard.app_writes().is_empty());
    // A click on that toast opens nothing more (it was opened already); Copy puts the URL on the clipboard.
    assert!(!w.core().toast_clicked(id));
    assert_eq!(w.env.opener.opened().len(), 1);
    assert!(w.core().toast_has_target(id), "the click did not use up the Copy button");
    let core = w.env.core.clone();
    assert!(core.toast_copy(id).await);
    assert_eq!(w.env.clipboard.app_writes(), [YOUTUBE]);
    // The history has it, like a text, with its title.
    let items = w.core().history_view()["items"].as_array().unwrap().clone();
    assert!(items.iter().any(|i| i["direction"] == "in"
        && i["isLink"] == true
        && i["title"] == "A video"
        && i["text"] == YOUTUBE
        && i["status"] == "done"));
}

#[tokio::test]
async fn with_the_setting_off_the_toast_has_open_and_copy_and_nothing_opens() {
    let (mut w, mut ws) = world_with_phone().await;
    w.core().set_open_links_automatically(false);
    assert!(!kin_lib::settings::Settings::load(w.env.dir.path()).open_links_automatically, "the choice is saved");
    drain(&mut w.env.events);
    phone_sends_link(&mut ws, "L2", "https://example.com/a?x=1", None).await;
    let mut toasts = Vec::new();
    wait_until("the toast", || {
        toasts.extend(link_toasts(&drain(&mut w.env.events)));
        !toasts.is_empty()
    })
    .await;
    assert!(w.env.opener.opened().is_empty(), "the setting is off: not opened");
    let target_id = toasts[0].0;
    assert!(w.core().toast_has_target(target_id));
    assert!(w.core().toast_clicked(target_id), "Open (and a click on the toast) opens it");
    assert_eq!(w.env.opener.opened(), ["https://example.com/a?x=1"]);
    // The history keeps it either way.
    let items = w.core().history_view()["items"].as_array().unwrap().clone();
    assert!(items.iter().any(|i| i["isLink"] == true && i["text"] == "https://example.com/a?x=1" && i["direction"] == "in"));
}

/// B1 and B2: a link whose address a browser reads in a way that can mislead (a look-alike or international host, an IP address in any spelling)
/// is never opened by itself, even with the setting on and the user at the desktop. The toast shows the address as the browser reads it, with a
/// warning sign, and Open and Copy; Open still works on purpose.
#[tokio::test]
async fn a_link_with_a_warning_is_offered_never_opened_by_itself() {
    let (mut w, mut ws) = world_with_phone().await;
    for (n, (url, shown)) in [
        ("https://\u{430}\u{440}\u{440}\u{4cf}\u{435}.com/login", "\u{26a0} xn--80ak6aa92e.com"),
        ("https://\u{ff47}\u{ff4f}\u{ff4f}\u{ff47}\u{ff4c}\u{ff45}.com/", "\u{26a0} google.com"),
        ("https://b\u{fc}cher.example/", "\u{26a0} xn--bcher-kva.example"),
        ("https://xn--bcher-kva.example/", "\u{26a0} xn--bcher-kva.example"),
        ("https://0x7f.1/", "\u{26a0} 127.0.0.1"),
        ("http://192.168.1.5:8080/x", "\u{26a0} 192.168.1.5:8080"),
        ("https://[::ffff:127.0.0.1]/", "\u{26a0} [::ffff:7f00:1]"),
        ("https://1.2.3.4.5/", "\u{26a0} 1.2.3.4.5"),
    ]
    .into_iter()
    .enumerate()
    {
        drain(&mut w.env.events);
        phone_sends_link(&mut ws, &format!("W{n}"), url, Some("paypal.com")).await;
        let (target, title, body, opened) = next_link_toast(&mut w).await;
        assert_eq!((title.as_str(), body.as_str(), opened), ("Link from phone", format!("paypal.com\n{shown}").as_str(), false), "{url}");
        assert!(w.env.opener.opened().is_empty(), "{url}: not opened by itself");
        assert!(w.core().toast_clicked(target), "{url}: Open works on purpose");
        assert_eq!(w.env.opener.opened(), [url]);
        w.env.opener.opened.lock().unwrap().clear();
    }
    // An ordinary link right after still opens at once.
    drain(&mut w.env.events);
    phone_sends_link(&mut ws, "W-ok", "https://example.com/fine", None).await;
    wait_until("the link to open", || !w.env.opener.opened().is_empty()).await;
    assert_eq!(w.env.opener.opened(), ["https://example.com/fine"]);
}

#[tokio::test]
async fn a_toast_that_offers_the_link_says_link_from_phone() {
    let (mut w, mut ws) = world_with_phone().await;
    w.core().set_open_links_automatically(false);
    drain(&mut w.env.events);
    phone_sends_link(&mut ws, "L3", "https://example.com/", Some("Title")).await;
    let mut toasts = Vec::new();
    wait_until("the toast", || {
        toasts.extend(link_toasts(&drain(&mut w.env.events)));
        !toasts.is_empty()
    })
    .await;
    let (_, title, body, opened) = toasts[0].clone();
    assert_eq!((title.as_str(), body.as_str(), opened), ("Link from phone", "Title\nexample.com", false));
}

#[tokio::test]
async fn in_a_fullscreen_game_a_presentation_or_quiet_time_the_link_is_not_opened_but_offered_and_kept() {
    let (mut w, mut ws) = world_with_phone().await;
    let state = Arc::new(FixedUserState::new(UserState::RunningD3dFullScreen));
    w.core().set_user_state_probe(state.clone());
    for (n, held) in
        [UserState::RunningD3dFullScreen, UserState::Busy, UserState::PresentationMode, UserState::QuietTime].into_iter().enumerate()
    {
        state.set(held);
        drain(&mut w.env.events);
        let url = format!("https://example.com/held/{n}");
        phone_sends_link(&mut ws, &format!("H{n}"), &url, Some("Held")).await;
        let mut toasts = Vec::new();
        wait_until("the toast", || {
            toasts.extend(link_toasts(&drain(&mut w.env.events)));
            !toasts.is_empty()
        })
        .await;
        assert_eq!((toasts[0].1.as_str(), toasts[0].3), ("Link from phone", false), "{held:?}");
        assert!(w.env.opener.opened().is_empty(), "{held:?}: the game keeps its focus");
        let items = w.core().history_view()["items"].as_array().unwrap().clone();
        assert!(items.iter().any(|i| i["text"] == url.as_str() && i["direction"] == "in"), "{held:?}: the history keeps it");
        assert!(w.core().toast_clicked(toasts[0].0), "{held:?}: Open works on purpose");
        assert_eq!(w.env.opener.opened(), std::slice::from_ref(&url));
        w.env.opener.opened.lock().unwrap().clear();
    }
    // Back at the desktop, the next link opens at once.
    state.set(UserState::AcceptsNotifications);
    phone_sends_link(&mut ws, "after", "https://example.com/free", None).await;
    wait_until("the link to open", || !w.env.opener.opened().is_empty()).await;
    assert_eq!(w.env.opener.opened(), ["https://example.com/free"]);
}

#[tokio::test]
async fn a_link_that_is_not_a_web_link_is_refused_by_the_pc_and_a_repeat_is_not_opened_twice() {
    let (mut w, mut ws) = world_with_phone().await;
    drain(&mut w.env.events);
    for (i, bad) in [
        "javascript:alert(1)",
        "file:///C:/Windows/notepad.exe",
        "ms-msdt:/id PCWDiagnostic",
        " https://example.com/",
        "https://example.com/a b",
        r"https://evil.example\@google.com",
        r"https://evil.example\@google.com/x",
        "https://google.com@evil.example/",
        "https://goog%6Ce.com/",
    ]
    .into_iter()
    .enumerate()
    {
        let reply = phone_sends_link(&mut ws, &format!("B{i}"), bad, None).await;
        assert_eq!(
            (reply["type"].as_str(), reply["code"].as_str(), reply["ref"].as_str()),
            (Some("error"), Some("bad_request"), Some(format!("B{i}").as_str())),
            "{bad:?}"
        );
    }
    assert!(w.env.opener.opened().is_empty());
    assert!(link_toasts(&drain(&mut w.env.events)).is_empty());
    assert_eq!(w.core().history_view()["items"].as_array().unwrap().len(), 0, "nothing was stored");
    // The rejected id is free: the corrected link with the same id is delivered.
    assert_eq!(phone_sends_link(&mut ws, "B0", "https://example.com/fixed", None).await["type"], "ack");
    next_link_toast(&mut w).await;
    assert_eq!(w.env.opener.opened().len(), 1);
    // A repeat of an id gets the same ack and is not opened again, and not shown again.
    assert_eq!(phone_sends_link(&mut ws, "B0", "https://example.com/fixed", None).await, json!({ "type": "ack", "ref": "B0" }));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(w.env.opener.opened(), ["https://example.com/fixed"]);
    assert!(link_toasts(&drain(&mut w.env.events)).is_empty());
    assert_eq!(w.core().history_view()["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_link_never_reaches_the_log_and_the_toast_never_shows_the_title_alone() {
    capture_logs();
    let (mut w, mut ws) = world_with_phone().await;
    drain(&mut w.env.events);
    phone_sends_link(&mut ws, "P1", "https://phone-secret-host.example/x?token=PHONESECRET", Some("PHONETITLE paypal.com")).await;
    let mut toasts = Vec::new();
    wait_until("the toast", || {
        toasts.extend(link_toasts(&drain(&mut w.env.events)));
        !toasts.is_empty()
    })
    .await;
    assert!(toasts[0].2.ends_with("\nphone-secret-host.example"), "the real host is under the title: {}", toasts[0].2);
    let log = captured_lines().join("\n");
    for secret in ["PHONESECRET", "PHONETITLE", "phone-secret-host"] {
        assert!(!log.contains(secret), "{secret} reached the log:\n{log}");
    }
}

#[tokio::test]
async fn a_new_setting_defaults_to_on_and_is_read_from_settings_json() {
    let w = World::new().await;
    assert!(kin_lib::settings::Settings::default().open_links_automatically);
    assert_eq!(w.core().settings_view(None)["openLinksAutomatically"], true);
    std::fs::write(kin_lib::settings::Settings::path(w.env.dir.path()), r#"{ "openLinksAutomatically": false }"#).unwrap();
    assert!(!kin_lib::settings::Settings::load(w.env.dir.path()).open_links_automatically);
}
