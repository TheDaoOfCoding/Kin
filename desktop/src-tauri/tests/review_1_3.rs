//! The 1.3 review fixes, one test for each, against the core directly (no TLS): the `PUT` is answered before the toast and the image
//! are made, the Mark of the Web, a `.part` file that is never adopted by another transfer, the clean-up that deletes only what a record
//! names, a transfer ID that is never used twice, a finish that outlives its request, the stored-bytes check, the cancel that races the
//! last `PUT`, the cleaned cancel reason, paths over 259 characters and the collision limit.
//!
//! Everything lives in a temporary folder. **Nothing here touches the real Downloads folder or the real clipboard.**
mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::transfer::*;
use common::*;
use kin_lib::cli::Config;
use kin_lib::clipboard::{ClipSnapshot, Clipboard, Exclusion, ImageOutcome, MemoryClipboard, MonotonicClock};
use kin_lib::core::{Core, Event, Services};
use kin_lib::links::RecordingOpener;
use kin_lib::storage::{Db, TransferFileRow, TransferRow};
use kin_lib::transfers::{FileSpec, PutParams, Status, UploadRequest};
use tokio::sync::mpsc::UnboundedReceiver;

/// A clipboard whose image write takes a while (a busy clipboard, a big picture) and remembers when it finished.
struct SlowClipboard {
    inner: Arc<MemoryClipboard>,
    image_delay: Duration,
}

impl Clipboard for SlowClipboard {
    fn read(&self) -> kin_lib::Result<ClipSnapshot> {
        self.inner.read()
    }
    fn write_text(&self, text: &str, mark: Option<Exclusion>) -> kin_lib::Result<()> {
        self.inner.write_text(text, mark)
    }
    fn write_text_sensitive(&self, text: &str) -> kin_lib::Result<()> {
        self.inner.write_text_sensitive(text)
    }
    fn write_image_file(&self, path: &Path) -> kin_lib::Result<ImageOutcome> {
        std::thread::sleep(self.image_delay);
        self.inner.write_image_file(path)
    }
}

struct Harness {
    core: Arc<Core>,
    events: UnboundedReceiver<Event>,
    clipboard: Arc<MemoryClipboard>,
    data: PathBuf,
    recv: PathBuf,
    _root: tempfile::TempDir,
}

fn open_core(
    data: &Path,
    recv: &Path,
    image_delay: Duration,
) -> (Arc<Core>, UnboundedReceiver<Event>, Arc<MemoryClipboard>, Arc<RecordingOpener>) {
    std::fs::create_dir_all(data).unwrap();
    let mut config = Config::new(data.to_path_buf());
    config.dev_bind = Some("127.0.0.1".parse().unwrap());
    config.dev_received_dir = Some(recv.to_path_buf());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let memory = Arc::new(MemoryClipboard::default());
    let opener = Arc::new(RecordingOpener::default());
    let clipboard: Arc<dyn Clipboard> =
        if image_delay.is_zero() { memory.clone() } else { Arc::new(SlowClipboard { inner: memory.clone(), image_delay }) };
    let services = Services { clipboard, opener: opener.clone(), clock: Arc::new(MonotonicClock::new()) };
    let key = &vector_keys()[0];
    let core = Arc::new(Core::new(config, identity_of(key), Db::open(&data.join("kin.db")).unwrap(), "Desk PC".into(), tx, services));
    (core, rx, memory, opener)
}

fn harness() -> Harness {
    harness_with(Duration::ZERO)
}

fn harness_with(image_delay: Duration) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let (data, recv) = (root.path().join("data"), root.path().join("recv"));
    let (core, events, clipboard, _opener) = open_core(&data, &recv, image_delay);
    Harness { core, events, clipboard, data, recv, _root: root }
}

/// The same data folder opened again, as after a restart.
fn restart(h: &Harness) -> (Arc<Core>, UnboundedReceiver<Event>) {
    let (core, events, _, _) = open_core(&h.data, &h.recv, Duration::ZERO);
    (core, events)
}

const PHONE: &str = "phone";

fn spec(id: &str, name: &str, size: u64) -> FileSpec {
    FileSpec { file_id: id.into(), name: name.into(), size, mime: "application/octet-stream".into() }
}

fn request(id: &str, files: Vec<FileSpec>) -> UploadRequest {
    UploadRequest { id: id.into(), files, origin: None }
}

fn put_params(offset: u64, length: u64) -> PutParams {
    PutParams { offset: Some(offset.to_string()), content_length: Some(length) }
}

