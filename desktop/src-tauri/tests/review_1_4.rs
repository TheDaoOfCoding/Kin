//! The fixes of the 1.4 review (docs/results/phase-1-4-desktop.md, "1.4 review fixes"; protocol.md changelog 52), each with the failure it
//! prevents: a queued long text that the clean-up of a day later deleted, an `ack` that undid a cancel, an offer marked as sent that never
//! left, a hotkey press at the moment of a reconnect, a double click on Retry, a cancel that waited for a whole flush, two paired phones,
//! `cancel` messages that were forgotten, an offer that was sent and not acked before a restart, a late `complete`, and the growth of the
//! database.
mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use futures_util::StreamExt;
use kin_lib::connections::{ConnCommand, SendOutcome};
use kin_lib::encoding::unix_ms;
use kin_lib::identity::Identity;
use kin_lib::queue::{DAY_MS, HOUR_MS};
use kin_lib::storage::NewDevice;
use kin_lib::sync::{Kind, SendResult, Tap};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

async fn send_text(w: &World, text: &str) -> SendResult {
    w.env.core.deliver(Kind::Text, text.to_string(), false, "test", Tap::now(), true).await
}

fn ack(frame: &Value) -> Value {
    json!({ "type": "ack", "ref": frame["id"] })
}

async fn reconnect(w: &World) -> Ws {
    let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
    assert_eq!(next_json(&mut ws).await["type"], "hello");
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

fn queued_ids(w: &World) -> Vec<String> {
    let device = w.phone.device_id.clone();
    w.core().db.queued_items(&device).unwrap().into_iter().map(|i| i.id).collect()
}

fn make_file(dir: &Path, name: &str, size: usize) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, vec![7u8; size]).unwrap();
    path
}

fn folders_in(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|r| r.flatten().count()).unwrap_or(0)
}

// ----- B1: a queued long text is not deleted after a day ------------------------------------------------------------------------

#[tokio::test]
async fn a_queued_long_text_survives_the_clean_up_of_a_day_later_and_is_delivered() {
    let mut w = World::new().await;
    w.pair().await;
    drain(&mut w.env.events);
    // Over 128 KiB, so it is not a `text` message but a `Text.txt` offer: its file is the only copy of the text until the phone has it.
    let text = "a long text, long enough to be a file. ".repeat(6_000);
    assert!(text.len() > 128 * 1024);
    assert_eq!(send_text(&w, &text).await, SendResult::PhoneOffline);
    let id = queued_ids(&w)[0].clone();
    let outgoing = w.env.dir.path().join("outgoing");
    assert_eq!(folders_in(&outgoing), 1, "the text is in a file of our own");
    // For contrast: a folder that no record names is an orphan, and a day later it goes.
    let orphan = outgoing.join("orphan");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("Text.txt"), "nobody wants me").unwrap();

    // 25 hours later, the hourly clean-up runs. (It used to look only at transfers in memory, and a queued offer is in the database alone.)
    w.env.core.clean_up_at(unix_ms() + 25 * HOUR_MS as u64).await;
    assert!(!orphan.exists(), "the clean-up did run, and an orphan is removed");
    assert_eq!(folders_in(&outgoing), 1, "the queued text's file is still there");
    assert_eq!(status_of(&w, &id), "queued");

    // The phone comes back, and the text is delivered: the file is served whole.
    let mut ws = reconnect(&w).await;
    let offer = next_json(&mut ws).await;
    assert_eq!(
        (offer["type"].as_str(), offer["id"].as_str(), offer["files"][0]["name"].as_str()),
        (Some("offer"), Some(id.as_str()), Some("Text.txt"))
    );
    assert_eq!(offer["files"][0]["size"].as_u64(), Some(text.len() as u64));
    send_json(&mut ws, &ack(&offer)).await;
    wait_until("the offer to be live", || status_of(&w, &id) == "active").await;
    let mut tls = w.tls().await;
    let got = request(&mut tls, "GET", &format!("/v1/transfers/{id}/files/f1"), &[], b"").await.unwrap();
    assert_eq!((got.status, got.body.len()), (200, text.len()));
    assert_eq!(got.body, text.as_bytes(), "the text arrived as it was written");
    send_json(&mut ws, &json!({ "type": "complete", "id": "m-done", "transferId": id })).await;
    wait_until("the offer to be done", || status_of(&w, &id) == "done").await;
    wait_until("the temporary file to go", || folders_in(&outgoing) == 0).await;
}

