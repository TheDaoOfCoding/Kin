//! The live WebSockets, one per device (protocol.md, "One connection per device"), and the commands the rest of the
//! app can send to a connection.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::messages::{CLOSE_REPLACED, CLOSE_UNAUTHORIZED};

/// How a message that needs an `ack` (`ping`, `text`, `clipboard`) ended.
#[derive(Debug, PartialEq)]
pub enum SendOutcome {
    /// The round-trip time, measured with a monotonic clock from sending the message to the matching `ack`.
    Ack(Duration),
    /// No `ack` within 5 seconds. The connection is closed as dead right after, so the phone counts as offline.
    NoReply,
    /// The phone answered with an `error` instead of an `ack`.
    Failed(String),
    /// The message was never written to the connection (no connection, or it was not open yet).
    NotConnected,
    /// The message was written, and the connection ended before the `ack` came: the phone may have it (the queue needs to know the
    /// difference: an offer that may have reached the phone must stay served).
    Lost,
}

pub enum ConnCommand {
    Close {
        code: u16,
        reason: &'static str,
    },
    Ping(oneshot::Sender<SendOutcome>),
    /// Send this message (it has an `id`) and report the `ack`. Used for `text` and `clipboard`. The connection checks that
    /// it is open, and that is all it does with the frame.
    Send {
        frame: serde_json::Value,
        reply: oneshot::Sender<SendOutcome>,
    },
    /// Send a `notif.*` message (changelog 56): only to a peer whose latest `hello` lists `notif` (else it is dropped, and `reply` hears `NotConnected`).
    /// With no `reply` it is ephemeral (`notif.dismiss`: never acked, never re-sent). With a `reply` it is a `notif.action`: the `ack` or `error` is
    /// reported, no `ack` in 5 seconds is `NoReply` **without** closing the connection, and it is never sent again.
    Notif {
        frame: serde_json::Value,
        reply: Option<oneshot::Sender<SendOutcome>>,
    },
    /// Send a `media.*` message (changelog 57): only to a peer whose latest `hello` lists `media` (else it is dropped, silently). Never acked, never queued, never re-sent.
    Media {
        frame: serde_json::Value,
    },
    /// Send a `presence` (changelog 58): only to a peer whose latest `hello` lists `presence` (else it is dropped, silently). Never acked, never queued, never
    /// re-sent: a fresh snapshot goes after each reconnect.
    Presence {
        frame: serde_json::Value,
    },
    /// Send a `ring.start` or `ring.stop` (changelog 58): only to a peer whose latest `hello` lists `ring` (else `reply` hears `NotConnected`). Acked, never queued,
    /// never re-sent: with a `reply` the `ack` or `error` is reported, no `ack` in 5 seconds is `NoReply` **without** closing the connection.
    Ring {
        frame: serde_json::Value,
        reply: Option<oneshot::Sender<SendOutcome>>,
    },
    /// Send a new `hello` (the name of this PC changed, or the setting "Show phone notifications" did).
    Hello,
    /// Tell the phone it was removed: an `unpair` message, then close after the `ack` (or 5 seconds).
    Unpair,
}

struct Entry {
    conn_id: u64,
    tx: UnboundedSender<ConnCommand>,
    /// When the phone's `hello` was accepted. A connection counts as connected only from then on.
    open_since: Option<Instant>,
    /// The `capabilities` of the phone's latest accepted `hello` of this connection.
    capabilities: Vec<String>,
}

#[derive(Default)]
pub struct Connections {
    next_id: AtomicU64,
    map: Mutex<HashMap<String, Entry>>,
}

