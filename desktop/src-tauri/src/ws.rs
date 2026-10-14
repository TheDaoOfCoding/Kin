//! One WebSocket connection of a paired phone: the `hello` exchange, `ack`, `error`, `ping` and `unpair`, the close codes,
//! and the liveness rules of architecture.md ("Connection lifecycle").
//!
//! The message rules are in `messages.rs`. This file only moves frames and keeps the timers.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::connections::{ConnCommand, SendOutcome};
use crate::core::{APP_VERSION, Core, Event, short};
use crate::encoding::{unix_ms, uuid_v7};
use crate::messages::{self, Action, Dedup, Frame, Outcome, Receiver, Role};

/// The PC marks a phone offline after this much silence. The phone's WebSocket ping every 4 minutes keeps it alive.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// An `ack` is due within this time. Without it the connection counts as dead.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the peer's close frame after sending ours.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// Remembers message IDs in the database, so a repeat gets the same answer even after a restart.
struct DbDedup<'a> {
    core: &'a Core,
    device_id: &'a str,
}

impl Dedup for DbDedup<'_> {
    fn lookup(&mut self, id: &str) -> Option<String> {
        self.core.db.seen_response(self.device_id, id).unwrap_or_else(|e| {
            log::error!("reading the message memory failed: {e}");
            None
        })
    }

    fn remember(&mut self, id: &str, response: &str) {
        if let Err(e) = self.core.db.remember_message(self.device_id, id, response, unix_ms() as i64) {
            log::error!("saving the message memory failed: {e}");
        }
    }
}

struct Pending {
    sent: Instant,
    reply: oneshot::Sender<SendOutcome>,
}

/// How a connection ended, for the log and for what to send.
enum End {
    /// We close with this code.
    Close {
        code: u16,
        reason: String,
    },
    /// The phone closed (or the socket ended). The close handshake is left to the library.
    PeerClosed(String),
    Error(String),
}

struct Conn {
    core: Arc<Core>,
    device_id: String,
    remote: SocketAddr,
    conn_id: u64,
    socket: WebSocket,
    receiver: Receiver,
    opened: Instant,
    last_rx: Instant,
    pending: HashMap<String, Pending>,
    /// The `id` of an `unpair` we sent and when we stop waiting for its `ack`.
    unpair: Option<(String, Instant)>,
    hello_accepted: bool,
    /// The device was unpaired, by either side. The phone forgets this PC and doesn't reconnect.
    unpaired: bool,
}

enum Timer {
    Hello,
    Idle,
    Ack,
    Unpair,
}

/// Runs a connection until it ends. `device_id` comes from the TLS client certificate, and the caller has checked
/// that it is paired.
pub async fn run(core: Arc<Core>, socket: WebSocket, device_id: String, remote: SocketAddr) {
    let (conn_id, mut commands) = core.connections.register(&device_id);
    log::info!("websocket opened: device {} from {remote}", short(&device_id));
    let now = Instant::now();
    let mut conn = Conn {
        core,
        device_id,
        remote,
        conn_id,
        socket,
        receiver: Receiver::new(Role::Pc),
        opened: now,
        last_rx: now,
        pending: HashMap::new(),
        unpair: None,
        hello_accepted: false,
        unpaired: false,
    };
    let end = conn.drive(&mut commands).await;
    conn.finish(end).await;
}

impl Conn {
    async fn drive(&mut self, commands: &mut tokio::sync::mpsc::UnboundedReceiver<ConnCommand>) -> End {
        let hello = json!({
            "type": "hello",
            "id": uuid_v7(unix_ms()),
            "protocol": messages::PROTOCOL,
            "name": self.core.pc_name,
            "platform": "windows",
            "appVersion": APP_VERSION,
        });
        if let Err(e) = self.send(&hello).await {
            return End::Error(e);
        }
        loop {
            let (deadline, timer) = self.next_timer();
            let end = tokio::select! {
                msg = self.socket.recv() => match msg {
                    None => Some(End::PeerClosed("the connection ended without a close frame".into())),
                    Some(Ok(msg)) => self.on_message(msg).await,
                    Some(Err(e)) => Some(classify_error(e)),
                },
                cmd = commands.recv() => match cmd {
                    Some(cmd) => self.on_command(cmd).await,
                    None => Some(End::Close { code: messages::CLOSE_NORMAL, reason: "app quit".into() }),
                },
                () = tokio::time::sleep_until(deadline) => Some(self.on_timer(timer)),
            };
            if let Some(end) = end {
                return end;
            }
        }
    }