#[tokio::test]
async fn an_offer_sent_and_not_acked_is_loaded_after_a_restart_and_expires_24_hours_after_it_was_sent() {
    let w = World::new_on_disk().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    let text = "a text that went out and was never acknowledged. ".repeat(4_000);
    let core = w.env.core.clone();
    let wire = text.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, wire, false, "test", Tap::now(), true).await });
    let offer = next_json(&mut ws).await;
    let id = offer["id"].as_str().unwrap().to_string();
    // No ack: the PC counts the connection as dead after 5 s. The phone may have the offer.
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    assert_eq!(sending.await.unwrap(), SendResult::NoReply);
    assert_eq!(status_of(&w, &id), "queued");
    let (sent_at, expires) = w.core().db.sent_and_expiry(&id).unwrap().unwrap();
    assert!(sent_at.is_some(), "it was written to the connection, so it counts as sent");
    assert!(expires.unwrap() - sent_at.unwrap() == DAY_MS, "and its 24 hours run from that moment");

    // The app restarts. The old clean-up would also see no transfer in memory and remove the file a day later.
    let w = w.restart().await;
    assert!(w.core().transfers.get(&id).is_some(), "the offer is loaded again: the phone may be downloading it");
    w.env.core.clean_up_at(unix_ms() + 25 * HOUR_MS as u64).await;
    assert_eq!(folders_in(&w.env.dir.path().join("outgoing")), 0, "its 24 hours are over, so the offer expired and the file went with it");
    assert_eq!(status_of(&w, &id), "failed");
    assert_eq!(w.core().db.history_item(&id).unwrap().unwrap().error_code.as_deref(), Some("expired"));
}

#[tokio::test]
async fn a_sent_and_unacked_offer_is_served_after_a_restart_before_any_hello() {
    let w = World::new_on_disk().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "served.bin", 5_000);
    let mut ws = w.open_ws().await;
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let offer = next_json(&mut ws).await;
    let id = offer["id"].as_str().unwrap().to_string();
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    assert_eq!(sending.await.unwrap().result, Some(SendResult::NoReply));

    let w = w.restart().await;
    let mut tls = w.tls().await;
    let got = request(&mut tls, "GET", &format!("/v1/transfers/{id}/files/f1"), &[], b"").await.unwrap();
    assert_eq!((got.status, got.body.len()), (200, 5_000), "the phone that had the offer can still download it after the restart");
    assert_eq!(status_of(&w, &id), "queued", "it is still a queued item, offered again at the next hello");
}

// ----- the ack that overwrote a cancel; "sent" marked too early ------------------------------------------------------------------

#[tokio::test]
async fn an_ack_that_arrives_after_a_cancel_does_not_undo_it() {
    let mut w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);
    let src = tempfile::tempdir().unwrap();
    let path = make_file(src.path(), "undone.bin", 100);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let offer = next_json(&mut ws).await;
    let id = offer["id"].as_str().unwrap().to_string();
    // The phone cancels while the PC is still waiting for its `ack` (the ack wait is up to 5 s).
    send_json(&mut ws, &json!({ "type": "cancel", "id": "c-1", "transferId": id, "reason": "user" })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "c-1" }));
    wait_until("the cancel to be recorded", || status_of(&w, &id) == "canceled").await;
    // The `ack` of the offer comes after all.
    send_json(&mut ws, &ack(&offer)).await;
    sending.await.unwrap();
    assert_eq!(status_of(&w, &id), "canceled", "the late ack must not make the item active again");
    assert_eq!(w.core().db.get_send(&id).unwrap().unwrap().result, "canceled");
    assert!(!toasts(&drain(&mut w.env.events)).iter().any(|t| t.starts_with("Sending ")), "no \"Sending\" toast for a canceled offer");
}

