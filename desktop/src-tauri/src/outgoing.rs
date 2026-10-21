//! PC to phone (protocol.md, "Files: PC -> phone"): turning files into an `offer`, serving them to the phone's `GET` requests
//! (with `Range` for a resume), and canceling from either side.
//!
//! The PC never pushes bytes. It sends the `offer` over the WebSocket, waits for the `ack` (or an `error`, such as `disk_full`),
//! and the phone downloads each file with its own HTTPS requests. **Since 1.4 an offer is queued first** (`queue.rs`): it is stored as
//! `queued` before the first attempt, and sent at once if the phone is connected, or when it is back. The files are read from where they are (nothing is copied, and
//! a file that changes after the offer makes the `GET` answer `409 source_changed`). An offer expires after 24 hours.
//!
//! **Folders.** A dropped folder is not sent in 1.3: it is skipped with a message ("Folders can't be sent yet"). Flattening it would
//! lose the structure and clash names, and sending the structure needs a protocol addition. The files that were dropped with it
//! are sent.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use crate::connections::{ConnCommand, SendOutcome};
use crate::core::{Core, Event, short_transfer};
use crate::encoding::{unix_ms, uuid_v7};
use crate::server::ApiError;
use crate::sync::SendResult;
use crate::transfers::{Direction, Entry, Failure, FileShared, Status, remove_temp_source};

/// How many files go in one `offer`: the limit of the protocol, which a receiver enforces (`messages::MAX_OFFER_FILES`).
pub const FILES_PER_OFFER: usize = crate::messages::MAX_OFFER_FILES;
/// The name of the `.txt` that a long text goes as (protocol.md, "Text messages").
pub const TEXT_FILE_NAME: &str = "Text.txt";
/// A temporary file of ours is kept in this folder of the data directory, one folder per transfer.
pub const OUTGOING_DIR: &str = "outgoing";

/// A file to send: where it is, and what the phone is told.
#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub temp: bool,
}

/// What `send_paths` did, for the toast and for tests.
#[derive(Debug, Default)]
pub struct SendReport {
    /// The transfer IDs of the offers that were made (in 1.3 one, unless there are more than 1,000 files).
    pub transfers: Vec<String>,
    pub files: usize,
    pub skipped_folders: usize,
    /// Files that couldn't be read, with the reason.
    pub unreadable: Vec<(String, String)>,
    pub result: Option<SendResult>,
}

pub fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

/// A MIME type from the extension, so the phone can tell photos and videos (which go to the gallery) from everything else.
pub fn mime_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e).to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "avif" => "image/avif",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        "wmv" => "video/x-ms-wmv",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned())
}

/// Looks at what was dropped or passed on the command line. Folders and files that can't be read are set aside.
pub fn inspect(paths: &[PathBuf]) -> (Vec<Source>, usize, Vec<(String, String)>) {
    let (mut sources, mut folders, mut unreadable) = (Vec::new(), 0usize, Vec::new());
    for path in paths {
        let name = file_name_of(path);
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => folders += 1,
            Ok(m) if m.is_file() => match std::fs::File::open(path) {
                Ok(_) => sources.push(Source { path: path.clone(), name, size: m.len(), mtime_ms: mtime_ms(&m), temp: false }),
                Err(e) => unreadable.push((name, e.to_string())),
            },
            Ok(_) => unreadable.push((name, "not a file".to_string())),
            Err(e) => unreadable.push((name, e.to_string())),
        }
    }
    (sources, folders, unreadable)
}