fn pattern_bytes(len: usize) -> Vec<u8> {
    Pattern::new(7).bytes(0, len)
}

/// `PUT` of a whole file from offset 0 with these bytes.
async fn put_all(core: &Arc<Core>, id: &str, file: &str, bytes: &[u8]) -> Result<u64, kin_lib::transfers::Failure> {
    core.upload_put(PHONE, id, file, put_params(0, bytes.len() as u64), axum::body::Body::from(bytes.to_vec())).await
}

/// `POST` and `PUT` for files given as `(fileId, name, bytes)`.
async fn upload(core: &Arc<Core>, id: &str, files: &[(&str, &str, Vec<u8>)]) {
    let specs = files.iter().map(|(f, n, b)| spec(f, n, b.len() as u64)).collect();
    core.upload_post(PHONE, &request(id, specs)).await.expect("POST");
    for (f, _, bytes) in files {
        put_all(core, id, f, bytes).await.expect("PUT");
    }
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

fn code_of(r: &Result<impl std::fmt::Debug, kin_lib::transfers::Failure>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(f) => f.error.code(),
    }
}

fn files_toasts(events: &mut UnboundedReceiver<Event>) -> usize {
    std::iter::from_fn(|| events.try_recv().ok()).filter(|e| matches!(e, Event::FilesToast { .. })).count()
}

// ----- the PUT never waits for the toast and the image ---------------------------------------------------------------------

#[tokio::test]
async fn the_put_is_answered_before_the_toast_and_the_image_are_made() {
    let mut h = harness_with(Duration::from_secs(2));
    h.core.upload_post(PHONE, &request("slow", vec![spec("f", "pic.png", TINY_PNG.len() as u64)])).await.unwrap();
    let started = Instant::now();
    assert_eq!(put_all(&h.core, "slow", "f", TINY_PNG).await.unwrap(), TINY_PNG.len() as u64);
    let answered = started.elapsed();
    // The image write takes 2 s: the answer must not have waited for it (nor for the decoder, the toast or anything else of the arrival).
    assert!(answered < Duration::from_millis(1500), "the PUT took {answered:?}");
    assert!(h.clipboard.app_images().is_empty(), "the image is not on the clipboard yet");
    assert!(h.recv.join("pic.png").exists() && !h.recv.join("pic.png.part").exists(), "the file is final when the 200 goes out");
    // The toast comes first, the image after the clipboard has had its turn.
    wait_until("the toast", || files_toasts(&mut h.events) > 0).await;
    wait_until("the image on the clipboard", || h.clipboard.app_images() == [h.recv.join("pic.png")]).await;
    assert!(started.elapsed() >= Duration::from_secs(2), "the slow clipboard did take its time");
}

// ----- the Mark of the Web -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_received_file_carries_the_mark_of_the_web() {
    let h = harness();
    upload(&h.core, "m1", &[("a", "photo.jpg", pattern_bytes(5000)), ("b", "empty.txt", Vec::new()), ("c", "run.ps1", b"calc".to_vec())])
        .await;
    for name in ["photo.jpg", "empty.txt", "run.ps1"] {
        let mark = kin_lib::motw::read(&h.recv.join(name)).unwrap_or_else(|| panic!("{name} has no Zone.Identifier"));
        assert!(mark.starts_with("[ZoneTransfer]") && mark.contains("ZoneId=3"), "{name}: {mark:?}");
        assert!(mark.contains("HostUrl=about:internet"), "{name}: {mark:?}");
    }
    assert_eq!(std::fs::read(h.recv.join("photo.jpg")).unwrap(), pattern_bytes(5000), "the data of the file is untouched");
    assert_eq!(listing(&h.recv), ["empty.txt", "photo.jpg", "run.ps1"], "a stream is not a file of the folder");
}

// ----- a .part file is never adopted by another transfer ---------------------------------------------------------------------

