//! Milestone 1.4 against the real server in the test process, with a fake phone: the queue (a send while the phone is away is stored
//! first and goes out, in order, after the `hello` of the reconnect), what a dead connection does to an item, expiry with a clock moved by
//! hand, cancel, the things that are never queued (the automatic clipboard, a `sensitive` text), the history, retry, the settings and the
//! debug numbers.
//!
//! **The done-when check of 1.4**: `a_text_sent_while_the_phone_is_offline_arrives_within_seconds_of_the_reconnect` measures how long
//! a text that the PC queued takes to arrive once the phone's `hello` is in.
//!
//! Everything listens on 127.0.0.1 with a random port, the clipboard is in memory (the real one is never touched), and the files are
//! made in temporary folders.
mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::*;
use futures_util::StreamExt;
use kin_lib::clipboard::{CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD, ClipFormat, EXCLUDE_FROM_MONITOR};
use kin_lib::core::Event;
use kin_lib::queue::{DAY_MS, HOUR_MS};
use kin_lib::sync::{Kind, SendResult, Tap};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const SENSITIVE_FORMATS: [&str; 3] = [EXCLUDE_FROM_MONITOR, CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD];

/// An explicit text from the PC, the way the hotkey sends it.
async fn send_text(w: &World, text: &str) -> SendResult {
    w.env.core.deliver(Kind::Text, text.to_string(), false, "test", Tap::now(), true).await
}

fn ack(frame: &Value) -> Value {
    json!({ "type": "ack", "ref": frame["id"] })
}

/// The PC opens a connection and does the `hello` exchange, but the test reads the frames that follow itself. Returns the socket.
async fn reconnect(w: &World) -> Ws {
    let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
    let hello = next_json(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    send_json(&mut ws, &phone_hello()).await;
    ws
}

async fn assert_silent(ws: &mut Ws, ms: u64) {
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        match tokio::time::timeout_at(until, ws.next()).await {
            Err(_) => return,
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(other) => panic!("the PC sent something it should not have: {other:?}"),
        }
    }
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

fn status_of(w: &World, id: &str) -> String {
    w.core().db.status_of(id).unwrap().map(|(s, _)| s).unwrap_or_else(|| "missing".into())
}

fn history_items(w: &World) -> Vec<Value> {
    w.core().history_view()["items"].as_array().unwrap().clone()
}

fn queued_ids(w: &World) -> Vec<String> {
    let device = w.phone.device_id.clone();
    w.core().db.queued_items(&device).unwrap().into_iter().map(|i| i.id).collect()
}

fn make_file(dir: &std::path::Path, name: &str, size: usize) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, vec![7u8; size]).unwrap();
    path
}

// ----- the queue ----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_text_sent_while_the_phone_is_offline_arrives_within_seconds_of_the_reconnect() {
    let mut w = World::new().await;
    w.pair().await;
    drain(&mut w.env.events);

    // The phone is away. The send is stored first, and the user is told it waits.
    let result = send_text(&w, "written while you were away").await;
    assert_eq!(result, SendResult::PhoneOffline);
    let ids = queued_ids(&w);
    assert_eq!(ids.len(), 1, "the text is a queued item");
    assert_eq!(toasts(&drain(&mut w.env.events)), ["The phone isn't connected. The text is queued and will be sent when it is back."]);
    let items = history_items(&w);
    assert_eq!(
        (items[0]["status"].as_str(), items[0]["canCancel"].as_bool(), items[0]["kind"].as_str()),
        (Some("queued"), Some(true), Some("text"))
    );
    assert_eq!(w.core().status_json()["queue"]["count"], 1);
    assert_eq!(w.core().snapshot().queued, 1, "the tray shows how many are waiting");

    // The phone is back. From its `hello` to the text arriving is what the done-when check of 1.4 measures.
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    let started = Instant::now();
    send_json(&mut ws, &phone_hello()).await;
    let frame = next_json(&mut ws).await;
    let arrived = started.elapsed();
    println!("QUEUE_ARRIVAL_MS {:.1}", arrived.as_secs_f64() * 1000.0);
    assert_eq!(
        (frame["type"].as_str(), frame["text"].as_str(), frame["id"].as_str()),
        (Some("text"), Some("written while you were away"), Some(ids[0].as_str()))
    );
    assert!(arrived < Duration::from_secs(2), "arrived after {arrived:?}");
    send_json(&mut ws, &ack(&frame)).await;
    let id = ids[0].clone();
    wait_until("the item to be done", || status_of(&w, &id) == "done").await;
    assert_eq!(w.core().snapshot().queued, 0);
    assert!(toasts(&drain(&mut w.env.events)).contains(&"Sent 1 queued item to Test Phone".to_string()));

    // It did not go through at the first try (the phone was away), and the record says so.
    let send = w.core().db.get_send(&id).unwrap().unwrap();
    assert_eq!((send.attempts, send.result.as_str(), send.offline_at_tap), (2, "ok", true));
}