impl Core {
    /// The tray's "Send files…", the drop window, Explorer's "Send to" and `Kin.exe --send <paths>`: offer these files to the
    /// phone. The offer is queued first. A phone that isn't connected gets it when it is back, and the user is told so.
    pub async fn send_paths(self: &Arc<Self>, paths: Vec<PathBuf>, trigger: &'static str) -> SendReport {
        let mut report = SendReport::default();
        let (sources, folders, unreadable) = tokio::task::spawn_blocking(move || inspect(&paths)).await.unwrap_or_default();
        report.skipped_folders = folders;
        report.unreadable = unreadable;
        log::info!(
            "send files ({trigger}): {} file(s), {folders} folder(s) skipped, {} unreadable",
            sources.len(),
            report.unreadable.len()
        );
        if folders > 0 {
            self.emit(Event::Toast(format!(
                "{} can't be sent yet (folders aren't supported). Send the files inside it instead.",
                if folders == 1 { "A folder".to_string() } else { format!("{folders} folders") }
            )));
        }
        for (name, why) in &report.unreadable {
            // A file name is the user's own: the log has a short hash of it (and the name at debug level, which is off by default).
            let tag = crate::core::name_tag(name);
            log::warn!("send files: {tag} can't be read: {why}");
            log::debug!("send files: {tag} is {name:?}");
            self.emit(Event::ToastRedacted {
                text: format!("{name} can't be read, so it was not sent."),
                log_text: format!("{tag} can't be read, so it was not sent."),
            });
        }
        if sources.is_empty() {
            if folders == 0 && report.unreadable.is_empty() {
                self.emit(Event::Toast("There is nothing to send.".to_string()));
            }
            return report;
        }
        let mut waiting = Vec::new();
        for chunk in sources.chunks(FILES_PER_OFFER) {
            match self.enqueue_offer(chunk, None) {
                Some((id, rx)) => {
                    report.files += chunk.len();
                    report.transfers.push(id.clone());
                    waiting.push((id, rx));
                }
                None => {
                    // No phone is paired (nothing to queue for), or the offer couldn't be saved.
                    let result = if self.snapshot().devices.is_empty() {
                        SendResult::NoPhonePaired
                    } else {
                        SendResult::Rejected("internal".into())
                    };
                    self.offer_failed_toast(&result, None);
                    for s in chunk.iter().filter(|s| s.temp) {
                        remove_temp_source(&s.path);
                    }
                    report.result = Some(result);
                    break;
                }
            }
        }
        if waiting.is_empty() {
            return report;
        }
        let connected = self.connections.most_recent_open().is_some();
        if connected {
            self.flush_queue().await;
        } else {
            let n: usize = report.files;
            self.emit(Event::Toast(format!(
                "The phone isn't connected. {} queued and will be sent when it is back.",
                if n == 1 { "1 file is".to_string() } else { format!("{n} files are") }
            )));
        }
        for (id, mut rx) in waiting {
            self.queue.forget(&id);
            let result = rx.try_recv().unwrap_or(SendResult::PhoneOffline);
            if report.result.as_ref().is_none_or(|r| matches!(r, SendResult::Acked { .. })) {
                report.result = Some(result);
            }
        }
        report
    }

    /// A text over 128 KiB goes as a `.txt` file through the same path (protocol.md, "Text messages"). The text is written to a
    /// file of our own, which is removed when the transfer ends (or at the latest when the offer expires).
    pub async fn send_text_file(self: &Arc<Self>, text: &str) -> SendResult {
        self.send_text_file_to(None, text).await
    }

    /// The same for a named phone (the browser extension may pick one).
    pub async fn send_text_file_to(self: &Arc<Self>, device: Option<&str>, text: &str) -> SendResult {
        self.send_temp_file(device, TEXT_FILE_NAME, text.as_bytes()).await.0
    }