#[tokio::test]
async fn a_resume_never_takes_over_another_transfers_part_file() {
    let h = harness();
    // Transfer A starts `a.bin` and keeps 2,000 of its 5,000 bytes; the user deletes its .part file.
    h.core.upload_post(PHONE, &request("A", vec![spec("f", "a.bin", 5000)])).await.unwrap();
    let body = pattern_bytes(2000);
    let _ = h.core.upload_put(PHONE, "A", "f", put_params(0, 5000), axum::body::Body::from(body)).await;
    std::fs::remove_file(h.recv.join("a.bin.part")).unwrap();
    // Transfer B arrives with the same name. A still holds the name `a.bin`, so B gets the next one.
    h.core.upload_post(PHONE, &request("B", vec![spec("g", "a.bin", 10)])).await.unwrap();
    assert!(h.recv.join("a (1).bin.part").exists(), "{:?}", listing(&h.recv));
    put_all(&h.core, "B", "g", b"B's 10 byt").await.unwrap();
    assert_eq!(std::fs::read(h.recv.join("a (1).bin")).unwrap(), b"B's 10 byt");
    // A resumes: it has nothing stored (its file is gone), it gets a new .part under its own name, and B's file is untouched.
    let offsets = h.core.upload_post(PHONE, &request("A", vec![spec("f", "a.bin", 5000)])).await.unwrap();
    assert_eq!(offsets, [("f".to_string(), 0)]);
    assert!(h.recv.join("a.bin.part").exists());
    let all = pattern_bytes(5000);
    put_all(&h.core, "A", "f", &all).await.unwrap();
    assert_eq!(std::fs::read(h.recv.join("a.bin")).unwrap(), all);
    assert_eq!(std::fs::read(h.recv.join("a (1).bin")).unwrap(), b"B's 10 byt", "B's bytes were never touched");
}

// ----- the clean-up deletes only what a record names -------------------------------------------------------------------------

fn record(id: &str, status: &str, dest: &Path, files: &[(&str, Option<&Path>, bool)]) {
    // (a row for the database file that the test core uses)
    let db = Db::open(&dest.parent().unwrap().join("data").join("kin.db")).unwrap();
    let row = TransferRow {
        id: id.into(),
        device_id: PHONE.into(),
        direction: "in".into(),
        status: status.into(),
        created_at: 1,
        updated_at: 1,
        finished_at: (status != "active").then_some(1),
        expires_at: None,
        total_bytes: 10,
        error_code: None,
        dest_dir: Some(dest.to_string_lossy().into_owned()),
    };
    let rows: Vec<TransferFileRow> = files
        .iter()
        .enumerate()
        .map(|(i, (name, path, finalized))| TransferFileRow {
            file_id: format!("f{i}"),
            idx: i as i64,
            name: (*name).into(),
            mime: String::new(),
            size: 10,
            bytes_done: 1,
            local_path: path.map(|p| p.to_string_lossy().into_owned()),
            finalized: *finalized,
            source_mtime_ms: None,
            temp_source: false,
            finalizing_name: None,
        })
        .collect();
    db.insert_transfer(&row, &rows).unwrap();
}

#[tokio::test]
async fn the_sweeper_deletes_only_paths_a_record_names() {
    let h = harness();
    std::fs::create_dir_all(&h.recv).unwrap();
    // A file the app left behind: a canceled transfer whose .part file is still recorded (the app stopped before removing it).
    let leftover = h.recv.join("left.bin.part");
    std::fs::write(&leftover, "x").unwrap();
    // The user's: a .part file that no record names (a browser download, say), and a file a transfer finished under a .part-like name.
    let browser = h.recv.join("movie.mp4.part");
    std::fs::write(&browser, "x").unwrap();
    let finished = h.recv.join("backup.part");
    std::fs::write(&finished, "mine").unwrap();
    record("dead", "canceled", &h.recv, &[("left.bin", Some(&leftover), false)]);
    record("done", "done", &h.recv, &[("backup.part", Some(&finished), true)]);
    // A stale record of a canceled transfer that points at the path that `done` finished under: it must not take that file with it.
    record("stale", "failed", &h.recv, &[("backup", Some(&finished), false)]);
    let old = std::time::SystemTime::now() - Duration::from_secs(400 * 24 * 3600);
    for p in [&browser, &finished] {
        std::fs::OpenOptions::new().write(true).open(p).unwrap().set_modified(old).unwrap();
    }
    let (core, _events) = restart(&h);
    core.clean_up().await;
    assert_eq!(listing(&h.recv), ["backup.part", "movie.mp4.part"], "only the recorded leftover went, however old the others are");
    assert_eq!(std::fs::read_to_string(&finished).unwrap(), "mine");
    let (_, files) = core.db.get_transfer("dead").unwrap().unwrap();
    assert!(files[0].local_path.is_none(), "the record no longer points at a file that is gone");
    // A second run finds nothing to do.
    core.clean_up().await;
    assert_eq!(listing(&h.recv), ["backup.part", "movie.mp4.part"]);
}

