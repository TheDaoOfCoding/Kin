//! SQLite storage. For milestone 1.1 that is the `devices` table, plus the memory of processed message IDs that
//! protocol.md asks for ("De-duplication"), which has to survive a restart.
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
CREATE TABLE IF NOT EXISTS seen_messages (
    device_id  TEXT NOT NULL,
    message_id TEXT NOT NULL,
    response   TEXT NOT NULL,
    seen_at    INTEGER NOT NULL,
    PRIMARY KEY (device_id, message_id)
);
";

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        Db::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Db> {
        Db::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.execute_batch(SCHEMA)?;
        Ok(Db(Mutex::new(conn)))
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Adds a device, or updates the record of one that pairs again.
    pub fn upsert_device(&self, d: &NewDevice<'_>, now_ms: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO devices (device_id, name, platform, model, app_version, paired_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(device_id) DO UPDATE SET name = ?2, platform = ?3, model = ?4, app_version = ?5, paired_at = ?6",
            params![d.device_id, d.name, d.platform, d.model, d.app_version, now_ms],
        )?;
        Ok(())
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

    /// Removes the device and the message IDs remembered for it. `true` if it was there.
    pub fn remove_device(&self, device_id: &str) -> Result<bool> {
        let conn = self.conn();
        conn.execute("DELETE FROM seen_messages WHERE device_id = ?1", [device_id])?;
        Ok(conn.execute("DELETE FROM devices WHERE device_id = ?1", [device_id])? > 0)
    }

    pub fn update_name(&self, device_id: &str, name: &str) -> Result<()> {
        self.conn().execute("UPDATE devices SET name = ?2 WHERE device_id = ?1", params![device_id, name])?;
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