impl Connections {
    fn map(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.map.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Registers a new connection of a device. An older one of the same device is closed with 4001.
    pub fn register(&self, device_id: &str) -> (u64, UnboundedReceiver<ConnCommand>) {
        let conn_id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = unbounded_channel();
        let old = self.map().insert(device_id.to_string(), Entry { conn_id, tx, open_since: None, capabilities: Vec::new() });
        if let Some(old) = old {
            let _ = old.tx.send(ConnCommand::Close { code: CLOSE_REPLACED, reason: "replaced by a new connection" });
        }
        (conn_id, rx)
    }

    pub fn mark_open(&self, device_id: &str, conn_id: u64) {
        if let Some(e) = self.map().get_mut(device_id).filter(|e| e.conn_id == conn_id) {
            e.open_since.get_or_insert_with(Instant::now);
        }
    }

    /// Remembers what the phone's latest accepted `hello` listed (every accepted `hello` replaces it).
    pub fn set_capabilities(&self, device_id: &str, conn_id: u64, capabilities: &[String]) {
        if let Some(e) = self.map().get_mut(device_id).filter(|e| e.conn_id == conn_id) {
            e.capabilities = capabilities.to_vec();
        }
    }

    /// Whether this device is connected (its `hello` was accepted) and its latest `hello` lists the capability.
    pub fn has_capability(&self, device_id: &str, capability: &str) -> bool {
        self.map().get(device_id).is_some_and(|e| e.open_since.is_some() && e.capabilities.iter().any(|c| c == capability))
    }

    /// Forgets a connection that ended. `false` if a newer one has taken its place already.
    pub fn remove(&self, device_id: &str, conn_id: u64) -> bool {
        let mut map = self.map();
        let ours = map.get(device_id).is_some_and(|e| e.conn_id == conn_id);
        if ours {
            map.remove(device_id);
        }
        ours
    }

    /// Devices with an accepted `hello`.
    pub fn open_device_ids(&self) -> Vec<String> {
        self.map().iter().filter(|(_, e)| e.open_since.is_some()).map(|(id, _)| id.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.map().is_empty()
    }

    /// The device whose `hello` was accepted last, with a way to send commands to it.
    pub fn most_recent_open(&self) -> Option<(String, UnboundedSender<ConnCommand>)> {
        self.map()
            .iter()
            .filter_map(|(id, e)| e.open_since.map(|at| (at, id, e)))
            .max_by_key(|(at, _, _)| *at)
            .map(|(_, id, e)| (id.clone(), e.tx.clone()))
    }

    /// A way to send commands to this device, if its `hello` was accepted (the queue sends to the device an item is for, never to
    /// whichever phone happens to be the newest).
    pub fn open_sender(&self, device_id: &str) -> Option<UnboundedSender<ConnCommand>> {
        self.map().get(device_id).filter(|e| e.open_since.is_some()).map(|e| e.tx.clone())
    }

    /// The device was removed: an open connection gets an `unpair`, one that hasn't said `hello` yet is closed with 4003.
    pub fn revoke(&self, device_id: &str) {
        if let Some(e) = self.map().get(device_id) {
            let _ = e.tx.send(if e.open_since.is_some() {
                ConnCommand::Unpair
            } else {
                ConnCommand::Close { code: CLOSE_UNAUTHORIZED, reason: "device removed" }
            });
        }
    }

    /// Every open connection gets a new `hello` (the PC's name changed).
    pub fn send_hello_to_all(&self) {
        for e in self.map().values().filter(|e| e.open_since.is_some()) {
            let _ = e.tx.send(ConnCommand::Hello);
        }
    }

    /// Every open connection whose peer's latest `hello` lists `capability` gets a new `hello` (the PC's own list of capabilities changed: a peer that
    /// doesn't list it has nothing to learn from it).
    pub fn send_hello_to_capable(&self, capability: &str) {
        for e in self.map().values().filter(|e| e.open_since.is_some() && e.capabilities.iter().any(|c| c == capability)) {
            let _ = e.tx.send(ConnCommand::Hello);
        }
    }

    pub fn close_all(&self, code: u16, reason: &'static str) {
        for e in self.map().values() {
            let _ = e.tx.send(ConnCommand::Close { code, reason });
        }
    }
}