#[tokio::test]
async fn a_canceled_upload_removes_its_part_files_and_forgets_the_path() {
    let h = harness();
    h.core.upload_post(PHONE, &request("c", vec![spec("f", "c.bin", 100), spec("g", "d.bin", 100)])).await.unwrap();
    put_all(&h.core, "c", "g", &pattern_bytes(100)).await.unwrap();
    assert!(h.core.upload_delete(PHONE, "c").await.is_ok());
    assert_eq!(listing(&h.recv), ["d.bin"], "the complete file stays, the partial one goes");
    let (row, files) = h.core.db.get_transfer("c").unwrap().unwrap();
    assert_eq!(row.status, "canceled", "the record stays");
    assert!(files[0].local_path.is_none() && files[1].local_path.is_some());
}

// ----- a transfer ID is never used twice ----------------------------------------------------------------------------------------

#[tokio::test]
async fn a_transfer_id_is_never_reused() {
    let h = harness();
    let specs = || vec![spec("f", "x.bin", 5)];
    // Finished: the offsets for a day, then 404 (and the same for a PUT and a DELETE).
    upload(&h.core, "fin", &[("f", "x.bin", pattern_bytes(5))]).await;
    assert_eq!(h.core.upload_post(PHONE, &request("fin", specs())).await.unwrap(), [("f".to_string(), 5)]);
    {
        let entry = h.core.transfers.get("fin").unwrap();
        entry.meta.lock().unwrap().finished_ms = Some(unix_ms_now() - 25 * 3_600_000);
    }
    assert_eq!(code_of(&h.core.upload_post(PHONE, &request("fin", specs())).await), "not_found", "a finished ID after a day");
    assert_eq!(code_of(&put_all(&h.core, "fin", "f", &pattern_bytes(5)).await), "not_found");
    assert_eq!(code_of(&h.core.upload_delete(PHONE, "fin").await), "not_found");
    // Canceled: 404 at once, a repeated DELETE is 404 too.
    h.core.upload_post(PHONE, &request("can", specs())).await.unwrap();
    assert!(h.core.upload_delete(PHONE, "can").await.is_ok());
    assert_eq!(code_of(&h.core.upload_delete(PHONE, "can").await), "not_found");
    assert_eq!(code_of(&h.core.upload_post(PHONE, &request("can", specs())).await), "not_found", "a canceled ID is never reused");
    // Another device's POST with a used ID is 404 as well, and nothing was created for it.
    assert_eq!(code_of(&h.core.upload_post("other", &request("can", specs())).await), "not_found");
    // The records are all still there: nothing wipes them.
    for id in ["fin", "can"] {
        assert!(h.core.db.get_transfer(id).unwrap().is_some(), "{id}");
    }
    assert_eq!(h.core.db.get_transfer("fin").unwrap().unwrap().1[0].name, "x.bin");
    // After a restart the answers are the same (the record of the finished one says it finished 25 hours ago, as the clock above did).
    h.core.db.set_transfer_status("fin", "done", (unix_ms_now() - 25 * 3_600_000) as i64, None).unwrap();
    let (again, _events) = restart(&h);
    assert_eq!(code_of(&again.upload_post(PHONE, &request("can", specs())).await), "not_found");
    assert_eq!(code_of(&again.upload_post(PHONE, &request("fin", specs())).await), "not_found");
    // A new ID is fine, and the folder holds exactly the one finished file: the two canceled/finished ones created nothing.
    assert!(again.upload_post(PHONE, &request("new", vec![spec("f", "y.bin", 1)])).await.is_ok());
    assert_eq!(listing(&h.recv), ["x.bin", "y.bin.part"]);
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

#[tokio::test]
async fn inserting_a_used_transfer_id_changes_nothing() {
    let db = Db::open_in_memory().unwrap();
    let row = |status: &str| TransferRow {
        id: "same".into(),
        device_id: PHONE.into(),
        direction: "in".into(),
        status: status.into(),
        created_at: 1,
        updated_at: 1,
        finished_at: None,
        expires_at: None,
        total_bytes: 3,
        error_code: None,
        dest_dir: None,
    };
    let file = |name: &str| TransferFileRow {
        file_id: "f".into(),
        idx: 0,
        name: name.into(),
        mime: String::new(),
        size: 3,
        bytes_done: 3,
        local_path: Some(name.into()),
        finalized: true,
        source_mtime_ms: None,
        temp_source: false,
        finalizing_name: None,
    };
    db.insert_transfer(&row("done"), &[file("first.bin")]).unwrap();
    assert!(db.insert_transfer(&row("active"), &[file("second.bin")]).is_err());
    let (kept, files) = db.get_transfer("same").unwrap().unwrap();
    assert_eq!((kept.status.as_str(), files[0].name.as_str()), ("done", "first.bin"), "the first record is untouched");
    assert!(db.transfer_exists("same").unwrap() && !db.transfer_exists("other").unwrap());
}

// ----- finishing a file does not depend on the request -----------------------------------------------------------------------

#[tokio::test]
async fn a_finish_that_is_cut_off_by_a_dropped_request_still_ends_and_a_retry_finds_the_file_complete() {
    let mut h = harness();
    h.core.hooks.finalize_delay_ms.store(500, std::sync::atomic::Ordering::Relaxed);
    let bytes = pattern_bytes(3000);
    h.core.upload_post(PHONE, &request("t", vec![spec("f", "doc.bin", 3000)])).await.unwrap();
    // The phone's request is dropped while the PC is finishing the file (the phone gave up waiting).
    let (core, body) = (h.core.clone(), bytes.clone());
    let put = tokio::spawn(async move { put_all(&core, "t", "f", &body).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    put.abort();
    assert!(put.await.unwrap_err().is_cancelled());
    // The finish carries on without it, and records the result.
    wait_until("the file to be final", || h.recv.join("doc.bin").exists()).await;
    wait_until("the transfer to be done", || h.core.transfers.get("t").is_some_and(|e| e.status() == Status::Done)).await;
    let (row, files) = h.core.db.get_transfer("t").unwrap().unwrap();
    assert_eq!(row.status, "done");
    assert!(files[0].finalized && files[0].finalizing_name.is_none());
    // The phone asks again, as it does: the file is complete (a POST says so, an empty PUT is a 200), and there is still one copy.
    h.core.hooks.finalize_delay_ms.store(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(h.core.upload_post(PHONE, &request("t", vec![spec("f", "doc.bin", 3000)])).await.unwrap(), [("f".to_string(), 3000)]);
    let again = h.core.upload_put(PHONE, "t", "f", put_params(3000, 0), axum::body::Body::empty()).await;
    assert_eq!(again.unwrap(), 3000);
    assert_eq!(listing(&h.recv), ["doc.bin"], "one file, no second copy and no .part");
    assert_eq!(std::fs::read(h.recv.join("doc.bin")).unwrap(), bytes);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(files_toasts(&mut h.events), 1, "the transfer was reported once");
}

#[tokio::test]
async fn bytes_that_arrived_are_kept_when_the_request_is_dropped_before_the_body_ends() {
    use futures_util::FutureExt;
    let h = harness();
    h.core.upload_post(PHONE, &request("drop", vec![spec("f", "d.bin", 5000)])).await.unwrap();
    // 3,000 of 5,000 bytes arrive and sit in the PC's buffer (less than a piece, so not yet written). Then the phone's connection breaks:
    // the server drops the handler's future, and a moment later the body ends.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<axum::body::Bytes, std::io::Error>>();
    let body =
        axum::body::Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) }));
    tx.send(Ok(axum::body::Bytes::from(pattern_bytes(3000)))).unwrap();
    let core = h.core.clone();
    let mut handler = Box::pin(async move { core.upload_put(PHONE, "drop", "f", put_params(0, 5000), body).await });
    for _ in 0..50 {
        assert!(handler.as_mut().now_or_never().is_none(), "the PUT waits for the rest of the body");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(handler); // the request is dropped at whatever it was waiting for
    drop(tx); // and the body ends
    wait_until("what arrived to be written and recorded", || h.core.transfers.get("drop").unwrap().files[0].done() == 3000).await;
    let offsets = h.core.upload_post(PHONE, &request("drop", vec![spec("f", "d.bin", 5000)])).await.unwrap();
    assert_eq!(offsets, [("f".to_string(), 3000)], "the 3,000 bytes were not lost with the request");
    assert_eq!(std::fs::read(h.recv.join("d.bin.part")).unwrap(), pattern_bytes(3000));
}

#[tokio::test]
async fn a_delete_whose_request_is_dropped_is_still_carried_out() {
    use futures_util::FutureExt;
    let h = harness();
    h.core.upload_post(PHONE, &request("gone", vec![spec("f", "g.bin", 100), spec("h", "h.bin", 100)])).await.unwrap();
    put_all(&h.core, "gone", "h", &pattern_bytes(100)).await.unwrap();
    // The first poll starts the cancel; the future is dropped while it waits (the phone hung up right after sending the DELETE).
    let core = h.core.clone();
    assert!(core.upload_delete(PHONE, "gone").now_or_never().is_none());
    wait_until("the transfer to be gone", || h.core.transfers.get("gone").is_none()).await;
    wait_until("the partial file to be removed", || !h.recv.join("g.bin.part").exists()).await;
    assert_eq!(listing(&h.recv), ["h.bin"]);
    assert_eq!(h.core.db.get_transfer("gone").unwrap().unwrap().0.status, "canceled");
    assert_eq!(code_of(&h.core.upload_post(PHONE, &request("gone", vec![spec("f", "g.bin", 100)])).await), "not_found");
}

#[tokio::test]
async fn the_put_checks_the_stored_bytes_before_it_says_the_file_is_complete() {
    let h = harness();
    h.core.hooks.finalize_delay_ms.store(500, std::sync::atomic::Ordering::Relaxed);
    h.core.upload_post(PHONE, &request("t", vec![spec("f", "short.bin", 4000)])).await.unwrap();
    let (core, body) = (h.core.clone(), pattern_bytes(4000));
    let put = tokio::spawn(async move { put_all(&core, "t", "f", &body).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    // While the PC is finishing the file, something cuts it short.
    std::fs::OpenOptions::new().write(true).open(h.recv.join("short.bin.part")).unwrap().set_len(1234).unwrap();
    let result = put.await.unwrap();
    assert_eq!(code_of(&result), "internal", "not a 200: the file isn't complete");
    assert!(h.recv.join("short.bin.part").exists() && !h.recv.join("short.bin").exists(), "{:?}", listing(&h.recv));
    h.core.hooks.finalize_delay_ms.store(0, std::sync::atomic::Ordering::Relaxed);
    let offsets = h.core.upload_post(PHONE, &request("t", vec![spec("f", "short.bin", 4000)])).await.unwrap();
    assert_eq!(offsets, [("f".to_string(), 1234)], "the PC reports what is really stored");
}

#[tokio::test]
async fn a_restart_in_the_middle_of_a_move_finds_the_file_final() {
    let h = harness();
    std::fs::create_dir_all(&h.recv).unwrap();
    // The record says the .part file was being moved to `x.bin`; the move happened (the .part file is gone, `x.bin` is whole).
    std::fs::write(h.recv.join("x.bin"), [7u8; 10]).unwrap();
    let part = h.recv.join("x.bin.part");
    let db = Db::open(&h.data.join("kin.db")).unwrap();
    let row = TransferRow {
        id: "mid".into(),
        device_id: PHONE.into(),
        direction: "in".into(),
        status: "active".into(),
        created_at: 1,
        updated_at: 1,
        finished_at: None,
        expires_at: None,
        total_bytes: 10,
        error_code: None,
        dest_dir: Some(h.recv.to_string_lossy().into_owned()),
    };
    let file = TransferFileRow {
        file_id: "f".into(),
        idx: 0,
        name: "x.bin".into(),
        mime: String::new(),
        size: 10,
        bytes_done: 10,
        local_path: Some(part.to_string_lossy().into_owned()),
        finalized: false,
        source_mtime_ms: None,
        temp_source: false,
        finalizing_name: Some("x.bin".into()),
    };
    db.insert_transfer(&row, &[file]).unwrap();
    drop(db);
    let (core, _events) = restart(&h);
    let entry = core.transfers.get("mid").unwrap();
    assert!(entry.files[0].data().finalized, "the move had happened: the file is final");
    // The phone's retry finishes the transfer; nothing is uploaded again.
    let offsets = core.upload_post(PHONE, &request("mid", vec![spec("f", "x.bin", 10)])).await.unwrap();
    assert_eq!(offsets, [("f".to_string(), 10)]);
    assert_eq!(core.transfers.get("mid").unwrap().status(), Status::Done);
    assert_eq!(listing(&h.recv), ["x.bin"]);
}

// ----- a cancel and the last PUT, and more than one cancel -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancel_racing_the_last_put_leaves_one_consistent_end_every_time() {
    let mut toasts_total = 0;
    for round in 0..40 {
        let mut h = harness();
        let id = format!("race{round}");
        let (a, b) = (pattern_bytes(300_000), pattern_bytes(200_000));
        h.core.upload_post(PHONE, &request(&id, vec![spec("a", "a.bin", 300_000), spec("b", "b.bin", 200_000)])).await.unwrap();
        put_all(&h.core, &id, "a", &a).await.unwrap();
        let (c1, c2, c3) = (h.core.clone(), h.core.clone(), h.core.clone());
        let (id1, id2, id3) = (id.clone(), id.clone(), id.clone());
        let last = tokio::spawn(async move { put_all(&c1, &id1, "b", &b).await.map(|_| ()) });
        let delete = tokio::spawn(async move { c2.upload_delete(PHONE, &id2).await });
        let message = tokio::spawn(async move { c3.peer_cancelled(PHONE, &id3, "user").await });
        let (last, delete, _) = (last.await.unwrap(), delete.await.unwrap(), message.await.unwrap());
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = h.core.db.get_transfer(&id).unwrap().unwrap().0.status;
        let files = listing(&h.recv);
        assert!(!files.iter().any(|f| f.ends_with(".part")), "round {round}: no .part file survives ({files:?})");
        assert!(status == "done" || status == "canceled", "round {round}: {status}");
        let toasts = files_toasts(&mut h.events);
        toasts_total += toasts;
        if status == "done" {
            assert_eq!(files, ["a.bin", "b.bin"], "round {round}: done means both files");
            assert_eq!(toasts, 1, "round {round}: reported once");
            assert!(last.is_ok(), "round {round}: the PUT that finished the transfer was answered 200 ({last:?})");
        } else {
            assert_eq!(files, ["a.bin"], "round {round}: a cancel keeps the complete file and drops the rest ({files:?})");
            assert_eq!(toasts, 0, "round {round}: a canceled transfer is not reported as received");
        }
        // The cancel took effect once. DELETE says 204 or (the message got there first) 404; the transfer is gone from memory.
        assert!(matches!(code_of(&delete), "ok" | "not_found"), "round {round}");
        assert!(h.core.transfers.get(&id).is_none(), "round {round}");
        assert_eq!(code_of(&h.core.upload_post(PHONE, &request(&id, vec![spec("a", "a.bin", 300_000)])).await), "not_found");
    }
    eprintln!("{toasts_total} of 40 rounds finished before the cancel");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_files_finishing_at_the_same_moment_end_the_transfer_once() {
    for round in 0..30 {
        let mut h = harness();
        let id = format!("pair{round}");
        let (a, b) = (pattern_bytes(250_000), pattern_bytes(150_000));
        h.core.upload_post(PHONE, &request(&id, vec![spec("a", "a.bin", 250_000), spec("b", "b.bin", 150_000)])).await.unwrap();
        let (c1, c2, i1, i2) = (h.core.clone(), h.core.clone(), id.clone(), id.clone());
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move { put_all(&c1, &i1, "a", &a).await }),
            tokio::spawn(async move { put_all(&c2, &i2, "b", &b).await })
        );
        assert!(ra.unwrap().is_ok() && rb.unwrap().is_ok(), "round {round}");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(h.core.db.get_transfer(&id).unwrap().unwrap().0.status, "done");
        assert_eq!(files_toasts(&mut h.events), 1, "round {round}: one toast for one transfer");
        assert_eq!(listing(&h.recv), ["a.bin", "b.bin"]);
    }
}

#[tokio::test]
async fn a_second_cancel_does_nothing_and_a_delete_after_done_keeps_the_files() {
    let h = harness();
    upload(&h.core, "d", &[("f", "kept.bin", pattern_bytes(50))]).await;
    assert!(h.core.upload_delete(PHONE, "d").await.is_ok(), "DELETE of a finished transfer: 204, the files stay");
    assert_eq!(listing(&h.recv), ["kept.bin"]);
    assert_eq!(h.core.db.get_transfer("d").unwrap().unwrap().0.status, "done", "a finished transfer is not turned into a canceled one");
    assert_eq!(code_of(&h.core.upload_delete(PHONE, "d").await), "not_found");
    h.core.peer_cancelled(PHONE, "d", "user").await; // nothing to do, and no panic
    assert_eq!(listing(&h.recv), ["kept.bin"]);
}

// ----- the cancel reason comes from the phone -----------------------------------------------------------------------------------

#[tokio::test]
async fn a_cancel_reason_is_cleaned_before_it_is_logged_or_stored() {
    capture_logs();
    let h = harness();
    h.core.upload_post(PHONE, &request("r", vec![spec("f", "r.bin", 10)])).await.unwrap();
    let hostile = format!("\u{1b}[31mred\r\nFAKE LOG LINE\u{0}\u{202e}reversed {}", "x".repeat(500));
    h.core.peer_cancelled(PHONE, "r", &hostile).await;
    let (row, _) = h.core.db.get_transfer("r").unwrap().unwrap();
    let stored = row.error_code.unwrap();
    assert!(stored.chars().count() <= 32 && !stored.chars().any(|c| c.is_control() || c == '\u{202e}'), "{stored:?}");
    assert!(stored.starts_with("[31mred"), "what is left is the text without its control characters: {stored:?}");
    let lines = captured_lines();
    let line = lines.iter().find(|l| l.contains("the phone canceled transfer")).expect("the cancel is logged");
    assert!(!line.contains('\u{1b}') && !line.contains('\n') && !line.contains('\r') && !line.contains('\u{0}'), "{line:?}");
    assert!(line.len() < 200, "capped: {line:?}");
    assert!(!lines.iter().any(|l| l.contains("FAKE LOG LINE") && !l.contains("canceled transfer")), "no line of its own");
    // An empty or all-control reason is "peer".
    h.core.upload_post(PHONE, &request("r2", vec![spec("f", "r2.bin", 10)])).await.unwrap();
    h.core.peer_cancelled(PHONE, "r2", "\u{1}\u{2}").await;
    assert_eq!(h.core.db.get_transfer("r2").unwrap().unwrap().0.error_code.as_deref(), Some("peer"));
}

#[test]
fn transfer_ids_that_share_a_start_get_different_short_forms() {
    use kin_lib::core::short_transfer;
    let (a, b) = ("0193a5c2-8000-7a11-8b22-0123456789ab", "0193a5c2-8000-7a11-8b22-0123456789cd");
    assert_ne!(short_transfer(a), short_transfer(b), "UUIDv7s of one moment share their first characters");
    assert_eq!(short_transfer(a), "456789ab");
    assert_eq!(short_transfer("t1"), "t1");
    assert_eq!(short_transfer(""), "");
}

// ----- paths over 259 characters --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_received_folder_deeper_than_259_characters_works_for_the_final_move() {
    let root = tempfile::tempdir().unwrap();
    let deep = ["d".repeat(60), "e".repeat(60), "f".repeat(60), "g".repeat(60), "h".repeat(40)]
        .iter()
        .fold(root.path().join("recv"), |p, s| p.join(s));
    assert!(deep.to_string_lossy().len() > 259 - 30, "a folder this deep with a file name in it is over 259");
    let (core, _events, _, _) = open_core(&root.path().join("data"), &deep, Duration::ZERO);
    let name = format!("{}.bin", "n".repeat(100));
    upload(&core, "long", &[("f", &name, pattern_bytes(2000))]).await;
    assert!(deep.join(&name).to_string_lossy().len() > 259);
    assert_eq!(listing(&deep), std::slice::from_ref(&name), "the .part file was moved to its final name");
    assert_eq!(std::fs::read(deep.join(&name)).unwrap(), pattern_bytes(2000));
    assert!(kin_lib::motw::read(&deep.join(&name)).is_some(), "and carries the mark");
    // Two with one name: the second gets a suffix, through the same move.
    upload(&core, "long2", &[("f", &name, pattern_bytes(10))]).await;
    assert_eq!(listing(&deep).len(), 2);
}

