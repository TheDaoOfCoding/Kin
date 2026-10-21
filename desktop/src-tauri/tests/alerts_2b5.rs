//! Milestone 2B.5, alerts, against the real core and the real pipe in the test process, with a fake phone: a pipe `send` of `kind: "alert"` goes to the phone as an `alert` (never
//! as a text), waits in the queue when the phone is offline and goes in order after the hello with how long it waited, fails locally to a phone that lists no `alert` and
//! once it has expired, is skipped while the PC is paused when it is event-driven, is checked again by Kin (nothing is cut silently), is rate limited by the pipe and by the
//! phone, and `Kin.exe --notify`'s client talks only to the pipe of this program. A text, a title or a name never reaches a log, `status.json` or any panel.
//!
//! Everything listens on 127.0.0.1 or in a named pipe of the test's own name. The clipboard is the in-memory one: **nothing here touches the real clipboard**.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use common::*;
use futures_util::StreamExt;
use kin_lib::cli_notify::{self, ConnectError};
use kin_lib::core::LinkState;
use kin_lib::nativehost::api::Api;
use kin_lib::nativehost::framing;
use kin_lib::nativehost::imagefetch::{FetchError, FetchFuture, Fetched, ImageFetcher};
use kin_lib::nativehost::pipe::{Handler, PipeServer, Reply};
use kin_lib::presence::pause::PauseChoice;
use serde_json::{Value, json};

struct NoFetch;

impl ImageFetcher for NoFetch {
    fn fetch<'a>(&'a self, _url: &'a str, _referer: Option<&'a str>) -> FetchFuture<'a> {
        Box::pin(async { Result::<Fetched, FetchError>::Err(FetchError::NeedsBytes("not used".into())) })
    }
}

static NEXT_PIPE: AtomicU32 = AtomicU32::new(0);

struct Rig {
    w: World,
    pipe: String,
    _server: PipeServer,
}

impl Rig {
    async fn new() -> Rig {
        capture_logs();
        let w = World::new().await;
        w.pair().await;
        let api = Arc::new(Api::new(w.env.core.clone(), Arc::new(NoFetch), None));
        let handler: Handler = {
            let api = api.clone();
            Arc::new(move |body| {
                let api = api.clone();
                Box::pin(async move { Reply::Respond(api.handle(&body).await) })
            })
        };
        let pipe = format!(r"\\.\pipe\app.kinrelay.dev.kintest-alerts-{}-{}", std::process::id(), NEXT_PIPE.fetch_add(1, Ordering::SeqCst));
        let _server = PipeServer::start(&pipe, handler, tokio::runtime::Handle::current()).unwrap();
        Rig { w, pipe, _server }
    }

    fn core(&self) -> &Arc<kin_lib::core::Core> {
        &self.w.env.core
    }

    /// The phone connects and says `hello` with these capabilities. The PC's own hello is read.
    async fn phone(&self, caps: &[&str]) -> Ws {
        let mut ws = self.w.try_open_ws().await.expect("the upgrade works for a paired phone");
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["type"], "hello");
        let mut phone = phone_hello();
        phone["capabilities"] = json!(caps);
        send_json(&mut ws, &phone).await;
        self.w.wait_for_state(LinkState::PhoneConnected).await;
        ws
    }

    async fn ask(&self, request: Value) -> Value {
        pipe_ask(self.pipe.clone(), request.to_string()).await
    }
}

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

fn uuid() -> String {
    kin_lib::encoding::uuid_v7(kin_lib::encoding::unix_ms())
}

fn alert_req(extra: Value) -> Value {
    let mut r = json!({ "v": 1, "id": uuid(), "type": "send", "kind": "alert", "source": "cli" });
    for (k, v) in extra.as_object().unwrap() {
        r[k] = v.clone();
    }
    r
}

/// The frames the phone gets for `ms`, of one type or any.
async fn frames_for(ws: &mut Ws, ms: u64) -> Vec<Value> {
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout_at(until, ws.next()).await {
            Err(_) => return out,
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => out.push(serde_json::from_str(t.as_str()).unwrap()),
            Ok(Some(Ok(_))) => continue,
            Ok(other) => panic!("the connection ended: {other:?}"),
        }
    }
}

const SECRET: &str = "SECRET-ALERT-TEXT-7731";
const SECRET_TITLE: &str = "SECRET-TITLE-5520";

