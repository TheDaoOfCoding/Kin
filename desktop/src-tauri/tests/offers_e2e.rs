//! PC to phone, end to end: the PC offers files with an `offer` message over the WebSocket, the fake phone acks it and downloads with
//! `GET /v1/transfers/{id}/files/{fileId}` (with `Range` to resume), over real TLS on loopback. Covered: single, several and 0-byte
//! files, a resume after a dropped connection, `416`, cancel from each side (also while a download is under way), `source_changed`,
//! an expired offer, a phone that can't take the files (`disk_full`), folders, a phone that isn't there, a text over 128 KiB that goes
//! as a `.txt`, and the speed of a 2 GB download (`#[ignore]`d).
//!
//! The files the PC offers are made by the tests in a temporary folder. **Nothing here touches the real clipboard.**
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::transfer::*;
use common::*;
use kin_lib::core::{Core, Event};
use kin_lib::sync::SendResult;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

const MIB: u64 = 1 << 20;

async fn connected(w: &World) -> Ws {
    w.pair().await;
    w.open_ws().await
}

fn drain_events(w: &mut World) -> Vec<Event> {
    std::iter::from_fn(|| w.env.events.try_recv().ok()).collect()
}

fn toast_texts(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Toast(t) => Some(t.clone()),
            Event::ToastRedacted { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Makes files in a folder: `(name, pattern, size)`.
fn make_files(dir: &Path, files: &[(&str, &Pattern, u64)]) -> Vec<PathBuf> {
    files
        .iter()
        .map(|(name, pat, size)| {
            let path = dir.join(name);
            pat.write_file(&path, *size);
            path
        })
        .collect()
}

/// What the phone sees of an offer: the PC sends it, the phone acks it (or does something else with `reply`), and the PC's
/// `send_paths` ends. Returns the offer message and the report.
async fn offer(
    core: &Arc<Core>,
    ws: &mut Ws,
    paths: Vec<PathBuf>,
    reply: impl FnOnce(&Value) -> Value,
) -> (Value, kin_lib::outgoing::SendReport) {
    let core = core.clone();
    let task = tokio::spawn(async move { core.send_paths(paths, "test").await });
    let frame = next_json(ws).await;
    assert_eq!(frame["type"], "offer", "{frame}");
    send_json(ws, &reply(&frame)).await;
    (frame, task.await.unwrap())
}

fn ack(frame: &Value) -> Value {
    json!({ "type": "ack", "ref": frame["id"] })
}

#[tokio::test]
async fn files_are_offered_and_downloaded_and_the_phone_says_it_is_done() {
    let mut w = World::new().await;
    let mut ws = connected(&w).await;
    drain_events(&mut w);
    let src = tempfile::tempdir().unwrap();
    let (p1, p2) = (Pattern::new(31), Pattern::new(32));
    let paths = make_files(src.path(), &[("photo.jpg", &p1, 3_000_000), ("empty.txt", &p1, 0), ("b.bin", &p2, 5)]);
    let (frame, report) = offer(&w.env.core, &mut ws, paths, ack).await;
    assert!(matches!(report.result, Some(SendResult::Acked { .. })), "{report:?}");
    assert_eq!(frame["id"].as_str().unwrap().len(), 36, "a UUIDv7");
    assert_eq!(frame["totalSize"], 3_000_005);
    assert_eq!(
        frame["files"],
        json!([
            { "fileId": "f1", "name": "photo.jpg", "size": 3_000_000, "mime": "image/jpeg" },
            { "fileId": "f2", "name": "empty.txt", "size": 0, "mime": "text/plain" },
            { "fileId": "f3", "name": "b.bin", "size": 5, "mime": "application/octet-stream" },
        ])
    );
    let tid = frame["id"].as_str().unwrap().to_string();
    let status = w.core().status_json();
    assert_eq!(
        (status["transfers"]["active"][0]["direction"].as_str(), status["transfers"]["active"][0]["files"].as_u64()),
        (Some("out"), Some(3))
    );

    let mut c = http(&w).await;
    let d = c.get(&tid, "f1", None, Some(&p1), None).await;
    assert_eq!((d.status, d.received, d.intact), (200, 3_000_000, true));
    assert_eq!(d.header("content-length"), Some("3000000"));
    assert_eq!(d.header("content-type"), Some("application/octet-stream"));
    assert_eq!(d.header("accept-ranges"), Some("bytes"));
    let d = c.get(&tid, "f2", None, None, None).await;
    assert_eq!((d.status, d.received, d.header("content-length")), (200, 0, Some("0")), "a 0-byte file: a plain GET");
    let d = c.get(&tid, "f3", None, Some(&p2), None).await;
    assert_eq!((d.status, d.received, d.intact), (200, 5, true));

    // The phone has saved everything and says so.
    send_json(&mut ws, &json!({ "type": "complete", "id": "m-complete-1", "transferId": tid })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-complete-1" }));
    let core = w.env.core.clone();
    wait_until("the offer to be done", || core.transfers.get(&tid).is_some_and(|e| e.status() == kin_lib::transfers::Status::Done)).await;
    assert!(toast_texts(&drain_events(&mut w)).iter().any(|t| t == "Sent 3 file(s) to Test Phone"));
    assert_eq!(w.core().db.get_transfer(&tid).unwrap().unwrap().0.status, "done");
    // A finished offer is not served any more.
    assert_eq!(c.get(&tid, "f1", None, None, None).await.status, 404);
}

#[tokio::test]
async fn a_download_that_breaks_resumes_with_a_range_request() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(33);
    let size = 20 * MIB + 17;
    let paths = make_files(src.path(), &[("movie.mp4", &pat, size)]);
    let (frame, _) = offer(&w.env.core, &mut ws, paths, ack).await;
    let tid = frame["id"].as_str().unwrap().to_string();
    assert_eq!(frame["files"][0]["mime"], "video/mp4");

    let mut c = http(&w).await;
    let first = c.get(&tid, "f1", None, Some(&pat), Some(7_123_456)).await;
    assert!(first.received >= 7_123_456 && first.intact);
    drop(c);
    let have = 7_123_456u64; // the phone keeps what it wrote
    let mut c = http(&w).await;
    let d = c.get(&tid, "f1", Some(have), Some(&pat), None).await;
    assert_eq!(d.status, 206);
    assert_eq!(d.header("content-range").map(str::to_string), Some(format!("bytes {have}-{}/{size}", size - 1)));
    assert_eq!((d.received, d.intact), (size - have, true));
    // The whole file from 0 as a ranged request is a 206 too; the last byte; and asking for what it already has is a 416.
    let d = c.get(&tid, "f1", Some(0), Some(&pat), Some(10)).await;
    assert_eq!(d.status, 206);
    let mut c = http(&w).await;
    let d = c.get(&tid, "f1", Some(size - 1), Some(&pat), None).await;
    assert_eq!(
        (d.status, d.received, d.header("content-range").map(str::to_string)),
        (206, 1, Some(format!("bytes {0}-{0}/{size}", size - 1)))
    );
    for from in [size, size + 100] {
        let d = c.get(&tid, "f1", Some(from), None, None).await;
        assert_eq!((d.status, d.header("content-range").map(str::to_string)), (416, Some(format!("bytes */{size}"))), "from {from}");
        c = http(&w).await;
    }
}

#[tokio::test]
async fn range_headers_other_than_one_open_ended_range_are_416() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(34);
    let paths = make_files(src.path(), &[("a.bin", &pat, 100_000), ("empty.bin", &pat, 0)]);
    let (frame, _) = offer(&w.env.core, &mut ws, paths, ack).await;
    let tid = frame["id"].as_str().unwrap().to_string();
    for bad in ["bytes=+5-", "bytes=007-", "bytes=-5", "bytes=0-9", "bytes=abc-", "bytes=9007199254740992-", "items=0-", "bytes=0-,5-"] {
        let mut tls = w.tls().await;
        let request = format!("GET /v1/transfers/{tid}/files/f1 HTTP/1.1\r\nHost: kin\r\nRange: {bad}\r\nConnection: close\r\n\r\n");
        tokio::io::AsyncWriteExt::write_all(&mut tls, request.as_bytes()).await.unwrap();
        let r = read_response(&mut tls).await.unwrap();
        assert_eq!(r.status, 416, "{bad}");
        assert_eq!(r.header("content-range"), Some("bytes */100000"), "{bad}");
        assert_eq!(r.json()["error"], "bad_request");
    }
    // A Range on a 0-byte file can never be satisfied.
    let mut c = http(&w).await;
    let d = c.get(&tid, "f2", Some(0), None, None).await;
    assert_eq!((d.status, d.header("content-range").map(str::to_string)), (416, Some("bytes */0".to_string())));
}

#[tokio::test]
async fn the_phone_canceling_stops_the_offer() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(35);
    let paths = make_files(src.path(), &[("a.bin", &pat, 10_000)]);
    let (frame, _) = offer(&w.env.core, &mut ws, paths, ack).await;
    let tid = frame["id"].as_str().unwrap().to_string();
    send_json(&mut ws, &json!({ "type": "cancel", "id": "m-cancel", "transferId": tid, "reason": "user" })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-cancel" }));
    let core = w.env.core.clone();
    wait_until("the offer to be dropped", || core.transfers.get(&tid).is_none()).await;
    let mut c = http(&w).await;
    assert_eq!(c.get(&tid, "f1", None, None, None).await.status, 404);
    assert_eq!(w.core().db.get_transfer(&tid).unwrap().unwrap().0.status, "canceled");
}

#[tokio::test]
async fn the_user_canceling_on_the_pc_stops_a_download_under_way_and_tells_the_phone() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(36);
    let size = 300 * MIB;
    let paths = make_files(src.path(), &[("big.bin", &pat, size)]);
    let (frame, _) = offer(&w.env.core, &mut ws, paths, ack).await;
    let tid = frame["id"].as_str().unwrap().to_string();
    let mut c = http(&w).await;
    // The phone reads 8 MiB and then stops reading (the screen went off), so the PC is mid-file when the user cancels.
    let d = c.get(&tid, "f1", None, Some(&pat), Some(8 * MIB)).await;
    assert!(d.received >= 8 * MIB && d.received < size);
    assert!(w.env.core.cancel_transfer(&tid).await);
    let msg = next_json(&mut ws).await;
    assert_eq!(
        (msg["type"].as_str(), msg["transferId"].as_str(), msg["reason"].as_str()),
        (Some("cancel"), Some(tid.as_str()), Some("user"))
    );
    send_json(&mut ws, &ack(&msg)).await;
    // What is left on the connection is only what was already on its way: the stream is cut.
    let mut rest = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    while let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(5), c.stream.read(&mut buf)).await {
        if n == 0 {
            break;
        }
        rest += n as u64;
    }
    assert!(d.received + rest < size - 100 * MIB, "the download went on after the cancel ({} of {size})", d.received + rest);
    assert_eq!(http(&w).await.get(&tid, "f1", None, None, None).await.status, 404);
}