#[tokio::test]
async fn queued_items_go_out_in_creation_order_one_at_a_time() {
    let mut w = World::new().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    assert_eq!(send_text(&w, "A").await, SendResult::PhoneOffline);
    assert_eq!(send_text(&w, "B").await, SendResult::PhoneOffline);
    let photo = make_file(src.path(), "photo.jpg", 1000);
    let report = w.env.core.send_paths(vec![photo], "test").await;
    assert_eq!((report.files, report.result.clone()), (1, Some(SendResult::PhoneOffline)));
    assert_eq!(send_text(&w, "C").await, SendResult::PhoneOffline);
    let order = queued_ids(&w);
    assert_eq!(order.len(), 4);
    drain(&mut w.env.events);

    let mut ws = reconnect(&w).await;
    let mut seen = Vec::new();
    for expected in ["text A", "text B", "offer", "text C"] {
        let frame = next_json(&mut ws).await;
        let what = match frame["type"].as_str().unwrap() {
            "text" => format!("text {}", frame["text"].as_str().unwrap()),
            other => other.to_string(),
        };
        assert_eq!(what, expected);
        seen.push(frame["id"].as_str().unwrap().to_string());
        // The next item is not sent before this one is acked.
        assert_silent(&mut ws, 150).await;
        send_json(&mut ws, &ack(&frame)).await;
    }
    assert_eq!(seen, order, "the order of creation, under the IDs the items were made with");
    wait_until("the queue to be empty", || w.core().snapshot().queued == 0).await;
    assert_eq!(status_of(&w, &order[2]), "active", "an acked offer is live: the phone downloads it now");
    assert_eq!(status_of(&w, &order[3]), "done");
}

#[tokio::test]
async fn a_send_into_a_dead_connection_requeues_the_item_and_it_goes_again_with_the_same_id() {
    let mut w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "anybody there".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    let id = frame["id"].as_str().unwrap().to_string();
    // The phone never answers. After 5 s the connection counts as dead, and the item is still there.
    let started = Instant::now();
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    assert!(started.elapsed() >= Duration::from_secs(4), "{:?}", started.elapsed());
    assert_eq!(sending.await.unwrap(), SendResult::NoReply);
    assert_eq!(status_of(&w, &id), "queued", "an item that got no ack is queued again, not failed");
    assert_eq!(
        toasts(&drain(&mut w.env.events)),
        ["No reply from the phone. The text is queued and will be sent again when the phone is back."]
    );
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;

    // The reconnect sends the same message again. The phone remembers the ID, so a repeat is harmless.
    let mut ws = reconnect(&w).await;
    let again = next_json(&mut ws).await;
    assert_eq!((again["id"].as_str(), again["text"].as_str()), (Some(id.as_str()), Some("anybody there")));
    send_json(&mut ws, &ack(&again)).await;
    wait_until("the item to be done", || status_of(&w, &id) == "done").await;
    let send = w.core().db.get_send(&id).unwrap().unwrap();
    assert_eq!((send.attempts, send.result.as_str()), (2, "ok"), "two attempts: not a first-try success");
}

#[tokio::test]
async fn an_error_from_the_phone_fails_the_item_and_it_is_not_sent_again() {
    let mut w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "too much".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "too_large", "message": "no" })).await;
    assert_eq!(sending.await.unwrap(), SendResult::Rejected("too_large".into()));
    let id = frame["id"].as_str().unwrap();
    assert_eq!(status_of(&w, id), "failed");
    let item = &history_items(&w)[0];
    assert_eq!(
        (item["status"].as_str(), item["error"].as_str(), item["canRetry"].as_bool()),
        (Some("failed"), Some("too_large"), Some(true))
    );
    // A reconnect does not send it again.
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;
    let mut ws = reconnect(&w).await;
    assert_silent(&mut ws, 400).await;
}

#[tokio::test]
async fn the_queue_survives_a_restart_of_the_app() {
    let w = World::new_on_disk().await;
    w.pair().await;
    assert_eq!(send_text(&w, "first").await, SendResult::PhoneOffline);
    assert_eq!(send_text(&w, "second").await, SendResult::PhoneOffline);
    let before = queued_ids(&w);
    assert_eq!(before.len(), 2);

    // Kin.exe is stopped and started again over the same data directory.
    let w = w.restart().await;
    assert_eq!(queued_ids(&w), before, "the items are still queued, in the same order");
    assert_eq!(w.core().snapshot().queued, 2);
    assert!(w.core().snapshot().devices.iter().any(|d| d.device_id == w.phone.device_id), "the phone is still paired");
    let items = history_items(&w);
    assert_eq!(items.len(), 2);

    let mut ws = reconnect(&w).await;
    for (expected, text) in before.iter().zip(["first", "second"]) {
        let frame = next_json(&mut ws).await;
        assert_eq!((frame["id"].as_str(), frame["text"].as_str()), (Some(expected.as_str()), Some(text)));
        send_json(&mut ws, &ack(&frame)).await;
    }
    wait_until("the queue to be empty", || w.core().snapshot().queued == 0).await;
}

