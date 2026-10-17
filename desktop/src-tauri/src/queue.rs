//! The queue of milestone 1.4 (protocol.md, "Queue"; docs/windows.md, "Sending to the phone").
//!
//! **What the rules are.**
//! * Every explicit send from the PC (a `text` from the hotkey, the tray or `--send-clipboard`, and a file offer from the drop window,
//!   "Send to" or `--send`) is a row of `transfers` with the status `queued` **before the first attempt**, so it survives a restart of
//!   `Kin.exe`. The automatic `clipboard` and a `sensitive` text are never queued (nothing of them is stored).
//! * After the phone's `hello` is accepted (and whenever something is queued while it is connected) the queue goes out **in creation
//!   order**, one item at a time, each under the `id` it was made with, so a repeat after a lost `ack` is the same message and the
//!   phone's memory of IDs makes it idempotent.
//! * **A send into a dead socket doesn't fail the item.** An item whose message gets no `ack` in 5 s (the connection then closes) stays
//!   `queued` and goes again after the next reconnect. Only an `error` from the phone fails it. There are no WebSocket pings from the
//!   PC to find out whether the phone is gone: they would wake its radio. The ack timeout and the queue cover it.
//! * **Expiry** (decided on the PC's clock, at start, every hour and before a flush): a queued `text` 7 days after it was made; an offer
//!   24 hours after it was sent (the last attempt on an open connection), or 7 days after it was made if it never was. An item over its
//!   time shows in the history as failed with the error `expired`.
//! * **Cancel** of a queued item: it never goes out. (An offer that was already sent into a dead socket may have reached the phone, so
//!   it is canceled the ordinary way, with a `cancel` message.)
//!
//! Everything here is decided with an explicit `now_ms`, so a test moves time with a fake clock instead of waiting a week.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::core::{Core, Event, lock, short_transfer};
use crate::encoding::{unix_ms, uuid_v7};
use crate::outgoing::{self, Source};
use crate::queuedb::QueueItem;
use crate::storage::{TransferFileRow, TransferRow};
use crate::sync::{SendResult, Tap};
use crate::transfers::{Direction, Entry, FileData, Status, remove_temp_source};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
/// A queued `text` expires this long after it was made.
pub const TEXT_TTL_MS: i64 = 7 * DAY_MS;
/// An offer that was queued and never sent expires this long after it was made.
pub const UNSENT_OFFER_TTL_MS: i64 = 7 * DAY_MS;
/// An offer that was sent expires this long after it was sent.
pub const SENT_OFFER_TTL_MS: i64 = 24 * HOUR_MS;

/// When a text, or an offer that has not been sent, made at `created_ms` expires.
pub fn expiry_when_queued(created_ms: i64, kind: &str) -> i64 {
    created_ms + if kind == "text" { TEXT_TTL_MS } else { UNSENT_OFFER_TTL_MS }
}

/// When an offer sent at `sent_ms` expires.
pub fn expiry_when_sent(sent_ms: i64) -> i64 {
    sent_ms + SENT_OFFER_TTL_MS
}

/// An item is expired from its expiry time on (at exactly 7 days, 0 ms it is expired, the same edge as the 24 hour window of a
/// finished transfer).
pub fn is_expired(expires_at: Option<i64>, now_ms: i64) -> bool {
    expires_at.is_some_and(|e| now_ms >= e)
}

struct Waiter {
    tx: oneshot::Sender<SendResult>,
    tap: Tap,
}