    /// The earliest of the things we are waiting for.
    fn next_timer(&self) -> (Instant, Timer) {
        let mut next = (self.last_rx + IDLE_TIMEOUT, Timer::Idle);
        let mut consider = |at: Instant, t: Timer| {
            if at < next.0 {
                next = (at, t);
            }
        };
        if !self.receiver.is_open() {
            consider(self.opened + Duration::from_millis(messages::HELLO_TIMEOUT_MS), Timer::Hello);
        }
        if let Some(at) = self.pending.values().map(|p| p.sent + ACK_TIMEOUT).min() {
            consider(at, Timer::Ack);
        }
        if let Some((_, at)) = &self.unpair {
            consider(*at, Timer::Unpair);
        }
        next
    }

    fn on_timer(&mut self, timer: Timer) -> End {
        match timer {
            Timer::Hello => {
                // No error message goes first: the connection is just closed (the "timers[]" vectors).
                let code = self.receiver.hello_timeout(self.opened.elapsed().as_millis() as u64).unwrap_or(messages::CLOSE_PROTOCOL_ERROR);
                End::Close { code, reason: "no hello within 10 s".into() }
            }
            Timer::Idle => End::Close { code: messages::CLOSE_NORMAL, reason: "idle for 10 minutes".into() },
            Timer::Ack => {
                // A message that isn't acknowledged in time means the connection is dead (architecture.md).
                let now = Instant::now();
                let late: Vec<String> =
                    self.pending.iter().filter(|(_, p)| now >= p.sent + ACK_TIMEOUT).map(|(id, _)| id.clone()).collect();
                for id in late {
                    if let Some(p) = self.pending.remove(&id) {
                        let _ = p.reply.send(SendOutcome::NoReply);
                    }
                }
                End::Close { code: messages::CLOSE_NORMAL, reason: "no ack within 5 s".into() }
            }
            Timer::Unpair => End::Close { code: messages::CLOSE_NORMAL, reason: "unpaired".into() },
        }
    }

    async fn on_message(&mut self, msg: Message) -> Option<End> {
        self.last_rx = Instant::now();
        let outcome = {
            let mut dedup = DbDedup { core: &self.core, device_id: &self.device_id };
            match &msg {
                Message::Text(t) => self.receiver.handle_frame(Frame::Text(t.as_bytes()), &mut dedup),
                Message::Binary(_) => self.receiver.handle_frame(Frame::Binary, &mut dedup),
                // WebSocket ping/pong frames are the liveness mechanism. The library answers pings itself.
                Message::Ping(_) | Message::Pong(_) => return None,
                Message::Close(frame) => {
                    let detail = frame.as_ref().map_or("no code".to_string(), |f| format!("code {} \"{}\"", f.code, f.reason.as_str()));
                    return Some(End::PeerClosed(format!("the phone closed the connection ({detail})")));
                }
            }
        };
        self.apply(outcome).await
    }

