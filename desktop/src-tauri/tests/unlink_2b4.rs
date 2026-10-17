//! 2B.4: unlinking a phone from the PC must stop it at once (design 6.5, tests a to e). A removed device's WebSocket is closed, its running
//! uploads stop and are marked failed (`peer_removed`), the very next request on a TLS connection that was already open is `401`, a fresh
//! connection with the old key is `401`, and the removal is in the database file for anyone who opens it.
//!
//! These were written **before** `remove_device` was changed (S4a of the design): the first one failed on the old code, because nothing in
//! `remove_device` touched an HTTP upload that was in flight.
mod common;

use std::time::Duration;

use common::transfer::*;
use common::*;
use kin_lib::core::LinkState;
use kin_lib::storage::Db;
use serde_json::json;
use tokio::io::AsyncWriteExt;

const MIB: u64 = 1 << 20;

async fn remove(w: &World) -> bool {
    w.env.core.remove_device(&w.phone.device_id).await.unwrap()
}

fn listing(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

/// (a) An upload that is in flight when the phone is removed ends, and writes no more bytes.
#[tokio::test]
async fn an_upload_in_flight_stops_when_the_phone_is_removed_and_the_transfer_is_marked_peer_removed() {
    let w = World::new().await;
    w.pair().await;
    let pat = Pattern::new(7);
    let mut c = http(&w).await;
    let size = 16 * MIB;
    let r = c.post_transfer("t-flight", &[("f1", "big.bin", size)]).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    // The head and the first 4 MiB of the body (the writer reports a piece when the next one is handed to the disk).
    let head = format!(
        "PUT /v1/transfers/t-flight/files/f1?offset=0 HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {size}\r\n\r\n"
    );
    c.stream.write_all(head.as_bytes()).await.unwrap();
    c.stream.write_all(&pat.bytes(0, (4 * MIB) as usize)).await.unwrap();
    c.stream.flush().await.unwrap();
    let entry = w.core().transfers.get("t-flight").expect("the transfer is known");
    wait_until("bytes of the upload to be written", || entry.moved.load(std::sync::atomic::Ordering::Relaxed) > 0).await;

    assert!(remove(&w).await);

    // The writer stopped: nothing more is written however much the phone sends, and the partial file is gone with the transfer.
    let dir = w.core().received_dir();
    wait_until("the .part file to be dropped", || listing(&dir).is_empty()).await;
    let mut sent_more = 0u64;
    for _ in 0..4 {
        if c.stream.write_all(&pat.bytes(4 * MIB + sent_more, MIB as usize)).await.is_err() {
            break;
        }
        sent_more += MIB;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(listing(&dir).is_empty(), "no file comes back after the removal: {:?}", listing(&dir));
    assert!(w.core().transfers.get("t-flight").is_none(), "the transfer is forgotten");
    let (row, _) = w.core().db.get_transfer("t-flight").unwrap().expect("the record stays");
    assert_eq!((row.status.as_str(), row.error_code.as_deref()), ("failed", Some("peer_removed")));
}

/// (b) An already-open TLS connection of the removed device gets `401` on its next request: authorization is per request.
#[tokio::test]
async fn an_open_tls_connection_of_the_removed_phone_gets_401_on_its_next_request() {
    let w = World::new().await;
    w.pair().await;
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("t-open", &[("f1", "a.bin", 10)]).await.status, 200);
    assert!(remove(&w).await);
    let r = c.post_transfer("t-open-2", &[("f1", "b.bin", 10)]).await;
    assert_eq!((r.status, r.json()["error"].as_str().map(str::to_string)), (401, Some("unauthorized".into())));
    let r = c.simple("DELETE", "/v1/transfers/t-open").await;
    assert_eq!(r.status, 401, "every endpoint, not only the first one");
}

/// (c) The WebSocket is closed (an `unpair`, then 1000).
#[tokio::test]
async fn the_websocket_of_the_removed_phone_is_closed() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    assert!(remove(&w).await);
    let unpair = next_json(&mut ws).await;
    assert_eq!(unpair["type"], "unpair");
    send_json(&mut ws, &json!({ "type": "ack", "ref": unpair["id"] })).await;
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(1000));
    w.wait_for_state(LinkState::NotPaired).await;
}

/// (d) A fresh connection with the old key is `401`, for the WebSocket and for HTTP.
#[tokio::test]
async fn a_fresh_connection_with_the_old_key_is_401() {
    let w = World::new().await;
    w.pair().await;
    assert!(remove(&w).await);
    assert_eq!(w.try_open_ws().await.err(), Some(401));
    let mut fresh = http(&w).await;
    assert_eq!(fresh.post_transfer("t-new", &[("f1", "a.bin", 1)]).await.status, 401);
}

/// (e) The removal is in the database file: a second connection to the same file sees it at once, and so does a restart.
#[tokio::test]
async fn the_removal_is_in_the_file_for_a_second_connection_and_survives_reopening_it() {
    let w = World::new_on_disk().await;
    w.pair().await;
    let path = w.env.dir.path().join("kin.db");
    let other = Db::open(&path).unwrap();
    assert!(other.is_paired(&w.phone.device_id).unwrap());
    assert!(remove(&w).await);
    assert!(!other.is_paired(&w.phone.device_id).unwrap(), "a second connection to the file sees the removal at once");
    drop(other);
    let w = w.restart().await;
    assert!(!w.core().db.is_paired(&w.phone.device_id).unwrap(), "after a restart the phone is still not paired");
    assert_eq!(w.try_open_ws().await.err(), Some(401));
}