#[tokio::test]
async fn a_queued_offer_survives_a_restart_and_is_offered_with_its_files() {
    let w = World::new_on_disk().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    let paths = vec![make_file(src.path(), "a.bin", 100), make_file(src.path(), "b.bin", 200)];
    let report = w.env.core.send_paths(paths, "test").await;
    assert_eq!(report.result, Some(SendResult::PhoneOffline));
    let id = report.transfers[0].clone();
    let w = w.restart().await;
    assert_eq!(status_of(&w, &id), "queued");
    let mut ws = reconnect(&w).await;
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].as_str(), frame["id"].as_str(), frame["totalSize"].as_u64()), (Some("offer"), Some(id.as_str()), Some(300)));
    assert_eq!(frame["files"].as_array().unwrap().len(), 2);
    send_json(&mut ws, &ack(&frame)).await;
    wait_until("the offer to be live", || status_of(&w, &id) == "active").await;
    assert!(w.core().transfers.get(&id).is_some(), "the files are served from where they are");
}

// ----- expiry (with a clock moved by hand), cancel -------------------------------------------------------------------------

#[tokio::test]
async fn a_queued_text_expires_after_7_days_and_shows_as_failed_with_expired() {
    let mut w = World::new().await;
    w.pair().await;
    assert_eq!(send_text(&w, "old news").await, SendResult::PhoneOffline);
    let id = queued_ids(&w)[0].clone();
    let created = w.core().db.history_item(&id).unwrap().unwrap().created_at;
    drain(&mut w.env.events);

    // The fake clock: nothing happens until the 7 days are over, to the millisecond.
    assert_eq!(w.env.core.expire_queue(created + 7 * DAY_MS - 1).await, 0);
    assert_eq!(status_of(&w, &id), "queued");
    assert_eq!(w.env.core.expire_queue(created + 7 * DAY_MS).await, 1, "at exactly 7 days it is expired");
    assert_eq!(status_of(&w, &id), "failed");
    let item = &history_items(&w)[0];
    assert_eq!((item["status"].as_str(), item["error"].as_str()), (Some("failed"), Some("expired")));
    assert_eq!(w.core().snapshot().queued, 0);
    let send = w.core().db.get_send(&id).unwrap().unwrap();
    assert_eq!((send.result.as_str(), send.error.as_deref()), ("failed", Some("expired")));

    // It is never sent, and a second pass finds nothing.
    assert_eq!(w.env.core.expire_queue(created + 30 * DAY_MS).await, 0);
    let mut ws = reconnect(&w).await;
    assert_silent(&mut ws, 400).await;
}

#[tokio::test]
async fn an_offer_never_sent_expires_after_7_days_and_a_sent_one_24_hours_after_it_was_sent() {
    let mut w = World::new().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    // 1. Queued and never sent: 7 days after it was made. Its temporary file goes with it.
    let report = w.env.core.send_paths(vec![make_file(src.path(), "never.bin", 10)], "test").await;
    let never = report.transfers[0].clone();
    let created = w.core().db.history_item(&never).unwrap().unwrap().created_at;
    assert_eq!(w.env.core.expire_queue(created + 7 * DAY_MS - 1).await, 0);
    assert_eq!(w.env.core.expire_queue(created + 7 * DAY_MS).await, 1);
    assert_eq!((status_of(&w, &never).as_str(), history_items(&w)[0]["error"].as_str()), ("failed", Some("expired")));

    // 2. Sent into a connection that gave no ack: the offer is in the queue and was sent. It lasts 24 hours from that send.
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);
    let core = w.env.core.clone();
    let path = make_file(src.path(), "sent.bin", 10);
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "offer");
    let sent_id = frame["id"].as_str().unwrap().to_string();
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000), "no ack: the connection is closed as dead");
    let report = sending.await.unwrap();
    assert_eq!(report.result, Some(SendResult::NoReply));
    assert_eq!(status_of(&w, &sent_id), "queued", "the offer stays queued");
    let row = w.core().db.history_item(&sent_id).unwrap().unwrap();
    let sent_at =
        w.core().db.queued_items(&w.phone.device_id).unwrap().into_iter().find(|i| i.id == sent_id).unwrap().sent_at.expect("it was sent");
    assert!(row.created_at <= sent_at);
    let expires = w.core().db.queued_items(&w.phone.device_id).unwrap().into_iter().find(|i| i.id == sent_id).unwrap().expires_at.unwrap();
    assert_eq!(expires, sent_at + 24 * HOUR_MS, "24 hours after it was sent, not 7 days after it was made");
    assert_eq!(w.env.core.expire_queue(expires - 1).await, 0);
    assert_eq!(w.env.core.expire_queue(expires).await, 1);
    assert_eq!((status_of(&w, &sent_id).as_str(), history_items(&w)[0]["error"].as_str()), ("failed", Some("expired")));
    assert!(w.core().transfers.get(&sent_id).is_none(), "the files are not served any more");
}

