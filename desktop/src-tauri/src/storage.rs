//! SQLite storage: the `devices` table, the memory of processed message IDs that protocol.md asks for ("De-duplication"), which
//! has to survive a restart, and (1.3) the `transfers` and `transfer_files` tables of architecture.md ("Storage").
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;

/// A repeat is recognised for at least this long, and never fewer than the newest `DEDUP_MIN_IDS` IDs.
const DEDUP_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
const DEDUP_MIN_IDS: i64 = 1000;
const LAST_ADDRESSES_KEPT: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub model: Option<String>,
    pub app_version: Option<String>,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    /// The most recent addresses the device connected from, newest first.
    pub last_addresses: Vec<String>,
}

pub struct NewDevice<'a> {
    pub device_id: &'a str,
    pub name: &'a str,
    pub platform: &'a str,
    pub model: Option<&'a str>,
    pub app_version: Option<&'a str>,
}

pub struct Db(Mutex<Connection>);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS devices (
    device_id      TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    platform       TEXT NOT NULL,
    model          TEXT,
    app_version    TEXT,
    paired_at      INTEGER NOT NULL,
    last_seen      INTEGER,
    last_addresses TEXT NOT NULL DEFAULT '[]'
);
-- Every name a phone has had here, kept when the phone is unpaired or renamed: the logs still hold those names, and the export
-- (export.rs) must take them out of the logs too. Not part of any view.
CREATE TABLE IF NOT EXISTS device_names (
    device_id TEXT NOT NULL,
    name      TEXT NOT NULL,
    PRIMARY KEY (device_id, name)
);
CREATE TABLE IF NOT EXISTS seen_messages (
    device_id  TEXT NOT NULL,
    message_id TEXT NOT NULL,
    response   TEXT NOT NULL,
    seen_at    INTEGER NOT NULL,
    PRIMARY KEY (device_id, message_id)
);
-- direction: 'in' (phone to PC) or 'out' (PC to phone). kind: 'file' or 'text' (1.4: the text is in `transfer_texts`). status: 'queued' (1.4: made
-- and persisted, not sent yet), 'active', 'done', 'failed' or 'canceled'. 1.4 adds `sent_at` and `retry_of` (see `ensure_column`).
CREATE TABLE IF NOT EXISTS transfers (
    id          TEXT PRIMARY KEY,
    device_id   TEXT NOT NULL,
    direction   TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'file',
    status      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    finished_at INTEGER,
    expires_at  INTEGER,
    total_bytes INTEGER NOT NULL,
    error_code  TEXT,
    dest_dir    TEXT
);
-- name: the final name of a received file (the one the PC chose), the sender's name for an outgoing one. local_path: the final
-- path once finalized, else the .part path (in), or the source file (out); NULL once a dropped transfer's .part file has been removed.
-- finalizing_name: the name the .part file is being moved to (written before the move, cleared when the move is recorded), so a
-- restart in the middle can tell a finished move from a lost file.
-- A transfer ID is never reused: rows of finished, canceled and failed transfers are kept, and a new transfer with a used ID is refused.
CREATE TABLE IF NOT EXISTS transfer_files (
    transfer_id     TEXT NOT NULL,
    file_id         TEXT NOT NULL,
    idx             INTEGER NOT NULL,
    name            TEXT NOT NULL,
    mime            TEXT NOT NULL,
    size            INTEGER NOT NULL,
    bytes_done      INTEGER NOT NULL DEFAULT 0,
    local_path      TEXT,
    finalized       INTEGER NOT NULL DEFAULT 0,
    source_mtime_ms INTEGER,
    temp_source     INTEGER NOT NULL DEFAULT 0,
    finalizing_name TEXT,
    PRIMARY KEY (transfer_id, file_id)
);
";

