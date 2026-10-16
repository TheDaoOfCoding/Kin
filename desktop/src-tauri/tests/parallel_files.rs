//! v0.1.1: several files of one transfer at the same time, in both directions. The 0.1.1 phone app runs up to 4 file requests of one transfer
//! at once (and sends small files back to back on one connection); protocol.md, "Concurrent file requests", says the PC must accept that. Covered:
//!
//! * phone to PC: 4 `PUT`s of one transfer at once, one file interrupted and resumed (with a `POST`) while the other three keep going,
//!   a cancel in the middle of a batch (`DELETE`, and the user's Cancel on the PC), small files pipelined on one connection
//! * PC to phone: 4 `GET`s of one offer at once, one resumed with `Range` while the others carry on, a cancel in the middle, small files
//!   pipelined on one connection
//! * what the progress view says with several files going (`filesActive`)
//!
//! Real TLS on loopback, a fake phone, a throwaway data directory: **nothing here touches the real Downloads folder or clipboard.**
//! The speed test at the end is `#[ignore]`d (`cargo test --release --test parallel_files -- --ignored --nocapture speed`).
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::transfer::*;
use common::*;
use kin_lib::core::Event;
use kin_lib::transfers::Status;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Barrier;

const MIB: u64 = 1 << 20;

async fn connected(w: &World) -> Ws {
    // The files these tests pause in the middle of a body are live, not dead connections: a POST must never take them over, however
    // slow the machine running the test is (the default is one second of silence).
    w.core().hooks.takeover_idle_ms.store(600_000, std::sync::atomic::Ordering::Relaxed);
    w.pair().await;
    w.open_ws().await
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

fn offsets(r: &HttpResponse) -> Vec<(String, u64)> {
    r.json()["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["fileId"].as_str().unwrap().to_string(), f["offset"].as_u64().unwrap()))
        .collect()
}

fn drain_events(w: &mut World) -> Vec<Event> {
    std::iter::from_fn(|| w.env.events.try_recv().ok()).collect()
}

fn files_toasts(events: &[Event]) -> usize {
    events.iter().filter(|e| matches!(e, Event::FilesToast { .. })).count()
}

/// How many files of transfer `id` the progress view says are moving right now.
fn files_active(w: &World, id: &str) -> u64 {
    let status = w.core().status_json();
    status["transfers"]["active"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .map_or(0, |t| t["filesActive"].as_u64().expect("the view has filesActive"))
}

async fn wait_active(w: &World, id: &str, n: u64) {
    for _ in 0..400 {
        if files_active(w, id) == n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {n} active files of {id}; the view says {}", files_active(w, id));
}

// ----- phone to PC: the fake phone's halves of a PUT ----------------------------------------------------------------------

/// Writes `len` bytes of `pat` from `from` to the connection.
async fn send_pattern(c: &mut Http, pat: &Pattern, from: u64, len: u64) {
    let mut buf = vec![0u8; MIB as usize];
    let mut sent = 0u64;
    while sent < len {
        let n = (len - sent).min(MIB) as usize;
        pat.fill(from + sent, &mut buf[..n]);
        c.stream.write_all(&buf[..n]).await.unwrap();
        sent += n as u64;
    }
    c.stream.flush().await.unwrap();
}

/// The head of a `PUT` of a file of `size` bytes from `offset`, and the first `first` bytes of the body: the rest is left to `put_end`.
async fn put_begin(c: &mut Http, transfer: &str, file: &str, offset: u64, size: u64, pat: &Pattern, first: u64) {
    let head = format!(
        "PUT /v1/transfers/{transfer}/files/{file}?offset={offset} HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
        size - offset
    );
    c.stream.write_all(head.as_bytes()).await.unwrap();
    send_pattern(c, pat, offset, first.min(size - offset)).await;
}

/// The rest of the body (from `from` to `size`), then the answer.
async fn put_end(c: &mut Http, pat: &Pattern, from: u64, size: u64) -> HttpResponse {
    send_pattern(c, pat, from, size - from).await;
    read_response(&mut c.stream).await.expect("an answer")
}

/// A batch of `n` files of 8 MiB (`f0` .. `f{n-1}`, `a0.bin` ..) as transfer `id`.
async fn post_batch(c: &mut Http, id: &str, n: usize, size: u64) {
    let names: Vec<(String, String)> = (0..n).map(|i| (format!("f{i}"), format!("a{i}.bin"))).collect();
    let files: Vec<(&str, &str, u64)> = names.iter().map(|(f, a)| (f.as_str(), a.as_str(), size)).collect();
    assert_eq!(c.post_transfer(id, &files).await.status, 200);
}

/// Starts `files` of transfer `id` on connections of their own, each stopping `first` bytes into its body, and leaves them there (the
/// answer is read by `finish`). Waits until the progress view says they are all going.
struct Batch {
    go: Arc<Barrier>,
    tasks: Vec<tokio::task::JoinHandle<HttpResponse>>,
}

impl Batch {
    /// `finish`: send the rest of every body and read the answers; otherwise just read the answers (the PC answers early after a cancel).
    async fn start(w: &World, id: &str, files: &[usize], size: u64, first: u64, finish: bool) -> Batch {
        let ready = Arc::new(Barrier::new(files.len() + 1));
        let go = Arc::new(Barrier::new(files.len() + 1));
        let mut tasks = Vec::new();
        for &i in files {
            let (mut c, id, ready, go) = (http(w).await, id.to_string(), ready.clone(), go.clone());
            tasks.push(tokio::spawn(async move {
                let pat = Pattern::new(500 + i as u64);
                put_begin(&mut c, &id, &format!("f{i}"), 0, size, &pat, first).await;
                ready.wait().await;
                if finish {
                    go.wait().await;
                    put_end(&mut c, &pat, first, size).await
                } else {
                    read_response(&mut c.stream).await.expect("an answer")
                }
            }));
        }
        ready.wait().await;
        wait_active(w, id, files.len() as u64).await;
        Batch { go, tasks }
    }

    async fn finish(self) -> Vec<HttpResponse> {
        self.go.wait().await;
        self.answers().await
    }

    async fn answers(self) -> Vec<HttpResponse> {
        let mut out = Vec::new();
        for t in self.tasks {
            out.push(t.await.unwrap());
        }
        out
    }
}

#[tokio::test]
async fn four_files_of_one_transfer_upload_at_once_and_all_arrive_intact() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let sizes = [5 * MIB + 1, 7 * MIB + 3, 3 * MIB, 9 * MIB + 5, 0, 4 * MIB + 9];
    let mut c = http(&w).await;
    let names: Vec<String> = (0..6).map(|i| format!("a{i}.bin")).collect();
    let files: Vec<(String, &str, u64)> = (0..6).map(|i| (format!("f{i}"), names[i].as_str(), sizes[i])).collect();
    let refs: Vec<(&str, &str, u64)> = files.iter().map(|(f, n, s)| (f.as_str(), *n, *s)).collect();
    assert_eq!(c.post_transfer("p1", &refs).await.status, 200);

    // Four files, four connections, every one of them half way through its body when the view is read.
    let ready = Arc::new(Barrier::new(5));
    let go = Arc::new(Barrier::new(5));
    let mut tasks = Vec::new();
    for (i, &size) in sizes.iter().enumerate().take(4) {
        let (mut conn, ready, go) = (http(&w).await, ready.clone(), go.clone());
        tasks.push(tokio::spawn(async move {
            let pat = Pattern::new(500 + i as u64);
            put_begin(&mut conn, "p1", &format!("f{i}"), 0, size, &pat, size / 2).await;
            ready.wait().await;
            go.wait().await;
            put_end(&mut conn, &pat, size / 2, size).await
        }));
    }
    ready.wait().await;
    wait_active(&w, "p1", 4).await;
    let view = w.core().status_json()["transfers"]["active"][0].clone();
    assert_eq!((view["files"].as_u64(), view["filesActive"].as_u64(), view["filesDone"].as_u64()), (Some(6), Some(4), Some(0)));
    go.wait().await;
    for (i, t) in tasks.into_iter().enumerate() {
        let r = t.await.unwrap();
        assert_eq!((r.status, r.json()), (200, json!({ "received": sizes[i] })), "file {i}: {:?}", r.json());
    }
    assert_eq!(files_active(&w, "p1"), 0, "nothing is moving between the two waves");
    assert_eq!(w.core().status_json()["transfers"]["active"][0]["filesDone"], 4);

    // The other two, on one connection, one after the other (a 0-byte file and a small one).
    let (r, _) = c.put("p1", "f4", 0, 0, &Pattern::new(504), None).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(w.core().status_json()["transfers"]["active"][0]["id"], "p1", "not done before the last file");
    let (r, _) = c.put("p1", "f5", 0, sizes[5], &Pattern::new(505), None).await.unwrap();
    assert_eq!(r.status, 200);

    let dir = w.core().received_dir();
    assert_eq!(listing(&dir), names);
    for (i, name) in names.iter().enumerate() {
        Pattern::new(500 + i as u64).check_file(&dir.join(name), sizes[i]).unwrap();
    }
    let core = w.env.core.clone();
    wait_until("the transfer to be done", || core.db.get_transfer("p1").unwrap().unwrap().0.status == "done").await;
    // Done exactly once, whatever the order the last two files finished in: one toast, one record of the send.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(files_toasts(&drain_events(&mut w)), 1);
    let (row, rows) = w.core().db.get_transfer("p1").unwrap().unwrap();
    assert_eq!(row.status, "done");
    assert!(rows.iter().all(|f| f.finalized && f.bytes_done == f.size), "{rows:?}");
}

#[tokio::test]
async fn one_file_is_interrupted_and_resumed_while_the_other_three_keep_going() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let size = 8 * MIB;
    let mut c = http(&w).await;
    post_batch(&mut c, "r4", 4, size).await;
    let dir = w.core().received_dir();

    // f1..f3 are 2 MiB into their bodies and stay there.
    let others = Batch::start(&w, "r4", &[1, 2, 3], size, 2 * MIB, true).await;
    // f0's connection dies 3,000,000 bytes in.
    let mut broken = http(&w).await;
    assert!(broken.put("r4", "f0", 0, size, &Pattern::new(500), Some(3_000_000)).await.is_none());
    drop(broken);
    wait_active(&w, "r4", 3).await;

    // A new connection: `POST` again. This is the call that used to stop the writer of every file of the transfer.
    let r = c.post_transfer("r4", &[("f0", "x", 1), ("f1", "x", 1), ("f2", "x", 1), ("f3", "x", 1)]).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    let got = offsets(&r);
    assert_eq!(got[0], ("f0".to_string(), 3_000_000), "the file that broke: every byte that reached the PC was written");
    for (id, off) in &got[1..] {
        assert!(*off <= 2 * MIB, "{id} is reported at what the PC has written so far, {off}");
    }
    assert_eq!(files_active(&w, "r4"), 3, "the three files that are moving were not stopped by the POST");

    // f0 continues from 3,000,000 on the new connection while the others are still in the middle of their bodies.
    let (r, _) = c.put("r4", "f0", 3_000_000, size, &Pattern::new(500), None).await.unwrap();
    assert_eq!((r.status, r.json()), (200, json!({ "received": size })), "{:?}", r.json());
    assert_eq!(w.core().status_json()["transfers"]["active"][0]["filesDone"], 1);

    // And the three finish, none of them told `offset_mismatch`.
    for (i, r) in others.finish().await.into_iter().enumerate() {
        assert_eq!((r.status, r.json()), (200, json!({ "received": size })), "file f{}: {:?}", i + 1, r.json());
    }
    for i in 0..4u64 {
        Pattern::new(500 + i).check_file(&dir.join(format!("a{i}.bin")), size).unwrap();
    }
    let core = w.env.core.clone();
    wait_until("the transfer to be done", || core.db.get_transfer("r4").unwrap().unwrap().0.status == "done").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(files_toasts(&drain_events(&mut w)), 1);
}

#[tokio::test]
async fn a_post_does_not_stop_a_file_that_is_still_receiving_but_a_put_for_it_takes_over() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let size = 8 * MIB;
    let mut c = http(&w).await;
    post_batch(&mut c, "k2", 2, size).await;
    let dir = w.core().received_dir();
    // f0 is being written (2 MiB in, the connection is open and quiet for less than a second): a POST must leave it alone.
    let mut live = http(&w).await;
    let pat = Pattern::new(500);
    put_begin(&mut live, "k2", "f0", 0, size, &pat, 2 * MIB).await;
    wait_active(&w, "k2", 1).await;
    let r = c.post_transfer("k2", &[("f0", "x", 1), ("f1", "x", 1)]).await;
    assert_eq!(r.status, 200);
    assert_eq!(files_active(&w, "k2"), 1, "the writer of f0 is still there after the POST");
    // The old request carries on and finishes.
    let r = put_end(&mut live, &pat, 2 * MIB, size).await;
    assert_eq!((r.status, r.json()), (200, json!({ "received": size })));
    // A PUT for a file that is being written does take over, as before: the old request gets 409 and the new one starts where the old one got to.
    let pat1 = Pattern::new(501);
    let mut old = http(&w).await;
    put_begin(&mut old, "k2", "f1", 0, size, &pat1, 3 * MIB).await;
    wait_active(&w, "k2", 1).await;
    let mut new = http(&w).await;
    let (r, _) = new.put("k2", "f1", 3 * MIB, size, &pat1, None).await.unwrap();
    assert_eq!(r.status, 200, "{:?}", r.json());
    let answer = read_response(&mut old.stream).await.expect("the old request is answered");
    assert_eq!((answer.status, answer.json()["offset"].clone()), (409, json!(3 * MIB)));
    pat.check_file(&dir.join("a0.bin"), size).unwrap();
    pat1.check_file(&dir.join("a1.bin"), size).unwrap();
}

#[tokio::test]
async fn a_delete_in_the_middle_of_a_batch_stops_every_file_and_keeps_the_complete_one() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let size = 8 * MIB;
    let mut c = http(&w).await;
    let mut files: Vec<(String, String, u64)> = (0..4).map(|i| (format!("f{i}"), format!("a{i}.bin"), size)).collect();
    files.push(("done".into(), "done.bin".into(), 1000));
    let refs: Vec<(&str, &str, u64)> = files.iter().map(|(f, n, s)| (f.as_str(), n.as_str(), *s)).collect();
    assert_eq!(c.post_transfer("x1", &refs).await.status, 200);
    assert_eq!(c.put("x1", "done", 0, 1000, &Pattern::new(9), None).await.unwrap().0.status, 200);

    let batch = Batch::start(&w, "x1", &[0, 1, 2, 3], size, 2 * MIB, false).await;
    let mut c2 = http(&w).await;
    assert_eq!(c2.simple("DELETE", "/v1/transfers/x1").await.status, 204);
    for (i, r) in batch.answers().await.into_iter().enumerate() {
        assert_eq!(r.status, 404, "file f{i}: a request of a canceled transfer is answered not_found: {:?}", r.json());
    }
    let dir = w.core().received_dir();
    assert_eq!(listing(&dir), ["done.bin"], "the four partial files are gone, the complete one stays");
    assert_eq!(w.core().db.get_transfer("x1").unwrap().unwrap().0.status, "canceled");
    assert!(w.core().status_json()["transfers"]["active"].as_array().unwrap().is_empty());
    let mut c3 = http(&w).await;
    assert_eq!(c3.put("x1", "f0", 0, 10, &Pattern::new(1), None).await.unwrap().0.status, 404, "the ID is used up");
    assert_eq!(c3.simple("DELETE", "/v1/transfers/x1").await.status, 404);
}

