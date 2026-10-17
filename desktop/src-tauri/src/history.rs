//! The history window of milestone 1.4: the recent transfers in both directions (text and files), what a click on one does, and the
//! Retry and Cancel buttons. The rows are the `transfers` table (see `queuedb.rs`).
//!
//! **What is never in it.** A `sensitive` text and the automatic `clipboard` sync are never stored, so they can't be listed (protocol.md,
//! "Sensitive text" and "Queue"). `history_view` lists only what the database has, which is why there is nothing to filter out here, and a
//! test checks it with both kinds of send.
//!
//! **What a click does.** On a finished file: the 1.3 rule. A single file of a type on the allow-list (`links::may_open_file`) is opened
//! with its default program, any other file is shown in its folder, selected; several files open their folder. On a text: it is copied to
//! the clipboard (without being sent back to the phone by the automatic sync: the loop guard is told). The history shows at most the
//! newest 500 items.
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::clipboard;
use crate::core::{Core, lock, short_transfer};
use crate::links;
use crate::queuedb::HistoryRow;
use crate::transfers::Status;

/// The history lists at most this many items (the newest).
pub const HISTORY_CAP: usize = 500;
/// A text item shows at most this much of its text.
pub const TEXT_PREVIEW_CHARS: usize = 160;

/// What a click on a history item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenAction {
    /// Open the file with its default program (its type is on the allow-list).
    OpenFile(PathBuf),
    /// Show the file in its folder, selected (any other type).
    ShowFile(PathBuf),
    /// Open the folder (several files).
    OpenFolder(PathBuf),
    /// Copy the text to the clipboard.
    CopyText,
    /// Nothing to do (the item isn't done, or its files are gone).
    Nothing,
}

/// What a click on `row` does. A text can be copied whatever its status (a text that is still queued, or that failed, is the user's own
/// and is worth getting back); a file is opened only when it is done.
pub fn open_action(row: &HistoryRow) -> OpenAction {
    if row.kind == "text" {
        return OpenAction::CopyText;
    }
    if row.status != "done" {
        return OpenAction::Nothing;
    }
    let paths: Vec<PathBuf> = row.files.iter().filter_map(|f| f.local_path.as_ref().map(PathBuf::from)).collect();
    match paths.as_slice() {
        [] => OpenAction::Nothing,
        [one] if links::may_open_file(one) => OpenAction::OpenFile(one.clone()),
        [one] => OpenAction::ShowFile(one.clone()),
        [first, ..] => first.parent().map_or(OpenAction::Nothing, |p| OpenAction::OpenFolder(p.to_path_buf())),
    }
}

fn one_line(text: &str, max_chars: usize) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    clipboard::preview(&flat, max_chars)
}

impl Core {
    /// What grows without limit is trimmed (at start and every hour): the rows of `sends` older than 90 days, and the **bodies** of texts beyond the
    /// newest 500 items, which is all the history shows. The `transfers` rows themselves stay for good (a transfer ID is never reused), and
    /// a body that a queued or active item still needs is never touched.
    pub fn prune_storage(&self, now_ms: i64) {
        if let Err(e) = self.db.prune_sends(now_ms) {
            log::warn!("pruning the send log failed: {e}");
        }
        match self.db.prune_text_bodies(HISTORY_CAP) {
            Ok(0) => {}
            Ok(n) => log::info!("{n} old text(s) beyond the newest {HISTORY_CAP} were deleted"),
            Err(e) => log::warn!("pruning old texts failed: {e}"),
        }
    }

    /// What the history window shows: the newest items, both directions, text and files.
    pub fn history_view(&self) -> Value {
        let rows = self.db.history(HISTORY_CAP).unwrap_or_else(|e| {
            log::error!("reading the history failed: {e}");
            Vec::new()
        });
        let snap = self.snapshot();
        let items: Vec<Value> = rows.iter().map(|r| self.history_item_json(r)).collect();
        json!({
            "items": items,
            "cap": HISTORY_CAP,
            "phone": snap.headline_device().map(|d| d.name.clone()),
            "connected": snap.state == crate::core::LinkState::PhoneConnected,
            "queued": snap.queued,
        })
    }

    fn history_item_json(&self, r: &HistoryRow) -> Value {
        let outgoing = r.direction == "out";
        let can_retry = outgoing && r.status == "failed" && !r.retried;
        json!({
            "id": r.id,
            "direction": r.direction,
            "kind": r.kind,
            "status": r.status,
            "createdMs": r.created_at,
            "finishedMs": r.finished_at,
            "totalBytes": r.total_bytes,
            "error": r.error_code,
            "files": r.files.len(),
            "names": r.files.iter().take(3).map(|f| f.name.clone()).collect::<Vec<_>>(),
            "text": r.text.as_deref().map(|t| one_line(t, TEXT_PREVIEW_CHARS)),
            "isUrl": r.is_url,
            "isLink": r.is_link,
            "title": r.title.as_deref().map(|t| one_line(t, TEXT_PREVIEW_CHARS)),
            "retryOf": r.retry_of.as_deref().map(short_transfer),
            "retried": r.retried,
            "canOpen": !matches!(open_action(r), OpenAction::Nothing),
            "opensText": r.kind == "text",
            "canRetry": can_retry,
            "canCancel": outgoing && matches!(r.status.as_str(), "queued" | "active"),
        })
    }