/// Adds a column to a table that was made before the column existed.
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let has = conn
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{column}'"))?
        .query_row([], |_| Ok(()))
        .optional()?
        .is_some();
    if !has {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path)?;
        // v0.1.1: write-ahead log, and no fsync at every commit (`synchronous = NORMAL`: the log is synced when it is checkpointed). A received
        // file used to cost the database four commits of two fsyncs each, one after the other behind one lock, on threads of the async runtime:
        // with 4 files arriving at once that was the slowest part of a photo batch (50 photos over loopback: 1.2 s, 0.3 to 0.6 s with this).
        // What it gives up: after a power cut the last moments of progress may be missing from the database (never a corrupt database). That is
        // safe here: a resume trusts the smaller of the database and the `.part` file on disk, so a progress that is short only makes the
        // phone send a little again, and the final move of a file is recorded after the file is already `fsync`ed. (The worst case: a file that
        // was being moved to its final name at the moment of the power cut arrives a second time, under the next free name.)
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Db::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Db::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(crate::queuedb::SCHEMA)?;
        // A database made before a column existed gets it: `finalizing_name` (1.3), `sent_at` and `retry_of` (1.4).
        ensure_column(&conn, "transfer_files", "finalizing_name", "TEXT")?;
        ensure_column(&conn, "transfers", "sent_at", "INTEGER")?;
        ensure_column(&conn, "transfers", "retry_of", "TEXT")?;
        // 2B.4: where an upload came from when the phone sent it by itself (`screenshot` or `camera`), for the wording of its toast and the clipboard rule.
        ensure_column(&conn, "transfers", "origin", "TEXT")?;
        // 2B.1: a text item may be a link (its body is the URL), with the title and the app that shared it.
        ensure_column(&conn, "transfer_texts", "is_link", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "transfer_texts", "link_title", "TEXT")?;
        ensure_column(&conn, "transfer_texts", "link_source", "TEXT")?;
        // 2B.5: a text item may be an alert (`Kin.exe --notify`, the extension's downloads): a text of the user's script that goes to the phone as an `alert` and never
        // to its clipboard. Its title is kept in `link_title`; its expiry is the item's own `expires_at`.
        ensure_column(&conn, "transfer_texts", "is_alert", "INTEGER NOT NULL DEFAULT 0")?;
        // The names of the phones that are paired now are remembered too, for a database that was made before `device_names` existed.
        conn.execute_batch("INSERT OR IGNORE INTO device_names (device_id, name) SELECT device_id, name FROM devices")?;
        // A failed item can be retried once: the database refuses a second retry of the same item (a double click on Retry). A database that
        // has two rows with one `retry_of` (made before this index) keeps the first link, so the index can be made.
        conn.execute_batch(
            "UPDATE transfers SET retry_of = NULL WHERE retry_of IS NOT NULL AND rowid NOT IN
               (SELECT MIN(rowid) FROM transfers WHERE retry_of IS NOT NULL GROUP BY retry_of);
             CREATE UNIQUE INDEX IF NOT EXISTS transfers_retry_of ON transfers (retry_of) WHERE retry_of IS NOT NULL;
             -- What the queries ask for: the queue (status, direction, device, creation order), the history (newest first), the expiry sweep, the
             -- recovery of open transfers and the `.part` clean-up (a file's path).
             CREATE INDEX IF NOT EXISTS transfers_queue ON transfers (status, direction, device_id, created_at);
             CREATE INDEX IF NOT EXISTS transfers_created ON transfers (created_at);
             CREATE INDEX IF NOT EXISTS transfers_expiry ON transfers (status, expires_at);
             CREATE INDEX IF NOT EXISTS transfers_finished ON transfers (status, finished_at);
             CREATE INDEX IF NOT EXISTS transfer_files_path ON transfer_files (local_path);",
        )?;
        Ok(Db(Mutex::new(conn)))
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs a write that has to survive a power cut (2B.4, R38): the list of paired devices. The database runs with `synchronous = NORMAL`
    /// (`Db::open`), where a commit is not on the disk until the log is checkpointed, so a removal could be undone by a power cut and a removed
    /// phone would work again. For the duration of this write the connection is `FULL` (the commit is synced), and it is `NORMAL` again after,
    /// whatever the outcome. The connection lock is held all the time, so no other write runs at the `FULL` level by accident or at `NORMAL`
    /// in the middle of this one. The write is one transaction.
    fn durable<T>(&self, write: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn();
        conn.execute_batch("PRAGMA synchronous = FULL;")?;
        let result = (|| -> Result<T> {
            conn.execute_batch("BEGIN IMMEDIATE")?;
            match write(&conn) {
                Ok(value) => {
                    // A COMMIT that fails (a full disk, a busy lock) leaves the transaction open: it is rolled back here, or the next write on this connection would
                    // fail with "cannot start a transaction within a transaction" and the `FULL` write could be committed by accident later.
                    if let Err(e) = conn.execute_batch("COMMIT") {
                        let _ = conn.execute_batch("ROLLBACK");
                        return Err(e.into());
                    }
                    Ok(value)
                }
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        })();
        if let Err(e) = conn.execute_batch("PRAGMA synchronous = NORMAL;") {
            log::warn!("putting the database back to synchronous = NORMAL failed: {e}");
        }
        result
    }

    /// The `synchronous` level of the connection now (0 OFF, 1 NORMAL, 2 FULL), for the tests of `durable`.
    #[doc(hidden)]
    pub fn synchronous_level(&self) -> i64 {
        self.conn().query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap_or(-1)
    }

    /// Adds a device, or updates the record of one that pairs again. **Durable**: a pairing the user saw is still there after a power cut.
    pub fn upsert_device(&self, d: &NewDevice<'_>, now_ms: i64) -> Result<()> {
        self.durable(|conn| {
            conn.execute(
                "INSERT INTO devices (device_id, name, platform, model, app_version, paired_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(device_id) DO UPDATE SET name = ?2, platform = ?3, model = ?4, app_version = ?5, paired_at = ?6",
                params![d.device_id, d.name, d.platform, d.model, d.app_version, now_ms],
            )?;
            conn.execute("INSERT OR IGNORE INTO device_names (device_id, name) VALUES (?1, ?2)", params![d.device_id, d.name])?;
            Ok(())
        })
    }

    /// Every phone name this PC has known, paired now or not, and every earlier name of a phone that was renamed: what the log export
    /// takes out of the logs. Longest first is up to the caller.
    pub fn known_device_names(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT name FROM device_names UNION SELECT name FROM devices")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn is_paired(&self, device_id: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT 1 FROM devices WHERE device_id = ?1", [device_id], |_| Ok(())).optional()?.is_some())
    }

    pub fn get_device(&self, device_id: &str) -> Result<Option<Device>> {
        Ok(self.conn().query_row(&format!("{SELECT_DEVICE} WHERE device_id = ?1"), [device_id], read_device).optional()?)
    }

    /// Oldest pairing first.
    pub fn list_devices(&self) -> Result<Vec<Device>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{SELECT_DEVICE} ORDER BY paired_at, device_id"))?;
        let rows = stmt.query_map([], read_device)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Removes the device and the message IDs remembered for it. `true` if it was there. **Durable** (see `durable`): a removal that was
    /// acted on (the phone's connection closed, its key refused) is on the disk before this returns, so a power cut cannot bring the phone back.
    pub fn remove_device(&self, device_id: &str) -> Result<bool> {
        self.durable(|conn| {
            conn.execute("DELETE FROM seen_messages WHERE device_id = ?1", [device_id])?;
            Ok(conn.execute("DELETE FROM devices WHERE device_id = ?1", [device_id])? > 0)
        })
    }

    /// Where an upload came from (`screenshot` or `camera`, 2B.4): written with the row by `insert_transfer_with_origin`.
    pub fn transfer_origin(&self, id: &str) -> Option<String> {
        self.conn()
            .query_row("SELECT origin FROM transfers WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0))
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    pub fn update_name(&self, device_id: &str, name: &str) -> Result<()> {
        let conn = self.conn();
        let changed = conn.execute("UPDATE devices SET name = ?2 WHERE device_id = ?1", params![device_id, name])?;
        if changed > 0 {
            conn.execute("INSERT OR IGNORE INTO device_names (device_id, name) VALUES (?1, ?2)", params![device_id, name])?;
        }
        Ok(())
    }

    /// Records a connection: the time, and the address it came from at the front of the list.
    pub fn touch(&self, device_id: &str, now_ms: i64, address: Option<&str>) -> Result<()> {
        let conn = self.conn();
        let current: Option<String> =
            conn.query_row("SELECT last_addresses FROM devices WHERE device_id = ?1", [device_id], |r| r.get(0)).optional()?;
        let Some(current) = current else {
            return Ok(());
        };
        let mut addresses: Vec<String> = serde_json::from_str(&current).unwrap_or_default();
        if let Some(a) = address {
            addresses.retain(|x| x != a);
            addresses.insert(0, a.to_string());
            addresses.truncate(LAST_ADDRESSES_KEPT);
        }
        conn.execute(
            "UPDATE devices SET last_seen = ?2, last_addresses = ?3 WHERE device_id = ?1",
            params![device_id, now_ms, serde_json::to_string(&addresses)?],
        )?;
        Ok(())
    }

    /// The first response to a message ID that was already processed.
    pub fn seen_response(&self, device_id: &str, message_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT response FROM seen_messages WHERE device_id = ?1 AND message_id = ?2", params![device_id, message_id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Remembers a processed ID with its response. An ID goes only when it is older than 24 hours *and* beyond the
    /// newest 1,000 of that device.
    pub fn remember_message(&self, device_id: &str, message_id: &str, response: &str, now_ms: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO seen_messages (device_id, message_id, response, seen_at) VALUES (?1, ?2, ?3, ?4)",
            params![device_id, message_id, response, now_ms],
        )?;
        conn.execute(
            "DELETE FROM seen_messages WHERE device_id = ?1 AND seen_at < ?2 AND rowid NOT IN
               (SELECT rowid FROM seen_messages WHERE device_id = ?1 ORDER BY seen_at DESC, rowid DESC LIMIT ?3)",
            params![device_id, now_ms - DEDUP_MAX_AGE_MS, DEDUP_MIN_IDS],
        )?;
        Ok(())
    }
}

/// A row of `transfers`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferRow {
    pub id: String,
    pub device_id: String,
    pub direction: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub total_bytes: i64,
    pub error_code: Option<String>,
    pub dest_dir: Option<String>,
}