// ----- sent -------------------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_alert_goes_to_a_capable_phone_as_an_alert_and_is_acked_and_never_touches_the_clipboard() {
    let rig = Rig::new().await;
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    let ask = tokio::spawn({
        let (pipe, req) =
            (rig.pipe.clone(), alert_req(json!({ "text": format!("  {SECRET}  "), "title": SECRET_TITLE, "ttlSec": 3600, "auto": false })));
        async move { pipe_ask(pipe, req.to_string()).await }
    });
    // The first frames can be presence snapshots; the alert is the one that matters.
    let frame = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            break f;
        }
        assert_ne!(f["type"], "text", "an alert is never a text");
    };
    assert_eq!(frame["text"], SECRET, "trimmed");
    assert_eq!(frame["title"], SECRET_TITLE);
    assert!(frame.get("waitedMs").is_none(), "a send with no wait carries no waitedMs");
    assert!(frame["id"].as_str().unwrap().len() == 36, "a UUIDv7");
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    let answer = ask.await.unwrap();
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["queued"], false);
    assert!(answer.get("code").is_none(), "a plain success has no notice");
    // The history row is an alert, done; the clipboard was not touched.
    let history = rig.core().history_view();
    let item = &history["items"][0];
    assert_eq!((item["isAlert"].clone(), item["status"].clone(), item["direction"].clone()), (json!(true), json!("done"), json!("out")));
    assert!(rig.w.env.clipboard.app_writes().is_empty(), "no clipboard write");
    assert_eq!(rig.core().queue_len(), 0);
}

#[tokio::test]
async fn an_alert_for_an_offline_phone_waits_and_goes_in_order_with_how_long_it_waited() {
    let rig = Rig::new().await;
    let first = rig.ask(alert_req(json!({ "text": "first", "ttlSec": 3600 }))).await;
    assert_eq!(
        (first["ok"].clone(), first["queued"].clone(), first["code"].clone()),
        (json!(true), json!(true), json!("DEVICE_OFFLINE_QUEUED"))
    );
    let second = rig.ask(alert_req(json!({ "text": "second", "ttlSec": 3600 }))).await;
    assert_eq!(second["queued"], true);
    assert_eq!(rig.core().queue_len(), 2);
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    let mut alerts = Vec::new();
    while alerts.len() < 2 {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            send_json(&mut ws, &json!({ "type": "ack", "ref": f["id"] })).await;
            alerts.push(f);
        }
    }
    assert_eq!((alerts[0]["text"].clone(), alerts[1]["text"].clone()), (json!("first"), json!("second")), "creation order");
    for a in &alerts {
        let waited = a["waitedMs"].as_u64().expect("a queued alert says how long it waited");
        assert!((1_000..30_000).contains(&waited), "{waited}");
    }
    for _ in 0..50 {
        if rig.core().queue_len() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(rig.core().queue_len(), 0, "both are delivered and acked");
}

// ----- failed locally ------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_connected_phone_without_alert_fails_the_item_and_nothing_is_sent_as_a_text() {
    let rig = Rig::new().await;
    let mut ws = rig.phone(&["link", "presence"]).await;
    let answer = rig.ask(alert_req(json!({ "text": SECRET }))).await;
    assert_eq!((answer["ok"].clone(), answer["code"].clone()), (json!(false), json!("INTERNAL")));
    assert_eq!(answer["message"], "Update Kin on your phone");
    let frames = frames_for(&mut ws, 500).await;
    assert!(
        frames.iter().all(|f| f["type"] != "alert" && f["type"] != "text" && f["type"] != "clipboard"),
        "nothing of it went on the wire: {frames:?}"
    );
    let history = rig.core().history_view();
    assert_eq!((history["items"][0]["status"].clone(), history["items"][0]["error"].clone()), (json!("failed"), json!("peer_unsupported")));
    assert_eq!(rig.core().queue_len(), 0);
    assert!(rig.w.env.clipboard.app_writes().is_empty());
}

#[tokio::test]
async fn an_alert_that_waited_past_its_expiry_is_never_sent() {
    let rig = Rig::new().await;
    let queued = rig.ask(alert_req(json!({ "text": "stale", "ttlSec": 60 }))).await;
    assert_eq!(queued["queued"], true);
    // Two minutes later (the PC's clock, moved by hand): the queue sweep fails it as `expired`.
    let swept = rig.core().expire_queue(kin_lib::encoding::unix_ms() as i64 + 120_000).await;
    assert_eq!(swept, 1);
    let history = rig.core().history_view();
    assert_eq!((history["items"][0]["status"].clone(), history["items"][0]["error"].clone()), (json!("failed"), json!("expired")));
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    let frames = frames_for(&mut ws, 500).await;
    assert!(frames.iter().all(|f| f["type"] != "alert"), "an expired alert is not sent after the phone connects: {frames:?}");
}