    /// A click on an item of the history. Returns a short message for the window (what was done, or why not).
    pub async fn history_open(self: &Arc<Self>, id: &str) -> Result<String, String> {
        let row = self.db.history_item(id).ok().flatten().ok_or("That item is not in the history.")?;
        match open_action(&row) {
            OpenAction::Nothing => Err(if row.status == "done" {
                "The file is not there any more.".into()
            } else {
                "Only a finished file can be opened.".into()
            }),
            OpenAction::CopyText => {
                let text = row.text.ok_or("The text is no longer stored.")?;
                self.copy_to_clipboard(&text).await?;
                log::info!("history: text {} copied to the clipboard", short_transfer(id));
                Ok("Copied to the clipboard.".into())
            }
            OpenAction::OpenFile(path) => {
                self.opener.open_path(&path).map_err(|e| format!("Could not open the file: {e}"))?;
                log::info!("history: opened a file of transfer {}", short_transfer(id));
                Ok("Opened.".into())
            }
            OpenAction::ShowFile(path) => {
                self.opener.show_in_folder(&path).map_err(|e| format!("Could not show the file: {e}"))?;
                log::info!("history: showed a file of transfer {} in its folder (its type is not on the allow-list)", short_transfer(id));
                Ok("Shown in its folder.".into())
            }
            OpenAction::OpenFolder(folder) => {
                self.opener.open_folder(&folder).map_err(|e| format!("Could not open the folder: {e}"))?;
                log::info!("history: opened the folder of transfer {}", short_transfer(id));
                Ok("Opened the folder.".into())
            }
        }
    }

    /// Puts `text` on the clipboard as the user's own copy of it, without it being sent to the phone again: the automatic sync would
    /// otherwise see a clipboard change and send it. The loop guard remembers the wire form, as it does for a text from the phone.
    pub async fn copy_to_clipboard(self: &Arc<Self>, text: &str) -> Result<(), String> {
        let wire = clipboard::to_wire(text);
        let to_set = clipboard::to_windows_newlines(&wire);
        lock(&self.loop_guard).note_set(&wire, self.clock.now_ms());
        let clipboard = self.clipboard.clone();
        let written = tokio::task::spawn_blocking(move || clipboard.write_text(&to_set, None)).await;
        match written {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                lock(&self.loop_guard).forget();
                Err(format!("The clipboard is busy: {e}"))
            }
            Err(e) => {
                lock(&self.loop_guard).forget();
                Err(e.to_string())
            }
        }
    }

    /// The Cancel button of the history: a queued item never goes out, an item that is going is canceled the ordinary way.
    pub async fn history_cancel(self: &Arc<Self>, id: &str) -> bool {
        if self.cancel_queued(id).await {
            return true;
        }
        match self.transfers.get(id) {
            Some(e) if e.status() == Status::Active => self.cancel_transfer(id).await,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queuedb::HistoryFile;

    fn row(kind: &str, status: &str, files: &[&str]) -> HistoryRow {
        HistoryRow {
            id: "t1".into(),
            device_id: "d".into(),
            direction: "in".into(),
            kind: kind.into(),
            status: status.into(),
            created_at: 1,
            finished_at: Some(2),
            total_bytes: 5,
            error_code: None,
            retry_of: None,
            retried: false,
            text: (kind == "text").then(|| "hello".to_string()),
            is_url: false,
            is_link: false,
            title: None,
            files: files
                .iter()
                .map(|p| HistoryFile { name: "n".into(), size: 1, local_path: Some((*p).into()), finalized: true })
                .collect(),
        }
    }

    #[test]
    fn a_click_follows_the_allow_list() {
        // A picture or a PDF opens, anything else is shown in its folder (the 1.3 rule).
        assert_eq!(open_action(&row("file", "done", &["C:\\r\\photo.jpg"])), OpenAction::OpenFile("C:\\r\\photo.jpg".into()));
        assert_eq!(open_action(&row("file", "done", &["C:\\r\\Report.PDF"])), OpenAction::OpenFile("C:\\r\\Report.PDF".into()));
        for risky in ["C:\\r\\setup.exe", "C:\\r\\run.ps1", "C:\\r\\a.pdf.exe", "C:\\r\\macro.xlsm", "C:\\r\\noext", "C:\\r\\x.sh"] {
            assert_eq!(open_action(&row("file", "done", &[risky])), OpenAction::ShowFile(risky.into()), "{risky}");
        }
        // Several files: their folder. A text: copy.
        assert_eq!(open_action(&row("file", "done", &["C:\\r\\a.jpg", "C:\\r\\b.jpg"])), OpenAction::OpenFolder("C:\\r".into()));
        assert_eq!(open_action(&row("text", "done", &[])), OpenAction::CopyText);
    }

    #[test]
    fn only_a_finished_file_can_be_opened_but_any_text_can_be_copied() {
        for status in ["queued", "active", "failed", "canceled"] {
            assert_eq!(open_action(&row("file", status, &["C:\\r\\photo.jpg"])), OpenAction::Nothing, "{status}");
            assert_eq!(open_action(&row("text", status, &[])), OpenAction::CopyText, "{status}");
        }
        assert_eq!(open_action(&row("file", "done", &[])), OpenAction::Nothing);
    }

    #[test]
    fn a_text_shows_on_one_line_and_at_most_160_characters() {
        let long = format!("first line\r\n\tsecond   line {}", "x".repeat(400));
        let shown = one_line(&long, TEXT_PREVIEW_CHARS);
        assert!(!shown.contains('\n') && !shown.contains('\t') && !shown.contains("  "), "{shown:?}");
        assert!(shown.chars().count() <= TEXT_PREVIEW_CHARS + 1, "{}", shown.chars().count());
        assert!(shown.starts_with("first line second line"));
    }
}