/// What the queue keeps in memory: the lock that makes the flush one at a time, the set of items that something is working on right now
/// (an item being sent, expired or canceled: a cancel waits for *that item*, never for the whole flush), and who is waiting for the
/// outcome of which item.
#[derive(Default)]
pub struct QueueState {
    flush: tokio::sync::Mutex<()>,
    claimed: Mutex<HashSet<String>>,
    waiters: Mutex<HashMap<String, Waiter>>,
    /// Test hook: runs once, right after a text was stored and before the queue looks at the connection again (the moment a phone can
    /// connect between the first look and the insert).
    after_text_insert: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl QueueState {
    pub fn register(&self, id: &str, tap: Tap) -> oneshot::Receiver<SendResult> {
        let (tx, rx) = oneshot::channel();
        lock(&self.waiters).insert(id.to_string(), Waiter { tx, tap });
        rx
    }

    fn tap_of(&self, id: &str) -> Tap {
        lock(&self.waiters).get(id).map_or_else(Tap::now, |w| w.tap)
    }

    fn fire(&self, id: &str, result: SendResult) -> bool {
        match lock(&self.waiters).remove(id) {
            Some(w) => w.tx.send(result).is_ok(),
            None => false,
        }
    }

    pub(crate) fn forget(&self, id: &str) {
        lock(&self.waiters).remove(id);
    }

    /// Takes the item if it is still `queued` and nothing else has it. The status is read under the same lock that guards the claim, so a
    /// cancel that wins the item first is seen here, and a send that wins it first is waited for by the cancel: an item is never sent and
    /// canceled at once.
    fn claim_if_queued(&self, db: &crate::storage::Db, id: &str) -> Option<Claim<'_>> {
        let mut claimed = lock(&self.claimed);
        if claimed.contains(id) || !matches!(db.status_of(id), Ok(Some((status, _))) if status == "queued") {
            return None;
        }
        claimed.insert(id.to_string());
        Some(Claim { state: self, id: id.to_string() })
    }