#[tokio::test]
async fn the_user_canceling_on_the_pc_in_the_middle_of_a_batch_tells_the_phone_and_stops_every_file() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let size = 8 * MIB;
    let mut c = http(&w).await;
    post_batch(&mut c, "x2", 4, size).await;
    let batch = Batch::start(&w, "x2", &[0, 1, 2, 3], size, 2 * MIB, false).await;
    let core = w.env.core.clone();
    assert!(core.cancel_transfer("x2").await);
    let msg = next_json(&mut ws).await;
    assert_eq!((msg["type"].as_str(), msg["transferId"].as_str(), msg["reason"].as_str()), (Some("cancel"), Some("x2"), Some("user")));
    send_json(&mut ws, &json!({ "type": "ack", "ref": msg["id"] })).await;
    for (i, r) in batch.answers().await.into_iter().enumerate() {
        assert_eq!(r.status, 404, "file f{i}: {:?}", r.json());
    }
    assert!(listing(&w.core().received_dir()).is_empty(), "every partial file is gone");
    assert_eq!(w.core().db.get_transfer("x2").unwrap().unwrap().0.status, "canceled");
    assert!(!core.cancel_transfer("x2").await, "nothing left to cancel");
}

// ----- small files on one connection ------------------------------------------------------------------------------------