    /// Writes `bytes` to a file of our own named `name` (already cleaned), offers it to `device` (`None`: the default phone) through the ordinary
    /// file transfer, and waits for the phone's `ack` if it is connected (the offer is queued either way). The file is removed when the transfer
    /// ends. Returns how it came out and the transfer's ID. (A long text, and an image that the browser extension sent.)
    pub async fn send_temp_file(self: &Arc<Self>, device: Option<&str>, name: &str, bytes: &[u8]) -> (SendResult, Option<String>) {
        let folder = self.config.data_dir.join(OUTGOING_DIR).join(uuid_v7(unix_ms()));
        let path = folder.join(name);
        let (write_folder, write_path, bytes) = (folder.clone(), path.clone(), bytes.to_vec());
        let written = tokio::task::spawn_blocking(move || {
            let written = std::fs::create_dir_all(&write_folder).and_then(|()| std::fs::write(&write_path, &bytes));
            match written {
                Ok(()) => std::fs::metadata(&write_path),
                Err(e) => {
                    log::error!("writing the file to send failed: {e}");
                    let _ = std::fs::remove_dir_all(&write_folder);
                    Err(e)
                }
            }
        })
        .await;
        let Ok(Ok(meta)) = written else {
            return (SendResult::Rejected("internal".to_string()), None);
        };
        let source = Source { path, name: name.to_string(), size: meta.len(), mtime_ms: mtime_ms(&meta), temp: true };
        let Some((id, mut rx)) = self.enqueue_offer_to(device, std::slice::from_ref(&source), None) else {
            remove_temp_source(&source.path);
            let result =
                if self.snapshot().devices.is_empty() { SendResult::NoPhonePaired } else { SendResult::Rejected("internal".into()) };
            return (result, None);
        };
        if self.connections.most_recent_open().is_some() {
            self.flush_queue().await;
        }
        self.queue.forget(&id);
        (rx.try_recv().unwrap_or(SendResult::PhoneOffline), Some(id))
    }

    pub(crate) fn offer_failed_toast(&self, result: &SendResult, phone: Option<&str>) {
        let who = phone.unwrap_or("The phone");
        let text = match result {
            SendResult::PhoneOffline => "The phone isn't connected. The files are queued and will be sent when it is back.".to_string(),
            SendResult::NoPhonePaired => "No phone is paired yet.".to_string(),
            SendResult::NoReply => "No reply from the phone. The files are queued and will be sent again when it is back.".to_string(),
            SendResult::Rejected(code) if code == "disk_full" => format!("{who} doesn't have enough free space for the files."),
            SendResult::Rejected(code) => format!("{who} couldn't take the files ({code})."),
            SendResult::TooLarge { .. } | SendResult::Acked { .. } => return,
        };
        self.emit(Event::Toast(text));
    }

    /// A `cancel` message for a transfer. **It is kept until the phone acks it** (changelog 52): it is stored first, sent at once if that
    /// phone is connected, and sent again, with the same message ID, after every `hello` (before the queue). A phone that was away when the
    /// user canceled therefore still hears of it, and does not have to find out through a `404`.
    pub fn send_cancel_message(self: &Arc<Self>, device_id: &str, transfer_id: &str, reason: &str) {
        let message_id =
            self.db.add_pending_cancel(device_id, transfer_id, reason, &uuid_v7(unix_ms()), unix_ms() as i64).unwrap_or_else(|e| {
                log::warn!("keeping the cancel message failed ({e}): it is sent once and not kept");
                uuid_v7(unix_ms())
            });
        let core = self.clone();
        let (device_id, transfer_id, reason) = (device_id.to_string(), transfer_id.to_string(), reason.to_string());
        tokio::spawn(async move {
            core.deliver_cancel(&device_id, &transfer_id, &reason, &message_id).await;
        });
    }