    /// Takes the item whatever its state, waiting (at most as long as one send takes, the 5 s ack timeout) while it is being sent or expired.
    async fn claim_when_free(&self, id: &str) -> Claim<'_> {
        loop {
            {
                let mut claimed = lock(&self.claimed);
                if !claimed.contains(id) {
                    claimed.insert(id.to_string());
                    return Claim { state: self, id: id.to_string() };
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

/// Holds an item of the queue; it is let go when this is dropped.
struct Claim<'a> {
    state: &'a QueueState,
    id: String,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        lock(&self.state.claimed).remove(&self.id);
    }
}

/// What a text item holds: a text, or (2B.1) a link with its title and the app that shared it.
pub enum TextBody<'a> {
    Text { text: &'a str, is_url: bool },
    Link { url: &'a str, title: Option<&'a str>, source_app: Option<&'a str> },
}

/// How a queued send came out for the caller that waits for it.
fn outcome_of(rx: &mut oneshot::Receiver<SendResult>, fallback: SendResult) -> SendResult {
    rx.try_recv().unwrap_or(fallback)
}

impl Core {
    // ----- looking at the queue ------------------------------------------------------------------------------------------

    pub fn queue_len(&self) -> usize {
        self.db.queued_count().unwrap_or(0).max(0) as usize
    }

    /// The queue for `status.json` and the debug window: how many, and for each item its kind and times. No text and no name.
    pub fn queue_status(&self) -> Value {
        let items = self.db.all_queued().unwrap_or_default();
        json!({
            "count": items.len(),
            "items": items.iter().map(|i| json!({
                "id": short_transfer(&i.id),
                "kind": i.kind,
                "createdMs": i.created_at,
                "expiresMs": i.expires_at,
                "sentMs": i.sent_at,
                "bytes": i.total_bytes,
            })).collect::<Vec<_>>(),
        })
    }

    /// The phone that new items are for: the connected one, else the one paired last. `None` when no phone is paired.
    pub(crate) fn queue_target(&self) -> Option<String> {
        self.snapshot().headline_device().map(|d| d.device_id.clone())
    }

    /// The phone an item is for when the sender may name one (the browser extension does): that device if it is paired (`None` if not), else
    /// the default one.
    pub(crate) fn queue_target_for(&self, device: Option<&str>) -> Option<String> {
        match device {
            Some(id) => self.db.is_paired(id).unwrap_or(false).then(|| id.to_string()),
            None => self.queue_target(),
        }
    }

    fn phone_connected(&self) -> bool {
        self.connections.most_recent_open().is_some()
    }

    /// Whether this device's `hello` was accepted: the items of a device are sent over its own connection only.
    fn device_connected(&self, device_id: &str) -> bool {
        self.connections.open_sender(device_id).is_some()
    }

    /// For tests: `hook` runs once, between the storing of the next explicit text and the second look at the connection.
    #[doc(hidden)]
    pub fn on_next_text_stored(&self, hook: impl FnOnce() + Send + 'static) {
        *lock(&self.queue.after_text_insert) = Some(Box::new(hook));
    }

    // ----- making items --------------------------------------------------------------------------------------------------

    /// An explicit `text`: stored as `queued` first, then sent if the phone is connected. Returns how this send came out *now*:
    /// `Acked`, or (the item stays queued) `PhoneOffline` / `NoReply`, or `Rejected` (the item failed). `NoPhonePaired` stores nothing.
    pub async fn queue_text(self: &Arc<Self>, id: &str, text: &str, is_url: bool, tap: Tap) -> SendResult {
        self.queue_text_item(None, id, TextBody::Text { text, is_url }, tap).await
    }

    /// An explicit `link` (the browser extension, 2B.1): queued the same way as a text, and sent as a `link` or, to a peer that doesn't know the
    /// type, as a text (`ws::wire_form`).
    pub async fn queue_link(
        self: &Arc<Self>,
        device: Option<&str>,
        id: &str,
        url: &str,
        title: Option<&str>,
        source_app: Option<&str>,
        tap: Tap,
    ) -> SendResult {
        self.queue_text_item(device, id, TextBody::Link { url, title, source_app }, tap).await
    }

    /// A text item (a text or a link) for `device` (`None`: the default phone).
    pub(crate) async fn queue_text_item(self: &Arc<Self>, device: Option<&str>, id: &str, body: TextBody<'_>, tap: Tap) -> SendResult {
        let Some(device_id) = self.queue_target_for(device) else {
            return SendResult::NoPhonePaired;
        };
        let now = unix_ms() as i64;
        let offline = !self.device_connected(&device_id);
        let expires = Some(expiry_when_queued(now, "text"));
        let stored = match body {
            TextBody::Text { text, is_url } => self.db.insert_text_item(id, &device_id, "out", "queued", now, expires, text, is_url, None),
            TextBody::Link { url, title, source_app } => {
                self.db.insert_link_item(id, &device_id, "out", "queued", now, expires, url, title, source_app, None)
            }
        };
        if let Err(e) = stored {
            log::error!("saving the text to send failed: {e}");
            return SendResult::Rejected("internal".into());
        }
        self.record_send_start(id, "text", i64::from(offline), offline);
        self.emit(Event::StateChanged);
        let mut rx = self.queue.register(id, tap);
        if let Some(hook) = lock(&self.queue.after_text_insert).take() {
            hook();
        }
        // Looked at again *after* the insert: a phone that connected between the first look and the insert had its flush before the item
        // existed, so a hotkey press at the moment of a reconnect would otherwise leave the item waiting for the next one.
        if self.device_connected(&device_id) {
            self.flush_queue().await;
        }
        self.queue.forget(id);
        outcome_of(&mut rx, SendResult::PhoneOffline)
    }

    /// Stores a file offer as `queued` (and its files). `None` if no phone is paired. The caller flushes the queue.
    pub fn enqueue_offer(&self, sources: &[Source], retry_of: Option<&str>) -> Option<(String, oneshot::Receiver<SendResult>)> {
        self.enqueue_offer_to(None, sources, retry_of)
    }

    /// The same for a named phone (the extension may pick one).
    pub fn enqueue_offer_to(
        &self,
        device: Option<&str>,
        sources: &[Source],
        retry_of: Option<&str>,
    ) -> Option<(String, oneshot::Receiver<SendResult>)> {
        let device_id = self.queue_target_for(device)?;
        let id = uuid_v7(unix_ms());
        let now = unix_ms() as i64;
        let offline = !self.device_connected(&device_id);
        let row = TransferRow {
            id: id.clone(),
            device_id,
            direction: "out".into(),
            status: "queued".into(),
            created_at: now,
            updated_at: now,
            finished_at: None,
            expires_at: Some(expiry_when_queued(now, "file")),
            total_bytes: sources.iter().map(|s| s.size as i64).sum(),
            error_code: None,
            dest_dir: None,
        };
        let files: Vec<TransferFileRow> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| TransferFileRow {
                file_id: format!("f{}", i + 1),
                idx: i as i64,
                name: s.name.clone(),
                mime: outgoing::mime_of(&s.name).to_string(),
                size: s.size as i64,
                bytes_done: 0,
                local_path: Some(s.path.to_string_lossy().into_owned()),
                finalized: false,
                source_mtime_ms: Some(s.mtime_ms),
                temp_source: s.temp,
                finalizing_name: None,
            })
            .collect();
        // The link of a retry is written with the item, in one transaction (a second retry of the same item is refused by the database).
        if let Err(e) = self.db.insert_transfer_retry(&row, &files, retry_of) {
            log::error!("saving the offer to send failed: {e}");
            return None;
        }
        self.record_send_start(&id, "offer", i64::from(offline), offline);
        self.emit(Event::StateChanged);
        let rx = self.queue.register(&id, Tap::now());
        Some((id, rx))
    }

    // ----- sending the queue ---------------------------------------------------------------------------------------------

    /// Sends what is queued, in creation order, one item at a time, over the connection that is open. Runs once at a time (a second
    /// call waits for the first and then looks again). Called when the phone's `hello` is accepted, and after something is queued
    /// while it is connected. It stops at the first item that gets no `ack`: the connection is dead and the rest waits for the next one.
    pub async fn flush_queue(self: &Arc<Self>) {
        let _flush = self.queue.flush.lock().await;
        self.expire_locked(unix_ms() as i64).await;
        // Every connected phone gets the items that were made for it, over its own connection.
        for device_id in self.connections.open_device_ids() {
            let quiet_sent = self.flush_device(&device_id).await;
            if quiet_sent > 0 {
                let phone = self.db.get_device(&device_id).ok().flatten().map_or_else(|| "your phone".to_string(), |d| d.name);
                log::info!("queue: {quiet_sent} item(s) sent after the reconnect");
                self.emit(Event::Toast(format!("Sent {quiet_sent} queued item{} to {phone}", if quiet_sent == 1 { "" } else { "s" })));
            }
        }
        self.emit(Event::StateChanged);
    }

    /// The queue of one device. Returns how many items were acked that nobody was waiting for (they were queued while the phone was away).
    async fn flush_device(self: &Arc<Self>, device_id: &str) -> usize {
        let (mut tried, mut quiet_sent): (HashSet<String>, usize) = (HashSet::new(), 0);
        while self.device_connected(device_id) {
            let items = self.db.queued_items(device_id).unwrap_or_else(|e| {
                log::error!("reading the queue failed: {e}");
                Vec::new()
            });
            let Some(item) = items.into_iter().find(|i| !tried.contains(&i.id)) else {
                break;
            };
            tried.insert(item.id.clone());
            // The item is held while it is sent: a cancel of *this* item waits for it (at most the 5 s ack timeout), a cancel of any other item
            // does not wait at all. The status is read again when the item is taken, so one the user canceled in the meantime is never sent.
            let Some(_claim) = self.queue.claim_if_queued(&self.db, &item.id) else {
                log::info!("queue: item {} is not queued any more (or is being canceled), it is not sent", short_transfer(&item.id));
                continue;
            };
            let outcome = if item.kind == "text" { self.send_queued_text(&item).await } else { self.send_queued_offer(&item).await };
            let Some(outcome) = outcome else {
                continue;
            };
            let dead = matches!(outcome, SendResult::NoReply | SendResult::PhoneOffline);
            let acked = matches!(outcome, SendResult::Acked { .. });
            if !self.queue.fire(&item.id, outcome) && acked {
                quiet_sent += 1;
            }
            if dead {
                break;
            }
        }
        quiet_sent
    }

    async fn send_queued_text(self: &Arc<Self>, item: &QueueItem) -> Option<SendResult> {
        let Ok(Some(stored)) = self.db.text_item(&item.id) else {
            log::error!("queued text {} has no body", short_transfer(&item.id));
            self.fail_item(&item.id, "text", "internal");
            return Some(SendResult::Rejected("internal".into()));
        };
        let tap = self.queue.tap_of(&item.id);
        let frame = if stored.is_link {
            // A `link`; the connection sends it as a text to a peer that doesn't list the capability (`ws::wire_form`).
            let mut frame = json!({ "type": "link", "id": item.id, "url": stored.body });
            if let Some(title) = &stored.title {
                frame["title"] = json!(title);
            }
            if let Some(source) = &stored.source_app {
                frame["sourceApp"] = json!(source);
            }
            frame
        } else {
            json!({ "type": "text", "id": item.id, "text": stored.body, "isUrl": stored.is_url })
        };
        let (result, _) = self.send_to_device(Some(&item.device_id), frame, tap).await;
        let now = unix_ms() as i64;
        match &result {
            SendResult::Acked { tap_to_ack, .. } => {
                self.record_send_attempt(&item.id, "text");
                let _ = self.db.mark_sent(&item.id, now);
                let _ = self.db.set_transfer_status(&item.id, "done", now, None);
                self.record_send_end(&item.id, "text", now, 1, "ok", None, Some(tap_to_ack.as_secs_f64() * 1000.0));
                log::info!("queued text {}: sent and acked", short_transfer(&item.id));
            }
            SendResult::Rejected(code) => {
                self.record_send_attempt(&item.id, "text");
                self.fail_item(&item.id, "text", code);
            }
            SendResult::NoReply => {
                self.record_send_attempt(&item.id, "text");
                self.record_send_error(&item.id, "no_reply");
                let _ = self.db.requeue(&item.id, now);
                log::warn!("queued text {}: no ack in 5 s, it stays queued", short_transfer(&item.id));
            }
            SendResult::PhoneOffline | SendResult::NoPhonePaired | SendResult::TooLarge { .. } => {}
        }
        Some(result)
    }

    async fn send_queued_offer(self: &Arc<Self>, item: &QueueItem) -> Option<SendResult> {
        let now = unix_ms() as i64;
        let entry = match self.transfers.get(&item.id) {
            Some(e) if e.direction == Direction::Out && e.status() == Status::Active => e,
            // It ended (the phone sent `complete` or `cancel`, which updated the database): nothing to send.
            Some(_) => return None,
            None => match self.entry_from_db(item) {
                Ok(e) => e,
                Err(code) => {
                    log::warn!("queued offer {}: {code}", short_transfer(&item.id));
                    self.fail_item(&item.id, "offer", code);
                    return Some(SendResult::Rejected(code.to_string()));
                }
            },
        };
        // The files are served from the moment the frame can reach the phone (it may ask for them at once), but the offer is **marked as sent,
        // and its 24 hours started, only after the frame was written to the connection**: an offer that never left (the phone was gone) is
        // still a queued item with its 7 days.
        let newly_served = self.transfers.get(&item.id).is_none();
        if newly_served {
            self.transfers.insert(entry.clone());
        }
        let frame = json!({
            "type": "offer",
            "id": item.id,
            "files": entry.files.iter().map(|f| json!({ "fileId": f.file_id, "name": f.data().name, "size": f.size, "mime": f.mime })).collect::<Vec<Value>>(),
            "totalSize": entry.total_bytes(),
        });
        log::info!(
            "offer {} to device {}: {} file(s), {} bytes",
            short_transfer(&item.id),
            crate::core::short(&entry.device_id),
            entry.files.len(),
            entry.total_bytes()
        );
        let tap = self.queue.tap_of(&item.id);
        let (result, _) = self.send_to_device(Some(&item.device_id), frame, tap).await;
        // "Sent" is the last attempt that was written to the connection, acked or not (changelog 51 b): the phone may have the offer.
        if matches!(result, SendResult::Acked { .. } | SendResult::Rejected(_) | SendResult::NoReply) {
            let sent_at = unix_ms() as i64;
            if entry.status() == Status::Active {
                let expires = expiry_when_sent(sent_at);
                entry.set_expires_ms(expires as u64);
                let _ = self.db.set_expiry(&item.id, Some(expires));
            }
            let _ = self.db.mark_sent(&item.id, sent_at);
        } else if newly_served {
            // Nothing was written, so nothing was offered, and nothing is served for it.
            self.transfers.remove(&item.id);
        }
        if !matches!(result, SendResult::PhoneOffline) {
            self.record_send_attempt(&item.id, "offer");
        }
        match &result {
            SendResult::Acked { .. } => {
                // Only a still-queued item becomes active: a cancel, a `complete` or an expiry that came during the ack wait stays what it is.
                let activated = self.db.activate_if_queued(&item.id, now).unwrap_or(false);
                if activated && entry.status() == Status::Active {
                    let phone = self.db.get_device(&item.device_id).ok().flatten().map(|d| d.name);
                    let names = if entry.files.len() == 1 { entry.files[0].data().name } else { format!("{} files", entry.files.len()) };
                    let hash = crate::clipboard::short_hash(&names);
                    let who = phone.as_deref().unwrap_or("your phone");
                    let log_text = if entry.files.len() == 1 {
                        format!("Sending <file {hash}> to {who}")
                    } else {
                        format!("Sending {names} to {who}")
                    };
                    self.emit(Event::ToastRedacted { text: format!("Sending {names} to {who}"), log_text });
                }
            }
            SendResult::Rejected(code) => {
                log::warn!("offer {} was not taken: {code}", short_transfer(&item.id));
                self.drop_transfer(&entry, Status::Failed, Some(code)).await;
                self.record_send_end(&item.id, "offer", now, 1, "failed", Some(code), None);
                let phone = self.db.get_device(&item.device_id).ok().flatten().map(|d| d.name);
                self.offer_failed_toast(&result, phone.as_deref());
            }
            SendResult::NoReply => {
                self.record_send_error(&item.id, "no_reply");
                log::warn!("offer {}: no ack in 5 s, it stays queued", short_transfer(&item.id));
            }
            SendResult::PhoneOffline | SendResult::NoPhonePaired | SendResult::TooLarge { .. } => {}
        }
        Some(result)
    }

    /// An item that ends as failed: its row, its record, its temporary files.
    fn fail_item(&self, id: &str, kind: &str, code: &str) {
        let now = unix_ms() as i64;
        let _ = self.db.set_transfer_status(id, "failed", now, Some(code));
        self.record_send_end(id, kind, now, 1, "failed", Some(code), None);
        self.discard_sources(id);
        self.emit(Event::StateChanged);
    }

    /// The temporary files of an item that is over (the `.txt` of a long text) go. The user's own files are never touched.
    pub(crate) fn discard_sources(&self, id: &str) {
        if let Ok(Some((_, files))) = self.db.get_transfer(id) {
            for f in files.iter().filter(|f| f.temp_source) {
                if let Some(path) = &f.local_path {
                    remove_temp_source(&PathBuf::from(path));
                }
            }
        }
    }

    /// The live state of a queued offer, from its rows. The files are checked: one that is gone or changed fails the item here, with a
    /// code, instead of failing at the phone's `GET`.
    fn entry_from_db(&self, item: &QueueItem) -> Result<Arc<Entry>, &'static str> {
        let (row, files) = self.db.get_transfer(&item.id).ok().flatten().ok_or("internal")?;
        let mut shared = Vec::new();
        for f in &files {
            let path = PathBuf::from(f.local_path.clone().unwrap_or_default());
            match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => {
                    if m.len() != f.size as u64 || f.source_mtime_ms.is_some_and(|t| t != outgoing::mtime_ms(&m)) {
                        return Err("source_changed");
                    }
                }
                _ => return Err("source_gone"),
            }
            let data =
                FileData { name: f.name.clone(), path, finalized: false, source_mtime_ms: f.source_mtime_ms, temp_source: f.temp_source };
            shared.push(crate::transfers::file_shared(&f.file_id, f.idx as usize, f.size as u64, &f.mime, data));
        }
        Ok(Entry::new(&row.id, &row.device_id, Direction::Out, row.created_at as u64, u64::MAX, PathBuf::new(), shared))
    }