#[tokio::test]
async fn an_offer_is_marked_sent_only_once_it_was_written_to_the_connection() {
    let w = World::new().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    let device = w.phone.device_id.clone();

    // A connection that is registered and open but whose end is gone: the frame can't be written.
    let (conn_id, rx) = w.core().connections.register(&device);
    w.core().connections.mark_open(&device, conn_id);
    drop(rx);
    let report = w.env.core.send_paths(vec![make_file(src.path(), "never-left.bin", 10)], "test").await;
    assert_eq!(report.result, Some(SendResult::PhoneOffline));
    let id = report.transfers[0].clone();
    let (sent_at, expires) = w.core().db.sent_and_expiry(&id).unwrap().unwrap();
    assert_eq!(sent_at, None, "nothing was written, so it was not sent");
    let created = w.core().db.history_item(&id).unwrap().unwrap().created_at;
    assert_eq!(expires, Some(created + 7 * DAY_MS), "it is still a queued item with its 7 days, not an offer with 24 hours");
    assert!(w.core().transfers.get(&id).is_none(), "and nothing is served for it");
    assert_eq!(status_of(&w, &id), "queued");
}

// ----- the hotkey at the moment of a reconnect ----------------------------------------------------------------------------------

#[tokio::test]
async fn a_text_stored_at_the_moment_the_phone_connects_is_not_left_waiting() {
    let w = World::new().await;
    w.pair().await;
    let device = w.phone.device_id.clone();
    // The phone is away when the send looks, and connects right after the text was stored: its flush (at its `hello`) ran before the item
    // existed. The hook stands in for that connection.
    let slot: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<ConnCommand>>>> = Arc::new(Mutex::new(None));
    let (core_for_hook, slot_for_hook) = (w.env.core.clone(), slot.clone());
    w.core().on_next_text_stored(move || {
        let (conn_id, rx) = core_for_hook.connections.register(&device);
        core_for_hook.connections.mark_open(&device, conn_id);
        *slot_for_hook.lock().unwrap() = Some(rx);
    });
    let core = w.env.core.clone();
    let sending =
        tokio::spawn(
            async move { core.deliver(Kind::Text, "pressed at the wrong moment".into(), false, "hotkey", Tap::now(), true).await },
        );
    wait_until("the connection to appear", || slot.lock().unwrap().is_some()).await;
    let mut rx = slot.lock().unwrap().take().unwrap();
    let command = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await.expect("the text is sent without waiting for a reconnect");
    let Some(ConnCommand::Send { frame, reply }) = command else { panic!("expected a send") };
    assert_eq!((frame["type"].as_str(), frame["text"].as_str()), (Some("text"), Some("pressed at the wrong moment")));
    reply.send(SendOutcome::Ack(Duration::from_millis(3))).unwrap();
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
}