#[tokio::test]
async fn an_offer_that_got_no_ack_is_offered_again_under_the_same_id_after_the_reconnect() {
    let mut w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);
    let src = tempfile::tempdir().unwrap();
    let core = w.env.core.clone();
    let path = make_file(src.path(), "again.bin", 64);
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let first = next_json(&mut ws).await;
    let id = first["id"].as_str().unwrap().to_string();
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    assert_eq!(sending.await.unwrap().result, Some(SendResult::NoReply));
    assert_eq!(status_of(&w, &id), "queued");
    assert!(w.core().transfers.get(&id).is_some(), "the files stay served: the phone may have had the offer");

    let mut ws = reconnect(&w).await;
    let second = next_json(&mut ws).await;
    assert_eq!((second["type"].as_str(), second["id"].as_str()), (Some("offer"), Some(id.as_str())), "the same offer, the same ID");
    assert_eq!(second["files"], first["files"]);
    send_json(&mut ws, &ack(&second)).await;
    wait_until("the offer to be live", || status_of(&w, &id) == "active").await;
    // The phone has everything and says so: the offer is done, and it is not offered again.
    send_json(&mut ws, &json!({ "type": "complete", "id": "m-done", "transferId": id })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-done" }));
    wait_until("the offer to be done", || status_of(&w, &id) == "done").await;
    assert_eq!(w.core().db.get_send(&id).unwrap().unwrap().result, "ok");
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;
    let mut ws = reconnect(&w).await;
    assert_silent(&mut ws, 400).await;
}

#[tokio::test]
async fn a_queued_item_that_is_canceled_never_goes_out() {
    let mut w = World::new().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    assert_eq!(send_text(&w, "never mind").await, SendResult::PhoneOffline);
    let report = w.env.core.send_paths(vec![make_file(src.path(), "skip.bin", 10)], "test").await;
    let (text_id, offer_id) = (queued_ids(&w)[0].clone(), report.transfers[0].clone());
    assert_eq!(history_items(&w).iter().filter(|i| i["canCancel"] == true).count(), 2, "Cancel is there for queued items");
    drain(&mut w.env.events);

    assert!(w.env.core.history_cancel(&text_id).await);
    assert!(w.env.core.history_cancel(&offer_id).await);
    assert!(!w.env.core.history_cancel(&text_id).await, "a second cancel finds nothing queued");
    assert_eq!((status_of(&w, &text_id).as_str(), status_of(&w, &offer_id).as_str()), ("canceled", "canceled"));
    assert_eq!(w.core().snapshot().queued, 0);
    let statuses: Vec<String> = history_items(&w).iter().map(|i| i["status"].as_str().unwrap().to_string()).collect();
    assert_eq!(statuses, ["canceled", "canceled"]);
    for send_id in [&text_id, &offer_id] {
        assert_eq!(w.core().db.get_send(send_id).unwrap().unwrap().result, "canceled", "a cancel is not a failure of the send");
    }

    // The phone connects: nothing arrives.
    let mut ws = reconnect(&w).await;
    assert_silent(&mut ws, 500).await;
}

#[tokio::test]
async fn the_automatic_clipboard_is_never_queued_and_a_sensitive_text_is_never_stored() {
    let mut w = World::new().await;
    w.pair().await;
    drain(&mut w.env.events);

    // Automatic clipboard while the phone is away: dropped. Nothing is stored, nothing is queued, nothing is in the history.
    w.env.clipboard.user_copy("a stale clipboard", vec![]);
    w.env.core.process_clipboard_change(Tap::now()).await;
    assert_eq!(w.core().snapshot().queued, 0);
    assert!(history_items(&w).is_empty());
    assert_eq!(w.core().status_json()["lastSent"]["clipboard"]["result"], "phone_offline");

    // A sensitive text (the hotkey with a password manager's formats on the clipboard) while the phone is away: not stored either.
    w.env.clipboard.user_copy("hunter2-secret", SENSITIVE_FORMATS.iter().map(|f| ClipFormat::with_dword(f, 0)).collect());
    let result = w.env.core.send_clipboard_text("hotkey", Tap::now()).await;
    assert_eq!(result, SendResult::PhoneOffline);
    assert_eq!(w.core().snapshot().queued, 0, "a sensitive text can't wait in a queue: it would be stored");
    assert!(history_items(&w).is_empty());
    assert!(toasts(&drain(&mut w.env.events)).iter().any(|t| t.contains("never queued")));

    // The phone connects: neither arrives.
    let mut ws = reconnect(&w).await;
    assert_silent(&mut ws, 500).await;
    // Nothing of the secret is anywhere in the database's texts.
    assert!(
        w.core()
            .db
            .history(10)
            .unwrap()
            .iter()
            .all(|r| r.text.as_deref() != Some("hunter2-secret") && r.text.as_deref() != Some("a stale clipboard"))
    );
}

