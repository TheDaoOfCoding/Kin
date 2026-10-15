//! File transfers, milestone 1.3 (protocol.md, "Files: phone -> PC" and "Files: PC -> phone"): the state of every transfer, and
//! the phone -> PC side in full: `POST /v1/transfers`, `PUT .../files/{fileId}` and `DELETE`, streamed to `<name>.part` and moved
//! to the final name without overwriting anything. The PC -> phone side (offers, `GET`) is in `outgoing.rs`, and the HTTP
//! handlers are in `fileserver.rs`.
//!
//! **Rules this file keeps**
//!
//! * A file is never held in memory. The body of a `PUT` goes to disk in 1 MiB pieces, with one write in flight while the next
//!   piece is read from the network, and the control channel (the WebSocket) never waits for any of it.
//! * What the PC reports as "stored" is what it has written to the `.part` file. The database is brought up to date every 32
//!   MiB (after the data has been flushed to disk) and whenever a request ends, and the file is `fsync`ed before it gets its final
//!   name. After a restart a file counts as stored up to the smaller of the database value and the length on disk.
//! * One writer per file. A new request for a file whose old connection is still open (a phone that came back before the PC
//!   noticed the old connection was dead) stops the old writer first, so the offsets it answers are exact.
//! * Nothing is overwritten: the `.part` file is created with `CREATE_NEW` and moved with `MoveFileExW` without
//!   `MOVEFILE_REPLACE_EXISTING` (see `filenames.rs`).
//! * **Finishing a file does not depend on the request.** When the last byte is in, the `fsync`, the Mark of the Web and the move run in
//!   a task of their own, with the database told before (the name the file is being moved to) and after (the file is final). A phone
//!   that gives up waiting and asks again finds the file complete, never a second copy.
//! * **A transfer ID is never used twice.** The records of a transfer stay in the database in every state, a known finished ID answers
//!   for 24 hours and is then `404`, and a canceled or failed ID is `404` at once. A transfer ends once (`dropped`, `end_if_active`),
//!   whatever the order of a cancel, a `PUT` and the last file.
//! * **Only what the database assigns is deleted or adopted.** A `.part` file belongs to the transfer whose record names it; the names
//!   of unfinished transfers count as taken even when their file is missing, and the clean-up deletes only the paths of records.
//! * File and folder work runs on the blocking pool, never on an async thread.
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http_body_util::BodyExt;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, watch};

use crate::core::{Core, Event, lock, short, short_transfer};
use crate::encoding::unix_ms;
use crate::filenames;
use crate::motw;
use crate::server::ApiError;
use crate::storage::{TransferFileRow, TransferRow};

const HOUR_MS: u64 = 3_600_000;
/// Offers expire after this long, and a `.part` file with no progress is kept this long for a resume (protocol.md, architecture.md).
pub const KEEP_MS: u64 = 24 * HOUR_MS;
/// Most files one transfer may have (`413` above it).
pub const MAX_FILES: usize = 10_000;
/// The biggest single file the PC accepts (`413` above it). "What counts as too large is up to the PC."
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1 << 40;
/// Written to disk in pieces of this size.
const WRITE_PIECE: usize = 1 << 20;
/// The database and an `fsync` are brought up to date this often while a file is written.
const CHECKPOINT_BYTES: u64 = 32 << 20;
/// A writer that a newer request has told to stop still takes what is arriving: one frame of the body waits at most this long ...
const STOP_FRAME_WAIT: Duration = Duration::from_millis(30);
/// ... and the whole of it at most this long.
const STOP_GRACE: Duration = Duration::from_millis(150);
/// How many finished transfers `status.json` lists.
const RECENT_KEPT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Phone to PC: the PC receives.
    In,
    /// PC to phone: the PC serves.
    Out,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Done,
    Failed,
    Canceled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Canceled => "canceled",
        }
    }

    fn parse(s: &str) -> Status {
        match s {
            "done" => Status::Done,
            "failed" => Status::Failed,
            "canceled" => Status::Canceled,
            _ => Status::Active,
        }
    }
}

/// Moves bytes: the speed of a transfer over the last few seconds.
#[derive(Default)]
pub struct Speed {
    samples: VecDeque<(Instant, u64)>,
    bps: f64,
}

impl Speed {
    /// `total` is the number of bytes moved so far in this session.
    pub fn add(&mut self, now: Instant, total: u64) {
        self.samples.push_back((now, total));
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) > Duration::from_secs(3) {
            self.samples.pop_front();
        }
        if let (Some(first), Some(last)) = (self.samples.front(), self.samples.back()) {
            let span = last.0.duration_since(first.0).as_secs_f64();
            if span >= 0.4 {
                self.bps = (last.1.saturating_sub(first.1)) as f64 / span;
            }
        }
    }

    /// Zero once nothing has moved for two seconds.
    pub fn bps(&self, now: Instant) -> f64 {
        match self.samples.back() {
            Some((at, _)) if now.duration_since(*at) < Duration::from_secs(2) => self.bps,
            _ => 0.0,
        }
    }
}

pub struct Meta {
    pub status: Status,
    pub finished_ms: Option<u64>,
    pub error: Option<String>,
    pub speed: Speed,
}

/// What can change about a file while it is being received or served.
#[derive(Clone, Debug)]
pub struct FileData {
    /// In: the name the PC chose (the `.part` file is `<name>.part`, and after the move the file is `<name>`). Out: the name the
    /// phone is told.
    pub name: String,
    /// In: the path of the `.part` file, then of the final file. Out: the file that is served.
    pub path: PathBuf,
    pub finalized: bool,
    /// Out: the modified time of the source when it was offered (Unix ms). A change makes the `GET` answer `409`.
    pub source_mtime_ms: Option<i64>,
    /// Out: the source is a temporary file of ours (the `.txt` of a long text), deleted when the transfer ends.
    pub temp_source: bool,
}

pub struct FileShared {
    pub file_id: String,
    pub idx: usize,
    pub size: u64,
    pub mime: String,
    pub data: Mutex<FileData>,
    /// In: the bytes written to the `.part` file (the whole size once finalized). Out: how far the phone has got.
    pub done: AtomicU64,
    /// Held by the one writer of the file.
    pub writer: AsyncMutex<()>,
    /// Held while the file is being finished (the move to its final name), and by a cancel that waits for it.
    pub finalizer: AsyncMutex<()>,
    /// Bumped to tell the current writer to stop.
    pub stop: watch::Sender<u64>,
}

/// A file of an offer (`done` starts at 0).
pub fn file_shared(file_id: &str, idx: usize, size: u64, mime: &str, data: FileData) -> Arc<FileShared> {
    FileShared::new(file_id, idx, size, mime, data, 0)
}

impl FileShared {
    fn new(file_id: &str, idx: usize, size: u64, mime: &str, data: FileData, done: u64) -> Arc<FileShared> {
        Arc::new(FileShared {
            file_id: file_id.to_string(),
            idx,
            size,
            mime: mime.to_string(),
            data: Mutex::new(data),
            done: AtomicU64::new(done),
            writer: AsyncMutex::new(()),
            finalizer: AsyncMutex::new(()),
            stop: watch::channel(0).0,
        })
    }

    pub fn data(&self) -> FileData {
        lock(&self.data).clone()
    }

    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// Tells the current writer to stop (it checkpoints and returns). The number returned is the caller's own: a writer stops when
    /// a bigger one has been asked for, so a request that asks and then waits for its turn is never stopped by its own ask.
    pub fn ask_to_stop(&self) -> u64 {
        let mut generation = 0;
        self.stop.send_modify(|n| {
            *n += 1;
            generation = *n;
        });
        generation
    }
}

pub struct Entry {
    pub id: String,
    pub device_id: String,
    pub direction: Direction,
    pub created_ms: u64,
    /// Out: when the offer expires (24 hours after it was sent; a send into a dead connection that is repeated after the reconnect
    /// starts the 24 hours again, see `queue.rs`). In: not used.
    expires: AtomicU64,
    /// In: the folder the files go to.
    pub dest_dir: PathBuf,
    pub files: Vec<Arc<FileShared>>,
    pub meta: Mutex<Meta>,
    /// Bytes moved in this session (a resume doesn't count what was there before), for the speed.
    pub moved: AtomicU64,
    /// Set when the transfer is canceled: streams of a `GET` stop, writers stop.
    pub canceled: AtomicBool,
    /// Set by the one call of `drop_transfer` that does the work (a second cancel, from the other side, finds it set and does nothing).
    pub dropped: AtomicBool,
    /// In: the phone had to ask again (a `POST` for a transfer that was already there), so it was not a first-try success (metrics.rs).
    pub resumed: AtomicBool,
}