    // ----- expiry, cancel, retry -----------------------------------------------------------------------------------------

    /// Fails the queued items that are over their time, as of `now_ms` (a test passes a clock it moved by hand; the app passes the
    /// wall clock). Returns how many.
    pub async fn expire_queue(self: &Arc<Self>, now_ms: i64) -> usize {
        let _flush = self.queue.flush.lock().await;
        self.expire_locked(now_ms).await
    }

    async fn expire_locked(self: &Arc<Self>, now_ms: i64) -> usize {
        let ids = self.db.expired_queued(now_ms).unwrap_or_default();
        let mut expired = 0;
        for id in &ids {
            // One item at a time, held, and only if it is still queued (the user may have canceled it since the list was read).
            let Some(_claim) = self.queue.claim_if_queued(&self.db, id) else {
                continue;
            };
            let Some((_, kind)) = self.db.status_of(id).ok().flatten() else {
                continue;
            };
            expired += 1;
            log::info!("queued {} {} expired", if kind == "text" { "text" } else { "offer" }, short_transfer(id));
            match self.transfers.get(id).filter(|e| e.status() == Status::Active) {
                Some(entry) => {
                    // An offer that was sent into a dead connection: the phone may have it, so it is told.
                    self.send_cancel_message(&entry.device_id, &entry.id, "expired");
                    self.drop_transfer(&entry, Status::Failed, Some("expired")).await;
                }
                None => {
                    let _ = self.db.set_transfer_status(id, "failed", now_ms, Some("expired"));
                    self.discard_sources(id);
                }
            }
            self.record_send_end(id, if kind == "text" { "text" } else { "offer" }, now_ms, 1, "failed", Some("expired"), None);
            self.fail_waiter(id);
        }
        if expired > 0 {
            self.emit(Event::StateChanged);
        }
        expired
    }