// ----- Retry twice -----------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_double_click_on_retry_makes_one_new_item_for_a_text_and_for_files() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    let src = tempfile::tempdir().unwrap();

    // A failed text.
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.deliver(Kind::Text, "retry me".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "too_large", "message": "no" })).await;
    sending.await.unwrap();
    let text_id = frame["id"].as_str().unwrap().to_string();
    // A failed offer.
    let core = w.env.core.clone();
    let path = make_file(src.path(), "retry.bin", 50);
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let offer = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": offer["id"], "code": "disk_full", "message": "no" })).await;
    sending.await.unwrap();
    let offer_id = offer["id"].as_str().unwrap().to_string();
    assert_eq!((status_of(&w, &text_id).as_str(), status_of(&w, &offer_id).as_str()), ("failed", "failed"));
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;

    for id in [&text_id, &offer_id] {
        let (a, b) = tokio::join!(w.env.core.retry_item(id), w.env.core.retry_item(id));
        let results = [a, b];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "exactly one of two clicks makes an item: {results:?}");
        let err = results.iter().find_map(|r| r.as_ref().err()).unwrap();
        assert_eq!(err, "That item was already sent again.");
        let made = w.core().db.history(50).unwrap().into_iter().filter(|r| r.retry_of.as_deref() == Some(id.as_str())).count();
        assert_eq!(made, 1, "one item points back at {id}");
    }
}

// ----- cancel during a flush --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_cancel_does_not_wait_for_a_flush_and_the_canceled_item_is_never_sent() {
    let mut w = World::new().await;
    w.pair().await;
    for t in ["A", "B", "C"] {
        assert_eq!(send_text(&w, t).await, SendResult::PhoneOffline);
    }
    let ids = queued_ids(&w);
    drain(&mut w.env.events);
    let mut ws = reconnect(&w).await;
    // A is on its way and the PC waits for its ack (up to 5 s). A cancel of C does not wait for that.
    let a = next_json(&mut ws).await;
    assert_eq!(a["text"], "A");
    let started = Instant::now();
    assert!(w.env.core.history_cancel(&ids[2]).await);
    assert!(started.elapsed() < Duration::from_millis(1500), "the cancel waited {:?}", started.elapsed());
    assert_eq!(status_of(&w, &ids[2]), "canceled");
    send_json(&mut ws, &ack(&a)).await;
    let b = next_json(&mut ws).await;
    assert_eq!(b["text"], "B");
    send_json(&mut ws, &ack(&b)).await;
    assert_silent(&mut ws, 600).await; // C never goes out
    assert_eq!(
        (status_of(&w, &ids[0]).as_str(), status_of(&w, &ids[1]).as_str(), status_of(&w, &ids[2]).as_str()),
        ("done", "done", "canceled")
    );

    // The item that is on its way can be canceled too: it waits for that one send, which here ends when the ack comes.
    let w2 = World::new().await;
    w2.pair().await;
    assert_eq!(send_text(&w2, "in flight").await, SendResult::PhoneOffline);
    let id = queued_ids(&w2)[0].clone();
    let mut ws = reconnect(&w2).await;
    let frame = next_json(&mut ws).await;
    let core = w2.env.core.clone();
    let id2 = id.clone();
    let canceling = tokio::spawn(async move { core.history_cancel(&id2).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!canceling.is_finished(), "it waits for the send that is under way");
    send_json(&mut ws, &ack(&frame)).await;
    assert!(!canceling.await.unwrap(), "the text was sent and acked first, so there was nothing queued to cancel");
    assert_eq!(status_of(&w2, &id), "done");
}