// ----- the history --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_history_never_shows_a_sensitive_text_or_an_automatic_clipboard_sync() {
    let mut w = World::new().await;
    let mut ws = w.connected_with_sync().await;
    drain(&mut w.env.events);

    // An automatic sync, acked.
    w.env.clipboard.user_copy("synced automatically", vec![]);
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "clipboard");
    send_json(&mut ws, &ack(&frame)).await;
    // A sensitive text through the hotkey, acked.
    w.env.clipboard.user_copy("pa55word!", SENSITIVE_FORMATS.iter().map(|f| ClipFormat::with_dword(f, 0)).collect());
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", Tap::now()).await });
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].as_str(), frame["sensitive"].as_bool()), (Some("text"), Some(true)));
    send_json(&mut ws, &ack(&frame)).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
    // An explicit text: this one is in the history.
    w.env.clipboard.user_copy("an ordinary text", vec![]);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", Tap::now()).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&frame)).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
    // A sensitive text from the phone, and an ordinary one.
    send_json(&mut ws, &json!({ "type": "text", "id": "m-secret", "text": "phone secret", "isUrl": false, "sensitive": true })).await;
    assert_eq!(next_json(&mut ws).await["type"], "ack");
    send_json(&mut ws, &json!({ "type": "text", "id": "m-plain", "text": "phone text", "isUrl": false })).await;
    assert_eq!(next_json(&mut ws).await["type"], "ack");
    wait_until("the texts to arrive", || history_items(&w).len() >= 2).await;

    let items = history_items(&w);
    let texts: Vec<(&str, &str)> = items.iter().map(|i| (i["direction"].as_str().unwrap(), i["text"].as_str().unwrap())).collect();
    assert_eq!(texts, [("in", "phone text"), ("out", "an ordinary text")], "only the explicit, non-sensitive texts, newest first");
    let everything = serde_json::to_string(&w.core().history_view()).unwrap();
    for secret in ["pa55word!", "phone secret", "synced automatically"] {
        assert!(!everything.contains(secret), "{secret} must not be in the history");
    }
    assert!(items.iter().all(|i| i["status"] == "done"));
}

#[tokio::test]
async fn a_click_on_a_finished_item_follows_the_allow_list_and_a_text_is_copied_without_going_back_to_the_phone() {
    let w = World::new().await;
    w.pair().await;
    let received = tempfile::tempdir().unwrap();
    let add = |id: &str, names: &[&str]| {
        let paths: Vec<PathBuf> = names.iter().map(|n| make_file(received.path(), n, 3)).collect();
        let row = kin_lib::storage::TransferRow {
            id: id.to_string(),
            device_id: w.phone.device_id.clone(),
            direction: "in".into(),
            status: "done".into(),
            created_at: kin_lib::encoding::unix_ms() as i64,
            updated_at: 0,
            finished_at: Some(kin_lib::encoding::unix_ms() as i64),
            expires_at: None,
            total_bytes: 3 * names.len() as i64,
            error_code: None,
            dest_dir: None,
        };
        let files: Vec<kin_lib::storage::TransferFileRow> = names
            .iter()
            .zip(&paths)
            .enumerate()
            .map(|(i, (n, p))| kin_lib::storage::TransferFileRow {
                file_id: format!("f{i}"),
                idx: i as i64,
                name: n.to_string(),
                mime: String::new(),
                size: 3,
                bytes_done: 3,
                local_path: Some(p.to_string_lossy().into_owned()),
                finalized: true,
                source_mtime_ms: None,
                temp_source: false,
                finalizing_name: None,
            })
            .collect();
        w.core().db.insert_transfer(&row, &files).unwrap();
        paths
    };
    let photo = add("t-photo", &["holiday.jpg"]);
    let exe = add("t-exe", &["setup.exe"]);
    let pair = add("t-two", &["a.jpg", "b.jpg"]);

    let core = &w.env.core;
    assert!(core.history_open("t-photo").await.is_ok());
    assert!(core.history_open("t-exe").await.is_ok());
    assert!(core.history_open("t-two").await.is_ok());
    assert_eq!(w.env.opener.opened_files(), photo, "a picture opens with its default program");
    assert_eq!(w.env.opener.shown_in_folder(), exe, "a program is only shown in its folder: a click must never run it");
    assert_eq!(w.env.opener.opened_folders(), vec![pair[0].parent().unwrap().to_path_buf()], "several files open their folder");
    assert!(core.history_open("no-such-item").await.is_err());

    // A text is copied. The automatic sync must not send that copy to the phone as a new clipboard.
    w.core()
        .db
        .insert_text_item(
            "t-text",
            &w.phone.device_id,
            "in",
            "done",
            kin_lib::encoding::unix_ms() as i64,
            None,
            "copy me\nplease",
            false,
            None,
        )
        .unwrap();
    assert!(core.history_open("t-text").await.is_ok());
    assert_eq!(w.env.clipboard.text().as_deref(), Some("copy me\r\nplease"), "with Windows line ends");
    core.process_clipboard_change(Tap::now()).await;
    let sent = core.status_json()["clipboardSync"]["lastSkip"]["reason"].clone();
    assert_eq!(sent, "echo_of_phone_text", "the copy is not sent back to the phone");
}

