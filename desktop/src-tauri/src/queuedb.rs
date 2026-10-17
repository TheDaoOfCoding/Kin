//! The database side of milestone 1.4: the queue (`transfers` rows with the status `queued`, and the body of a text in
//! `transfer_texts`), the history (the same rows, newest first), the send log that the debug window's success rate is computed from
//! (`sends`), and the connection history (`connection_log`).
//!
//! **What is stored, and what is not.** An explicit send (`text`, files) is a `transfers` row from the moment it is made, so it
//! survives a restart. A `sensitive` text and the automatic `clipboard` are never a row of `transfers` or `transfer_texts` (see
//! protocol.md, "Queue"): the history can't show what was never stored. The `sends` table holds no text and no file name, only the
//! kind, the times and the outcome.
use rusqlite::{OptionalExtension, params};

use crate::Result;
use crate::storage::Db;

/// How many rows of the connection history are kept.
const CONNECTION_LOG_KEPT: i64 = 200;
/// How long a row of `sends` is kept.
const SENDS_KEPT_MS: i64 = 90 * 24 * 3_600_000;

/// A queued item, as the flush needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct QueueItem {
    pub id: String,
    pub device_id: String,
    /// `text` or `file`
    pub kind: String,
    pub created_at: i64,
    /// When the item stops being valid (a queued text 7 days after it was made, an offer 7 days after it was made until it is sent and
    /// 24 hours after that).
    pub expires_at: Option<i64>,
    /// When it was first sent (the first attempt on an open connection): an offer that has been sent and is waiting for its download to
    /// start, or to be sent again.
    pub sent_at: Option<i64>,
    pub total_bytes: i64,
}

/// A file of a history row.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryFile {
    pub name: String,
    pub size: i64,
    pub local_path: Option<String>,
    pub finalized: bool,
}

/// One row of the history window.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryRow {
    pub id: String,
    pub device_id: String,
    pub direction: String,
    pub kind: String,
    pub status: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub total_bytes: i64,
    pub error_code: Option<String>,
    pub retry_of: Option<String>,
    /// A later item was made from this one by a retry.
    pub retried: bool,
    /// A text item: its text.
    pub text: Option<String>,
    pub is_url: bool,
    /// A link item (2B.1): `text` is its URL.
    pub is_link: bool,
    /// The cleaned title of a link item.
    pub title: Option<String>,
    pub files: Vec<HistoryFile>,
}

/// The stored body of a text item (a text, or a link).
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    pub body: String,
    pub is_url: bool,
    pub is_link: bool,
    pub title: Option<String>,
    pub source_app: Option<String>,
}

/// One send, for the success rate and the latency numbers of the debug window. No text and no file name.
#[derive(Clone, Debug, PartialEq)]
pub struct SendRow {
    pub id: String,
    pub at: i64,
    /// `text`, `clipboard`, `offer` (PC to phone), `upload` (phone to PC) or `ping`
    pub kind: String,
    /// How many times it was tried: 1 means it went through at the first attempt.
    pub attempts: i64,
    /// `pending`, `ok`, `failed` or `canceled`
    pub result: String,
    pub error: Option<String>,
    /// Tap to `ack` for `text` and `clipboard`, the round trip for `ping`.
    pub latency_ms: Option<f64>,
    /// The phone was not connected when the user asked, so the send was queued: not a failure of the network.
    pub offline_at_tap: bool,
}

/// A line of the connection history.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ConnectionEvent {
    pub at: i64,
    /// `connected` or `disconnected`
    pub event: String,
    pub detail: String,
}