#[tokio::test]
async fn retrying_a_failed_alert_keeps_the_alerts_own_lifetime() {
    let rig = Rig::new().await;
    // A 30 minute alert (a finished download) that ran out while the phone was away.
    assert_eq!(rig.ask(alert_req(json!({ "text": "report.pdf", "ttlSec": 1800 }))).await["queued"], true);
    assert_eq!(rig.core().expire_queue(kin_lib::encoding::unix_ms() as i64 + 1_900_000).await, 1);
    let failed = rig.core().db.history(10).unwrap().into_iter().next().expect("the failed alert");
    assert_eq!(failed.status, "failed");
    let new_id = rig.core().retry_item(&failed.id).await.expect("a failed alert can be retried");
    let retried = rig.core().db.history_item(&new_id).unwrap().unwrap();
    let (_, expires) = rig.core().db.sent_and_expiry(&new_id).unwrap().unwrap();
    assert_eq!(expires.unwrap() - retried.created_at, 1_800_000, "30 minutes again, not the 24 hours of the default");
}

#[tokio::test]
async fn the_phone_can_turn_an_alert_away_and_the_item_fails_with_rate_limited() {
    let rig = Rig::new().await;
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    let ask = tokio::spawn({
        let (pipe, req) = (rig.pipe.clone(), alert_req(json!({ "text": "flood" })));
        async move { pipe_ask(pipe, req.to_string()).await }
    });
    let frame = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            break f;
        }
    };
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "rate_limited", "message": "too many" })).await;
    let answer = ask.await.unwrap();
    assert_eq!(
        (answer["ok"].clone(), answer["code"].clone(), answer["rateLimited"].clone()),
        (json!(false), json!("INTERNAL"), json!(true))
    );
    assert!(!answer["message"].as_str().unwrap().to_lowercase().contains("alert"), "the wording fits every kind of send: {answer}");
    let history = rig.core().history_view();
    assert_eq!((history["items"][0]["status"].clone(), history["items"][0]["error"].clone()), (json!("failed"), json!("rate_limited")));
}

// ----- the pause --------------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_event_driven_alert_is_skipped_while_the_pc_is_paused_and_a_scripts_own_alert_is_not() {
    let rig = Rig::new().await;
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    let skipped = rig.ask(alert_req(json!({ "text": "a download", "source": "downloads", "auto": true, "ttlSec": 1800 }))).await;
    assert_eq!(
        (skipped["ok"].clone(), skipped["queued"].clone(), skipped["code"].clone()),
        (json!(true), json!(false), json!("PAUSED_SKIPPED"))
    );
    assert_eq!(rig.core().queue_len(), 0, "nothing was stored");
    assert!(rig.core().history_view()["items"].as_array().unwrap().is_empty(), "no history row");
    // The script's own alert is sent even while paused.
    let ask = tokio::spawn({
        let (pipe, req) = (rig.pipe.clone(), alert_req(json!({ "text": "my script", "auto": false })));
        async move { pipe_ask(pipe, req.to_string()).await }
    });
    let frame = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            break f;
        }
    };
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    assert_eq!(ask.await.unwrap()["ok"], true);
    // Not paused: an auto alert is sent.
    rig.core().set_pause(None);
    let ask = tokio::spawn({
        let (pipe, req) =
            (rig.pipe.clone(), alert_req(json!({ "text": "a download", "source": "downloads", "auto": true, "ttlSec": 1800 })));
        async move { pipe_ask(pipe, req.to_string()).await }
    });
    let frame = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            break f;
        }
    };
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    assert_eq!(ask.await.unwrap()["ok"], true);
}

// ----- checked again by Kin ----------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn kin_checks_every_limit_again_and_cuts_nothing_silently() {
    let rig = Rig::new().await;
    for (extra, why) in [
        (json!({ "text": "a".repeat(301) }), "a text over 300"),
        (json!({ "text": "" }), "no text"),
        (json!({ "text": "   " }), "only spaces"),
        (json!({ "text": "x", "title": "t".repeat(61) }), "a title over 60"),
        (json!({ "text": "x", "title": 5 }), "a title that is a number"),
        (json!({ "text": "x", "ttlSec": 59 }), "a ttl under a minute"),
        (json!({ "text": "x", "ttlSec": 604_801 }), "a ttl over 7 days"),
        (json!({ "text": "x", "ttlSec": "60" }), "a ttl that is a string"),
        (json!({ "text": "x", "auto": "yes" }), "auto that is a string"),
        (json!({ "text": 7 }), "a text that is a number"),
    ] {
        let answer = rig.ask(alert_req(extra)).await;
        assert_eq!((answer["ok"].clone(), answer["code"].clone()), (json!(false), json!("BAD_REQUEST")), "{why}: {answer}");
    }
    let long = rig.ask(alert_req(json!({ "text": "a".repeat(301) }))).await;
    assert!(long["message"].as_str().unwrap().contains("301"), "the answer says how long it was: {long}");
    assert_eq!(rig.core().queue_len(), 0, "nothing was stored");
    // A text of exactly 300 and a ttl at both ends are fine.
    for extra in [json!({ "text": "a".repeat(300) }), json!({ "text": "x", "ttlSec": 60 }), json!({ "text": "x", "ttlSec": 604_800 })] {
        let answer = rig.ask(alert_req(extra)).await;
        assert_eq!((answer["ok"].clone(), answer["queued"].clone()), (json!(true), json!(true)), "{answer}");
    }
}