/// Reads `n` responses that arrive one after the other on a connection (a client that sent its requests without waiting).
async fn read_pipelined<S: AsyncReadExt + Unpin>(stream: &mut S, n: usize) -> Vec<(u16, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    while out.len() < n {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_string();
            let mut lines = head.split("\r\n");
            let status: u16 = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
            let length = lines
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, v)| v.trim().parse::<usize>().unwrap());
            if buf.len() >= i + 4 + length {
                out.push((status, buf[i + 4..i + 4 + length].to_vec()));
                buf.drain(..i + 4 + length);
                continue;
            }
        }
        let mut chunk = vec![0u8; 64 * 1024];
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk)).await.expect("a response within 10 s").unwrap();
        assert!(read > 0, "the connection closed after {} of {n} responses", out.len());
        buf.extend_from_slice(&chunk[..read]);
    }
    out
}

#[tokio::test]
async fn small_files_sent_back_to_back_on_one_connection_are_all_answered_in_order() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let n = 20usize;
    let sizes: Vec<u64> = (0..n).map(|i| 1_000 + i as u64 * 7_919).collect();
    let pat = Pattern::new(77);
    let file_specs: Vec<Value> = (0..n)
        .map(|i| json!({ "fileId": format!("s{i}"), "name": format!("IMG_{i:03}.jpg"), "size": sizes[i], "mime": "image/jpeg" }))
        .collect();
    let body = json!({ "id": "pipe", "files": file_specs }).to_string();
    // The POST and the 20 PUTs go out in one write, without waiting for an answer.
    let mut wire = format!(
        "POST /v1/transfers HTTP/1.1\r\nHost: kin\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();
    for (i, size) in sizes.iter().enumerate() {
        wire.extend_from_slice(
            format!(
                "PUT /v1/transfers/pipe/files/s{i}?offset=0 HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
                size
            )
            .as_bytes(),
        );
        wire.extend_from_slice(&pat.bytes(0, *size as usize));
    }
    let mut c = http(&w).await;
    c.stream.write_all(&wire).await.unwrap();
    c.stream.flush().await.unwrap();
    let answers = read_pipelined(&mut c.stream, n + 1).await;
    assert_eq!(answers[0].0, 200, "{}", String::from_utf8_lossy(&answers[0].1));
    for (i, (status, body)) in answers[1..].iter().enumerate() {
        assert_eq!(*status, 200, "file {i}: {}", String::from_utf8_lossy(body));
        let v: Value = serde_json::from_slice(body).unwrap();
        assert_eq!(v, json!({ "received": sizes[i] }), "the answers come in the order of the requests");
    }
    let dir = w.core().received_dir();
    for (i, size) in sizes.iter().enumerate() {
        pat.check_file(&dir.join(format!("IMG_{i:03}.jpg")), *size).unwrap();
    }
    let core = w.env.core.clone();
    wait_until("the transfer to be done", || core.db.get_transfer("pipe").unwrap().unwrap().0.status == "done").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(files_toasts(&drain_events(&mut w)), 1);
}