impl Entry {
    pub fn new(
        id: &str,
        device_id: &str,
        direction: Direction,
        now_ms: u64,
        expires_ms: u64,
        dest_dir: PathBuf,
        files: Vec<Arc<FileShared>>,
    ) -> Arc<Entry> {
        Arc::new(Entry {
            id: id.to_string(),
            device_id: device_id.to_string(),
            direction,
            created_ms: now_ms,
            expires: AtomicU64::new(expires_ms),
            dest_dir,
            files,
            meta: Mutex::new(Meta { status: Status::Active, finished_ms: None, error: None, speed: Speed::default() }),
            moved: AtomicU64::new(0),
            canceled: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
            resumed: AtomicBool::new(false),
        })
    }

    pub fn status(&self) -> Status {
        lock(&self.meta).status
    }

    /// When the offer expires, in Unix milliseconds (`u64::MAX` for an upload).
    pub fn expires_ms(&self) -> u64 {
        self.expires.load(Ordering::Relaxed)
    }

    pub fn set_expires_ms(&self, at: u64) {
        self.expires.store(at, Ordering::Relaxed);
    }

    /// Whether a transfer that is over has been known for as long as the PC answers for it (24 hours since it ended): its ID is then
    /// `404`, and its record stays in the database for good.
    pub fn past_keeping(&self, now_ms: u64) -> bool {
        let meta = lock(&self.meta);
        meta.status != Status::Active && meta.finished_ms.is_some_and(|f| now_ms.saturating_sub(f) >= KEEP_MS)
    }

    /// Ends the transfer with `status` if it is still going. Exactly one caller gets `true`, so what follows (the toast of a finished
    /// upload, say) happens once however the last file and a cancel are ordered.
    fn end_if_active(&self, status: Status, now_ms: u64, error: Option<&str>) -> bool {
        let mut meta = lock(&self.meta);
        if meta.status != Status::Active {
            return false;
        }
        meta.status = status;
        meta.finished_ms = Some(now_ms);
        meta.error = error.map(str::to_string);
        true
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn file(&self, file_id: &str) -> Option<&Arc<FileShared>> {
        self.files.iter().find(|f| f.file_id == file_id)
    }

    /// Notes that `n` more bytes moved, for the speed.
    pub fn note_moved(&self, n: u64) {
        let total = self.moved.fetch_add(n, Ordering::Relaxed) + n;
        lock(&self.meta).speed.add(Instant::now(), total);
    }

    fn set_status(&self, status: Status, now_ms: u64, error: Option<&str>) {
        let mut meta = lock(&self.meta);
        meta.status = status;
        meta.finished_ms = (status != Status::Active).then_some(now_ms);
        meta.error = error.map(str::to_string);
    }
}

/// The transfers the app knows about, by ID. Uploads and offers share one ID space.
#[derive(Default)]
pub struct Transfers {
    map: Mutex<HashMap<String, Arc<Entry>>>,
    /// Held while a new upload is created, so two identical `POST`s that arrive together (a phone that retries before the first answer
    /// came back) make one transfer and not two.
    creating: Mutex<()>,
}

impl Transfers {
    pub fn get(&self, id: &str) -> Option<Arc<Entry>> {
        lock(&self.map).get(id).cloned()
    }

    pub fn insert(&self, entry: Arc<Entry>) {
        lock(&self.map).insert(entry.id.clone(), entry);
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Entry>> {
        lock(&self.map).remove(id)
    }

    pub fn all(&self) -> Vec<Arc<Entry>> {
        lock(&self.map).values().cloned().collect()
    }

    pub fn active_count(&self) -> usize {
        lock(&self.map).values().filter(|e| e.status() == Status::Active).count()
    }

    /// The names (folded) that unfinished uploads into `dir` hold, whether or not their `.part` file is there at this moment: a file
    /// the user deleted, or one that is being moved, keeps its name. `skip` is the file (`(transfer ID, file ID)`) that asks.
    pub fn claimed_names(&self, dir: &Path, skip: Option<(&str, &str)>) -> HashSet<String> {
        let mut names = HashSet::new();
        for entry in lock(&self.map).values() {
            if entry.direction != Direction::In || entry.dest_dir != dir {
                continue;
            }
            for file in &entry.files {
                if skip == Some((entry.id.as_str(), file.file_id.as_str())) {
                    continue;
                }
                let data = file.data();
                if !data.finalized {
                    names.insert(filenames::fold(&data.name));
                }
            }
        }
        names
    }

    /// The `.part` paths of unfinished uploads (lower case), for the clean-up.
    fn live_part_paths(&self) -> HashSet<String> {
        let mut paths = HashSet::new();
        for entry in lock(&self.map).values().filter(|e| e.direction == Direction::In && !e.dropped.load(Ordering::Relaxed)) {
            for file in entry.files.iter().map(|f| f.data()).filter(|d| !d.finalized) {
                paths.insert(file.path.to_string_lossy().to_lowercase());
            }
        }
        paths
    }
}

/// Settings of the transfers that tests and developers can change.
pub struct Hooks {
    /// Pretends the disk has this many free bytes (the developer hook of the spec: `--dev-free-bytes`).
    pub free_bytes: Mutex<Option<u64>>,
    pub max_file_bytes: AtomicU64,
    /// Every request about files answers `500 internal` (the `internal-error` vector).
    pub fail_internal: AtomicBool,
    /// Test hook: finishing a file (the move to its final name) waits this many milliseconds first, so a test can drop the request or
    /// change the file while the finish is under way. Zero (the default) outside tests.
    pub finalize_delay_ms: AtomicU64,
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks {
            free_bytes: Mutex::new(None),
            max_file_bytes: AtomicU64::new(DEFAULT_MAX_FILE_BYTES),
            fail_internal: AtomicBool::new(false),
            finalize_delay_ms: AtomicU64::new(0),
        }
    }
}

/// An HTTP-level failure: the code, and the message of the body.
#[derive(Debug)]
pub struct Failure {
    pub error: ApiError,
    pub message: String,
    /// `offset_mismatch` carries the actual offset.
    pub offset: Option<u64>,
}

impl Failure {
    pub fn new(error: ApiError, message: &str) -> Failure {
        Failure { error, message: message.to_string(), offset: None }
    }

    pub fn not_found() -> Failure {
        Failure::new(ApiError::NotFound, "Transfer not found.")
    }