    /// Sends one `cancel` to `device_id` if it is connected, and forgets it once the phone acked it (or answered with an `error`, which ends a
    /// message's life too). Without an `ack` it stays stored for the next `hello`. `true` if it is done.
    async fn deliver_cancel(&self, device_id: &str, transfer_id: &str, reason: &str, message_id: &str) -> bool {
        let Some(tx) = self.connections.open_sender(device_id) else {
            return false;
        };
        let frame = json!({ "type": "cancel", "id": message_id, "transferId": transfer_id, "reason": reason });
        let (reply, wait) = tokio::sync::oneshot::channel();
        if tx.send(ConnCommand::Send { frame, reply }).is_err() {
            return false;
        }
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(7), wait).await;
        let done = matches!(outcome, Ok(Ok(SendOutcome::Ack(_) | SendOutcome::Failed(_))));
        log::info!(
            "cancel message for {}: {}",
            short_transfer(transfer_id),
            if done { "acked" } else { "no ack, it is kept for the next connection" }
        );
        if done {
            let _ = self.db.delete_pending_cancel(message_id);
        }
        done
    }

    /// After a `hello`: every `cancel` the phone has not acked goes again, oldest first and one at a time. Before the queue (the phone should
    /// not start something that was already canceled).
    pub async fn flush_pending_cancels(self: &Arc<Self>, device_id: &str) {
        let pending = self.db.pending_cancels(device_id).unwrap_or_default();
        if !pending.is_empty() {
            log::info!("{} cancel message(s) are sent again after the hello", pending.len());
        }
        for (message_id, transfer_id, reason) in pending {
            if !self.deliver_cancel(device_id, &transfer_id, &reason, &message_id).await
                && self.connections.open_sender(device_id).is_none()
            {
                break;
            }
        }
    }

    /// The user canceled a transfer (the drop window's button), in either direction. The phone is told with a `cancel` message
    /// (reason `user`) and partial files go.
    pub async fn cancel_transfer(self: &Arc<Self>, id: &str) -> bool {
        let Some(entry) = self.transfers.get(id).filter(|e| e.status() == Status::Active) else {
            return false;
        };
        log::info!("the user canceled transfer {} ({})", short_transfer(id), entry.direction.as_str());
        self.send_cancel_message(&entry.device_id, &entry.id, "user");
        self.drop_transfer(&entry, Status::Canceled, Some("user")).await;
        true
    }

    /// `Kin.exe --cancel-transfers`: the Cancel button for every transfer that is going.
    pub async fn cancel_all_transfers(self: &Arc<Self>) {
        for view in self.active_transfer_views() {
            self.cancel_transfer(&view.id).await;
        }
    }

    // ----- GET /v1/transfers/{id}/files/{fileId} ----------------------------------------------------------------------

    /// The offer and file a `GET` is about. `404` for an unknown transfer (or one of another device, or an upload), one that
    /// expired, was canceled or is done, and an unknown file. Then `409` if the source changed.
    pub async fn offer_file(&self, device_id: &str, transfer_id: &str, file_id: &str) -> Result<(Arc<Entry>, Arc<FileShared>), Failure> {
        if self.hooks.fail_internal.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Failure::internal());
        }
        let entry = self
            .transfers
            .get(transfer_id)
            .filter(|e| e.device_id == device_id && e.direction == Direction::Out)
            .ok_or_else(Failure::not_found)?;
        if entry.status() != Status::Active || entry.expires_ms() <= unix_ms() {
            return Err(Failure::new(ApiError::NotFound, "The offer expired or was canceled."));
        }
        let file = entry.file(file_id).cloned().ok_or_else(|| Failure::new(ApiError::NotFound, "File not found."))?;
        let data = file.data();
        let (size, expected_mtime) = (file.size, data.source_mtime_ms);
        let unchanged = tokio::task::spawn_blocking(move || {
            std::fs::metadata(&data.path).is_ok_and(|m| m.is_file() && m.len() == size && expected_mtime.is_none_or(|t| mtime_ms(&m) == t))
        })
        .await
        .unwrap_or(false);
        if !unchanged {
            log::warn!("offer {}: {:?} changed or is gone since it was offered", short_transfer(transfer_id), file_id);
            return Err(Failure::new(ApiError::SourceChanged, "The file changed after it was offered."));
        }
        Ok((entry, file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_by_extension() {
        assert_eq!(mime_of("IMG_0001.JPG"), "image/jpeg");
        assert_eq!(mime_of("clip.mp4"), "video/mp4");
        assert_eq!(mime_of("notes.txt"), "text/plain");
        assert_eq!(mime_of("archive.tar.gz"), "application/octet-stream");
        assert_eq!(mime_of("noextension"), "application/octet-stream");
        assert_eq!(mime_of(".hidden"), "application/octet-stream");
    }

    #[test]
    fn inspect_sets_folders_and_unreadable_files_aside() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hello").unwrap();
        let folder = dir.path().join("sub");
        std::fs::create_dir(&folder).unwrap();
        let (sources, folders, unreadable) = inspect(&[file.clone(), folder, dir.path().join("missing.bin")]);
        assert_eq!(sources.len(), 1);
        assert_eq!((sources[0].name.as_str(), sources[0].size), ("a.txt", 5));
        assert_eq!(folders, 1);
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].0, "missing.bin");
    }
}