    fn fail_waiter(&self, id: &str) {
        self.queue.forget(id);
    }

    /// The user cancels an item that is still queued: it never goes out. Returns whether there was such an item.
    pub async fn cancel_queued(self: &Arc<Self>, id: &str) -> bool {
        // Waits only if *this* item is being sent right now (at most the 5 s ack timeout), never for the rest of a flush: the flush takes each
        // item with its status read again, so an item is never sent and canceled at once.
        let _claim = self.queue.claim_when_free(id).await;
        let Ok(Some((status, kind))) = self.db.status_of(id) else {
            return false;
        };
        if status != "queued" {
            return false;
        }
        let now = unix_ms() as i64;
        if let Some(entry) = self.transfers.get(id).filter(|e| e.direction == Direction::Out && e.status() == Status::Active) {
            // It was sent once into a connection that died, so the phone may know it: the ordinary cancel, with the message.
            log::info!("the user canceled queued offer {} that had been sent once", short_transfer(id));
            self.send_cancel_message(&entry.device_id, &entry.id, "user");
            self.drop_transfer(&entry, Status::Canceled, Some("user")).await;
        } else {
            let _ = self.db.set_transfer_status(id, "canceled", now, Some("user"));
            self.discard_sources(id);
            log::info!(
                "the user canceled queued {} {}: it will never be sent",
                if kind == "text" { "text" } else { "offer" },
                short_transfer(id)
            );
        }
        self.record_send_end(id, if kind == "text" { "text" } else { "offer" }, now, 1, "canceled", Some("user"), None);
        self.fail_waiter(id);
        self.emit(Event::StateChanged);
        true
    }