#[tokio::test]
async fn the_pipes_own_limit_of_30_per_10_s_answers_with_the_rate_limited_flag() {
    let rig = Rig::new().await;
    let mut limited = None;
    for i in 0..32 {
        let answer = rig.ask(alert_req(json!({ "text": format!("n{i}") }))).await;
        if answer["ok"] == false {
            limited = Some((i, answer));
            break;
        }
    }
    let (at, answer) = limited.expect("the 31st request in a burst is refused");
    assert_eq!(at, 30);
    assert_eq!((answer["code"].clone(), answer["rateLimited"].clone()), (json!("INTERNAL"), json!(true)));
}

// ----- Kin.exe --notify's client ------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_notify_client_talks_only_to_a_pipe_whose_server_is_the_expected_program() {
    let rig = Rig::new().await;
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    let me = std::env::current_exe().unwrap();
    // The server of the test pipe is this test program: the client accepts it and a whole exchange works.
    let args = cli_notify::parse_args(&["--title".into(), "CI".into(), "Done".into()], Vec::new).unwrap();
    let request = cli_notify::send_request(&uuid(), &args, None);
    let pipe = rig.pipe.clone();
    let exchange = tokio::task::spawn_blocking({
        let me = me.clone();
        move || {
            let file = cli_notify::connect(&pipe, &me, Duration::from_secs(3)).expect("the expected server is accepted");
            cli_notify::exchange(file, &request, Duration::from_secs(10))
        }
    });
    let frame = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            break f;
        }
    };
    assert_eq!((frame["text"].clone(), frame["title"].clone()), (json!("Done"), json!("CI")));
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    let answer = exchange.await.unwrap().expect("an answer");
    assert_eq!(cli_notify::exit_for(Some(&answer)).0, 0);
    // Any other expected program: refused before a byte of the request is written, so nothing is stored or sent.
    let pipe = rig.pipe.clone();
    let refused = tokio::task::spawn_blocking(move || {
        cli_notify::connect(&pipe, &std::env::temp_dir().join("Kin.exe"), Duration::from_secs(3)).map(|_| ())
    })
    .await
    .unwrap();
    assert!(matches!(refused, Err(ConnectError::Untrusted(_))), "{refused:?}");
    assert_eq!(rig.core().queue_len(), 0);
    let frames = frames_for(&mut ws, 300).await;
    assert!(frames.iter().all(|f| f["type"] != "alert"));
}

// ----- nothing of the text is logged or kept besides the text row ---------------------------------------------------------------------------

#[tokio::test]
async fn no_alert_text_or_title_reaches_a_log_status_json_or_a_panel() {
    let rig = Rig::new().await;
    let mut ws = rig.phone(&["link", "presence", "alert"]).await;
    let ask = tokio::spawn({
        let (pipe, req) = (rig.pipe.clone(), alert_req(json!({ "text": SECRET, "title": SECRET_TITLE })));
        async move { pipe_ask(pipe, req.to_string()).await }
    });
    let frame = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "alert" {
            break f;
        }
    };
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    ask.await.unwrap();
    // A second one that fails (too long) and one for a phone that is away and expires.
    rig.ask(alert_req(json!({ "text": format!("{SECRET}{}", "a".repeat(400)) }))).await;
    let core = rig.core();
    for (what, text) in [
        ("the log", captured_lines().join("\n")),
        ("status.json", core.status_json().to_string()),
        ("the presence panel", core.presence_panel().to_string()),
        ("the queue status", core.queue_status().to_string()),
    ] {
        assert!(!text.contains(SECRET) && !text.contains(SECRET_TITLE), "an alert text or title is in {what}");
    }
    // The only place the text is: the history row of the user's own script (bounded by the 500 newest items).
    assert!(core.history_view().to_string().contains(SECRET), "the history shows what the user's script said");
    assert!(captured_lines().iter().any(|l| l.contains("alert")), "the log has the word of the outcome");
}