/// A row of `transfer_files`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferFileRow {
    pub file_id: String,
    pub idx: i64,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub bytes_done: i64,
    pub local_path: Option<String>,
    pub finalized: bool,
    pub source_mtime_ms: Option<i64>,
    pub temp_source: bool,
    /// A received file: the final name its `.part` file was being moved to when the app last wrote to the database.
    pub finalizing_name: Option<String>,
}

const SELECT_TRANSFER: &str = "SELECT id, device_id, direction, status, created_at, updated_at, finished_at, expires_at, total_bytes, error_code, dest_dir FROM transfers";

fn read_transfer(r: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRow> {
    Ok(TransferRow {
        id: r.get(0)?,
        device_id: r.get(1)?,
        direction: r.get(2)?,
        status: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
        finished_at: r.get(6)?,
        expires_at: r.get(7)?,
        total_bytes: r.get(8)?,
        error_code: r.get(9)?,
        dest_dir: r.get(10)?,
    })
}

fn read_transfer_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<TransferFileRow> {
    Ok(TransferFileRow {
        file_id: r.get(0)?,
        idx: r.get(1)?,
        name: r.get(2)?,
        mime: r.get(3)?,
        size: r.get(4)?,
        bytes_done: r.get(5)?,
        local_path: r.get(6)?,
        finalized: r.get::<_, i64>(7)? != 0,
        source_mtime_ms: r.get(8)?,
        temp_source: r.get::<_, i64>(9)? != 0,
        finalizing_name: r.get(10)?,
    })
}

const SELECT_TRANSFER_FILE: &str = "SELECT file_id, idx, name, mime, size, bytes_done, local_path, finalized, source_mtime_ms, temp_source, finalizing_name FROM transfer_files";

fn files_of(conn: &Connection, id: &str) -> Result<Vec<TransferFileRow>> {
    let mut stmt = conn.prepare(&format!("{SELECT_TRANSFER_FILE} WHERE transfer_id = ?1 ORDER BY idx"))?;
    Ok(stmt.query_map([id], read_transfer_file)?.collect::<std::result::Result<_, _>>()?)
}

fn with_files(conn: &Connection, rows: Vec<TransferRow>) -> Result<Vec<(TransferRow, Vec<TransferFileRow>)>> {
    rows.into_iter()
        .map(|t| {
            let files = files_of(conn, &t.id)?;
            Ok((t, files))
        })
        .collect()
}

impl Db {
    /// Stores a transfer and its files. A transfer ID is never reused, so an ID that is already there is an error and nothing is
    /// changed: the old records are never replaced or wiped.
    pub fn insert_transfer(&self, t: &TransferRow, files: &[TransferFileRow]) -> Result<()> {
        self.insert_transfer_retry(t, files, None)
    }

    /// The same, for an item made by a **retry** of `retry_of`: the link is written in the same transaction as the item (a second retry of the
    /// same item is refused by a unique index, so a double click makes one item).
    pub fn insert_transfer_retry(&self, t: &TransferRow, files: &[TransferFileRow], retry_of: Option<&str>) -> Result<()> {
        self.insert_transfer_full(t, files, retry_of, None)
    }

    /// An upload with the `origin` the phone gave it (`screenshot` or `camera`), written **in the same transaction as the row** (2B.4 review): a crash between the
    /// row and a later write of the origin used to leave a camera photo with no origin, which a restart would then treat as a manual send and put on the clipboard.
    pub fn insert_transfer_with_origin(&self, t: &TransferRow, files: &[TransferFileRow], origin: Option<&str>) -> Result<()> {
        self.insert_transfer_full(t, files, None, origin)
    }

    fn insert_transfer_full(&self, t: &TransferRow, files: &[TransferFileRow], retry_of: Option<&str>, origin: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO transfers (id, device_id, direction, status, created_at, updated_at, finished_at, expires_at, total_bytes, error_code, dest_dir, retry_of, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                t.id,
                t.device_id,
                t.direction,
                t.status,
                t.created_at,
                t.updated_at,
                t.finished_at,
                t.expires_at,
                t.total_bytes,
                t.error_code,
                t.dest_dir,
                retry_of,
                origin
            ],
        )?;
        for f in files {
            tx.execute(
                "INSERT INTO transfer_files (transfer_id, file_id, idx, name, mime, size, bytes_done, local_path, finalized, source_mtime_ms, temp_source, finalizing_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    t.id,
                    f.file_id,
                    f.idx,
                    f.name,
                    f.mime,
                    f.size,
                    f.bytes_done,
                    f.local_path,
                    f.finalized as i64,
                    f.source_mtime_ms,
                    f.temp_source as i64,
                    f.finalizing_name
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_transfer_status(&self, id: &str, status: &str, now_ms: i64, error: Option<&str>) -> Result<()> {
        let finished = (status != "active").then_some(now_ms);
        self.conn().execute(
            "UPDATE transfers SET status = ?2, updated_at = ?3, finished_at = ?4, error_code = ?5 WHERE id = ?1",
            params![id, status, now_ms, finished, error],
        )?;
        Ok(())
    }

    /// Notes that something happened to the transfer (a file made progress), so the 24 hour clock of a `.part` file restarts.
    pub fn touch_transfer(&self, id: &str, now_ms: i64) -> Result<()> {
        self.conn().execute("UPDATE transfers SET updated_at = ?2 WHERE id = ?1", params![id, now_ms])?;
        Ok(())
    }

    pub fn set_file_progress(&self, transfer_id: &str, file_id: &str, bytes_done: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE transfer_files SET bytes_done = ?3 WHERE transfer_id = ?1 AND file_id = ?2",
            params![transfer_id, file_id, bytes_done as i64],
        )?;
        Ok(())
    }

    /// A received file got its final name and path (the `.part` file was moved).
    pub fn set_file_final(&self, transfer_id: &str, file_id: &str, name: &str, path: &str, bytes_done: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE transfer_files SET name = ?3, local_path = ?4, finalized = 1, bytes_done = ?5, finalizing_name = NULL WHERE transfer_id = ?1 AND file_id = ?2",
            params![transfer_id, file_id, name, path, bytes_done as i64],
        )?;
        Ok(())
    }

    /// A received file's `.part` name (it is created again if it was lost, possibly with another name).
    pub fn set_file_part(&self, transfer_id: &str, file_id: &str, name: &str, path: &str, bytes_done: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE transfer_files SET name = ?3, local_path = ?4, finalized = 0, bytes_done = ?5, finalizing_name = NULL WHERE transfer_id = ?1 AND file_id = ?2",
            params![transfer_id, file_id, name, path, bytes_done as i64],
        )?;
        Ok(())
    }

    /// The `.part` file of a received file is about to be moved to `final_name` (written before the move; `set_file_final` clears it).
    pub fn set_file_finalizing(&self, transfer_id: &str, file_id: &str, final_name: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE transfer_files SET finalizing_name = ?3 WHERE transfer_id = ?1 AND file_id = ?2",
            params![transfer_id, file_id, final_name],
        )?;
        Ok(())
    }

    /// The `.part` file of a dropped transfer is gone: the record no longer points at a path (so nothing ever deletes it again).
    pub fn clear_file_path(&self, transfer_id: &str, file_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE transfer_files SET local_path = NULL, finalizing_name = NULL WHERE transfer_id = ?1 AND file_id = ?2 AND finalized = 0",
            params![transfer_id, file_id],
        )?;
        Ok(())
    }

    /// Whether a transfer with this ID has ever been recorded, in any state. An ID is never used twice.
    pub fn transfer_exists(&self, id: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT 1 FROM transfers WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
    }

    pub fn get_transfer(&self, id: &str) -> Result<Option<(TransferRow, Vec<TransferFileRow>)>> {
        let conn = self.conn();
        let Some(t) = conn.query_row(&format!("{SELECT_TRANSFER} WHERE id = ?1"), [id], read_transfer).optional()? else {
            return Ok(None);
        };
        let files = files_of(&conn, &t.id)?;
        Ok(Some((t, files)))
    }

    /// Transfers that are still going (`active`), the ones that finished after `since_ms` (so a repeated request for a transfer that just
    /// ended can still be answered), and **an offer that was sent but never acked** (`queued` with a `sent_at`): the phone may have it and
    /// may be downloading it, so its files must be served after a restart too.
    pub fn load_open_transfers(&self, since_ms: i64) -> Result<Vec<(TransferRow, Vec<TransferFileRow>)>> {
        let conn = self.conn();
        let rows: Vec<TransferRow> = {
            let mut stmt = conn.prepare(&format!(
                "{SELECT_TRANSFER} WHERE kind = 'file' AND (status = 'active' OR (status = 'done' AND finished_at >= ?1)
                   OR (status = 'queued' AND direction = 'out' AND sent_at IS NOT NULL))"
            ))?;
            stmt.query_map([since_ms], read_transfer)?.collect::<std::result::Result<_, _>>()?
        };
        with_files(&conn, rows)
    }

    /// The newest transfers, newest first (for `status.json` and, later, the history).
    pub fn recent_transfers(&self, limit: usize) -> Result<Vec<(TransferRow, Vec<TransferFileRow>)>> {
        let conn = self.conn();
        let rows: Vec<TransferRow> = {
            let mut stmt = conn.prepare(&format!(
                "{SELECT_TRANSFER} WHERE kind = 'file' AND status != 'queued' ORDER BY created_at DESC, rowid DESC LIMIT ?1"
            ))?;
            stmt.query_map([limit as i64], read_transfer)?.collect::<std::result::Result<_, _>>()?
        };
        with_files(&conn, rows)
    }

    /// The `.part` files that the database still assigns to a received transfer that is over (done, failed or canceled): what is left
    /// when the app stopped between ending a transfer and deleting its files. `(transfer ID, file ID, path)`. A path that any record
    /// has as a *finished* file is never in the list: a phone can send a file that is named like a `.part` file.
    pub fn leftover_part_files(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT f.transfer_id, f.file_id, f.local_path FROM transfer_files f JOIN transfers t ON t.id = f.transfer_id
             WHERE t.direction = 'in' AND t.status != 'active' AND f.finalized = 0 AND f.local_path IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM transfer_files g WHERE g.finalized = 1 AND g.local_path = f.local_path)",
        )?;
        Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?)
    }
}