#[tokio::test]
async fn retry_makes_a_new_item_for_a_failed_text_and_for_failed_files() {
    let mut w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);
    let src = tempfile::tempdir().unwrap();

    // A text that the phone refuses.
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "retry me".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    let failed_id = frame["id"].as_str().unwrap().to_string();
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "internal", "message": "oops" })).await;
    sending.await.unwrap();
    assert!(history_items(&w).iter().any(|i| i["id"] == failed_id && i["canRetry"] == true));

    let new_id = w.env.core.retry_item(&failed_id).await.unwrap();
    assert_ne!(new_id, failed_id, "an ID is never used twice");
    let again = next_json(&mut ws).await;
    assert_eq!((again["id"].as_str(), again["text"].as_str()), (Some(new_id.as_str()), Some("retry me")));
    send_json(&mut ws, &ack(&again)).await;
    wait_until("the retry to be done", || status_of(&w, &new_id) == "done").await;
    let items = history_items(&w);
    let old = items.iter().find(|i| i["id"] == failed_id).unwrap();
    let new = items.iter().find(|i| i["id"] == new_id.as_str()).unwrap();
    assert_eq!(
        (old["canRetry"].as_bool(), old["retried"].as_bool(), old["status"].as_str()),
        (Some(false), Some(true), Some("failed")),
        "the old one stays, and can be retried once"
    );
    assert_eq!(new["retryOf"].as_str(), Some(&failed_id[failed_id.len() - 8..]));
    assert!(w.env.core.retry_item(&failed_id).await.is_err(), "a second retry of the same item is refused");
    assert!(w.env.core.retry_item(&new_id).await.is_err(), "a finished item is not retried");

    // Files that the phone refuses (disk full): the retry offers the same files again, under a new transfer ID.
    let core = w.env.core.clone();
    let path = make_file(src.path(), "big.bin", 500);
    let task = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let offer = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": offer["id"], "code": "disk_full", "message": "no room" })).await;
    let report = task.await.unwrap();
    assert_eq!(report.result, Some(SendResult::Rejected("disk_full".into())));
    let failed_offer = offer["id"].as_str().unwrap().to_string();
    assert_eq!(status_of(&w, &failed_offer), "failed");
    let retried = w.env.core.retry_item(&failed_offer).await.unwrap();
    let second = next_json(&mut ws).await;
    assert_eq!((second["type"].as_str(), second["id"].as_str()), (Some("offer"), Some(retried.as_str())));
    assert_eq!(second["files"][0]["name"], "big.bin");
    send_json(&mut ws, &ack(&second)).await;
    wait_until("the retried offer to be live", || status_of(&w, &retried) == "active").await;

    // When the files are gone, a retry says so and makes nothing.
    let core = w.env.core.clone();
    let gone = make_file(src.path(), "gone.bin", 5);
    let gone_copy = gone.clone();
    let task = tokio::spawn(async move { core.send_paths(vec![gone_copy], "test").await });
    let offer = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": offer["id"], "code": "disk_full", "message": "no room" })).await;
    task.await.unwrap();
    std::fs::remove_file(&gone).unwrap();
    let err = w.env.core.retry_item(offer["id"].as_str().unwrap()).await.unwrap_err();
    assert!(err.contains("can't be found"), "{err}");
}

// ----- settings, debug numbers -------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_new_device_name_is_saved_and_the_connected_phone_gets_a_new_hello() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    assert_eq!(w.core().pc_name(), "Desk PC");
    assert_eq!(w.core().set_device_name("  Anna's laptop  "), "Anna's laptop");
    // The phone that is connected learns it from a later hello (which is not acked).
    let hello = next_json(&mut ws).await;
    assert_eq!(
        (hello["type"].as_str(), hello["name"].as_str(), hello["platform"].as_str()),
        (Some("hello"), Some("Anna's laptop"), Some("windows"))
    );
    assert_eq!(w.core().status_json()["pcName"], "Anna's laptop");
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(w.env.dir.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["deviceName"], "Anna's laptop");
    // An empty name is the computer's name again; a very long one is cut to 64 characters.
    assert_eq!(w.core().set_device_name(""), "Desk PC");
    assert_eq!(next_json(&mut ws).await["name"], "Desk PC");
    assert_eq!(w.core().set_device_name(&"N".repeat(100)).chars().count(), 64);
    // The next connection says it too, and so does the pairing answer.
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["name"], "N".repeat(64));
}