#[tokio::test]
async fn a_file_that_changed_after_the_offer_is_409_source_changed() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(37);
    let paths = make_files(src.path(), &[("a.bin", &pat, 50_000), ("same.bin", &pat, 50_000), ("grown.bin", &pat, 50_000)]);
    let (frame, _) = offer(&w.env.core, &mut ws, paths.clone(), ack).await;
    let tid = frame["id"].as_str().unwrap().to_string();
    // a.bin: its modified time moves. grown.bin: it gets bigger. same.bin: untouched.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&paths[0])
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    std::fs::OpenOptions::new().write(true).open(&paths[2]).unwrap().set_len(50_001).unwrap();
    let mut c = http(&w).await;
    assert_eq!(c.get(&tid, "f2", None, Some(&pat), None).await.status, 200);
    for (file, name) in [("f1", "a.bin"), ("f3", "grown.bin")] {
        let r = c.get(&tid, file, None, None, None).await;
        assert_eq!(r.status, 409, "{name}");
        c = http(&w).await;
    }
    // The file is gone altogether: the same answer.
    std::fs::remove_file(&paths[1]).unwrap();
    assert_eq!(c.get(&tid, "f2", None, None, None).await.status, 409);
    // The phone gives up on the whole offer, as the spec says, with reason `error`.
    send_json(&mut ws, &json!({ "type": "cancel", "id": "m-err", "transferId": tid, "reason": "error" })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-err" }));
}