    async fn apply(&mut self, outcome: Outcome) -> Option<End> {
        if let Some(reply) = &outcome.reply
            && let Err(e) = self.send(reply).await
        {
            return Some(End::Error(e));
        }
        match outcome.action {
            Action::HandshakeOk => self.on_hello(),
            Action::MarkDelivered { reference } => {
                if let Some(p) = self.pending.remove(&reference) {
                    let _ = p.reply.send(SendOutcome::Ack(p.sent.elapsed()));
                } else if self.unpair.as_ref().is_some_and(|(id, _)| *id == reference) {
                    return Some(End::Close { code: messages::CLOSE_NORMAL, reason: "unpaired".into() });
                }
            }
            Action::MarkFailed { reference, code } => {
                log::warn!("phone answered with an error ({code})");
                if let Some(p) = self.pending.remove(&reference) {
                    let _ = p.reply.send(SendOutcome::Failed(code));
                }
            }
            Action::ShowPingToast => {
                let name = self.receiver.peer().map_or("phone", |p| p.name.as_str()).to_string();
                log::info!("ping from {name}");
                self.core.emit(Event::Toast(format!("Ping from {name}")));
            }
            // The `ack` is out already (acks are due as soon as the message is valid and recorded). Now the text goes to the
            // clipboard and a toast. The text itself is never logged.
            Action::DeliverText { id, text, is_url } => {
                let from = self.receiver.peer().map_or("phone".to_string(), |p| p.name.clone());
                self.core.receive_text(&self.device_id, &from, &id, text, is_url).await;
            }
            Action::RemovePeerAndClose => {
                self.unpaired = true;
                match self.core.db.remove_device(&self.device_id) {
                    Ok(_) => log::info!("device {} unpaired itself", short(&self.device_id)),
                    Err(e) => log::error!("removing the unpaired device failed: {e}"),
                }
                self.core.emit(Event::PairingChanged);
            }
            // A `clipboard` message only goes PC to phone: the receiver answers `unsupported_type` before it gets here.
            Action::None | Action::ShowErrorOnly | Action::IgnoreDuplicate | Action::CloseOnly | Action::SetClipboard { .. } => {}
        }
        outcome.close.map(|code| End::Close { code, reason: close_reason(code).into() })
    }

    /// The phone's `hello` was accepted: it counts as connected now.
    fn on_hello(&mut self) {
        let Some(peer) = self.receiver.peer().cloned() else {
            return;
        };
        if let Err(e) = self.core.db.update_name(&self.device_id, &peer.name) {
            log::error!("saving the phone's name failed: {e}");
        }
        if self.hello_accepted {
            return;
        }
        self.hello_accepted = true;
        let _ = self.core.db.touch(&self.device_id, unix_ms() as i64, Some(&self.remote.to_string()));
        self.core.connections.mark_open(&self.device_id, self.conn_id);
        log::info!(
            "phone connected: {:?} ({}, app {}) device {} from {}",
            peer.name,
            peer.platform,
            peer.app_version,
            short(&self.device_id),
            self.remote
        );
        self.core.emit(Event::StateChanged);
    }

    async fn on_command(&mut self, cmd: ConnCommand) -> Option<End> {
        match cmd {
            ConnCommand::Close { code, reason } => Some(End::Close { code, reason: reason.into() }),
            ConnCommand::Ping(reply) => {
                let id = uuid_v7(unix_ms());
                let frame = json!({ "type": "ping", "id": id, "sentAt": unix_ms() });
                self.send_tracked(id, frame, reply).await
            }
            ConnCommand::Send { frame, reply } => match frame.get("id").and_then(|v| v.as_str()).map(str::to_string) {
                Some(id) => self.send_tracked(id, frame, reply).await,
                None => {
                    let _ = reply.send(SendOutcome::Failed("internal".into()));
                    None
                }
            },
            ConnCommand::Unpair => {
                let id = uuid_v7(unix_ms());
                match self.send(&json!({ "type": "unpair", "id": id })).await {
                    Ok(()) => {
                        self.unpaired = true;
                        self.unpair = Some((id, Instant::now() + ACK_TIMEOUT));
                        None
                    }
                    Err(e) => Some(End::Error(e)),
                }
            }
        }
    }