const SELECT_DEVICE: &str = "SELECT device_id, name, platform, model, app_version, paired_at, last_seen, last_addresses FROM devices";

fn read_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    let addresses: String = r.get(7)?;
    Ok(Device {
        device_id: r.get(0)?,
        name: r.get(1)?,
        platform: r.get(2)?,
        model: r.get(3)?,
        app_version: r.get(4)?,
        paired_at: r.get(5)?,
        last_seen: r.get(6)?,
        last_addresses: serde_json::from_str(&addresses).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device<'a>(id: &'a str, name: &'a str) -> NewDevice<'a> {
        NewDevice { device_id: id, name, platform: "android", model: Some("Pixel"), app_version: Some("0.1.0") }
    }

    #[test]
    fn a_database_from_before_finalizing_name_gets_the_column_and_keeps_its_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transfers (id TEXT PRIMARY KEY, device_id TEXT NOT NULL, direction TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'file',
               status TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, finished_at INTEGER, expires_at INTEGER,
               total_bytes INTEGER NOT NULL, error_code TEXT, dest_dir TEXT);
             CREATE TABLE transfer_files (transfer_id TEXT NOT NULL, file_id TEXT NOT NULL, idx INTEGER NOT NULL, name TEXT NOT NULL,
               mime TEXT NOT NULL, size INTEGER NOT NULL, bytes_done INTEGER NOT NULL DEFAULT 0, local_path TEXT, finalized INTEGER NOT NULL DEFAULT 0,
               source_mtime_ms INTEGER, temp_source INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (transfer_id, file_id));
             INSERT INTO transfers (id, device_id, direction, status, created_at, updated_at, total_bytes) VALUES ('old', 'd', 'in', 'done', 1, 1, 5);
             INSERT INTO transfer_files (transfer_id, file_id, idx, name, mime, size, finalized) VALUES ('old', 'f', 0, 'a.bin', '', 5, 1);",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        let (_, files) = db.get_transfer("old").unwrap().unwrap();
        assert_eq!((files[0].name.as_str(), files[0].finalizing_name.clone()), ("a.bin", None));
        db.set_file_finalizing("old", "f", "b.bin").unwrap();
        assert_eq!(db.get_transfer("old").unwrap().unwrap().1[0].finalizing_name.as_deref(), Some("b.bin"));
        // Opening it a second time (the column is there now) is fine too.
        assert!(db.transfer_exists("old").unwrap());
    }

    #[test]
    fn a_database_with_two_retries_of_one_item_keeps_the_first_link_and_gets_the_unique_index() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transfers (id TEXT PRIMARY KEY, device_id TEXT NOT NULL, direction TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'file',
               status TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, finished_at INTEGER, expires_at INTEGER,
               total_bytes INTEGER NOT NULL, error_code TEXT, dest_dir TEXT, sent_at INTEGER, retry_of TEXT);
             INSERT INTO transfers (id, device_id, direction, status, created_at, updated_at, total_bytes, retry_of) VALUES
               ('r1', 'd', 'out', 'queued', 1, 1, 5, 'orig'), ('r2', 'd', 'out', 'queued', 2, 2, 5, 'orig'), ('x', 'd', 'out', 'done', 3, 3, 5, NULL);",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        let links: Vec<(String, Option<String>)> = {
            let conn = db.conn();
            let mut stmt = conn.prepare("SELECT id, retry_of FROM transfers ORDER BY id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(
            links,
            [("r1".into(), Some("orig".into())), ("r2".into(), None), ("x".into(), None)],
            "the first link stays, the second is dropped"
        );
        let second = db.conn().execute("INSERT INTO transfers (id, device_id, direction, status, created_at, updated_at, total_bytes, retry_of) VALUES ('r3', 'd', 'out', 'queued', 4, 4, 5, 'orig')", []);
        assert!(second.is_err(), "the index refuses a third");
        // The indexes the queries use are there.
        let names: Vec<String> = {
            let conn = db.conn();
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name IN ('transfers', 'transfer_files')")
                .unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        for wanted in
            ["transfers_retry_of", "transfers_queue", "transfers_created", "transfers_expiry", "transfers_finished", "transfer_files_path"]
        {
            assert!(names.iter().any(|n| n == wanted), "{wanted} is missing: {names:?}");
        }
    }

    #[test]
    fn pairing_again_updates_the_record() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_device(&device("a", "Old name"), 1).unwrap();
        db.upsert_device(&device("a", "New name"), 2).unwrap();
        let all = db.list_devices().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].name.as_str(), all[0].paired_at), ("New name", 2));
        assert!(db.is_paired("a").unwrap() && !db.is_paired("b").unwrap());
    }

    #[test]
    fn touch_keeps_the_newest_addresses_first() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_device(&device("a", "Phone"), 1).unwrap();
        for (i, addr) in ["1.1.1.1:1", "2.2.2.2:2", "1.1.1.1:1"].iter().enumerate() {
            db.touch("a", 10 + i as i64, Some(addr)).unwrap();
        }
        let d = db.get_device("a").unwrap().unwrap();
        assert_eq!(d.last_addresses, ["1.1.1.1:1", "2.2.2.2:2"]);
        assert_eq!(d.last_seen, Some(12));
    }

    #[test]
    fn removing_a_device_forgets_its_message_ids() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_device(&device("a", "Phone"), 1).unwrap();
        db.remember_message("a", "m1", "{}", 5).unwrap();
        assert_eq!(db.seen_response("a", "m1").unwrap().as_deref(), Some("{}"));
        assert!(db.remove_device("a").unwrap());
        assert!(db.seen_response("a", "m1").unwrap().is_none());
        assert!(!db.remove_device("a").unwrap());
    }

    /// R38: the list of devices is written at `synchronous = FULL`, and the connection goes back to `NORMAL` after, whatever the outcome.
    #[test]
    fn a_write_of_the_device_list_is_full_inside_and_normal_before_and_after() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("kin.db")).unwrap();
        assert_eq!(db.synchronous_level(), 1, "NORMAL: the speed setting of v0.1.1 stays for everything else");
        let inside = db.durable(|conn| Ok(conn.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))?)).unwrap();
        assert_eq!(inside, 2, "FULL inside the helper");
        assert_eq!(db.synchronous_level(), 1, "NORMAL again after it");
        // A write that fails is rolled back and the level is still put back.
        let failed: Result<()> = db.durable(|conn| {
            conn.execute("INSERT INTO devices (device_id, name, platform, paired_at) VALUES ('x', 'X', 'android', 1)", [])?;
            Err("the write failed half way".into())
        });
        assert!(failed.is_err());
        assert_eq!(db.synchronous_level(), 1);
        assert!(!db.is_paired("x").unwrap(), "the half-done write was rolled back");
        // Pairing and removing both go through it, and neither leaves the level up.
        db.upsert_device(&device("a", "Phone"), 1).unwrap();
        assert_eq!(db.synchronous_level(), 1);
        assert!(db.remove_device("a").unwrap());
        assert_eq!(db.synchronous_level(), 1);
    }

    /// 2B.4 review: the `origin` of an auto-sent upload is in the **same transaction** as its row, so no moment exists in which the row is there and a restart would take
    /// a camera photo for a manual send (and put it on the clipboard).
    #[test]
    fn the_origin_of_an_upload_is_written_with_its_row() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_device(&device("a", "Phone"), 1).unwrap();
        let row = |id: &str| TransferRow {
            id: id.into(),
            device_id: "a".into(),
            direction: "in".into(),
            status: "active".into(),
            created_at: 1,
            updated_at: 1,
            finished_at: None,
            expires_at: None,
            total_bytes: 5,
            error_code: None,
            dest_dir: Some("D:/r".into()),
        };
        db.insert_transfer_with_origin(&row("with"), &[], Some("camera")).unwrap();
        db.insert_transfer_with_origin(&row("without"), &[], None).unwrap();
        db.insert_transfer(&row("plain"), &[]).unwrap();
        assert_eq!(db.transfer_origin("with").as_deref(), Some("camera"), "there from the first moment");
        assert_eq!((db.transfer_origin("without"), db.transfer_origin("plain")), (None, None));
    }

    /// 2B.4 review: a COMMIT that itself fails (here a deferred foreign key that is broken at the end of the transaction) is rolled back, so the connection is not left in
    /// a transaction, the level is `NORMAL` again, and the next durable write works.
    #[test]
    fn a_commit_that_fails_is_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("kin.db")).unwrap();
        {
            let conn = db.conn();
            conn.execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE fk_parent (id INTEGER PRIMARY KEY);
                 CREATE TABLE fk_child (pid INTEGER REFERENCES fk_parent(id) DEFERRABLE INITIALLY DEFERRED);",
            )
            .unwrap();
        }
        let failed: Result<()> = db.durable(|conn| {
            conn.execute("INSERT INTO fk_child (pid) VALUES (42)", [])?;
            Ok(())
        });
        assert!(failed.is_err(), "the COMMIT must fail on the broken foreign key");
        assert!(db.conn().is_autocommit(), "no transaction is left open");
        assert_eq!(db.synchronous_level(), 1, "NORMAL again");
        assert_eq!(db.conn().query_row("SELECT COUNT(*) FROM fk_child", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "nothing of it was kept");
        db.durable(|conn| {
            conn.execute("INSERT INTO fk_parent (id) VALUES (1)", [])?;
            Ok(())
        })
        .expect("the next durable write works");
    }

    /// The removal is in the file: a second connection to the same file sees it at once, and so does a new `Db` on it.
    #[test]
    fn a_removal_is_visible_to_a_second_connection_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kin.db");
        let db = Db::open(&path).unwrap();
        db.upsert_device(&device("a", "Phone"), 1).unwrap();
        let other = Connection::open(&path).unwrap();
        let paired =
            |c: &Connection| c.query_row("SELECT COUNT(*) FROM devices WHERE device_id = 'a'", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(paired(&other), 1, "a pairing is visible at once too");
        assert!(db.remove_device("a").unwrap());
        assert_eq!(paired(&other), 0);
        drop(db);
        assert!(!Db::open(&path).unwrap().is_paired("a").unwrap());
    }

    #[test]
    fn a_name_is_remembered_after_the_phone_is_unpaired_or_renamed() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_device(&device("a", "Anna's Pixel 8"), 1).unwrap();
        db.upsert_device(&device("b", "Berg tablet"), 2).unwrap();
        db.update_name("b", "Berg tablet 2").unwrap();
        assert!(db.remove_device("a").unwrap());
        let mut names = db.known_device_names().unwrap();
        names.sort();
        assert_eq!(
            names,
            ["Anna's Pixel 8", "Berg tablet", "Berg tablet 2"],
            "the unpaired phone and the old name of the renamed one stay"
        );
        assert_eq!(db.list_devices().unwrap().len(), 1, "the device list itself is only the paired phone");
        // A rename of a phone that isn't paired adds nothing.
        db.update_name("nobody", "Ghost").unwrap();
        assert!(!db.known_device_names().unwrap().contains(&"Ghost".to_string()));
    }

    #[test]
    fn a_database_made_before_the_names_table_gets_the_names_of_the_paired_phones() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE devices (device_id TEXT PRIMARY KEY, name TEXT NOT NULL, platform TEXT NOT NULL, model TEXT, app_version TEXT,
               paired_at INTEGER NOT NULL, last_seen INTEGER, last_addresses TEXT NOT NULL DEFAULT '[]');
             INSERT INTO devices (device_id, name, platform, paired_at) VALUES ('a', 'Old Pixel', 'android', 1);",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        assert!(db.remove_device("a").unwrap());
        assert_eq!(db.known_device_names().unwrap(), ["Old Pixel"], "remembered although it was removed after the upgrade");
    }

    #[test]
    fn dedup_keeps_ids_that_are_young_or_among_the_newest_1000() {
        let db = Db::open_in_memory().unwrap();
        let day = DEDUP_MAX_AGE_MS;
        // 1,005 old IDs. With the young one below there are 1,006, and the oldest 6 are beyond the newest 1,000.
        for i in 0..1005 {
            db.remember_message("a", &format!("old{i}"), "r", i).unwrap();
        }
        // A young ID triggers a prune at "now" = 2 days.
        db.remember_message("a", "young", "r", 2 * day).unwrap();
        assert!(db.seen_response("a", "old0").unwrap().is_none());
        assert!(db.seen_response("a", "old5").unwrap().is_none());
        assert!(db.seen_response("a", "old6").unwrap().is_some());
        assert!(db.seen_response("a", "young").unwrap().is_some());
        // A different device has its own count.
        db.remember_message("b", "x", "r", 0).unwrap();
        db.remember_message("a", "young2", "r", 2 * day + 1).unwrap();
        assert!(db.seen_response("b", "x").unwrap().is_some());
    }
}