#[tokio::test]
async fn the_switches_and_the_received_folder_apply_at_once_and_are_kept() {
    let w = World::new().await;
    let core = w.core();
    core.set_auto_clipboard_sync(false);
    core.set_images_to_clipboard(false);
    let view = core.settings_view(Some(false));
    assert_eq!(
        (view["autoClipboardSync"].as_bool(), view["imagesToClipboard"].as_bool(), view["startWithWindows"].as_bool()),
        (Some(false), Some(false), Some(false))
    );
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(w.env.dir.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!((saved["autoClipboardSync"].as_bool(), saved["imagesToClipboard"].as_bool()), (Some(false), Some(false)));

    // The folder is checked: a folder that can't be used is refused and the old one stays.
    let before = core.received_dir();
    assert!(core.set_received_folder(Some("not\\absolute")).is_err());
    assert_eq!(core.received_dir(), before);
    // (The test setup has a developer folder, which wins over the setting: the setting itself is what is saved.)
    let target = tempfile::tempdir().unwrap();
    let chosen = target.path().join("Phone files");
    core.set_received_folder(Some(&chosen.to_string_lossy())).unwrap();
    assert!(chosen.is_dir(), "the folder was made");
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(w.env.dir.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["receivedFolder"], chosen.to_string_lossy().as_ref());
    core.set_received_folder(None).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(w.env.dir.path().join("settings.json")).unwrap()).unwrap();
    assert!(saved["receivedFolder"].is_null(), "reset to the default");
}

#[tokio::test]
async fn a_retried_send_counts_once_and_is_never_a_first_try_success() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    // A text that went through at the first try: one settled send, one first-try success.
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "fine".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&frame)).await;
    sending.await.unwrap();
    // A text that the phone refuses, retried, and the retry acked.
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "retry me".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    let failed_id = frame["id"].as_str().unwrap().to_string();
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "internal", "message": "oops" })).await;
    sending.await.unwrap();
    let view = w.core().debug_view(json!({ "networks": [] }));
    assert_eq!(
        (view["successRate"]["firstTry"]["settled"].as_u64(), view["successRate"]["firstTry"]["firstTryOk"].as_u64()),
        (Some(2), Some(1)),
        "before the retry the failed send is counted: {view}"
    );
    let new_id = w.env.core.retry_item(&failed_id).await.unwrap();
    let again = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&again)).await;
    wait_until("the retry to be done", || status_of(&w, &new_id) == "done").await;
    wait_until("the retry's send record to settle", || {
        w.core().debug_view(json!({ "networks": [] }))["successRate"]["firstTry"]["settled"].as_u64() == Some(2)
    })
    .await;
    let view = w.core().debug_view(json!({ "networks": [] }));
    let rate = &view["successRate"]["firstTry"];
    assert_eq!(
        (rate["settled"].as_u64(), rate["firstTryOk"].as_u64()),
        (Some(2), Some(1)),
        "the failed send and its retry are one send, and the retry is not a first try: {view}"
    );
    assert_eq!(view["successRate"]["byKind"]["text"]["settled"], 2);
    assert_eq!(view["errors"][0]["code"], "internal", "the error of the first try is still listed");
}

#[tokio::test]
async fn the_debug_view_counts_first_try_successes_and_keeps_the_connection_history_and_the_errors() {
    let w = World::new().await;
    w.pair().await;
    // One send while the phone is away: it will go through, but not at the first try.
    send_text(&w, "queued first").await;
    let mut ws = w.open_ws().await;
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&frame)).await;
    wait_until("the queued text", || w.core().snapshot().queued == 0).await;
    // Two sends that go through at the first try.
    for text in ["one", "two"] {
        let core = w.env.core.clone();
        let t = text.to_string();
        let sending = tokio::spawn(async move { core.deliver(Kind::Text, t, false, "test", Tap::now(), true).await });
        let frame = next_json(&mut ws).await;
        send_json(&mut ws, &ack(&frame)).await;
        assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
    }
    // A ping, for the latency.
    let core = w.env.core.clone();
    let pinging = tokio::spawn(async move { core.ping_phone().await });
    let ping = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&ping)).await;
    pinging.await.unwrap();
    // One that fails.
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "refused".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "disk_full", "message": "no" })).await;
    sending.await.unwrap();
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;

    let view = w.core().debug_view(json!({ "networks": [{ "name": "Home", "category": "Private" }] }));
    let rate = &view["successRate"]["firstTry"];
    assert_eq!(
        (rate["settled"].as_u64(), rate["firstTryOk"].as_u64()),
        (Some(4), Some(2)),
        "two of four sends worked at the first try: {view}"
    );
    assert_eq!(rate["percent"].as_f64(), Some(50.0));
    let connected = &view["successRate"]["firstTryWhileConnected"];
    assert_eq!(
        (connected["settled"].as_u64(), connected["firstTryOk"].as_u64()),
        (Some(3), Some(2)),
        "the send asked for while the phone was away is left out"
    );
    assert_eq!(view["successRate"]["byKind"]["text"]["settled"], 4);
    assert_eq!(view["errors"][0]["code"], "disk_full", "the newest error comes first, with its code");
    assert!(view["latency"]["tapToAck"]["n"].as_u64().unwrap() >= 3);
    assert_eq!(view["latency"]["ping"]["n"], 1);
    let events: Vec<&str> = view["connections"].as_array().unwrap().iter().map(|e| e["event"].as_str().unwrap()).collect();
    assert_eq!(events, ["disconnected", "connected"], "newest first");
    assert_eq!(view["network"]["networks"][0]["category"], "Private");
    assert_eq!(view["state"], "phone_offline");
}