pub(crate) const SCHEMA: &str = "
-- The body of a text item (queued, sent or received). Only an explicit, non-sensitive text is ever here.
CREATE TABLE IF NOT EXISTS transfer_texts (
    transfer_id TEXT PRIMARY KEY,
    body        TEXT NOT NULL,
    is_url      INTEGER NOT NULL DEFAULT 0
);
-- One row per send, for the success rate (targets.md #1) and the latency numbers. No text, no file name.
CREATE TABLE IF NOT EXISTS sends (
    id             TEXT PRIMARY KEY,
    at             INTEGER NOT NULL,
    kind           TEXT NOT NULL,
    attempts       INTEGER NOT NULL DEFAULT 1,
    result         TEXT NOT NULL,
    error          TEXT,
    latency_ms     REAL,
    offline_at_tap INTEGER NOT NULL DEFAULT 0
);
-- A `cancel` message that the phone has not acked. It is kept until it is, and sent again (same message id) after every `hello`, before the queue
-- (protocol.md, changelog 52). One row per transfer.
CREATE TABLE IF NOT EXISTS pending_cancels (
    transfer_id TEXT PRIMARY KEY,
    message_id  TEXT NOT NULL,
    device_id   TEXT NOT NULL,
    reason      TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS connection_log (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    at     INTEGER NOT NULL,
    event  TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);
";

const SELECT_HISTORY: &str = "SELECT t.id, t.device_id, t.direction, t.kind, t.status, t.created_at, t.finished_at, t.total_bytes, t.error_code, t.retry_of, x.body, COALESCE(x.is_url, 0), EXISTS (SELECT 1 FROM transfers r WHERE r.retry_of = t.id), COALESCE(x.is_link, 0), x.link_title
     FROM transfers t LEFT JOIN transfer_texts x ON x.transfer_id = t.id";

impl Db {
    // ----- the queue -----------------------------------------------------------------------------------------------------

    /// Stores a text item (an explicit, non-sensitive text) with the given status: `queued` for a send that has not gone out,
    /// `done` for one that was already acked or that arrived from the phone.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_text_item(
        &self,
        id: &str,
        device_id: &str,
        direction: &str,
        status: &str,
        created_at: i64,
        expires_at: Option<i64>,
        body: &str,
        is_url: bool,
        retry_of: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let finished = (status != "queued" && status != "active").then_some(created_at);
        tx.execute(
            "INSERT INTO transfers (id, device_id, direction, kind, status, created_at, updated_at, finished_at, expires_at, total_bytes, error_code, dest_dir, retry_of)
             VALUES (?1, ?2, ?3, 'text', ?4, ?5, ?5, ?6, ?7, ?8, NULL, NULL, ?9)",
            params![id, device_id, direction, status, created_at, finished, expires_at, body.len() as i64, retry_of],
        )?;
        tx.execute("INSERT INTO transfer_texts (transfer_id, body, is_url) VALUES (?1, ?2, ?3)", params![id, body, is_url as i64])?;
        tx.commit()?;
        Ok(())
    }

    /// Stores a link item (2B.1): like a text item, with the URL as its body and the title and sharing app next to it. `is_url` is true.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_link_item(
        &self,
        id: &str,
        device_id: &str,
        direction: &str,
        status: &str,
        created_at: i64,
        expires_at: Option<i64>,
        url: &str,
        title: Option<&str>,
        source_app: Option<&str>,
        retry_of: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let finished = (status != "queued" && status != "active").then_some(created_at);
        tx.execute(
            "INSERT INTO transfers (id, device_id, direction, kind, status, created_at, updated_at, finished_at, expires_at, total_bytes, error_code, dest_dir, retry_of)
             VALUES (?1, ?2, ?3, 'text', ?4, ?5, ?5, ?6, ?7, ?8, NULL, NULL, ?9)",
            params![id, device_id, direction, status, created_at, finished, expires_at, url.len() as i64, retry_of],
        )?;
        tx.execute(
            "INSERT INTO transfer_texts (transfer_id, body, is_url, is_link, link_title, link_source) VALUES (?1, ?2, 1, 1, ?3, ?4)",
            params![id, url, title, source_app],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The whole stored body of a text item: text or link.
    pub fn text_item(&self, id: &str) -> Result<Option<TextItem>> {
        Ok(self
            .conn()
            .query_row("SELECT body, is_url, is_link, link_title, link_source FROM transfer_texts WHERE transfer_id = ?1", [id], |r| {
                Ok(TextItem {
                    body: r.get(0)?,
                    is_url: r.get::<_, i64>(1)? != 0,
                    is_link: r.get::<_, i64>(2)? != 0,
                    title: r.get(3)?,
                    source_app: r.get(4)?,
                })
            })
            .optional()?)
    }

    /// The text of a text item, and whether it is a link.
    pub fn text_body(&self, id: &str) -> Result<Option<(String, bool)>> {
        Ok(self
            .conn()
            .query_row("SELECT body, is_url FROM transfer_texts WHERE transfer_id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get::<_, i64>(1)? != 0))
            })
            .optional()?)
    }

    /// Whether a retry was made of this item.
    pub fn is_retried(&self, id: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT EXISTS (SELECT 1 FROM transfers WHERE retry_of = ?1)", [id], |r| r.get::<_, i64>(0))? != 0)
    }

    /// The expiry of a transfer (its `expires_at`): a queued item's 7 days, or 24 hours after an offer was sent.
    pub fn set_expiry(&self, id: &str, expires_at: Option<i64>) -> Result<()> {
        self.conn().execute("UPDATE transfers SET expires_at = ?2 WHERE id = ?1", params![id, expires_at])?;
        Ok(())
    }

    /// When an item was first sent: the first attempt on an open connection. (A repeat after a reconnect doesn't move it.)
    pub fn mark_sent(&self, id: &str, now_ms: i64) -> Result<()> {
        self.conn().execute("UPDATE transfers SET sent_at = COALESCE(sent_at, ?2), updated_at = ?2 WHERE id = ?1", params![id, now_ms])?;
        Ok(())
    }

    /// An offer was acked: `active`, but only if it is still `queued`. A cancel, a `complete` or an expiry that came while the `ack` was awaited
    /// has ended it already, and that is not undone. Returns whether it was changed.
    pub fn activate_if_queued(&self, id: &str, now_ms: i64) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE transfers SET status = 'active', updated_at = ?2, finished_at = NULL, error_code = NULL WHERE id = ?1 AND status = 'queued'",
            params![id, now_ms],
        )? > 0)
    }

    /// Whether a `complete` from `device_id` for this transfer, which is not in memory, still means something: an offer that was sent and never
    /// acked, or one that failed because a repeat got `bad_request`.
    pub fn late_complete_candidate(&self, id: &str, device_id: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM transfers WHERE id = ?1 AND device_id = ?2 AND direction = 'out' AND kind = 'file'
               AND ((status = 'failed' AND error_code = 'bad_request') OR (status = 'queued' AND sent_at IS NOT NULL)))",
            params![id, device_id],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    /// The temporary files (the `.txt` of a long text) that a transfer still needs: queued, or active. The clean-up of `outgoing/` never
    /// removes these, whatever their age: a queued offer lives in the database alone until it is flushed.
    pub fn live_temp_sources(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT f.local_path FROM transfer_files f JOIN transfers t ON t.id = f.transfer_id
             WHERE f.temp_source = 1 AND f.local_path IS NOT NULL AND t.status IN ('queued', 'active')",
        )?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?)
    }

    /// A text body goes once its item is beyond the newest `keep` rows the history shows, unless the item is still waiting or going (a queued
    /// text needs its body). The `transfers` row stays for good: a transfer ID is never reused. Returns how many bodies went.
    pub fn prune_text_bodies(&self, keep: usize) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM transfer_texts WHERE transfer_id IN (
               SELECT t.id FROM transfers t WHERE t.status NOT IN ('queued', 'active')
                 AND t.id NOT IN (SELECT id FROM transfers ORDER BY created_at DESC, rowid DESC LIMIT ?1))",
            [keep as i64],
        )?)
    }

    /// An item whose send got no `ack` (or whose phone went away) goes back to `queued`, in its place in the order.
    pub fn requeue(&self, id: &str, now_ms: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE transfers SET status = 'queued', finished_at = NULL, error_code = NULL, updated_at = ?2 WHERE id = ?1 AND status IN ('queued', 'active')",
            params![id, now_ms],
        )?;
        Ok(())
    }

    /// Queued items of a device, oldest first (the order they go out in).
    pub fn queued_items(&self, device_id: &str) -> Result<Vec<QueueItem>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, device_id, kind, created_at, expires_at, sent_at, total_bytes FROM transfers
             WHERE status = 'queued' AND direction = 'out' AND device_id = ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([device_id], read_queue_item)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every queued item, whatever the device (for the tray and `status.json`).
    pub fn all_queued(&self) -> Result<Vec<QueueItem>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, device_id, kind, created_at, expires_at, sent_at, total_bytes FROM transfers
             WHERE status = 'queued' AND direction = 'out' ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([], read_queue_item)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn queued_count(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM transfers WHERE status = 'queued' AND direction = 'out'", [], |r| r.get(0))?)
    }

    /// When an item was first sent, and when it expires.
    pub fn sent_and_expiry(&self, id: &str) -> Result<Option<(Option<i64>, Option<i64>)>> {
        Ok(self
            .conn()
            .query_row("SELECT sent_at, expires_at FROM transfers WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    /// The status of a transfer and its kind, if it exists.
    pub fn status_of(&self, id: &str) -> Result<Option<(String, String)>> {
        Ok(self.conn().query_row("SELECT status, kind FROM transfers WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    /// Items that are over their time: queued ones past `expires_at`. (An offer that was sent and is live expires through
    /// `Core::clean_up`, which tells the phone.) Returns their IDs.
    pub fn expired_queued(&self, now_ms: i64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id FROM transfers WHERE status = 'queued' AND direction = 'out' AND expires_at IS NOT NULL AND expires_at <= ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([now_ms], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ----- `cancel` messages that wait for their ack ----------------------------------------------------------------------

    /// Keeps a `cancel` for `transfer_id` until the phone acks it. Returns the message ID to send it with: a second cancel of the same transfer
    /// is the same message.
    pub fn add_pending_cancel(&self, device_id: &str, transfer_id: &str, reason: &str, message_id: &str, now_ms: i64) -> Result<String> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO pending_cancels (transfer_id, message_id, device_id, reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![transfer_id, message_id, device_id, reason, now_ms],
        )?;
        Ok(conn.query_row("SELECT message_id FROM pending_cancels WHERE transfer_id = ?1", [transfer_id], |r| r.get(0))?)
    }

    /// The `cancel` messages not acked yet for a device, oldest first: `(message ID, transfer ID, reason)`.
    pub fn pending_cancels(&self, device_id: &str) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT message_id, transfer_id, reason FROM pending_cancels WHERE device_id = ?1 ORDER BY created_at, rowid")?;
        Ok(stmt.query_map([device_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?)
    }

    /// The phone acked (or answered with an error to) the `cancel` with this message ID: it is done.
    pub fn delete_pending_cancel(&self, message_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM pending_cancels WHERE message_id = ?1", [message_id])?;
        Ok(())
    }

    /// A device that was unpaired has no use for the cancels that were waiting for it.
    pub fn delete_pending_cancels_of(&self, device_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM pending_cancels WHERE device_id = ?1", [device_id])?;
        Ok(())
    }

    pub fn pending_cancel_count(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM pending_cancels", [], |r| r.get(0))?)
    }

    // ----- the history ---------------------------------------------------------------------------------------------------

    /// The newest `limit` items, newest first, with their files and (for a text) the text.
    pub fn history(&self, limit: usize) -> Result<Vec<HistoryRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{SELECT_HISTORY} ORDER BY t.created_at DESC, t.rowid DESC LIMIT ?1"))?;
        let mut rows: Vec<HistoryRow> = stmt.query_map([limit as i64], read_history)?.collect::<std::result::Result<_, _>>()?;
        let mut files = conn.prepare("SELECT name, size, local_path, finalized FROM transfer_files WHERE transfer_id = ?1 ORDER BY idx")?;
        for row in rows.iter_mut().filter(|r| r.kind == "file") {
            row.files = files
                .query_map([&row.id], |r| {
                    Ok(HistoryFile { name: r.get(0)?, size: r.get(1)?, local_path: r.get(2)?, finalized: r.get::<_, i64>(3)? != 0 })
                })?
                .collect::<std::result::Result<_, _>>()?;
        }
        Ok(rows)
    }

    pub fn history_item(&self, id: &str) -> Result<Option<HistoryRow>> {
        let conn = self.conn();
        let row = conn.query_row(&format!("{SELECT_HISTORY} WHERE t.id = ?1"), [id], read_history).optional()?;
        let Some(mut row) = row else {
            return Ok(None);
        };
        if row.kind == "file" {
            let mut files =
                conn.prepare("SELECT name, size, local_path, finalized FROM transfer_files WHERE transfer_id = ?1 ORDER BY idx")?;
            row.files = files
                .query_map([&row.id], |r| {
                    Ok(HistoryFile { name: r.get(0)?, size: r.get(1)?, local_path: r.get(2)?, finalized: r.get::<_, i64>(3)? != 0 })
                })?
                .collect::<std::result::Result<_, _>>()?;
        }
        Ok(Some(row))
    }

    // ----- the send log --------------------------------------------------------------------------------------------------

    /// Writes (or replaces) the record of a send. The record of one send is written when it is first tried, and updated when it ends.
    pub fn upsert_send(&self, s: &SendRow) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sends (id, at, kind, attempts, result, error, latency_ms, offline_at_tap) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET attempts = ?4, result = ?5, error = ?6, latency_ms = COALESCE(?7, latency_ms)",
            params![s.id, s.at, s.kind, s.attempts, s.result, s.error, s.latency_ms, s.offline_at_tap as i64],
        )?;
        Ok(())
    }

    /// A send that was recorded as failed turns out to have worked (a late `complete`): its record is open again, so it can end as `ok`.
    pub fn reopen_send(&self, id: &str) -> Result<()> {
        self.conn().execute("UPDATE sends SET result = 'pending', error = NULL WHERE id = ?1 AND result = 'failed'", [id])?;
        Ok(())
    }

    pub fn get_send(&self, id: &str) -> Result<Option<SendRow>> {
        Ok(self
            .conn()
            .query_row("SELECT id, at, kind, attempts, result, error, latency_ms, offline_at_tap FROM sends WHERE id = ?1", [id], read_send)
            .optional()?)
    }

    /// The sends since `since_ms`, newest first. **A send that the user retried is one send, counted by the item that took its place**
    /// (the phone does the same): the failed one that was retried is left out.
    pub fn sends_since(&self, since_ms: i64) -> Result<Vec<SendRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, kind, attempts, result, error, latency_ms, offline_at_tap FROM sends
             WHERE at >= ?1 AND id NOT IN (SELECT retry_of FROM transfers WHERE retry_of IS NOT NULL) ORDER BY at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([since_ms], read_send)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The newest failed sends, newest first.
    pub fn recent_send_errors(&self, limit: usize) -> Result<Vec<SendRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, kind, attempts, result, error, latency_ms, offline_at_tap FROM sends WHERE error IS NOT NULL ORDER BY at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], read_send)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ----- the connection history ----------------------------------------------------------------------------------------

    pub fn log_connection(&self, at_ms: i64, event: &str, detail: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("INSERT INTO connection_log (at, event, detail) VALUES (?1, ?2, ?3)", params![at_ms, event, detail])?;
        conn.execute(
            "DELETE FROM connection_log WHERE id NOT IN (SELECT id FROM connection_log ORDER BY id DESC LIMIT ?1)",
            [CONNECTION_LOG_KEPT],
        )?;
        Ok(())
    }

    /// The newest connection events, newest first.
    pub fn connection_events(&self, limit: usize) -> Result<Vec<ConnectionEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT at, event, detail FROM connection_log ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| Ok(ConnectionEvent { at: r.get(0)?, event: r.get(1)?, detail: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Old rows of the send log go.
    pub fn prune_sends(&self, now_ms: i64) -> Result<()> {
        self.conn().execute("DELETE FROM sends WHERE at < ?1", [now_ms - SENDS_KEPT_MS])?;
        Ok(())
    }
}

fn read_queue_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<QueueItem> {
    Ok(QueueItem {
        id: r.get(0)?,
        device_id: r.get(1)?,
        kind: r.get(2)?,
        created_at: r.get(3)?,
        expires_at: r.get(4)?,
        sent_at: r.get(5)?,
        total_bytes: r.get(6)?,
    })
}

fn read_history(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryRow> {
    Ok(HistoryRow {
        id: r.get(0)?,
        device_id: r.get(1)?,
        direction: r.get(2)?,
        kind: r.get(3)?,
        status: r.get(4)?,
        created_at: r.get(5)?,
        finished_at: r.get(6)?,
        total_bytes: r.get(7)?,
        error_code: r.get(8)?,
        retry_of: r.get(9)?,
        text: r.get(10)?,
        is_url: r.get::<_, i64>(11)? != 0,
        retried: r.get::<_, i64>(12)? != 0,
        is_link: r.get::<_, i64>(13)? != 0,
        title: r.get(14)?,
        files: Vec::new(),
    })
}

fn read_send(r: &rusqlite::Row<'_>) -> rusqlite::Result<SendRow> {
    Ok(SendRow {
        id: r.get(0)?,
        at: r.get(1)?,
        kind: r.get(2)?,
        attempts: r.get(3)?,
        result: r.get(4)?,
        error: r.get(5)?,
        latency_ms: r.get(6)?,
        offline_at_tap: r.get::<_, i64>(7)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::NewDevice;

    fn db() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.upsert_device(&NewDevice { device_id: "phone", name: "Pixel", platform: "android", model: None, app_version: None }, 1).unwrap();
        db
    }

    #[test]
    fn queued_items_come_back_oldest_first_and_only_for_their_device() {
        let db = db();
        db.insert_text_item("b", "phone", "out", "queued", 20, Some(1000), "second", false, None).unwrap();
        db.insert_text_item("a", "phone", "out", "queued", 10, Some(1000), "first", false, None).unwrap();
        db.insert_text_item("c", "other", "out", "queued", 5, Some(1000), "elsewhere", false, None).unwrap();
        db.insert_text_item("d", "phone", "in", "done", 1, None, "received", false, None).unwrap();
        let ids: Vec<String> = db.queued_items("phone").unwrap().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["a", "b"], "creation order, this device, outgoing only");
        assert_eq!(db.queued_count().unwrap(), 3);
        assert_eq!(db.text_body("a").unwrap(), Some(("first".to_string(), false)));
    }

    #[test]
    fn requeue_puts_an_active_item_back_and_a_done_one_stays_done() {
        let db = db();
        db.insert_text_item("a", "phone", "out", "queued", 10, None, "x", false, None).unwrap();
        db.set_transfer_status("a", "done", 11, None).unwrap();
        db.requeue("a", 12).unwrap();
        assert_eq!(db.status_of("a").unwrap().unwrap().0, "done");
        db.insert_text_item("b", "phone", "out", "active", 10, None, "x", false, None).unwrap();
        db.requeue("b", 12).unwrap();
        assert_eq!(db.status_of("b").unwrap().unwrap().0, "queued");
    }

    #[test]
    fn expired_queued_lists_only_queued_items_at_or_past_their_time() {
        let db = db();
        db.insert_text_item("old", "phone", "out", "queued", 1, Some(100), "x", false, None).unwrap();
        db.insert_text_item("edge", "phone", "out", "queued", 2, Some(200), "x", false, None).unwrap();
        db.insert_text_item("fresh", "phone", "out", "queued", 3, Some(300), "x", false, None).unwrap();
        db.insert_text_item("done", "phone", "out", "done", 4, Some(100), "x", false, None).unwrap();
        assert_eq!(db.expired_queued(200).unwrap(), ["old", "edge"], "at exactly the expiry it is expired");
        assert_eq!(db.expired_queued(199).unwrap(), ["old"]);
    }

    fn offer_row(id: &str, status: &str, created: i64) -> (crate::storage::TransferRow, Vec<crate::storage::TransferFileRow>) {
        let row = crate::storage::TransferRow {
            id: id.into(),
            device_id: "phone".into(),
            direction: "out".into(),
            status: status.into(),
            created_at: created,
            updated_at: created,
            finished_at: None,
            expires_at: Some(created + 1000),
            total_bytes: 3,
            error_code: None,
            dest_dir: None,
        };
        let file = crate::storage::TransferFileRow {
            file_id: "f1".into(),
            idx: 0,
            name: "Text.txt".into(),
            mime: "text/plain".into(),
            size: 3,
            bytes_done: 0,
            local_path: Some(format!("C:\\data\\outgoing\\{id}\\Text.txt")),
            finalized: false,
            source_mtime_ms: Some(1),
            temp_source: true,
            finalizing_name: None,
        };
        (row, vec![file])
    }

    #[test]
    fn an_item_can_be_retried_once_the_database_refuses_a_second_retry_of_it() {
        let db = db();
        db.insert_text_item("failed", "phone", "out", "failed", 1, None, "x", false, None).unwrap();
        db.insert_text_item("again-1", "phone", "out", "queued", 2, None, "x", false, Some("failed")).unwrap();
        assert!(
            db.insert_text_item("again-2", "phone", "out", "queued", 3, None, "x", false, Some("failed")).is_err(),
            "a second text retry"
        );
        let (row, files) = offer_row("offer-retry", "queued", 4);
        assert!(db.insert_transfer_retry(&row, &files, Some("failed")).is_err(), "a second retry as an offer");
        assert!(!db.transfer_exists("offer-retry").unwrap(), "nothing of the refused retry is left (one transaction)");
        let (row, files) = offer_row("offer-ok", "queued", 5);
        db.insert_transfer_retry(&row, &files, Some("failed-other")).unwrap();
        assert!(db.is_retried("failed-other").unwrap());
    }

    #[test]
    fn an_ack_only_activates_an_item_that_is_still_queued() {
        let db = db();
        db.insert_text_item("a", "phone", "out", "queued", 1, None, "x", false, None).unwrap();
        db.insert_text_item("b", "phone", "out", "queued", 1, None, "x", false, None).unwrap();
        db.set_transfer_status("b", "canceled", 5, Some("user")).unwrap();
        assert!(db.activate_if_queued("a", 10).unwrap());
        assert!(!db.activate_if_queued("b", 10).unwrap(), "a cancel that came during the ack wait stays");
        assert_eq!(db.status_of("a").unwrap().unwrap().0, "active");
        assert_eq!(db.status_of("b").unwrap().unwrap().0, "canceled");
        assert!(!db.activate_if_queued("missing", 10).unwrap());
    }

    #[test]
    fn temporary_files_of_queued_and_active_items_count_as_in_use_and_those_of_finished_ones_do_not() {
        let db = db();
        for (id, status) in [("q", "queued"), ("a", "active"), ("d", "done"), ("f", "failed"), ("c", "canceled")] {
            let (row, files) = offer_row(id, status, 1);
            db.insert_transfer(&row, &files).unwrap();
        }
        let mut live = db.live_temp_sources().unwrap();
        live.sort();
        assert_eq!(live, [r"C:\data\outgoing\a\Text.txt", r"C:\data\outgoing\q\Text.txt"]);
    }

    #[test]
    fn a_cancel_message_is_kept_per_transfer_until_it_is_acked_or_the_device_is_gone() {
        let db = db();
        let first = db.add_pending_cancel("phone", "t1", "user", "m-1", 10).unwrap();
        assert_eq!(first, "m-1");
        assert_eq!(db.add_pending_cancel("phone", "t1", "expired", "m-2", 20).unwrap(), "m-1", "the same transfer is the same message");
        db.add_pending_cancel("phone", "t2", "user", "m-3", 30).unwrap();
        db.add_pending_cancel("other", "t3", "user", "m-4", 5).unwrap();
        let pending: Vec<String> = db.pending_cancels("phone").unwrap().into_iter().map(|(m, _, _)| m).collect();
        assert_eq!(pending, ["m-1", "m-3"], "oldest first, this device only");
        db.delete_pending_cancel("m-1").unwrap();
        assert_eq!(db.pending_cancels("phone").unwrap().len(), 1);
        db.delete_pending_cancels_of("phone").unwrap();
        assert_eq!((db.pending_cancels("phone").unwrap().len(), db.pending_cancel_count().unwrap()), (0, 1));
    }

    #[test]
    fn text_bodies_beyond_the_kept_ones_go_but_never_those_of_queued_or_active_items() {
        let db = db();
        db.insert_text_item("old-queued", "phone", "out", "queued", 1, None, "keep me", false, None).unwrap();
        db.insert_text_item("old-active", "phone", "out", "active", 2, None, "keep me too", false, None).unwrap();
        db.insert_text_item("old-done", "phone", "in", "done", 3, None, "go", false, None).unwrap();
        db.insert_text_item("new-1", "phone", "in", "done", 4, None, "stay", false, None).unwrap();
        db.insert_text_item("new-2", "phone", "in", "done", 5, None, "stay", false, None).unwrap();
        assert_eq!(db.prune_text_bodies(2).unwrap(), 1);
        assert_eq!(db.text_body("old-done").unwrap(), None);
        assert!(db.text_body("old-queued").unwrap().is_some() && db.text_body("old-active").unwrap().is_some());
        assert!(db.text_body("new-1").unwrap().is_some() && db.text_body("new-2").unwrap().is_some());
        assert_eq!(db.prune_text_bodies(2).unwrap(), 0, "nothing more to do");
        assert!(db.transfer_exists("old-done").unwrap(), "the row stays: a transfer ID is never reused");
    }

    #[test]
    fn a_sent_and_unacked_offer_is_loaded_at_start_and_a_late_complete_is_known() {
        let db = db();
        let (row, files) = offer_row("sent", "queued", 1);
        db.insert_transfer(&row, &files).unwrap();
        let (row, files) = offer_row("never-sent", "queued", 2);
        db.insert_transfer(&row, &files).unwrap();
        db.mark_sent("sent", 5).unwrap();
        let loaded: Vec<String> = db.load_open_transfers(0).unwrap().into_iter().map(|(t, _)| t.id).collect();
        assert_eq!(loaded, ["sent"], "only an offer that was written to a connection");
        assert!(db.late_complete_candidate("sent", "phone").unwrap());
        assert!(!db.late_complete_candidate("sent", "someone-else").unwrap(), "another device's complete means nothing");
        assert!(!db.late_complete_candidate("never-sent", "phone").unwrap());
        db.set_transfer_status("never-sent", "failed", 6, Some("bad_request")).unwrap();
        assert!(db.late_complete_candidate("never-sent", "phone").unwrap(), "failed because a repeat got bad_request");
        db.set_transfer_status("sent", "failed", 6, Some("disk_full")).unwrap();
        assert!(!db.late_complete_candidate("sent", "phone").unwrap(), "failed for another reason: the phone refused it");
    }

    #[test]
    fn the_history_has_text_bodies_and_newest_first() {
        let db = db();
        db.insert_text_item("a", "phone", "in", "done", 10, None, "hello", true, None).unwrap();
        db.insert_text_item("b", "phone", "out", "queued", 20, Some(5), "later", false, Some("a")).unwrap();
        let rows = db.history(10).unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!((rows[0].text.as_deref(), rows[0].retry_of.as_deref()), (Some("later"), Some("a")));
        assert!(rows[1].is_url);
        assert_eq!(db.history(1).unwrap().len(), 1, "capped");
    }

    #[test]
    fn a_send_record_is_written_at_the_first_try_and_updated_when_it_ends() {
        let db = db();
        let mut s = SendRow {
            id: "s".into(),
            at: 5,
            kind: "text".into(),
            attempts: 1,
            result: "pending".into(),
            error: None,
            latency_ms: None,
            offline_at_tap: true,
        };
        db.upsert_send(&s).unwrap();
        s.attempts = 3;
        s.result = "ok".into();
        s.latency_ms = Some(41.0);
        s.offline_at_tap = false; // the update does not change what was true at the tap
        db.upsert_send(&s).unwrap();
        let got = db.get_send("s").unwrap().unwrap();
        assert_eq!((got.attempts, got.result.as_str(), got.latency_ms, got.offline_at_tap), (3, "ok", Some(41.0), true));
    }

    #[test]
    fn the_connection_history_keeps_the_newest_200() {
        let db = db();
        for i in 0..230 {
            db.log_connection(i, if i % 2 == 0 { "connected" } else { "disconnected" }, "x").unwrap();
        }
        let events = db.connection_events(1000).unwrap();
        assert_eq!(events.len(), 200);
        assert_eq!(events[0].at, 229, "newest first");
    }
}