// ----- two paired phones --------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_item_goes_to_the_phone_it_was_made_for_and_never_to_another_one() {
    let w = World::new().await;
    w.pair().await;
    let mut ws1 = w.open_ws().await; // the first phone is connected, and the item will be made for it (the oldest pairing)

    // A second phone is paired and connects after it, so it is the phone that connected last.
    let second = Identity::generate().unwrap();
    let key = TestKey { pkcs8: second.key_der, cert: second.cert_der, device_id: second.device_id, short_id: second.short_id };
    w.core()
        .db
        .upsert_device(
            &NewDevice { device_id: &key.device_id, name: "Second Phone", platform: "android", model: None, app_version: None },
            unix_ms() as i64 + 5,
        )
        .unwrap();
    let stream = connect(w.server.local_addr, phone_config(&w.pc.device_id, &key)).await.unwrap();
    let (mut ws2, _) =
        tokio_tungstenite::client_async(format!("ws://127.0.0.1:{}/v1/ws", w.server.local_addr.port()), stream).await.unwrap();
    assert_eq!(next_json(&mut ws2).await["type"], "hello");
    send_json(
        &mut ws2,
        &json!({ "type": "hello", "id": "h-second", "protocol": 1, "name": "Second Phone", "platform": "android", "appVersion": "0.1.0" }),
    )
    .await;
    assert_silent(&mut ws2, 400).await;

    // A text for the first phone. The queue used to look at the phone that connected last, and send to it.
    let core = w.env.core.clone();
    let sending =
        tokio::spawn(async move { core.deliver(Kind::Text, "for the first phone".into(), false, "test", Tap::now(), true).await });
    let frame = next_json(&mut ws1).await;
    assert_eq!(frame["text"], "for the first phone");
    send_json(&mut ws1, &ack(&frame)).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
    assert_silent(&mut ws2, 400).await; // the second phone never got it
}

// ----- `cancel` messages are kept until acked ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_cancel_message_is_kept_until_acked_and_sent_after_every_hello_before_the_queue() {
    let w = World::new().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    let mut ws = w.open_ws().await;
    let core = w.env.core.clone();
    let path = make_file(src.path(), "cancel-me.bin", 100);
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let offer = next_json(&mut ws).await;
    send_json(&mut ws, &ack(&offer)).await;
    sending.await.unwrap();
    let id = offer["id"].as_str().unwrap().to_string();
    wait_until("the offer to be live", || status_of(&w, &id) == "active").await;

    // The phone goes away. The user cancels: the message can't be sent now, so it is kept.
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;
    assert!(w.env.core.cancel_transfer(&id).await);
    assert_eq!(status_of(&w, &id), "canceled");
    assert_eq!(w.core().db.pending_cancel_count().unwrap(), 1);
    // And a text is queued behind it.
    assert_eq!(send_text(&w, "after the cancel").await, SendResult::PhoneOffline);

    // The next connection: the cancel comes first, then the queue. This time the connection dies before the ack.
    let mut ws = reconnect(&w).await;
    let cancel = next_json(&mut ws).await;
    assert_eq!(
        (cancel["type"].as_str(), cancel["transferId"].as_str(), cancel["reason"].as_str()),
        (Some("cancel"), Some(id.as_str()), Some("user"))
    );
    let message_id = cancel["id"].as_str().unwrap().to_string();
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;
    assert_eq!(w.core().db.pending_cancel_count().unwrap(), 1, "no ack: it is kept");

    // The next one: the same message again, then the queue. Now it is acked and gone.
    let mut ws = reconnect(&w).await;
    let again = next_json(&mut ws).await;
    assert_eq!((again["type"].as_str(), again["id"].as_str()), (Some("cancel"), Some(message_id.as_str())), "the same message ID");
    send_json(&mut ws, &ack(&again)).await;
    let text = next_json(&mut ws).await;
    assert_eq!((text["type"].as_str(), text["text"].as_str()), (Some("text"), Some("after the cancel")), "the queue follows the cancel");
    send_json(&mut ws, &ack(&text)).await;
    wait_until("the cancel to be forgotten", || w.core().db.pending_cancel_count().unwrap() == 0).await;
    drop(ws);
    w.wait_for_state(kin_lib::core::LinkState::PhoneOffline).await;
    let mut ws = reconnect(&w).await;
    assert_silent(&mut ws, 400).await;
}