    /// Sends a message that needs an `ack` and waits for it: `Timer::Ack` ends the connection after 5 s, and `on_message`
    /// resolves it when the `ack` (or an `error`) with this `ref` arrives. The frame's content is not logged, only its type.
    async fn send_tracked(&mut self, id: String, frame: serde_json::Value, reply: oneshot::Sender<SendOutcome>) -> Option<End> {
        if !self.receiver.is_open() {
            let _ = reply.send(SendOutcome::NotConnected);
            return None;
        }
        let sent = Instant::now();
        if let Err(e) = self.send(&frame).await {
            let _ = reply.send(SendOutcome::NotConnected);
            return Some(End::Error(e));
        }
        log::info!("{} {id} sent to device {}", frame["type"].as_str().unwrap_or("message"), short(&self.device_id));
        self.pending.insert(id, Pending { sent, reply });
        None
    }

    async fn send(&mut self, value: &serde_json::Value) -> Result<(), String> {
        self.socket.send(Message::Text(value.to_string().into())).await.map_err(|e| format!("sending failed: {e}"))
    }

    /// Sends the close frame if we are the one closing, then cleans up and logs why the connection ended.
    async fn finish(mut self, end: End) {
        let (why, code) = match end {
            End::Close { code, reason } => {
                let frame = CloseFrame { code, reason: reason.clone().into() };
                if self.socket.send(Message::Close(Some(frame))).await.is_ok() {
                    // Give the phone a moment to answer with its own close frame, so ours isn't cut off.
                    let _ = tokio::time::timeout(CLOSE_GRACE, async { while self.socket.recv().await.is_some() {} }).await;
                }
                (format!("closed by the PC: {reason} (code {code})"), Some(code))
            }
            End::PeerClosed(text) => {
                let _ = tokio::time::timeout(CLOSE_GRACE, async { while self.socket.recv().await.is_some() {} }).await;
                (text, None)
            }
            End::Error(text) => (text, None),
        };
        for (_, p) in self.pending.drain() {
            let _ = p.reply.send(SendOutcome::NotConnected);
        }
        let current = self.core.connections.remove(&self.device_id, self.conn_id);
        if self.hello_accepted {
            let _ = self.core.db.touch(&self.device_id, unix_ms() as i64, None);
        }
        // What the phone will do next is the other half of the story; it makes a log easier to read.
        let phone_next = if self.unpaired { "forget this PC (unpaired)" } else { messages::client_action(code, None).vector_name() };
        log::info!(
            "websocket ended: device {} from {}, after {} s: {why}. The phone will: {phone_next}{}",
            short(&self.device_id),
            self.remote,
            self.opened.elapsed().as_secs(),
            if current { "" } else { " (a newer connection of this device is active)" }
        );
        self.core.emit(Event::StateChanged);
    }
}

/// The two errors the library reports where our rules want a close code (protocol.md, "Frames").
fn classify_error(e: axum::Error) -> End {
    match e.into_inner().downcast::<tungstenite::Error>().map(|b| *b) {
        Ok(tungstenite::Error::Capacity(_)) => {
            End::Close { code: messages::CLOSE_TOO_BIG, reason: close_reason(messages::CLOSE_TOO_BIG).into() }
        }
        Ok(tungstenite::Error::Utf8(_)) => {
            End::Close { code: messages::CLOSE_INVALID_PAYLOAD, reason: close_reason(messages::CLOSE_INVALID_PAYLOAD).into() }
        }
        Ok(other) => End::Error(format!("connection error: {other}")),
        Err(other) => End::Error(format!("connection error: {other}")),
    }
}

/// Short human text for the close frame (at most 123 bytes; nothing matches on it).
fn close_reason(code: u16) -> &'static str {
    match code {
        messages::CLOSE_NORMAL => "normal closure",
        messages::CLOSE_PROTOCOL_ERROR => "protocol error",
        messages::CLOSE_UNSUPPORTED_DATA => "binary frames are not supported",
        messages::CLOSE_INVALID_PAYLOAD => "invalid UTF-8",
        messages::CLOSE_TOO_BIG => "message over 1 MiB",
        messages::CLOSE_REPLACED => "replaced by a new connection",
        messages::CLOSE_UNSUPPORTED_VERSION => "unsupported protocol version",
        messages::CLOSE_UNAUTHORIZED => "device removed",
        _ => "closed",
    }
}