    /// The user retries a failed item that the PC sent: a new item with a new ID (an ID is never used twice), linked to the old one.
    /// Returns the new ID, or why it can't be done.
    pub async fn retry_item(self: &Arc<Self>, id: &str) -> Result<String, String> {
        let row = self.db.history_item(id).ok().flatten().ok_or("That item is not in the history.")?;
        if row.direction != "out" || row.status != "failed" {
            return Err("Only a failed send from this PC can be sent again.".into());
        }
        if self.db.is_retried(id).unwrap_or(false) {
            return Err("That item was already sent again.".into());
        }
        if self.queue_target().is_none() {
            return Err("No phone is paired yet.".into());
        }
        let new_id = if row.kind == "text" {
            let text = row.text.clone().ok_or("The text is no longer stored.")?;
            let new_id = uuid_v7(unix_ms());
            let now = unix_ms() as i64;
            let device_id = self.queue_target().ok_or("No phone is paired yet.")?;
            let offline = !self.phone_connected();
            let stored = if row.is_link {
                let source = self.db.text_item(id).ok().flatten().and_then(|t| t.source_app);
                self.db.insert_link_item(
                    &new_id,
                    &device_id,
                    "out",
                    "queued",
                    now,
                    Some(expiry_when_queued(now, "text")),
                    &text,
                    row.title.as_deref(),
                    source.as_deref(),
                    Some(id),
                )
            } else {
                self.db.insert_text_item(
                    &new_id,
                    &device_id,
                    "out",
                    "queued",
                    now,
                    Some(expiry_when_queued(now, "text")),
                    &text,
                    row.is_url,
                    Some(id),
                )
            };
            stored.map_err(|e| self.retry_error(id, &e.to_string()))?;
            self.record_send_start(&new_id, "text", i64::from(offline), offline);
            new_id
        } else {
            let paths: Vec<PathBuf> = row.files.iter().filter_map(|f| f.local_path.as_ref().map(PathBuf::from)).collect();
            let (sources, folders, unreadable) =
                tokio::task::spawn_blocking(move || outgoing::inspect(&paths)).await.map_err(|e| e.to_string())?;
            if sources.len() != row.files.len() || folders > 0 || !unreadable.is_empty() {
                return Err("The files can't be found or read any more, so they can't be sent again.".into());
            }
            let (new_id, _rx) =
                self.enqueue_offer(&sources, Some(id)).ok_or_else(|| self.retry_error(id, "The offer could not be saved."))?;
            new_id
        };
        // The new item takes the failed one's place in the success rate (metrics.rs) and carries its attempt: a retried send is never a
        // first-try success, as on the phone.
        self.record_send_attempt(&new_id, if row.kind == "text" { "text" } else { "offer" });
        log::info!("the user retried {} as {}", short_transfer(id), short_transfer(&new_id));
        self.emit(Event::StateChanged);
        if self.phone_connected() {
            let core = self.clone();
            tokio::spawn(async move { core.flush_queue().await });
        }
        Ok(new_id)
    }