#[tokio::test]
async fn an_offer_expires_after_a_day_and_the_phone_is_told() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(38);
    let paths = make_files(src.path(), &[("a.bin", &pat, 1000)]);
    let phone = w.phone.device_id.clone();
    let entry = w.core().seed_offer(&phone, "stale", &[("f1", paths[0].as_path())], true);
    assert!(entry.expires_ms() <= kin_lib::encoding::unix_ms());
    assert_eq!(http(&w).await.get("stale", "f1", None, None, None).await.status, 404, "an expired offer is not served");
    // The clean-up drops it and tells the phone with a cancel message (reason expired).
    let core = w.env.core.clone();
    let cleaning = tokio::spawn(async move { core.clean_up().await });
    let msg = next_json(&mut ws).await;
    assert_eq!(
        (msg["type"].as_str(), msg["transferId"].as_str(), msg["reason"].as_str()),
        (Some("cancel"), Some("stale"), Some("expired"))
    );
    send_json(&mut ws, &ack(&msg)).await;
    cleaning.await.unwrap();
    assert!(w.core().transfers.get("stale").is_none());
    assert_eq!(w.core().db.get_transfer("stale").unwrap().unwrap().0.status, "failed");
}

#[tokio::test]
async fn a_phone_that_cannot_take_the_files_says_so_with_an_error_and_the_offer_is_dropped() {
    let mut w = World::new().await;
    let mut ws = connected(&w).await;
    drain_events(&mut w);
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(39);
    let paths = make_files(src.path(), &[("a.bin", &pat, 1000)]);
    let (frame, report) = offer(
        &w.env.core,
        &mut ws,
        paths,
        |f| json!({ "type": "error", "ref": f["id"], "code": "disk_full", "message": "The phone is full." }),
    )
    .await;
    assert_eq!(report.result, Some(SendResult::Rejected("disk_full".to_string())));
    let tid = frame["id"].as_str().unwrap();
    assert!(w.core().transfers.get(tid).is_none(), "nothing is served for an offer the phone refused");
    assert!(toast_texts(&drain_events(&mut w)).iter().any(|t| t.contains("enough free space")), "the user is told");
    assert_eq!(w.core().db.get_transfer(tid).unwrap().unwrap().0.status, "failed");
}