    pub fn internal() -> Failure {
        Failure::new(ApiError::Internal, "Internal error. Check the logs.")
    }
}

#[derive(Clone)]
pub struct FileSpec {
    pub file_id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

pub struct UploadRequest {
    pub id: String,
    pub files: Vec<FileSpec>,
}

/// What a `PUT` carries besides the body: the raw `offset` text and the `Content-Length` (`None` when it is missing or isn't a
/// number, which is the same `400`).
pub struct PutParams {
    pub offset: Option<String>,
    pub content_length: Option<u64>,
}

/// ASCII digits only, no sign, no leading zeros, at most 2^53 - 1 (protocol.md, "Digits").
pub fn parse_strict_u64(text: &str) -> Option<u64> {
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let no_leading_zero = text == "0" || !text.starts_with('0');
    (digits && no_leading_zero && text.len() <= 16).then(|| text.parse::<u64>().ok()).flatten().filter(|n| *n <= crate::json::MAX_SAFE_INT)
}

/// For `status.json` and the drop window.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferView {
    pub id: String,
    pub direction: &'static str,
    pub peer: String,
    pub status: &'static str,
    pub files: usize,
    pub files_done: usize,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub error: Option<String>,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    /// The first few names (the names the files have, or will have, on the PC; the names offered, for an offer).
    pub names: Vec<String>,
    pub cancelable: bool,
}

/// How much of a cancel reason is kept (characters).
const MAX_REASON_CHARS: usize = 32;

/// The reason of a `cancel` message, made safe to log and store: control characters and bidi controls removed, at most 32 characters.
/// (It comes from the phone. A log line made of it is escaped with `{:?}` as well.)
pub fn sanitize_reason(reason: &str) -> String {
    reason
        .chars()
        .filter(|c| !c.is_control() && !filenames::is_bidi_control(*c))
        .take(MAX_REASON_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Removes the partial files of a received transfer and the temporary source of an offer. Returns the IDs of the files whose `.part`
/// file is gone (removed, or already missing); one that couldn't be removed (a scanner holds it) stays recorded, and the clean-up tries again.
fn drop_parts(entry: &Entry) -> Vec<String> {
    let mut gone = Vec::new();
    for file in &entry.files {
        let data = file.data();
        if entry.direction == Direction::In && !data.finalized {
            match std::fs::remove_file(&data.path) {
                Ok(()) => gone.push(file.file_id.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => gone.push(file.file_id.clone()),
                Err(e) => log::warn!("removing a partial file of {} failed: {e}", short_transfer(&entry.id)),
            }
        }
        if data.temp_source {
            remove_temp_source(&data.path);
        }
    }
    gone
}

/// A temporary source file lives in its own folder under `outgoing\`; both go.
pub fn remove_temp_source(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent()
        && dir.parent().and_then(Path::file_name).is_some_and(|n| n == "outgoing")
    {
        let _ = std::fs::remove_dir(dir);
    }
}

impl Core {
    // ----- where things go ---------------------------------------------------------------------------------------------

    /// The folder received files go to: the developer override, `receivedFolder` of `settings.json`, or `Downloads\Kin`.
    pub fn received_dir(&self) -> PathBuf {
        if let Some(dir) = &self.config.dev_received_dir {
            return dir.clone();
        }
        if let Some(dir) = lock(&self.settings).received_folder.clone().filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        crate::paths::downloads_dir().join("Kin")
    }

    /// Free space on the disk of `dir`, or the developer hook's number.
    pub fn free_space(&self, dir: &Path) -> u64 {
        if let Some(n) = *lock(&self.hooks.free_bytes) {
            return n;
        }
        crate::paths::free_bytes(dir).unwrap_or(u64::MAX)
    }

    pub fn set_fake_free_bytes(&self, free: Option<u64>) {
        *lock(&self.hooks.free_bytes) = free;
    }

    fn internal_hook(&self) -> Result<(), Failure> {
        if self.hooks.fail_internal.load(Ordering::Relaxed) { Err(Failure::internal()) } else { Ok(()) }
    }

    /// The free-space check of the spec, with no reserve: it passes when the missing bytes fit.
    fn check_space(&self, dir: &Path, missing: u64) -> Result<(), Failure> {
        if missing > 0 && missing > self.free_space(dir) {
            log::warn!("not enough free space for {missing} bytes in {}", dir.display());
            return Err(Failure::new(ApiError::DiskFull, "Not enough free space on the PC."));
        }
        Ok(())
    }

    fn now_ms() -> u64 {
        unix_ms()
    }

    // ----- loading what the last run left ------------------------------------------------------------------------------

    /// Reads the transfers that were going (and the ones that finished less than a day ago) from the database. A received file
    /// counts as stored up to the smaller of the database value and what is on the disk. A file whose move to its final name was under
    /// way when the app stopped is finished here if the move happened (the `.part` file is gone and the file is there, whole).
    pub fn recover_transfers(&self) {
        let now = Core::now_ms();
        let rows = match self.db.load_open_transfers(now.saturating_sub(KEEP_MS) as i64) {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("reading the transfers failed: {e}");
                return;
            }
        };
        for (row, files) in rows {
            let direction = if row.direction == "out" { Direction::Out } else { Direction::In };
            let status = Status::parse(&row.status);
            let dest = PathBuf::from(row.dest_dir.clone().unwrap_or_default());
            let shared: Vec<Arc<FileShared>> = files
                .iter()
                .map(|f| {
                    let mut path = PathBuf::from(f.local_path.clone().unwrap_or_default());
                    let mut name = f.name.clone();
                    let mut finalized = f.finalized;
                    let size = f.size as u64;
                    if !finalized
                        && direction == Direction::In
                        && let Some(target) = f.finalizing_name.as_deref()
                    {
                        let final_path = dest.join(target);
                        let moved = std::fs::metadata(&path).is_err()
                            && std::fs::metadata(&final_path).is_ok_and(|m| m.is_file() && m.len() == size);
                        if moved {
                            log::info!(
                                "upload {}: file {} had been moved to its final name before the restart",
                                short_transfer(&row.id),
                                f.idx + 1
                            );
                            name = target.to_string();
                            path = final_path;
                            finalized = true;
                            let _ = self.db.set_file_final(&row.id, &f.file_id, &name, &path.to_string_lossy(), size);
                        }
                    }
                    let done = if finalized {
                        size
                    } else if direction == Direction::In {
                        let on_disk = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        (f.bytes_done as u64).min(on_disk).min(size)
                    } else {
                        f.bytes_done as u64
                    };
                    let data = FileData { name, path, finalized, source_mtime_ms: f.source_mtime_ms, temp_source: f.temp_source };
                    FileShared::new(&f.file_id, f.idx as usize, size, &f.mime, data, done)
                })
                .collect();
            let expires = row.expires_at.map_or(u64::MAX, |e| e as u64);
            let entry = Entry::new(&row.id, &row.device_id, direction, row.created_at as u64, expires, dest, shared);
            entry.set_status(status, row.finished_at.map_or(now, |f| f as u64), row.error_code.as_deref());
            self.transfers.insert(entry);
        }
        let open = self.transfers.all().len();
        if open > 0 {
            log::info!("{open} transfer(s) loaded from the database");
        }
    }

    /// The upload `transfer_id` of `device_id`, as long as the PC answers for it: a finished one for 24 hours, then it is gone.
    fn find_upload(&self, device_id: &str, transfer_id: &str) -> Option<Arc<Entry>> {
        let entry = self.transfers.get(transfer_id).filter(|e| e.device_id == device_id && e.direction == Direction::In)?;
        (!entry.past_keeping(Core::now_ms())).then_some(entry)
    }

    /// `check_space` on the blocking pool (it asks the disk).
    async fn check_space_async(self: &Arc<Self>, dir: &Path, missing: u64) -> Result<(), Failure> {
        let (core, dir) = (self.clone(), dir.to_path_buf());
        tokio::task::spawn_blocking(move || core.check_space(&dir, missing)).await.unwrap_or_else(|_| Err(Failure::internal()))
    }

    // ----- POST /v1/transfers ------------------------------------------------------------------------------------------

    /// `POST /v1/transfers`: a new upload, or a resume of one the device already started with the same `fileId`s. Answers, for
    /// every file in the order of the request, how many bytes the PC has. An ID that was used before and is over (finished more than a
    /// day ago, canceled, failed) is `404`: an ID is never used twice.
    pub async fn upload_post(self: &Arc<Self>, device_id: &str, req: &UploadRequest) -> Result<Vec<(String, u64)>, Failure> {
        self.internal_hook()?;
        let existing = self.transfers.get(&req.id);
        let entry = match existing {
            // Another device's transfer, an offer, or one that ended more than a day ago: not found, and nothing is created.
            Some(e) if e.device_id != device_id || e.direction == Direction::Out || e.past_keeping(Core::now_ms()) => {
                return Err(Failure::not_found());
            }
            Some(e) => {
                e.resumed.store(true, Ordering::Relaxed);
                let have: HashSet<&str> = e.files.iter().map(|f| f.file_id.as_str()).collect();
                let want: HashSet<&str> = req.files.iter().map(|f| f.file_id.as_str()).collect();
                if have != want {
                    return Err(Failure::new(ApiError::BadRequest, "The transfer already exists with different files."));
                }
                // A resumed POST is checked again, on the stored sizes: names, sizes and types in the request are ignored.
                self.check_too_large(e.files.iter().map(|f| f.size))?;
                self.stop_writers(&e).await;
                let mut missing = 0u64;
                for file in &e.files {
                    missing += file.size.saturating_sub(self.stored_now(&e, file).await);
                }
                self.check_space_async(&e.dest_dir, missing).await?;
                // Anything that was complete but never got its final name (the move was cut short).
                for file in &e.files {
                    let _writer = file.writer.lock().await;
                    if file.done() == file.size && !file.data().finalized && e.status() == Status::Active {
                        self.finalize_file(&e, file, None).await?;
                    }
                }
                // Every file has its final name but the transfer was never marked done (the app stopped in between).
                if e.status() == Status::Active && e.files.iter().all(|f| f.data().finalized) {
                    self.upload_done(&e);
                }
                e
            }
            None => self.create_upload(device_id, req.id.as_str(), &req.files).await?,
        };
        let answer = req
            .files
            .iter()
            .map(|spec| {
                let file = entry.file(&spec.file_id).expect("the file IDs were checked");
                (spec.file_id.clone(), if file.data().finalized { file.size } else { file.done() })
            })
            .collect();
        Ok(answer)
    }

    fn check_too_large(&self, sizes: impl Iterator<Item = u64>) -> Result<(), Failure> {
        let max = self.hooks.max_file_bytes.load(Ordering::Relaxed);
        let (mut count, mut too_big) = (0usize, false);
        for s in sizes {
            count += 1;
            too_big |= s > max;
        }
        if too_big || count > MAX_FILES {
            return Err(Failure::new(ApiError::TooLarge, "The transfer is over the size limit."));
        }
        Ok(())
    }

    /// A new upload: the folder, the `.part` files and the records. On the blocking pool (up to 10,000 files are created).
    async fn create_upload(self: &Arc<Self>, device_id: &str, id: &str, specs: &[FileSpec]) -> Result<Arc<Entry>, Failure> {
        let core = self.clone();
        let (device_id, id, specs) = (device_id.to_string(), id.to_string(), specs.to_vec());
        tokio::task::spawn_blocking(move || core.create_upload_blocking(&device_id, &id, &specs))
            .await
            .unwrap_or_else(|_| Err(Failure::internal()))
    }

    fn create_upload_blocking(&self, device_id: &str, id: &str, specs: &[FileSpec]) -> Result<Arc<Entry>, Failure> {
        let _creating = lock(&self.transfers.creating);
        if let Some(known) = self.transfers.get(id) {
            // Two identical POSTs that arrive together make one transfer; anything else with a known ID is not found.
            return if known.device_id == device_id && known.direction == Direction::In && !known.past_keeping(Core::now_ms()) {
                Ok(known)
            } else {
                Err(Failure::not_found())
            };
        }
        // An ID that was used and is over (finished and past its day, canceled, failed) is never used again.
        match self.db.transfer_exists(id) {
            Ok(false) => {}
            Ok(true) => {
                log::info!("upload {}: that ID was used before, answered 404", short_transfer(id));
                return Err(Failure::not_found());
            }
            Err(e) => {
                log::error!("reading the transfers failed: {e}");
                return Err(Failure::internal());
            }
        }
        self.check_too_large(specs.iter().map(|s| s.size))?;
        let dir = self.received_dir();
        self.check_space(&dir, specs.iter().map(|s| s.size).sum())?;
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error!("creating the folder for received files {} failed: {e}", dir.display());
            return Err(Failure::internal());
        }
        // The names taken: what is in the folder (a `.part` file claims its final name), what the unfinished uploads hold even when their
        // file is missing, and, as they are chosen, this transfer's own.
        let mut taken = filenames::existing_names(&dir);
        taken.extend(self.transfers.claimed_names(&dir, None));
        let mut files = Vec::with_capacity(specs.len());
        for (idx, spec) in specs.iter().enumerate() {
            match filenames::create_new(&spec.name, &mut taken, |name| filenames::create_part(&dir, name)) {
                Ok((name, handle)) => {
                    drop(handle);
                    let data = FileData {
                        name: name.clone(),
                        path: filenames::part_path(&dir, &name),
                        finalized: false,
                        source_mtime_ms: None,
                        temp_source: false,
                    };
                    files.push(FileShared::new(&spec.file_id, idx, spec.size, &spec.mime, data, 0));
                }
                Err(reason) => {
                    log::error!("creating the file for {:?} failed: {reason}", spec.file_id);
                    for made in &files {
                        let _ = std::fs::remove_file(&made.data().path);
                    }
                    return Err(Failure::internal());
                }
            }
        }
        let now = Core::now_ms();
        let entry = Entry::new(id, device_id, Direction::In, now, u64::MAX, dir.clone(), files);
        if let Err(e) = self.db.insert_transfer(&row_of(&entry, now), &file_rows(&entry)) {
            log::error!("saving the transfer failed: {e}");
        }
        log::info!(
            "upload {} from device {}: {} file(s), {} bytes, into {}",
            short_transfer(id),
            short(device_id),
            entry.files.len(),
            entry.total_bytes(),
            dir.display()
        );
        self.transfers.insert(entry.clone());
        Ok(entry)
    }

    /// What is stored for a file, with the disk as the check (on the blocking pool).
    async fn stored_now(self: &Arc<Self>, entry: &Arc<Entry>, file: &Arc<FileShared>) -> u64 {
        if file.data().finalized {
            return file.size;
        }
        let (core, entry, file) = (self.clone(), entry.clone(), file.clone());
        tokio::task::spawn_blocking(move || core.stored_now_blocking(&entry, &file)).await.unwrap_or(0)
    }

    /// What is stored for a file, with the disk as the check: the `.part` file the record names is the file, and what it holds up to
    /// the recorded count is what counts. If it has gone missing it is created again (empty, under a free name).
    fn stored_now_blocking(&self, entry: &Entry, file: &FileShared) -> u64 {
        let data = file.data();
        if data.finalized {
            return file.size;
        }
        match std::fs::metadata(&data.path) {
            Ok(m) => {
                let stored = file.done().min(m.len()).min(file.size);
                file.done.store(stored, Ordering::Relaxed);
                stored
            }
            Err(_) => {
                file.done.store(0, Ordering::Relaxed);
                self.recreate_part(entry, file);
                0
            }
        }
    }

    /// The `.part` file of a file that has none (the user deleted it, or the disk was changed): a new one, under the name the record
    /// has. Another unfinished upload can't have taken that name, because every unfinished upload's name counts as taken.
    fn recreate_part(&self, entry: &Entry, file: &FileShared) {
        let data = file.data();
        let mut taken = filenames::existing_names(&entry.dest_dir);
        taken.extend(self.transfers.claimed_names(&entry.dest_dir, Some((&entry.id, &file.file_id))));
        let created = filenames::create_new(&data.name, &mut taken, |name| filenames::create_part(&entry.dest_dir, name));
        match created {
            Ok((name, handle)) => {
                drop(handle);
                let path = filenames::part_path(&entry.dest_dir, &name);
                log::warn!("the .part file of {} was missing: created a new one", short_transfer(&entry.id));
                {
                    let mut d = lock(&file.data);
                    d.name = name.clone();
                    d.path = path.clone();
                }
                let _ = self.db.set_file_part(&entry.id, &file.file_id, &name, &path.to_string_lossy(), 0);
            }
            Err(reason) => log::error!("creating the .part file of {} again failed: {reason}", short_transfer(&entry.id)),
        }
    }

    /// Stops the writers of every file of the transfer and waits until each has let go.
    async fn stop_writers(&self, entry: &Entry) {
        for file in &entry.files {
            file.ask_to_stop();
        }
        for file in &entry.files {
            let _guard = file.writer.lock().await;
        }
    }

    // ----- PUT /v1/transfers/{id}/files/{fileId} ----------------------------------------------------------------------

    /// `PUT`: the order of checks of protocol.md. (`401` was the caller's.) The body is streamed to the `.part` file. Returns the
    /// size of the file, which the handler answers as `{ "received": size }`. It answers as soon as the file has its final name:
    /// the toast and the clipboard come after, in a task of their own.
    ///
    /// **The work runs in a task of its own.** When the phone's connection breaks, the server drops the handler (this future) at
    /// whatever it is waiting for; the task is not dropped with it, so it reads what has arrived, writes it, records how far the file
    /// is and lets go of the file, exactly as if the connection had broken while it was reading. Nothing the PC has received is lost
    /// because the request went away at the wrong moment.
    pub async fn upload_put(
        self: &Arc<Self>,
        device_id: &str,
        transfer_id: &str,
        file_id: &str,
        params: PutParams,
        body: axum::body::Body,
    ) -> Result<u64, Failure> {
        let (core, device_id, transfer_id, file_id) = (self.clone(), device_id.to_string(), transfer_id.to_string(), file_id.to_string());
        tokio::spawn(async move { core.upload_put_task(&device_id, &transfer_id, &file_id, params, body).await }).await.unwrap_or_else(
            |e| {
                log::error!("the task of a PUT failed: {e}");
                Err(Failure::internal())
            },
        )
    }

    async fn upload_put_task(
        self: &Arc<Self>,
        device_id: &str,
        transfer_id: &str,
        file_id: &str,
        params: PutParams,
        mut body: axum::body::Body,
    ) -> Result<u64, Failure> {
        self.internal_hook()?;
        let entry = self.find_upload(device_id, transfer_id).ok_or_else(Failure::not_found)?;
        let file = entry.file(file_id).cloned().ok_or_else(|| Failure::new(ApiError::NotFound, "File not found."))?;
        let bad = |m: &str| Failure::new(ApiError::BadRequest, m);
        let offset = params.offset.as_deref().and_then(parse_strict_u64).ok_or_else(|| bad("Missing or invalid offset."))?;
        let length = params.content_length.ok_or_else(|| bad("Content-Length is required."))?;

        // One writer per file. If an older request is still writing (its connection is not known to be dead yet), ask it to stop and
        // take its place: `generation` is mine, and a writer stops when a bigger one has been asked for.
        let generation = file.ask_to_stop();
        let _writer = file.writer.lock().await;
        if !matches!(entry.status(), Status::Active | Status::Done) || entry.canceled.load(Ordering::Relaxed) {
            return Err(Failure::not_found());
        }
        let stored = self.stored_now(&entry, &file).await;
        if offset != stored {
            return Err(offset_mismatch(stored));
        }
        if length != file.size - offset {
            return Err(bad("Content-Length must equal size minus offset."));
        }
        let missing = file.size - stored;
        self.check_space_async(&entry.dest_dir, missing).await?;
        if stored > 0 && missing > 0 {
            log::info!("upload {}: file {} continues from byte {} of {}", short_transfer(&entry.id), file.idx + 1, stored, file.size);
        }

        let mut handle = None;
        if missing > 0 && !file.data().finalized {
            let path = file.data().path;
            let opened = tokio::task::spawn_blocking({
                let path = path.clone();
                move || {
                    std::fs::OpenOptions::new().write(true).open(&path).and_then(|mut f| {
                        // Anything past the stored bytes (left behind by a crash) is overwritten.
                        f.set_len(stored)?;
                        f.seek(SeekFrom::Start(stored))?;
                        Ok(f)
                    })
                }
            })
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e)));
            match opened {
                Ok(f) => handle = Some(f),
                Err(e) => {
                    // The path of a `.part` file has the file's name in it: the log says which transfer and file number, not which file.
                    log::error!("upload {}: opening the .part file of file {} failed: {e}", short_transfer(&entry.id), file.idx + 1);
                    return Err(Failure::internal());
                }
            }
            let outcome = self.write_body(&entry, &file, handle.take().expect("opened"), &mut body, missing, generation).await;
            handle = outcome.handle;
            match outcome.end {
                WriteEnd::Complete => {}
                WriteEnd::Stopped => return Err(offset_mismatch(file.done())),
                WriteEnd::BodyError => {
                    log::info!(
                        "upload {}: the connection broke during {:?}; {} of {} bytes are kept",
                        short_transfer(&entry.id),
                        file.file_id,
                        file.done(),
                        file.size
                    );
                    return Err(bad("The request body ended early."));
                }
                WriteEnd::DiskError(e) if e.kind() == std::io::ErrorKind::StorageFull => {
                    log::warn!("upload {}: the disk is full", short_transfer(&entry.id));
                    return Err(Failure::new(ApiError::DiskFull, "Not enough free space on the PC."));
                }
                WriteEnd::DiskError(e) => {
                    log::error!("upload {}: writing {:?} failed: {e}", short_transfer(&entry.id), file.file_id);
                    return Err(Failure::internal());
                }
            }
        }
        if file.done() == file.size && !file.data().finalized {
            self.finalize_file(&entry, &file, handle).await?;
        }
        // The answer says the file is complete: that is only true if every byte is stored and the file has its final name.
        if file.done() != file.size || !file.data().finalized {
            log::error!(
                "upload {}: file {} answered complete but {} of {} bytes are stored",
                short_transfer(&entry.id),
                file.idx + 1,
                file.done(),
                file.size
            );
            return Err(Failure::internal());
        }
        Ok(file.size)
    }

    /// Streams `body` into the file. The body is cut into pieces of 1 MiB, and one write is in flight while the next piece is
    /// read, so the network and the disk overlap and nothing bigger than two pieces is ever held. Stops early when a newer request
    /// for the file has asked for it (`generation`) or when the body ends early.
    async fn write_body(
        &self,
        entry: &Arc<Entry>,
        file: &Arc<FileShared>,
        handle: std::fs::File,
        body: &mut axum::body::Body,
        missing: u64,
        generation: u64,
    ) -> WriteOutcome {
        let mut stop = file.stop.subscribe();
        let mut w = Pipeline {
            core: self,
            entry,
            file,
            handle: Some(handle),
            pending: None,
            in_flight: 0,
            buffer: Vec::with_capacity(WRITE_PIECE),
            spare: Vec::new(),
            since_checkpoint: 0,
        };
        let mut taken = 0u64; // accepted from the body: written, in flight or buffered
        let end = loop {
            if taken >= missing {
                break WriteEnd::Complete;
            }
            if *stop.borrow() > generation {
                // A newer request wants the file. What has reached the PC, or is on its way in this instant, is kept (a phone that came
                // back on a new connection does not send it again): a frame is waited for at most 30 ms, and the whole grace is 150 ms,
                // so a connection that is dead or has gone quiet is let go at once and a live one can't hold the new request for long.
                let (mut failed, grace_ends) = (None, Instant::now() + STOP_GRACE);
                while taken < missing {
                    let wait = STOP_FRAME_WAIT.min(grace_ends.saturating_duration_since(Instant::now()));
                    match tokio::time::timeout(wait, body.frame()).await {
                        Ok(Some(Ok(frame))) => {
                            if let Some(data) = frame.data_ref()
                                && let Err(e) = w.accept(data, missing, &mut taken).await
                            {
                                failed = Some(e);
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                break failed.map_or(WriteEnd::Stopped, WriteEnd::DiskError);
            }
            tokio::select! {
                frame = body.frame() => match frame {
                    Some(Ok(frame)) => {
                        if let Some(data) = frame.data_ref()
                            && let Err(e) = w.accept(data, missing, &mut taken).await
                        {
                            break WriteEnd::DiskError(e);
                        }
                    }
                    Some(Err(_)) | None => break WriteEnd::BodyError,
                },
                _ = stop.changed() => {}
            }
        };
        // Whatever the end, what was received is written, the last write is waited for, and it is flushed and recorded: a resume
        // starts from there.
        let end = match w.finish().await {
            Ok(()) => end,
            Err(e) => WriteEnd::DiskError(e),
        };
        w.sync_and_record().await;
        WriteOutcome { handle: w.handle.take(), end }
    }

    /// Records progress in the database, after the data was flushed (`sync_data`).
    fn checkpoint(&self, entry: &Entry, file: &FileShared) {
        let _ = self.db.set_file_progress(&entry.id, &file.file_id, file.done());
        let _ = self.db.touch_transfer(&entry.id, Core::now_ms() as i64);
    }

    /// The last byte is in: `fsync`, the Mark of the Web, the move to the final name without replacing anything, and the record. When
    /// every file of the transfer is done the transfer is done.
    ///
    /// The work runs in a task of its own, so it finishes even if the request that asked is dropped (a phone that gave up waiting); the
    /// request only waits for it. A second caller (the phone's retry) waits for the first and finds the file final. Called with the
    /// file's writer lock held, or from the resume path, which holds it too.
    async fn finalize_file(
        self: &Arc<Self>,
        entry: &Arc<Entry>,
        file: &Arc<FileShared>,
        handle: Option<std::fs::File>,
    ) -> Result<(), Failure> {
        let (core, entry, file) = (self.clone(), entry.clone(), file.clone());
        tokio::spawn(async move { core.finalize_task(&entry, &file, handle).await }).await.unwrap_or_else(|e| {
            log::error!("the task that finishes a file failed: {e}");
            Err(Failure::internal())
        })
    }

    async fn finalize_task(
        self: &Arc<Self>,
        entry: &Arc<Entry>,
        file: &Arc<FileShared>,
        handle: Option<std::fs::File>,
    ) -> Result<(), Failure> {
        // One at a time per file; a cancel waits for this lock too, and a file of a canceled transfer is not finished.
        let _once = file.finalizer.lock().await;
        if file.data().finalized {
            return Ok(());
        }
        if entry.canceled.load(Ordering::Relaxed) {
            return Err(Failure::not_found());
        }
        let claimed = file.data().name;
        let (core, dir, transfer_id, file_id, size) =
            (self.clone(), entry.dest_dir.clone(), entry.id.clone(), file.file_id.clone(), file.size);
        let moved = tokio::task::spawn_blocking(move || core.finish_blocking(&dir, &transfer_id, &file_id, &claimed, size, handle))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let name = match moved {
            Ok(name) => name,
            Err(reason) => {
                log::error!("moving the .part file of {} to its final name failed: {reason}", short_transfer(&entry.id));
                return Err(Failure::internal());
            }
        };
        let path = entry.dest_dir.join(&name);
        {
            let mut d = lock(&file.data);
            d.name = name.clone();
            d.path = path.clone();
            d.finalized = true;
        }
        file.done.store(file.size, Ordering::Relaxed);
        let _ = self.db.set_file_final(&entry.id, &file.file_id, &name, &path.to_string_lossy(), file.size);
        log::info!("upload {}: file {} is complete ({} bytes)", short_transfer(&entry.id), file.idx + 1, file.size);
        if entry.files.iter().all(|f| f.data().finalized) {
            self.upload_done(entry);
        }
        Ok(())
    }

    /// The blocking part of finishing a file: `fsync`, the length check (the `.part` file has exactly `size` bytes), the Mark of the
    /// Web, and the move. The database is told the name before each try of the move (`finalizing_name`), so a restart in the middle can
    /// tell a finished move from a lost file; `set_file_final` after it clears that.
    fn finish_blocking(
        &self,
        dir: &Path,
        transfer_id: &str,
        file_id: &str,
        claimed: &str,
        size: u64,
        handle: Option<std::fs::File>,
    ) -> Result<String, String> {
        let delay = self.hooks.finalize_delay_ms.load(Ordering::Relaxed);
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let part = filenames::part_path(dir, claimed);
        let file = match handle {
            Some(f) => f,
            None => std::fs::OpenOptions::new().write(true).open(&part).map_err(|e| e.to_string())?,
        };
        file.sync_all().map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len != size {
            return Err(format!("the .part file has {len} bytes, not {size}"));
        }
        drop(file);
        if let Err(e) = motw::write(&part) {
            // A volume with no alternate data streams (FAT, exFAT, some network shares) can't carry the mark.
            log::warn!(
                "upload {}: the Mark of the Web could not be written ({e}); the file is kept without it",
                short_transfer(transfer_id)
            );
        }
        filenames::create_new(claimed, &mut HashSet::new(), |name| {
            let _ = self.db.set_file_finalizing(transfer_id, file_id, name);
            filenames::move_no_replace(&part, &dir.join(name))
        })
        .map(|(name, ())| name)
    }

    /// Every file of the upload has its final name: the transfer is done, once. The toast and the image for the clipboard are made in a
    /// task of their own, so the `PUT` that finished the last file is answered without waiting for any of it.
    fn upload_done(self: &Arc<Self>, entry: &Arc<Entry>) {
        let now = Core::now_ms();
        if entry.canceled.load(Ordering::Relaxed) || !entry.end_if_active(Status::Done, now, None) {
            return;
        }
        let _ = self.db.set_transfer_status(&entry.id, "done", now as i64, None);
        let attempts = if entry.resumed.load(Ordering::Relaxed) { 2 } else { 1 };
        self.record_send_end(&entry.id, "upload", entry.created_ms as i64, attempts, "ok", None, None);
        let from = self.db.get_device(&entry.device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name);
        log::info!("upload {} is done: {} file(s), {} bytes", short_transfer(&entry.id), entry.files.len(), entry.total_bytes());
        self.emit(Event::StateChanged);
        let (core, entry) = (self.clone(), entry.clone());
        tokio::spawn(async move { core.files_received(&entry, &from).await });
    }

    // ----- cancel ------------------------------------------------------------------------------------------------------

    /// `DELETE /v1/transfers/{id}`: the PC drops the partial files (files that are complete stay). `404` for an unknown transfer,
    /// one of another device, an offer, or one that is already dropped (a repeated `DELETE`).
    pub async fn upload_delete(self: &Arc<Self>, device_id: &str, transfer_id: &str) -> Result<(), Failure> {
        self.internal_hook()?;
        let entry = self.find_upload(device_id, transfer_id).ok_or_else(Failure::not_found)?;
        if self.drop_transfer(&entry, Status::Canceled, Some("user")).await { Ok(()) } else { Err(Failure::not_found()) }
    }

    /// Stops a transfer for good: writers and streams stop, a finish that is running ends first, partial files go, and it is
    /// forgotten (a repeated `DELETE` is `404`; the ID is never used again). It happens once however many cancels arrive and
    /// whatever else is going on: the first call does the work and returns `true`, any other returns `false` at once. A transfer
    /// that was already done stays done (its files stay).
    pub async fn drop_transfer(self: &Arc<Self>, entry: &Arc<Entry>, status: Status, error: Option<&str>) -> bool {
        if entry.dropped.swap(true, Ordering::SeqCst) {
            return false;
        }
        // The work runs in a task of its own: a cancel whose request is dropped (the phone hung up right after it sent the `DELETE`) is
        // still carried out, never left half done.
        let (core, entry, error) = (self.clone(), entry.clone(), error.map(str::to_string));
        tokio::spawn(async move { core.drop_transfer_work(&entry, status, error.as_deref()).await }).await.is_ok()
    }

    async fn drop_transfer_work(&self, entry: &Arc<Entry>, status: Status, error: Option<&str>) {
        entry.canceled.store(true, Ordering::Relaxed);
        let now = Core::now_ms();
        // From here a `PUT` that gets its turn answers `404` and the last file's finish does not mark the transfer done.
        entry.end_if_active(status, now, error);
        self.stop_writers(entry).await;
        for file in &entry.files {
            let _finishing = file.finalizer.lock().await;
        }
        let gone = {
            let entry = entry.clone();
            tokio::task::spawn_blocking(move || drop_parts(&entry)).await.unwrap_or_default()
        };
        let final_status = entry.status();
        let _ = self.db.set_transfer_status(
            &entry.id,
            final_status.as_str(),
            now as i64,
            if final_status == Status::Done { None } else { error },
        );
        if entry.direction == Direction::In {
            for file_id in gone {
                let _ = self.db.clear_file_path(&entry.id, &file_id);
            }
        }
        // The record of the send: a cancel the user made (here or on the phone) is not a failure, anything else that ended it is.
        if final_status != Status::Done {
            let kind = if entry.direction == Direction::In { "upload" } else { "offer" };
            let attempts = if entry.resumed.load(Ordering::Relaxed) { 2 } else { 1 };
            match (final_status, error) {
                (Status::Canceled, Some("user" | "peer") | None) => {
                    self.record_send_end(&entry.id, kind, entry.created_ms as i64, attempts, "canceled", None, None)
                }
                (_, code) => self.record_send_end(
                    &entry.id,
                    kind,
                    entry.created_ms as i64,
                    attempts,
                    "failed",
                    Some(code.unwrap_or("canceled")),
                    None,
                ),
            }
        }
        self.transfers.remove(&entry.id);
        log::info!("transfer {} {}", short_transfer(&entry.id), final_status.as_str());
        self.emit(Event::StateChanged);
    }

    /// A `cancel` message from the phone. For a transfer it doesn't know or doesn't own it is acked and ignored (the receiver
    /// already acked); this does the work for one it owns. It is the same as `DELETE`, for an upload. The reason is text from the phone:
    /// it is cleaned (no control characters, at most 32 characters) before it is logged or stored.
    pub async fn peer_cancelled(self: &Arc<Self>, device_id: &str, transfer_id: &str, reason: &str) {
        let Some(entry) = self.transfers.get(transfer_id).filter(|e| e.device_id == device_id) else {
            log::info!("cancel for an unknown transfer {} from device {}: ignored", short_transfer(transfer_id), short(device_id));
            return;
        };
        if entry.status() != Status::Active {
            return;
        }
        let reason = sanitize_reason(reason);
        log::info!("the phone canceled transfer {} ({reason:?})", short_transfer(transfer_id));
        self.drop_transfer(&entry, Status::Canceled, Some(if reason.is_empty() { "peer" } else { &reason })).await;
    }

    /// A `complete` message from the phone: the offer is done and is not served any more.
    pub async fn peer_completed(&self, device_id: &str, transfer_id: &str) {
        let Some(entry) = self.transfers.get(transfer_id).filter(|e| e.device_id == device_id && e.direction == Direction::Out) else {
            self.late_complete(device_id, transfer_id);
            return;
        };
        let now = Core::now_ms();
        if !entry.end_if_active(Status::Done, now, None) {
            return;
        }
        for file in &entry.files {
            file.done.store(file.size, Ordering::Relaxed);
        }
        let _ = self.db.set_transfer_status(&entry.id, "done", now as i64, None);
        self.record_send_end(&entry.id, "offer", entry.created_ms as i64, 1, "ok", None, None);
        let temp: Vec<PathBuf> = entry.files.iter().map(|f| f.data()).filter(|d| d.temp_source).map(|d| d.path).collect();
        if !temp.is_empty() {
            let _ = tokio::task::spawn_blocking(move || temp.iter().for_each(|p| remove_temp_source(p))).await;
        }
        log::info!("the phone has all the files of offer {}", short_transfer(transfer_id));
        let from = self.db.get_device(device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name);
        self.emit(Event::Toast(format!("Sent {} file(s) to {from}", entry.files.len())));
        self.emit(Event::StateChanged);
    }

    /// A `complete` for an offer that is not in memory. Usually it is for nothing we know (ignored, as the spec says). But the phone did get
    /// the offer if our record says it was sent and not acked (`queued` with a send time), or if we failed it because a phone answered a
    /// repeat with `bad_request` ("that ID was used before", which is what an older phone says to an offer it has already finished): its
    /// `complete` is the truth, so the offer is **done** (changelog 52).
    fn late_complete(&self, device_id: &str, transfer_id: &str) {
        if !self.db.late_complete_candidate(transfer_id, device_id).unwrap_or(false) {
            log::info!("complete for an unknown transfer {} from device {}: ignored", short_transfer(transfer_id), short(device_id));
            return;
        }
        let now = Core::now_ms();
        let _ = self.db.set_transfer_status(transfer_id, "done", now as i64, None);
        let _ = self.db.reopen_send(transfer_id); // it was recorded as failed; the phone says it worked
        self.record_send_end(transfer_id, "offer", now as i64, 1, "ok", None, None);
        self.discard_sources(transfer_id);
        log::info!("a late complete: the phone has all the files of offer {}", short_transfer(transfer_id));
        let from = self.db.get_device(device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name);
        self.emit(Event::Toast(format!("Sent files to {from}")));
        self.emit(Event::StateChanged);
    }

    // ----- what the UI and status.json show ------------------------------------------------------------------------------

    pub fn transfer_view(&self, entry: &Entry) -> TransferView {
        let meta = lock(&entry.meta);
        let peer = self.db.get_device(&entry.device_id).ok().flatten().map_or_else(|| "phone".to_string(), |d| d.name);
        let files_done =
            entry.files.iter().filter(|f| f.done() >= f.size && (entry.direction == Direction::Out || f.data().finalized)).count();
        let status = meta.status;
        TransferView {
            id: entry.id.clone(),
            direction: entry.direction.as_str(),
            peer,
            status: status.as_str(),
            files: entry.files.len(),
            files_done,
            bytes_done: entry.files.iter().map(|f| if f.data().finalized { f.size } else { f.done().min(f.size) }).sum(),
            total_bytes: entry.total_bytes(),
            speed_bps: if status == Status::Active { meta.speed.bps(Instant::now()) } else { 0.0 },
            error: meta.error.clone(),
            started_ms: entry.created_ms,
            finished_ms: meta.finished_ms,
            names: entry.files.iter().take(3).map(|f| f.data().name).collect(),
            cancelable: status == Status::Active,
        }
    }

    /// The transfers that are going, newest first.
    pub fn active_transfer_views(&self) -> Vec<TransferView> {
        let mut entries: Vec<_> = self.transfers.all().into_iter().filter(|e| e.status() == Status::Active).collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.created_ms));
        entries.iter().map(|e| self.transfer_view(e)).collect()
    }

    /// "sending 45%", "receiving 45%" or "transferring 45%": the progress of everything that is going, for the tray's tooltip.
    pub fn transfer_summary(&self) -> Option<String> {
        let active: Vec<Arc<Entry>> = self.transfers.all().into_iter().filter(|e| e.status() == Status::Active).collect();
        if active.is_empty() {
            return None;
        }
        let total: u64 = active.iter().map(|e| e.total_bytes()).sum();
        let done: u64 =
            active.iter().flat_map(|e| e.files.iter()).map(|f| if f.data().finalized { f.size } else { f.done().min(f.size) }).sum();
        let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0);
        let (any_in, any_out) = (active.iter().any(|e| e.direction == Direction::In), active.iter().any(|e| e.direction == Direction::Out));
        let verb = match (any_in, any_out) {
            (true, false) => "receiving",
            (false, true) => "sending",
            _ => "transferring",
        };
        Some(format!("{verb} {percent}%"))
    }

    /// The state of the drop window: the phone, what is going and what finished lately.
    pub fn drop_view(&self) -> Value {
        let snap = self.snapshot();
        let phone = snap.headline_device().map(|d| d.name.clone());
        let status = self.transfers_status();
        json!({
            "phone": phone,
            "connected": snap.state == crate::core::LinkState::PhoneConnected,
            "active": status["active"],
            "recent": status["recent"],
            "folder": status["receivedFolder"],
        })
    }

    pub fn has_active_transfers(&self) -> bool {
        self.transfers.active_count() > 0
    }

    /// The `transfers` part of `status.json`: what is going, what finished lately (from the database), with bytes and speed.
    pub fn transfers_status(&self) -> Value {
        let active = self.active_transfer_views();
        let recent: Vec<Value> = self
            .db
            .recent_transfers(RECENT_KEPT + active.len())
            .unwrap_or_default()
            .into_iter()
            .filter(|(t, _)| t.status != "active")
            .take(RECENT_KEPT)
            .map(|(t, files)| {
                // The drop window says "To <phone>" / "From <phone>" for these rows too.
                let peer = self.db.get_device(&t.device_id).ok().flatten().map_or_else(|| "phone".to_string(), |d| d.name);
                recent_json(&t, &files, &peer)
            })
            .collect();
        json!({ "active": active, "recent": recent, "receivedFolder": self.received_dir().to_string_lossy() })
    }

    // ----- for tests --------------------------------------------------------------------------------------------------

    /// Test helper: an upload that is already under way. Its `.part` files hold `stored` bytes (sparse zeros), and it belongs to
    /// `owner`. `files` are `(fileId, size, stored)`.
    #[doc(hidden)]
    pub fn seed_upload(&self, owner: &str, id: &str, files: &[(&str, u64, u64)]) -> Arc<Entry> {
        let dir = self.received_dir();
        std::fs::create_dir_all(&dir).expect("the test folder exists");
        let mut shared = Vec::new();
        for (idx, (file_id, size, stored)) in files.iter().enumerate() {
            let name = format!("{file_id}.bin");
            let path = filenames::part_path(&dir, &name);
            let file = std::fs::File::create(&path).expect("a .part file");
            file.set_len(*stored).expect("sparse");
            let data = FileData { name, path, finalized: false, source_mtime_ms: None, temp_source: false };
            shared.push(FileShared::new(file_id, idx, *size, "application/octet-stream", data, *stored));
        }
        let now = Core::now_ms();
        let entry = Entry::new(id, owner, Direction::In, now, u64::MAX, dir, shared);
        insert_row(self, &entry);
        self.transfers.insert(entry.clone());
        entry
    }

    /// Test helper: an offer that is already made, with the files at `sources` (`(fileId, path)`; the size is the file's).
    #[doc(hidden)]
    pub fn seed_offer(&self, owner: &str, id: &str, sources: &[(&str, &Path)], expired: bool) -> Arc<Entry> {
        let now = Core::now_ms();
        let mut shared = Vec::new();
        for (idx, (file_id, path)) in sources.iter().enumerate() {
            let meta = std::fs::metadata(path).expect("the source exists");
            let name = path.file_name().map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
            let data = FileData {
                name,
                path: path.to_path_buf(),
                finalized: false,
                source_mtime_ms: Some(crate::outgoing::mtime_ms(&meta)),
                temp_source: false,
            };
            shared.push(FileShared::new(file_id, idx, meta.len(), "application/octet-stream", data, 0));
        }
        let expires = if expired { now.saturating_sub(1) } else { now + KEEP_MS };
        let entry = Entry::new(id, owner, Direction::Out, now, expires, PathBuf::new(), shared);
        insert_row(self, &entry);
        self.transfers.insert(entry.clone());
        entry
    }

    // ----- housekeeping ------------------------------------------------------------------------------------------------

    /// Drops what is a day old: offers that expired (the phone gets a `cancel` with reason `expired` if it is connected), received
    /// transfers with no progress for a day (their `.part` files go), finished transfers that don't need to be remembered in
    /// memory any more, and the `.part` files that the database still assigns to a transfer that is over. Run at start and every hour.
    /// **Nothing is deleted that no record names:** a file in the received folder is the user's unless a record says it is ours.
    pub async fn clean_up(self: &Arc<Self>) {
        self.clean_up_at(Core::now_ms()).await;
    }

    /// `clean_up` as of `now` (a test moves the clock by hand).
    pub async fn clean_up_at(self: &Arc<Self>, now: u64) {
        for entry in self.transfers.all() {
            let status = entry.status();
            match (entry.direction, status) {
                (Direction::Out, Status::Active) if entry.expires_ms() <= now => {
                    log::info!("offer {} expired", short_transfer(&entry.id));
                    self.send_cancel_message(&entry.device_id, &entry.id, "expired");
                    self.drop_transfer(&entry, Status::Failed, Some("expired")).await;
                }
                (Direction::In, Status::Active) => {
                    let last = self.db.get_transfer(&entry.id).ok().flatten().map_or(entry.created_ms, |(t, _)| t.updated_at as u64);
                    if now.saturating_sub(last) >= KEEP_MS && !entry.files.iter().any(|f| f.writer.try_lock().is_err()) {
                        if entry.files.iter().all(|f| f.data().finalized) {
                            // Every file is complete and has its name; only the end of the transfer was never recorded.
                            self.upload_done(&entry);
                        } else {
                            log::info!("upload {} had no progress for a day: its .part files are removed", short_transfer(&entry.id));
                            self.drop_transfer(&entry, Status::Failed, Some("expired")).await;
                        }
                    }
                }
                (_, Status::Done | Status::Failed | Status::Canceled) => {
                    let finished = lock(&entry.meta).finished_ms.unwrap_or(0);
                    if now.saturating_sub(finished) >= KEEP_MS {
                        self.transfers.remove(&entry.id);
                    }
                }
                _ => {}
            }
        }
        let core = self.clone();
        let _ = tokio::task::spawn_blocking(move || {
            core.remove_leftover_parts();
            core.remove_orphan_outgoing(now);
        })
        .await;
    }

    /// A temporary `.txt` of a long text that no offer uses any more (the app was closed before the offer ended) goes after a day. **In use**
    /// means a transfer in memory points at it *or the database says a queued or active item does*: a queued offer (a long text waiting for the
    /// phone) lives in the database alone until it is flushed, and its `.txt` is the only copy of the text.
    fn remove_orphan_outgoing(&self, now_ms: u64) {
        let Ok(read) = std::fs::read_dir(self.config.data_dir.join(crate::outgoing::OUTGOING_DIR)) else {
            return;
        };
        let mut used: HashSet<PathBuf> =
            self.transfers.all().iter().flat_map(|e| e.files.iter().map(|f| f.data()).filter(|d| d.temp_source).map(|d| d.path)).collect();
        match self.db.live_temp_sources() {
            Ok(paths) => used.extend(paths.into_iter().map(PathBuf::from)),
            Err(e) => {
                // Without the list nothing can be told to be an orphan: delete nothing.
                log::warn!("reading the temporary files in use failed ({e}): none is removed");
                return;
            }
        }
        for item in read.flatten() {
            let folder = item.path();
            let in_use = std::fs::read_dir(&folder).is_ok_and(|r| r.flatten().any(|f| used.contains(&f.path())));
            let modified = item.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            if !in_use && modified.is_some_and(|m| now_ms.saturating_sub(m.as_millis() as u64) >= KEEP_MS) {
                log::info!("removing the temporary file of a text offer that is over");
                let _ = std::fs::remove_dir_all(&folder);
            }
        }
    }

    /// Deletes the `.part` files that the database still assigns to a received transfer that is over (the app stopped between ending
    /// a transfer and removing its files, or a scanner held a file when it was canceled). **Only paths a record names are touched:** the
    /// received folder is the user's, and a `.part` file in it that no record names (a download of another program, a file the user
    /// saved) is left alone. A path that a record has as a finished file, or that an unfinished upload uses, is never deleted either.
    fn remove_leftover_parts(&self) {
        let leftovers = match self.db.leftover_part_files() {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("reading the leftover files failed: {e}");
                return;
            }
        };
        let live = self.transfers.live_part_paths();
        for (transfer_id, file_id, path) in leftovers {
            let file = PathBuf::from(&path);
            let is_part = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("part"));
            if !is_part || live.contains(&path.to_lowercase()) {
                continue;
            }
            match std::fs::remove_file(&file) {
                Ok(()) => log::info!("removed a .part file left by transfer {}", short_transfer(&transfer_id)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    log::warn!("removing a leftover .part file of {} failed: {e}", short_transfer(&transfer_id));
                    continue;
                }
            }
            let _ = self.db.clear_file_path(&transfer_id, &file_id);
        }
    }
}