// ----- PC to phone ------------------------------------------------------------------------------------------------------

fn make_files(dir: &Path, files: &[(String, Pattern, u64)]) -> Vec<PathBuf> {
    files
        .iter()
        .map(|(name, pat, size)| {
            let path = dir.join(name);
            pat.write_file(&path, *size);
            path
        })
        .collect()
}

/// Offers files (`f1`, `f2`, ... in order) and has the fake phone ack. Returns the transfer ID.
async fn offer(w: &World, ws: &mut Ws, paths: Vec<PathBuf>) -> String {
    let core = w.env.core.clone();
    let task = tokio::spawn(async move { core.send_paths(paths, "test").await });
    let frame = next_json(ws).await;
    assert_eq!(frame["type"], "offer", "{frame}");
    send_json(ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    frame["id"].as_str().unwrap().to_string()
}

/// After `get` has returned with the first bytes, reads the rest of a body of `remaining` more bytes from `abs`, checking the pattern. Returns how many
/// bytes came and whether they were right.
async fn drain_body(c: &mut Http, pat: &Pattern, mut abs: u64, remaining: u64) -> (u64, bool) {
    let mut buf = vec![0u8; MIB as usize];
    let (mut got, mut intact) = (0u64, true);
    while got < remaining {
        let want = (remaining - got).min(MIB) as usize;
        match c.stream.read(&mut buf[..want]).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                intact &= pat.matches(abs, &buf[..n]);
                abs += n as u64;
                got += n as u64;
            }
        }
    }
    (got, intact)
}