#[tokio::test]
async fn the_log_export_is_a_zip_with_the_logs_and_a_status_file_with_nothing_personal_in_it() {
    let w = World::new().await;
    w.pair().await;
    // A log file with the kind of things a log has in it.
    let logs = w.env.dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join("kin.log"),
        "2026 INFO kin: phone connected: \"Test Phone\" from 192.168.1.20:50123, data dir C:\\Users\\SomeoneElse\\AppData\\Roaming\\Kin\n",
    )
    .unwrap();
    std::fs::write(logs.join("kin.1.log"), "older line\n").unwrap();

    let dest = w.env.dir.path().join("export.zip");
    let report = w.core().export_logs(&dest, json!({ "networks": [] })).unwrap();
    assert!(report.bytes > 100 && dest.is_file());
    let zip = std::fs::read(&dest).unwrap();
    let files = kin_lib::export::read_zip(&zip).unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for expected in ["README.txt", "logs/kin.log", "logs/kin.1.log", "status.json", "debug.json"] {
        assert!(names.contains(&expected), "{expected} is in the export: {names:?}");
    }
    let all = files.iter().map(|(_, d)| String::from_utf8_lossy(d).to_string()).collect::<String>();
    assert!(!all.contains("192.168.1.20"), "addresses are taken out");
    assert!(!all.contains("Test Phone"), "the phone's name is taken out");
    assert!(all.contains("<ip>") && all.contains("<phone>"));
    let status: Value = serde_json::from_slice(&files.iter().find(|(n, _)| n == "status.json").unwrap().1).unwrap();
    assert_eq!(status["deviceId"].as_str().unwrap().len(), 8, "device IDs are cut");
    assert!(status["queue"]["count"].is_number());
    // No temporary file is left where the export was written.
    assert!(!w.env.dir.path().join("export.zip.tmp").exists());
}

#[tokio::test]
async fn the_export_takes_out_the_name_of_a_phone_that_was_unpaired_too() {
    let w = World::new().await;
    w.pair().await;
    let logs = w.env.dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("kin.log"), "2026 INFO kin: phone connected: \"Test Phone\"\n").unwrap();
    // The phone is unpaired: its row is gone from the device list, and the log line that names it is still in the log.
    assert!(w.core().remove_device(&w.phone.device_id).unwrap());
    assert!(w.core().snapshot().devices.is_empty());
    let files = w.core().export_files(json!({ "networks": [] }));
    let log = files.iter().find(|(n, _)| n == "logs/kin.log").map(|(_, d)| String::from_utf8_lossy(d).to_string()).unwrap();
    assert!(!log.contains("Test Phone"), "the name of an unpaired phone is taken out: {log}");
    assert!(log.contains("<phone>"));
}

#[tokio::test]
async fn a_name_in_a_toast_is_not_in_the_log() {
    capture_logs();
    let mut w = World::new().await;
    let mut ws = w.connected_with_sync().await;
    drain(&mut w.env.events);
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "My Secret Plan.docx", 20);
    let core = w.env.core.clone();
    let task = tokio::spawn(async move { core.send_paths(vec![path, src.path().join("Missing Tax Return.pdf")], "test").await });
    let offer = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&offer)).await;
    task.await.unwrap();
    // The user is shown the names ...
    let events = drain(&mut w.env.events);
    let shown = toasts(&events);
    assert!(shown.iter().any(|t| t.contains("My Secret Plan.docx")), "{shown:?}");
    assert!(shown.iter().any(|t| t.contains("Missing Tax Return.pdf")), "{shown:?}");
    // ... and the log has only a hash of each.
    let log = captured_lines().join("\n");
    assert!(!log.contains("Secret Plan") && !log.contains("Tax Return"), "a file name reached the log:\n{log}");
    assert!(log.contains("<file "), "the log has a short hash instead");
    // The event for the log is the redacted one.
    assert!(events.iter().any(|e| matches!(e, Event::ToastRedacted { log_text, .. } if log_text.contains("<file "))));
}

impl World {
    /// A paired, connected phone with the clipboard sync running (the same as in `fake_phone.rs`).
    async fn connected_with_sync(&self) -> Ws {
        self.pair().await;
        let ws = self.open_ws().await;
        let core = self.env.core.clone();
        let changes = self.env.clipboard.changes();
        tokio::spawn(async move { core.run_clipboard_sync(changes).await });
        ws
    }
}