enum WriteEnd {
    Complete,
    /// A newer request for the file asked this writer to stop.
    Stopped,
    BodyError,
    DiskError(std::io::Error),
}

struct WriteOutcome {
    handle: Option<std::fs::File>,
    end: WriteEnd,
}

fn offset_mismatch(stored: u64) -> Failure {
    Failure { error: ApiError::OffsetMismatch, message: format!("The PC has {stored} bytes of this file."), offset: Some(stored) }
}

type WriteTask = tokio::task::JoinHandle<std::io::Result<(std::fs::File, Vec<u8>, bool)>>;

/// The writing half of a `PUT`: pieces of the body are collected in `buffer`, and a full one is handed to a blocking task that
/// writes it while the next is collected. `done` of the file grows when a write has finished, never before.
struct Pipeline<'a> {
    core: &'a Core,
    entry: &'a Arc<Entry>,
    file: &'a Arc<FileShared>,
    handle: Option<std::fs::File>,
    pending: Option<WriteTask>,
    in_flight: usize,
    buffer: Vec<u8>,
    spare: Vec<u8>,
    since_checkpoint: u64,
}

impl Pipeline<'_> {
    /// Waits for the write in flight, if there is one, and takes the file back.
    async fn settle(&mut self) -> Result<(), std::io::Error> {
        let Some(task) = self.pending.take() else {
            return Ok(());
        };
        let (handle, mut used, synced) = task.await.map_err(std::io::Error::other)??;
        self.handle = Some(handle);
        self.file.done.fetch_add(self.in_flight as u64, Ordering::Relaxed);
        self.entry.note_moved(self.in_flight as u64);
        self.in_flight = 0;
        used.clear();
        self.spare = used;
        if synced {
            self.core.checkpoint(self.entry, self.file);
        }
        Ok(())
    }

    /// Takes a piece of the body into the buffer (at most up to `missing` bytes in all; `taken` counts what has been accepted) and
    /// hands a full buffer to the disk.
    async fn accept(&mut self, data: &[u8], missing: u64, taken: &mut u64) -> Result<(), std::io::Error> {
        let take = (data.len() as u64).min(missing - *taken) as usize;
        self.buffer.extend_from_slice(&data[..take]);
        *taken += take as u64;
        if self.buffer.len() >= WRITE_PIECE { self.flush().await } else { Ok(()) }
    }

    /// Hands the buffer to the disk (after the previous write has finished, so there is one in flight at most).
    async fn flush(&mut self) -> Result<(), std::io::Error> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.settle().await?;
        let fresh = std::mem::take(&mut self.spare);
        let piece = std::mem::replace(&mut self.buffer, if fresh.capacity() == 0 { Vec::with_capacity(WRITE_PIECE) } else { fresh });
        let mut file = self.handle.take().expect("settled");
        self.in_flight = piece.len();
        self.since_checkpoint += piece.len() as u64;
        let checkpoint = self.since_checkpoint >= CHECKPOINT_BYTES;
        if checkpoint {
            self.since_checkpoint = 0;
        }
        self.pending = Some(tokio::task::spawn_blocking(move || {
            file.write_all(&piece)?;
            if checkpoint {
                file.sync_data()?;
            }
            Ok((file, piece, checkpoint))
        }));
        Ok(())
    }

    /// The rest of the buffer is written and the last write is waited for.
    async fn finish(&mut self) -> Result<(), std::io::Error> {
        self.flush().await?;
        self.settle().await
    }

    /// `sync_data`, then the database learns how far the file is.
    async fn sync_and_record(&mut self) {
        if let Some(file) = self.handle.take() {
            self.handle = tokio::task::spawn_blocking(move || {
                let _ = file.sync_data();
                file
            })
            .await
            .ok();
        }
        self.core.checkpoint(self.entry, self.file);
    }
}