// ----- the collision limit ------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_hundred_names_taken_fail_the_file_with_internal_and_leave_nothing_behind() {
    let h = harness();
    std::fs::create_dir_all(&h.recv).unwrap();
    std::fs::write(h.recv.join("dup.txt"), "0").unwrap();
    for n in 1..100 {
        std::fs::write(h.recv.join(format!("dup ({n}).txt")), "x").unwrap();
    }
    let before = listing(&h.recv);
    assert_eq!(before.len(), 100);
    let r = h.core.upload_post(PHONE, &request("full", vec![spec("f", "dup.txt", 5)])).await;
    assert_eq!(code_of(&r), "internal", "all 100 candidates are taken");
    assert_eq!(listing(&h.recv), before, "nothing was created, no .part file is left");
    // With one of them free it works, and it is the 100th candidate: ` (99)`.
    std::fs::remove_file(h.recv.join("dup (99).txt")).unwrap();
    assert!(h.core.upload_post(PHONE, &request("ok", vec![spec("f", "dup.txt", 5)])).await.is_ok());
    assert!(h.recv.join("dup (99).txt.part").exists(), "{:?}", listing(&h.recv));
    let names: HashSet<String> = listing(&h.recv).into_iter().collect();
    assert_eq!(names.len(), 100);
}