/// Review round 1, item 1: the upgrade checks the pairing, but the connection is only registered a little later. A phone unlinked in that gap has no connection to
/// tell, and used to get a live one whose messages were handled. Now it is closed (4003) the moment it is registered, and nothing of it is handled.
#[tokio::test]
async fn a_websocket_that_opens_while_the_phone_is_being_unlinked_is_closed_4003_and_handles_nothing() {
    let w = World::new().await;
    w.pair().await;
    w.core().hooks.ws_register_delay_ms.store(700, std::sync::atomic::Ordering::Relaxed);
    // The upgrade is answered (the pairing was checked), the registration waits: this is the gap.
    let mut ws = w.try_open_ws().await.expect("the upgrade is accepted: the phone is still paired");
    assert!(remove(&w).await);
    w.core().hooks.ws_register_delay_ms.store(0, std::sync::atomic::Ordering::Relaxed);
    // Whatever the phone sends in the meantime is not handled.
    let _ = futures_util::SinkExt::send(
        &mut ws,
        tokio_tungstenite::tungstenite::Message::Text(
            json!({ "type": "text", "id": "00000000-0000-7000-8000-000000000001", "text": "from a removed phone" }).to_string().into(),
        ),
    )
    .await;
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(4003), "closed as an unauthorized device");
    assert!(w.env.clipboard.app_writes().is_empty(), "a removed phone's text was not put on the clipboard");
    assert!(w.core().db.history_item("00000000-0000-7000-8000-000000000001").unwrap().is_none(), "nor kept");
    assert_eq!(w.try_open_ws().await.err(), Some(401));
}

/// Review round 1, item 2: the pairing is checked before the body of `POST /v1/transfers` is read, and the body can be slow. An unlink while it arrives must leave
/// no upload and no `.part` file behind.
#[tokio::test]
async fn a_post_whose_body_is_slow_leaves_no_upload_when_the_phone_is_unlinked_meanwhile() {
    let w = World::new().await;
    w.pair().await;
    let mut c = http(&w).await;
    let body = json!({ "id": "t-slow", "files": [{ "fileId": "f1", "name": "late.bin", "size": 10, "mime": "application/octet-stream" }] })
        .to_string();
    let (first, second) = body.as_bytes().split_at(body.len() / 2);
    let head = format!(
        "POST /v1/transfers HTTP/1.1\r\nHost: kin\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    c.stream.write_all(head.as_bytes()).await.unwrap();
    c.stream.write_all(first).await.unwrap();
    c.stream.flush().await.unwrap();
    // The server has the head (the pairing was checked) and is waiting for the rest.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(remove(&w).await);
    c.stream.write_all(second).await.unwrap();
    c.stream.flush().await.unwrap();
    let r = read_response(&mut c.stream).await.unwrap();
    assert_eq!((r.status, r.json()["error"].as_str().map(str::to_string)), (401, Some("unauthorized".into())));
    assert!(w.core().transfers.get("t-slow").is_none(), "no upload entry for a removed phone");
    assert!(w.core().transfers.all().is_empty());
    assert!(!w.core().db.transfer_exists("t-slow").unwrap(), "no record either");
    assert!(listing(&w.core().received_dir()).is_empty(), "no .part file: {:?}", listing(&w.core().received_dir()));
}

/// Review round 1, item 7: an offer that was sent and not acked is still `queued` in the database while its files are served. An unlink fails it like the rest of the
/// queue (`peer_changed`), not as `peer_removed`.
#[tokio::test]
async fn a_queued_offer_that_was_served_fails_as_peer_changed_when_the_phone_is_removed() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("offer.bin");
    std::fs::write(&path, vec![7u8; 64]).unwrap();
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_paths(vec![path], "test").await });
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "offer");
    let id = frame["id"].as_str().unwrap().to_string();
    // No ack: the connection is closed as dead, the offer stays queued, and its files stay served (the phone may have had the offer).
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    sending.await.unwrap();
    assert_eq!(w.core().db.status_of(&id).unwrap().map(|(s, _)| s).as_deref(), Some("queued"));
    assert!(w.core().transfers.get(&id).is_some(), "the files are served");

    assert!(remove(&w).await);
    let (row, _) = w.core().db.get_transfer(&id).unwrap().expect("the record stays");
    assert_eq!((row.status.as_str(), row.error_code.as_deref()), ("failed", Some("peer_changed")));
    assert!(w.core().transfers.get(&id).is_none(), "not served any more");
    assert_eq!(w.core().queue_len(), 0);
}

/// Queued sends for a removed phone are not kept for a phone that was never meant to get them: they fail (`peer_changed`).
#[tokio::test]
async fn queued_sends_of_the_removed_phone_fail_as_peer_changed() {
    let w = World::new().await;
    w.pair().await;
    // No connection: a text waits in the queue.
    let result = w.env.core.queue_text("q-1", "hello later", false, kin_lib::sync::Tap::now()).await;
    assert_eq!(result, kin_lib::sync::SendResult::PhoneOffline);
    assert_eq!(w.core().queue_len(), 1);
    assert!(remove(&w).await);
    assert_eq!(w.core().queue_len(), 0, "nothing waits for a phone that is gone");
    let (row, _) = w.core().db.get_transfer("q-1").unwrap().expect("the record stays");
    assert_eq!((row.status.as_str(), row.error_code.as_deref()), ("failed", Some("peer_changed")));
}