fn row_of(entry: &Entry, now_ms: u64) -> TransferRow {
    TransferRow {
        id: entry.id.clone(),
        device_id: entry.device_id.clone(),
        direction: entry.direction.as_str().to_string(),
        status: entry.status().as_str().to_string(),
        created_at: entry.created_ms as i64,
        updated_at: now_ms as i64,
        finished_at: None,
        expires_at: (entry.expires_ms() != u64::MAX).then_some(entry.expires_ms() as i64),
        total_bytes: entry.total_bytes() as i64,
        error_code: None,
        dest_dir: Some(entry.dest_dir.to_string_lossy().into_owned()),
    }
}

pub fn file_rows(entry: &Entry) -> Vec<TransferFileRow> {
    entry
        .files
        .iter()
        .map(|f| {
            let d = f.data();
            TransferFileRow {
                file_id: f.file_id.clone(),
                idx: f.idx as i64,
                name: d.name.clone(),
                mime: f.mime.clone(),
                size: f.size as i64,
                bytes_done: f.done() as i64,
                local_path: Some(d.path.to_string_lossy().into_owned()),
                finalized: d.finalized,
                source_mtime_ms: d.source_mtime_ms,
                temp_source: d.temp_source,
                finalizing_name: None,
            }
        })
        .collect()
}