// ----- a late `complete`; no 3 second wait ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_late_complete_marks_the_offer_done_and_a_bad_request_does_not_hold_up_the_queue() {
    let mut w = World::new().await;
    w.pair().await;
    let src = tempfile::tempdir().unwrap();
    let mut ws = w.open_ws().await;
    let core = w.env.core.clone();
    let path = make_file(src.path(), "finished.bin", 100);
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let offer = next_json(&mut ws).await;
    let id = offer["id"].as_str().unwrap().to_string();
    // No ack: the connection is closed as dead and the offer waits in the queue (the phone may have it).
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    sending.await.unwrap();
    assert_eq!(send_text(&w, "behind the offer").await, SendResult::PhoneOffline);
    drain(&mut w.env.events);

    // The phone (an older one) says an offer it has already finished is a reuse. The item fails at once, and the queue goes on: no 3 s wait.
    let mut ws = reconnect(&w).await;
    let again = next_json(&mut ws).await;
    assert_eq!((again["type"].as_str(), again["id"].as_str()), (Some("offer"), Some(id.as_str())));
    let started = Instant::now();
    send_json(&mut ws, &json!({ "type": "error", "ref": again["id"], "code": "bad_request", "message": "that ID was used before" })).await;
    let text = next_json(&mut ws).await;
    assert_eq!(text["text"], "behind the offer");
    assert!(started.elapsed() < Duration::from_millis(1500), "the queue waited {:?}", started.elapsed());
    send_json(&mut ws, &ack(&text)).await;
    assert_eq!(status_of(&w, &id), "failed");

    // Its `complete` comes later: the files did arrive, so the offer is done.
    send_json(&mut ws, &json!({ "type": "complete", "id": "m-late", "transferId": id })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-late" }));
    wait_until("the late complete", || status_of(&w, &id) == "done").await;
    assert_eq!(w.core().db.get_send(&id).unwrap().unwrap().result, "ok");
    assert!(w.core().db.history_item(&id).unwrap().unwrap().error_code.is_none());
}

// ----- the database does not grow without limit ------------------------------------------------------------------------------------------

#[tokio::test]
async fn text_bodies_beyond_the_newest_500_go_but_the_rows_and_queued_bodies_stay() {
    let w = World::new().await;
    w.pair().await;
    let device = w.phone.device_id.clone();
    let db = &w.core().db;
    // A queued text that is the oldest of all: its body is needed.
    db.insert_text_item("queued-old", &device, "out", "queued", 1, Some(i64::MAX), "still waiting", false, None).unwrap();
    for i in 0..600i64 {
        db.insert_text_item(&format!("t{i:03}"), &device, "in", "done", 1000 + i, None, &format!("text {i}"), false, None).unwrap();
    }
    w.core().prune_storage(unix_ms() as i64);
    // The history shows the newest 500 (and the queued one is not among them: it is the oldest).
    let shown = db.history(500).unwrap();
    assert!(shown.iter().all(|r| r.text.is_some()), "every text the history shows has its body");
    // 600 done rows + 1 queued = 601 rows; the 101 oldest are beyond the 500. The 100 oldest done ones lost their bodies, the queued one kept it.
    assert_eq!(db.text_body("t000").unwrap(), None);
    assert_eq!(db.text_body("t099").unwrap(), None);
    assert_eq!(db.text_body("t100").unwrap(), Some(("text 100".to_string(), false)));
    assert_eq!(db.text_body("t599").unwrap(), Some(("text 599".to_string(), false)));
    assert_eq!(db.text_body("queued-old").unwrap(), Some(("still waiting".to_string(), false)), "a queued text keeps its body");
    // The rows stay: a transfer ID is never reused.
    assert!(db.transfer_exists("t000").unwrap() && db.transfer_exists("queued-old").unwrap());
    assert!(db.insert_text_item("t000", &device, "in", "done", 5, None, "reused", false, None).is_err(), "an old ID is still taken");
}