    /// Why a retry could not be saved: a second click on Retry loses against the unique index of `retry_of`, which is not an error the user
    /// should read as one.
    fn retry_error(&self, id: &str, other: &str) -> String {
        if self.db.is_retried(id).unwrap_or(false) { "That item was already sent again.".to_string() } else { other.to_string() }
    }

    /// The phone's `hello` was accepted: the `cancel` messages it never acked go first (changelog 52), then the queue. Runs in the
    /// background, so the connection's own task never waits.
    pub fn phone_connected_flush(self: &Arc<Self>, device_id: &str) {
        let core = self.clone();
        let device_id = device_id.to_string();
        tokio::spawn(async move {
            core.flush_pending_cancels(&device_id).await;
            core.flush_queue().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_expires_after_7_days_and_an_unsent_offer_too() {
        let created = 1_000_000;
        assert_eq!(expiry_when_queued(created, "text"), created + 7 * 24 * 3_600_000);
        assert_eq!(expiry_when_queued(created, "file"), created + 7 * 24 * 3_600_000);
        assert_eq!(expiry_when_sent(created), created + 24 * 3_600_000, "a sent offer: 24 hours after it was sent");
    }

    #[test]
    fn expiry_is_decided_at_the_edge_with_an_explicit_clock() {
        let expires = Some(expiry_when_queued(0, "text"));
        assert!(!is_expired(expires, TEXT_TTL_MS - 1), "one millisecond before 7 days");
        assert!(is_expired(expires, TEXT_TTL_MS), "at exactly 7 days");
        assert!(is_expired(expires, TEXT_TTL_MS + 1));
        assert!(!is_expired(None, i64::MAX), "an item with no expiry never expires");
    }
}