#[tokio::test]
async fn folders_are_skipped_with_a_message_and_the_files_are_sent() {
    let mut w = World::new().await;
    let mut ws = connected(&w).await;
    drain_events(&mut w);
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(40);
    let mut paths = make_files(src.path(), &[("a.bin", &pat, 100)]);
    std::fs::create_dir(src.path().join("album")).unwrap();
    paths.push(src.path().join("album"));
    paths.push(src.path().join("missing.bin"));
    let (frame, report) = offer(&w.env.core, &mut ws, paths, ack).await;
    assert_eq!(frame["files"].as_array().unwrap().len(), 1);
    assert_eq!((report.files, report.skipped_folders, report.unreadable.len()), (1, 1, 1));
    let toasts = toast_texts(&drain_events(&mut w));
    assert!(toasts.iter().any(|t| t.contains("folder") && t.contains("can't be sent yet")), "{toasts:?}");
    assert!(toasts.iter().any(|t| t.contains("missing.bin")), "{toasts:?}");
    // Only a folder: nothing is offered at all.
    let report = w.env.core.send_paths(vec![src.path().join("album")], "test").await;
    assert_eq!((report.files, report.transfers.len()), (0, 0));
}

#[tokio::test]
async fn nothing_is_sent_to_a_phone_that_is_not_connected_or_not_paired() {
    let mut w = World::new().await;
    let src = tempfile::tempdir().unwrap();
    let paths = make_files(src.path(), &[("a.bin", &Pattern::new(41), 100)]);
    drain_events(&mut w);
    let report = w.env.core.send_paths(paths.clone(), "test").await;
    assert_eq!(report.result, Some(SendResult::NoPhonePaired));
    assert!(toast_texts(&drain_events(&mut w)).iter().any(|t| t.contains("No phone is paired")));
    w.pair().await;
    let report = w.env.core.send_paths(paths, "test").await;
    assert_eq!(report.result, Some(SendResult::PhoneOffline));
    assert!(toast_texts(&drain_events(&mut w)).iter().any(|t| t.contains("isn't connected")));
    assert!(w.core().transfers.all().is_empty(), "no offer was registered");
}