// ----- window security: commands, capabilities and the pages agree -----------------------------------------------------------------------

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The quoted names between `start` and the next `]`.
fn quoted_list(text: &str, start: &str) -> Vec<String> {
    let from = text.find(start).unwrap_or_else(|| panic!("{start} not found"));
    let list = &text[from..from + text[from..].find("];").or_else(|| text[from..].find(']')).unwrap()];
    list.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

#[test]
fn every_window_may_call_exactly_the_commands_its_page_calls_and_nothing_else() {
    let manifest: std::collections::BTreeSet<String> = quoted_list(&read("build.rs"), "const COMMANDS").into_iter().collect();
    // the commands registered with the app
    let app = read("src/app.rs");
    let handler = &app[app.find("generate_handler![").unwrap()..];
    let handler = &handler[..handler.find("])").unwrap()];
    let registered: std::collections::BTreeSet<String> =
        handler.lines().filter_map(|l| l.trim().strip_prefix("ui::")).map(|n| n.trim_end_matches(',').to_string()).collect();
    assert_eq!(manifest, registered, "build.rs (AppManifest) and generate_handler! in app.rs list the same commands");

    let mut allowed_anywhere = std::collections::BTreeSet::new();
    for window in ["pairing", "drop", "history", "settings", "debug"] {
        let cap: Value = serde_json::from_str(&read(&format!("capabilities/{window}.json"))).unwrap();
        assert_eq!(cap["windows"], json!([window]), "{window}: the capability is for that one window");
        let permissions: Vec<String> = cap["permissions"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()).collect();
        let mut commands = std::collections::BTreeSet::new();
        for p in &permissions {
            match p.as_str() {
                "core:event:allow-listen" | "core:event:allow-unlisten" => {}
                other => {
                    let name =
                        other.strip_prefix("allow-").unwrap_or_else(|| panic!("{window}: unexpected permission {other}")).replace('-', "_");
                    assert!(manifest.contains(&name), "{window}: {other} is not a command of the app");
                    commands.insert(name);
                }
            }
        }
        assert!(!permissions.iter().any(|p| p.starts_with("core:default") || p == "core:event:default"), "{window}: no core:default");
        // what the page calls
        let page = read(&format!("../src/{window}.ts"));
        let mut called = std::collections::BTreeSet::new();
        for part in page.split("\"").skip(1).step_by(2) {
            if manifest.contains(part) {
                called.insert(part.to_string());
            }
        }
        let missing: Vec<_> = called.difference(&commands).collect();
        assert!(missing.is_empty(), "{window}: its page calls {missing:?}, which its capability does not allow");
        let extra: Vec<_> = commands.difference(&called).filter(|c| c.as_str() != "close_window").collect();
        assert!(extra.is_empty(), "{window}: its capability allows {extra:?}, which its page never calls");
        allowed_anywhere.extend(commands);
    }
    assert_eq!(allowed_anywhere, manifest, "every command is allowed for some window, and no window has a command that isn't one");
    assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json").exists(), "the catch-all capability is gone");
}

#[test]
fn every_command_checks_the_label_of_the_window_that_called_it() {
    let ui = read("src/ui.rs");
    let mut commands = 0;
    for chunk in ui.split("#[tauri::command]").skip(1) {
        let signature = &chunk[..chunk.find('{').unwrap()];
        let name = signature.split("fn ").nth(1).unwrap().split('(').next().unwrap();
        let body_start = chunk.find('{').unwrap();
        let body: String = chunk[body_start..].lines().take(4).collect::<Vec<_>>().join("\n");
        assert!(signature.contains("window: WebviewWindow"), "{name} takes the calling window");
        assert!(body.contains("only(&window"), "{name} checks the label of the window first");
        commands += 1;
    }
    assert_eq!(commands, 26, "the 26 commands of the app");
}

#[test]
fn the_csp_forbids_base_uri_and_plugins() {
    let conf: Value = serde_json::from_str(&read("tauri.conf.json")).unwrap();
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    for part in ["default-src 'self'", "base-uri 'none'", "object-src 'none'", "connect-src ipc: http://ipc.localhost"] {
        assert!(csp.contains(part), "the CSP has {part}: {csp}");
    }
    assert!(!csp.contains("unsafe-eval") && !csp.contains("script-src *"));
}