/// Four downloads of one offer started and left after their first bytes (the server's socket is full, so each one stays active).
async fn start_downloads(w: &World, tid: &str, pats: &[Pattern], size: u64) -> Vec<(Http, u64)> {
    let mut conns = Vec::new();
    for (i, pat) in pats.iter().enumerate() {
        let mut c = http(w).await;
        let d = c.get(tid, &format!("f{}", i + 1), None, Some(pat), Some(1)).await;
        assert_eq!((d.status, d.intact), (200, true));
        assert_eq!(d.header("content-length"), Some(size.to_string().as_str()));
        conns.push((c, d.received));
    }
    wait_active(w, tid, pats.len() as u64).await;
    conns
}

#[tokio::test]
async fn four_files_of_one_offer_download_at_once_and_one_resumes_with_a_range_while_the_others_carry_on() {
    let mut w = World::new().await;
    let mut ws = connected(&w).await;
    drain_events(&mut w);
    let size = 40 * MIB + 5;
    let src = tempfile::tempdir().unwrap();
    let pats: Vec<Pattern> = (0..4).map(|i| Pattern::new(600 + i)).collect();
    let files: Vec<(String, Pattern, u64)> = (0..4).map(|i| (format!("d{i}.bin"), pats[i].clone(), size)).collect();
    let tid = offer(&w, &mut ws, make_files(src.path(), &files)).await;
    let mut conns = start_downloads(&w, &tid, &pats, size).await;
    let view = w.core().status_json()["transfers"]["active"][0].clone();
    assert_eq!((view["direction"].as_str(), view["files"].as_u64(), view["filesActive"].as_u64()), (Some("out"), Some(4), Some(4)));

    // f1's connection is dropped after what it has read; the other three read on to the end.
    let (broken, read1) = conns.remove(0);
    drop(broken);
    let mut readers = Vec::new();
    for (k, (mut c, got)) in conns.into_iter().enumerate() {
        let pat = pats[k + 1].clone();
        readers.push(tokio::spawn(async move { drain_body(&mut c, &pat, got, size - got).await }));
    }
    // f1 resumes with a Range request on a new connection.
    let mut c = http(&w).await;
    let d = c.get(&tid, "f1", Some(read1), Some(&pats[0]), None).await;
    assert_eq!((d.status, d.received, d.intact), (206, size - read1, true));
    assert_eq!(d.header("content-range"), Some(format!("bytes {read1}-{}/{size}", size - 1).as_str()));
    for (k, r) in readers.into_iter().enumerate() {
        let (got, intact) = r.await.unwrap();
        assert!(intact, "file f{}", k + 2);
        assert!(got > 0);
    }
    send_json(&mut ws, &json!({ "type": "complete", "id": "m-c1", "transferId": tid })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-c1" }));
    let core = w.env.core.clone();
    wait_until("the offer to be done", || core.transfers.get(&tid).is_some_and(|e| e.status() == Status::Done)).await;
    assert_eq!(w.core().db.get_transfer(&tid).unwrap().unwrap().0.status, "done");
}

#[tokio::test]
async fn canceling_an_offer_in_the_middle_of_a_batch_stops_every_download() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let size = 40 * MIB;
    let src = tempfile::tempdir().unwrap();
    let pats: Vec<Pattern> = (0..4).map(|i| Pattern::new(700 + i)).collect();
    let files: Vec<(String, Pattern, u64)> = (0..4).map(|i| (format!("d{i}.bin"), pats[i].clone(), size)).collect();
    let tid = offer(&w, &mut ws, make_files(src.path(), &files)).await;
    let conns = start_downloads(&w, &tid, &pats, size).await;

    // The phone cancels with a message (the user's Cancel on the PC does the same from the other side).
    send_json(&mut ws, &json!({ "type": "cancel", "id": "m-x1", "transferId": tid, "reason": "user" })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-x1" }));
    let mut readers = Vec::new();
    for (k, (mut c, got)) in conns.into_iter().enumerate() {
        let pat = pats[k].clone();
        readers.push(tokio::spawn(async move { drain_body(&mut c, &pat, got, size - got).await }));
    }
    for (k, r) in readers.into_iter().enumerate() {
        let (got, intact) = r.await.unwrap();
        assert!(intact);
        assert!(got < size, "download f{} was cut off by the cancel after {got} more bytes", k + 1);
    }
    assert_eq!(w.core().db.get_transfer(&tid).unwrap().unwrap().0.status, "canceled");
    let mut c = http(&w).await;
    assert_eq!(c.get(&tid, "f1", None, None, None).await.status, 404);
    assert_eq!(files_active(&w, &tid), 0);
    assert!(w.core().status_json()["transfers"]["active"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_user_canceling_on_the_pc_in_the_middle_of_downloads_stops_them_and_tells_the_phone() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let size = 40 * MIB;
    let src = tempfile::tempdir().unwrap();
    let pats: Vec<Pattern> = (0..4).map(|i| Pattern::new(800 + i)).collect();
    let files: Vec<(String, Pattern, u64)> = (0..4).map(|i| (format!("d{i}.bin"), pats[i].clone(), size)).collect();
    let tid = offer(&w, &mut ws, make_files(src.path(), &files)).await;
    let conns = start_downloads(&w, &tid, &pats, size).await;
    let core = w.env.core.clone();
    assert!(core.cancel_transfer(&tid).await);
    let msg = next_json(&mut ws).await;
    assert_eq!(
        (msg["type"].as_str(), msg["transferId"].as_str(), msg["reason"].as_str()),
        (Some("cancel"), Some(tid.as_str()), Some("user"))
    );
    send_json(&mut ws, &json!({ "type": "ack", "ref": msg["id"] })).await;
    for (k, (mut c, got)) in conns.into_iter().enumerate() {
        let (more, _) = drain_body(&mut c, &pats[k], got, size - got).await;
        assert!(more < size - got, "download f{} was cut off", k + 1);
    }
    assert_eq!(w.core().db.get_transfer(&tid).unwrap().unwrap().0.status, "canceled");
}

#[tokio::test]
async fn small_downloads_requested_back_to_back_on_one_connection_are_all_answered_in_order() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let src = tempfile::tempdir().unwrap();
    let n = 12usize;
    let pats: Vec<Pattern> = (0..n).map(|i| Pattern::new(900 + i as u64)).collect();
    let sizes: Vec<u64> = (0..n).map(|i| 500 + i as u64 * 13_331).collect();
    let files: Vec<(String, Pattern, u64)> = (0..n).map(|i| (format!("s{i:02}.jpg"), pats[i].clone(), sizes[i])).collect();
    let tid = offer(&w, &mut ws, make_files(src.path(), &files)).await;
    let mut wire = String::new();
    for i in 0..n {
        wire.push_str(&format!("GET /v1/transfers/{tid}/files/f{} HTTP/1.1\r\nHost: kin\r\n\r\n", i + 1));
    }
    let mut c = http(&w).await;
    c.stream.write_all(wire.as_bytes()).await.unwrap();
    c.stream.flush().await.unwrap();
    let answers = read_pipelined(&mut c.stream, n).await;
    for (i, (status, body)) in answers.iter().enumerate() {
        assert_eq!(*status, 200, "file {i}");
        assert_eq!(body.len() as u64, sizes[i]);
        assert!(pats[i].matches(0, body), "the answers come in the order of the requests: file {i}");
    }
}

// ----- speed (not run by default) ---------------------------------------------------------------------------------------

/// The time the PC needs for 50 files of about 3.4 MB (phone to PC, loopback, the database is a file), with the phone's requests in 3
/// shapes: one at a time, four connections at once, and pipelined on one connection. Prints the three times; asserts only that all arrive.
#[tokio::test]
#[ignore]
async fn speed_of_a_photo_batch_one_at_a_time_in_parallel_and_pipelined() {
    let n = 50usize;
    let size = 3_425_500u64;
    for shape in ["one at a time", "4 at once", "pipelined"] {
        let w = World::new_on_disk().await;
        let _ws = connected(&w).await;
        let pat = Pattern::new(42);
        let mut c = http(&w).await;
        let names: Vec<String> = (0..n).map(|i| format!("IMG_{i:04}.jpg")).collect();
        let files: Vec<(String, &str, u64)> = (0..n).map(|i| (format!("f{i}"), names[i].as_str(), size)).collect();
        let refs: Vec<(&str, &str, u64)> = files.iter().map(|(f, a, s)| (f.as_str(), *a, *s)).collect();
        assert_eq!(c.post_transfer("sp", &refs).await.status, 200);
        let started = Instant::now();
        match shape {
            "one at a time" => {
                for i in 0..n {
                    assert_eq!(c.put("sp", &format!("f{i}"), 0, size, &pat, None).await.unwrap().0.status, 200);
                }
            }
            "4 at once" => {
                let mut tasks = Vec::new();
                for lane in 0..4usize {
                    let (mut conn, pat) = (http(&w).await, pat.clone());
                    tasks.push(tokio::spawn(async move {
                        for i in (lane..n).step_by(4) {
                            assert_eq!(conn.put("sp", &format!("f{i}"), 0, size, &pat, None).await.unwrap().0.status, 200);
                        }
                    }));
                }
                for t in tasks {
                    t.await.unwrap();
                }
            }
            _ => {
                let one = pat.bytes(0, size as usize);
                let mut wire = Vec::new();
                for i in 0..n {
                    wire.extend_from_slice(
                        format!(
                            "PUT /v1/transfers/sp/files/f{i}?offset=0 HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {size}\r\n\r\n"
                        )
                        .as_bytes(),
                    );
                    wire.extend_from_slice(&one);
                }
                c.stream.write_all(&wire).await.unwrap();
                c.stream.flush().await.unwrap();
                assert!(read_pipelined(&mut c.stream, n).await.iter().all(|(s, _)| *s == 200));
            }
        }
        eprintln!("50 photos of 3.4 MB, {shape}: {:?}", started.elapsed());
        let dir = w.core().received_dir();
        assert_eq!(listing(&dir).len(), n);
    }
}