#[tokio::test]
async fn more_files_than_fit_in_one_offer_go_in_several() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let per_offer = kin_lib::outgoing::FILES_PER_OFFER;
    let paths: Vec<PathBuf> = (0..per_offer + 1)
        .map(|i| {
            let p = src.path().join(format!("f{i:04}.txt"));
            std::fs::write(&p, format!("{i}")).unwrap();
            p
        })
        .collect();
    let core = w.env.core.clone();
    let task = tokio::spawn(async move { core.send_paths(paths, "test").await });
    let mut sizes = Vec::new();
    for _ in 0..2 {
        let frame = next_json(&mut ws).await;
        assert_eq!(frame["type"], "offer");
        sizes.push(frame["files"].as_array().unwrap().len());
        assert!(frame.to_string().len() < 1_048_576);
        send_json(&mut ws, &ack(&frame)).await;
    }
    let report = task.await.unwrap();
    assert_eq!(sizes, [per_offer, 1]);
    assert_eq!((report.files, report.transfers.len()), (per_offer + 1, 2));
}

// ----- a text over 128 KiB -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_text_over_128_kib_from_the_hotkey_goes_as_a_txt_file_with_lf_line_ends() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let line = "line of text with a \u{e9} and an emoji \u{1F600}\r\n";
    let text = line.repeat(5000); // about 200 KiB with CRLF
    assert!(text.len() > 128 * 1024);
    w.env.clipboard.user_copy(&text, vec![]);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "offer", "a text that long is not a text message");
    assert_eq!(frame["files"][0]["name"], "Text.txt");
    assert_eq!(frame["files"][0]["mime"], "text/plain");
    let wire = text.replace("\r\n", "\n");
    assert_eq!(frame["files"][0]["size"], wire.len());
    send_json(&mut ws, &ack(&frame)).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
    let tid = frame["id"].as_str().unwrap().to_string();
    // The phone downloads it: the bytes are the text with LF line ends, UTF-8, no byte order mark.
    let mut tls = w.tls().await;
    let request = format!("GET /v1/transfers/{tid}/files/f1 HTTP/1.1\r\nHost: kin\r\nConnection: close\r\n\r\n");
    tokio::io::AsyncWriteExt::write_all(&mut tls, request.as_bytes()).await.unwrap();
    let r = read_response(&mut tls).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body, wire.as_bytes());
    // The temporary file is gone when the phone says it is done.
    let outgoing = w.core().config.data_dir.join("outgoing");
    assert!(std::fs::read_dir(&outgoing).is_ok_and(|mut d| d.next().is_some()), "the text was written to a temporary file");
    send_json(&mut ws, &json!({ "type": "complete", "id": "m-c", "transferId": tid })).await;
    assert_eq!(next_json(&mut ws).await["type"], "ack");
    wait_until("the temporary file to go", || !std::fs::read_dir(&outgoing).is_ok_and(|mut d| d.next().is_some())).await;
}