pub fn insert_row(core: &Core, entry: &Entry) {
    if let Err(e) = core.db.insert_transfer(&row_of(entry, Core::now_ms()), &file_rows(entry)) {
        log::error!("saving the transfer failed: {e}");
    }
}

fn recent_json(t: &TransferRow, files: &[TransferFileRow], peer: &str) -> Value {
    let done: i64 = files.iter().map(|f| if f.finalized { f.size } else { f.bytes_done }).sum();
    json!({
        "id": t.id,
        "direction": t.direction,
        "peer": peer,
        "status": t.status,
        "files": files.len(),
        "bytesDone": done,
        "totalBytes": t.total_bytes,
        "error": t.error_code,
        "startedMs": t.created_at,
        "finishedMs": t.finished_at,
        "names": files.iter().take(3).map(|f| f.name.clone()).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_numbers() {
        for ok in ["0", "7", "1048576", "9007199254740991"] {
            assert!(parse_strict_u64(ok).is_some(), "{ok}");
        }
        for bad in
            ["", "007", "00", "+5", "-5", "1.5", "abc", " 5", "5 ", "9007199254740992", "\u{663}", "\u{ff13}", "99999999999999999999"]
        {
            assert!(parse_strict_u64(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn speed_is_bytes_per_second_over_the_last_seconds_and_zero_when_idle() {
        let mut s = Speed::default();
        let t0 = Instant::now();
        s.add(t0, 0);
        s.add(t0 + Duration::from_secs(1), 10_000_000);
        assert!((s.bps(t0 + Duration::from_secs(1)) - 10_000_000.0).abs() < 1.0);
        assert_eq!(s.bps(t0 + Duration::from_secs(4)), 0.0, "nothing moved for 3 s");
    }

    /// protocol.md, "The 24 hour window": answered while now - done time < 24 h, `404` from exactly 24 h on. The done time is the moment
    /// the transfer ended (the last file got its final name) and nothing moves it.
    #[test]
    fn a_finished_transfer_is_answered_until_exactly_24_hours_after_its_done_time() {
        let entry = Entry::new("t", "d", Direction::In, 0, u64::MAX, PathBuf::new(), Vec::new());
        assert!(!entry.past_keeping(u64::MAX / 2), "a transfer that is going is never past its window");
        assert!(entry.end_if_active(Status::Done, 1_000, None));
        assert!(!entry.past_keeping(1_000), "the moment it finished");
        assert!(!entry.past_keeping(1_000 + KEEP_MS - 1), "one millisecond before 24 hours");
        assert!(entry.past_keeping(1_000 + KEEP_MS), "at exactly 24 hours it is 404");
        assert!(entry.past_keeping(1_000 + KEEP_MS + 1));
        // Ending it again (a late cancel, a repeated last PUT) does not move the done time: `end_if_active` is once.
        assert!(!entry.end_if_active(Status::Done, 5_000_000, None));
        assert_eq!(lock(&entry.meta).finished_ms, Some(1_000));
    }
}