#[tokio::test]
async fn a_text_up_to_128_kib_is_a_text_message_and_a_sensitive_one_over_it_is_not_sent_at_all() {
    let mut w = World::new().await;
    let mut ws = connected(&w).await;
    drain_events(&mut w);
    // Exactly 128 KiB of UTF-8 on the wire (with LF): a message, with the CRLF of the clipboard turned into LF.
    let exact = "x".repeat(128 * 1024 - 1) + "\n";
    w.env.clipboard.user_copy(&exact.replace('\n', "\r\n"), vec![]);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "text");
    assert_eq!(frame["text"].as_str().unwrap(), exact, "LF on the wire");
    assert!(frame.get("sensitive").is_none(), "an ordinary text carries no flag");
    send_json(&mut ws, &ack(&frame)).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
    // A sensitive text over the limit: not sent, and the user is told.
    let long = "y".repeat(128 * 1024 + 1);
    w.env.clipboard.user_copy(&long, vec![kin_lib::clipboard::ClipFormat::named(kin_lib::clipboard::EXCLUDE_FROM_MONITOR)]);
    drain_events(&mut w);
    let result = w.env.core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await;
    assert!(matches!(result, SendResult::TooLarge { .. }), "{result:?}");
    assert!(toast_texts(&drain_events(&mut w)).iter().any(|t| t.contains("sensitive")));
    assert!(w.core().transfers.all().is_empty(), "no file was made of a secret");
    assert!(
        !w.core().config.data_dir.join("outgoing").exists()
            || std::fs::read_dir(w.core().config.data_dir.join("outgoing")).unwrap().next().is_none()
    );
}

// ----- 2 GB ----------------------------------------------------------------------------------------------------------------

/// Run with `cargo test --release large_download -- --ignored --nocapture`. The source file is made in a temporary folder and deleted
/// with it.
#[tokio::test]
#[ignore = "moves 2 GB: run with --ignored --nocapture in release mode"]
async fn large_download_of_2_gb_and_a_resume_in_the_middle() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let pat = Pattern::new(50);
    let size = 2 * 1024 * MIB;
    let made = std::time::Instant::now();
    let paths = make_files(src.path(), &[("huge.bin", &pat, size)]);
    println!("made the 2 GB source in {:.1} s", made.elapsed().as_secs_f64());
    let (frame, _) = offer(&w.env.core, &mut ws, paths, ack).await;
    let tid = frame["id"].as_str().unwrap().to_string();
    let mut c = http(&w).await;
    let d = c.get(&tid, "f1", None, Some(&pat), None).await;
    assert_eq!((d.status, d.received, d.intact), (200, size, true));
    println!(
        "THROUGHPUT download 2 GB (PC -> phone, loopback TLS, read from disk): {:.0} MB/s in {:.1} s",
        mb_per_s(size, d.elapsed),
        d.elapsed.as_secs_f64()
    );
    // A break in the middle and a resume.
    drop(c);
    let mut c = http(&w).await;
    let first = c.get(&tid, "f1", None, Some(&pat), Some(1_000 * MIB + 4_321)).await;
    assert!(first.intact);
    drop(c);
    let have = 1_000 * MIB + 4_321;
    let mut c = http(&w).await;
    let d = c.get(&tid, "f1", Some(have), Some(&pat), None).await;
    assert_eq!((d.status, d.received, d.intact), (206, size - have, true));
    println!("THROUGHPUT resumed download ({} bytes): {:.0} MB/s", size - have, mb_per_s(size - have, d.elapsed));
}
