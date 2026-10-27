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
use crate::messages::{self, Action, Dedup, Frame, NotifDrop, Outcome, Receiver, Role};

/// The PC marks a phone offline after this much silence. The phone's WebSocket ping every 4 minutes keeps it alive.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// An `ack` is due within this time. Without it the connection counts as dead.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the peer's close frame after sending ours.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The `hello` of this PC: its name (the `deviceName` setting, else the computer's), and the version.
fn hello_frame(core: &Core) -> serde_json::Value {
    json!({
        "type": "hello",
        "id": uuid_v7(unix_ms()),
        "protocol": messages::PROTOCOL,
        "name": core.pc_name(),
        "platform": "windows",
        "appVersion": APP_VERSION,
        "capabilities": core.capabilities(),
    })
}

/// A `link` goes on the wire as a `link` only to a peer whose latest `hello` lists the `link` capability. To any other peer (a 0.1.x phone) the same
/// item goes as a `text`: the same `id`, the URL as the text, `isUrl` true, no title (protocol.md, "Versioning"). The choice is made here, when the
/// message goes out, so a link that waited in the queue is sent in the form the peer understands now. Every other frame is left alone.
pub fn wire_form(frame: serde_json::Value, peer: Option<&messages::Peer>) -> serde_json::Value {
    if frame["type"] != "link" || peer.is_some_and(|p| p.has("link")) {
        return frame;
    }
    json!({ "type": "text", "id": frame["id"], "text": frame["url"], "isUrl": true })
}

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

/// How long a `notif.action` waits for its `ack`. Unlike any other message, no answer does **not** close the connection: the press is reported as
/// "no answer" and never sent again (protocol.md, changelog 56).
const ACTION_ACK_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Nothing ends: the timer was only a late `notif.action`.
    Continue,
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
    /// The `notif.action` messages that are waiting for their `ack`.
    actions: HashMap<String, Pending>,
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
    ActionAck,
    Unpair,
}

/// Runs a connection until it ends. `device_id` comes from the TLS client certificate, and the caller has checked
/// that it is paired.
pub async fn run(core: Arc<Core>, socket: WebSocket, device_id: String, remote: SocketAddr) {
    let delay = core.hooks.ws_register_delay_ms.load(std::sync::atomic::Ordering::Relaxed);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let (conn_id, mut commands) = core.connections.register(&device_id);
    log::info!("websocket opened: device {} from {remote}", short(&device_id));
    let now = Instant::now();
    let mut receiver = Receiver::new(Role::Pc);
    receiver.set_own_capabilities(&core.capabilities());
    let mut conn = Conn {
        core,
        device_id,
        remote,
        conn_id,
        socket,
        receiver,
        opened: now,
        last_rx: now,
        pending: HashMap::new(),
        actions: HashMap::new(),
        unpair: None,
        hello_accepted: false,
        unpaired: false,
    };
    // The upgrade checked the pairing, but the connection is only known to `connections` from here: an unlink in between had no connection
    // to tell. The removal is in the database before anything is revoked, so a check now sees it. Nothing of this phone is handled.
    let end = if conn.core.db.is_paired(&conn.device_id).unwrap_or(false) {
        conn.drive(&mut commands).await
    } else {
        log::info!("device {} was unlinked while its WebSocket was opening: closed", short(&conn.device_id));
        End::Close { code: messages::CLOSE_UNAUTHORIZED, reason: close_reason(messages::CLOSE_UNAUTHORIZED).into() }
    };
    conn.finish(end).await;
}

impl Conn {
    async fn drive(&mut self, commands: &mut tokio::sync::mpsc::UnboundedReceiver<ConnCommand>) -> End {
        if let Err(e) = self.send(&hello_frame(&self.core)).await {
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
            match end {
                Some(End::Continue) | None => {}
                Some(end) => return end,
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
        if let Some(at) = self.actions.values().map(|p| p.sent + ACTION_ACK_TIMEOUT).min() {
            consider(at, Timer::ActionAck);
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
            Timer::ActionAck => {
                // The press gets "no answer" and the connection goes on.
                let now = Instant::now();
                let late: Vec<String> =
                    self.actions.iter().filter(|(_, p)| now >= p.sent + ACTION_ACK_TIMEOUT).map(|(id, _)| id.clone()).collect();
                for id in late {
                    if let Some(p) = self.actions.remove(&id) {
                        let _ = p.reply.send(SendOutcome::NoReply);
                    }
                }
                End::Continue
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
                if let Some(p) = self.pending.remove(&reference).or_else(|| self.actions.remove(&reference)) {
                    let _ = p.reply.send(SendOutcome::Ack(p.sent.elapsed()));
                } else if self.unpair.as_ref().is_some_and(|(id, _)| *id == reference) {
                    return Some(End::Close { code: messages::CLOSE_NORMAL, reason: "unpaired".into() });
                }
            }
            Action::MarkFailed { reference, code } => {
                // The code comes from the phone: only a word of the protocol's table reaches the log, and (as the `SendOutcome`) the history
                // row, `transfers.error_code`, `debug.json` and every message made from it. Anything else is `other`.
                let word = messages::error_code_word(&code);
                log::warn!("phone answered with an error ({word})");
                if let Some(p) = self.pending.remove(&reference).or_else(|| self.actions.remove(&reference)) {
                    let _ = p.reply.send(SendOutcome::Failed(word.to_string()));
                }
            }
            Action::ShowPingToast => {
                let name = self.receiver.peer().map_or("phone", |p| p.name.as_str()).to_string();
                log::info!("ping from {name}");
                self.core.emit(Event::Toast(format!("Ping from {name}")));
            }
            // The `ack` is out already (acks are due as soon as the message is valid and recorded). Now the text goes to the
            // clipboard and a toast. The text itself is never logged.
            Action::DeliverText { id, text, is_url, sensitive } => {
                let from = self.receiver.peer().map_or("phone".to_string(), |p| p.name.clone());
                self.core.receive_text(&self.device_id, &from, &id, text, is_url, sensitive).await;
            }
            // A link is opened (or offered) by the core, which also decides about fullscreen programs. It never goes to the clipboard.
            // It runs as a task of its own: opening a browser can take a moment and the connection's task must go on answering pings and acks.
            Action::DeliverLink { id, url, title, source_app } => {
                let (core, device_id) = (self.core.clone(), self.device_id.clone());
                tokio::spawn(async move { core.receive_link(&device_id, &id, url, title, source_app).await });
            }
            // The control channel never waits for files: what these do (stopping a writer, deleting partial files) runs on its own.
            Action::MarkTransferDone { transfer_id } => {
                let (core, device_id) = (self.core.clone(), self.device_id.clone());
                tokio::spawn(async move { core.peer_completed(&device_id, &transfer_id).await });
            }
            Action::CancelTransfer { transfer_id, reason } => {
                let (core, device_id) = (self.core.clone(), self.device_id.clone());
                tokio::spawn(async move { core.peer_cancelled(&device_id, &transfer_id, &reason).await });
            }
            Action::RemovePeerAndClose => {
                self.unpaired = true;
                // The same as removing it on the PC, except that there is no `unpair` to send back: the phone is the one that asked.
                match self.core.forget_device(&self.device_id, false).await {
                    Ok(_) => log::info!("device {} unpaired itself", short(&self.device_id)),
                    Err(e) => log::error!("removing the unpaired device failed: {e}"),
                }
                self.core.emit(Event::PairingChanged);
            }
            // A phone notification (changelog 56): shown, replaced, removed or synced by the mirror. The only thing that can come back is a `rate_limited`
            // error (no `ref`), at most one per 10 seconds.
            Action::Notif(msg) => {
                if let Some(frame) = self.core.mirror_message(&self.device_id, msg).await
                    && let Err(e) = self.send(&frame).await
                {
                    return Some(End::Error(e));
                }
            }
            Action::NotifDropped(NotifDrop::Capability) => self.core.mirror_dropped_by_receiver(),
            Action::NotifDropped(NotifDrop::BadIcon) => self.core.mirror_icon_dropped(),
            // What plays on the phone, and what the phone asks of the PC's media (changelog 57). The only thing that can come back is an `error`: `disabled`,
            // `rate_limited`, `not_found`, `stale` or `action_failed` for a `media.cmd` (with its `ref`), or a `rate_limited` for too many states (no `ref`).
            Action::Media(inbound) => {
                if let Some(frame) = self.core.media_message(&self.device_id, inbound).await
                    && let Err(e) = self.send(&frame).await
                {
                    return Some(End::Error(e));
                }
            }
            Action::MediaDropped => self.core.media_dropped_by_receiver(),
            // The phone's battery and pause (changelog 58), or what its ring says. The only thing that can come back is a `rate_limited` error (no `ref`), at most
            // one per 10 seconds.
            Action::Presence(msg) => {
                if let Some(frame) = self.core.presence_message(&self.device_id, msg)
                    && let Err(e) = self.send(&frame).await
                {
                    return Some(End::Error(e));
                }
            }
            Action::PresenceDropped => self.core.presence_dropped_by_receiver(),
            // `ring.start` and `ring.stop` go PC to phone only: the PC answers `unsupported_type` before it gets here.
            Action::RingCommand { .. } => {}
            // An `alert` goes PC to phone only too (changelog 59): the PC answers `unsupported_type` before it gets here.
            Action::DeliverAlert { .. } => {}
            // A `probe`: the `ack` that went out is all of it.
            Action::AnswerProbe => {}
            // `clipboard` and `offer` only go PC to phone: the receiver answers `unsupported_type` before it gets here.
            Action::None
            | Action::ShowErrorOnly
            | Action::IgnoreDuplicate
            | Action::RepeatedOffer { .. }
            | Action::CloseOnly
            | Action::SetClipboard { .. }
            | Action::StartDownload { .. } => {}
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
        // The latest accepted `hello` decides what may be sent to the phone (and the PC's own list is what it said last).
        self.core.connections.set_capabilities(&self.device_id, self.conn_id, &peer.capabilities);
        if self.hello_accepted {
            // Media (changelog 57): after each accepted `hello` the peer is told this side's `media.status` again.
            self.core.media_peer_hello(&self.device_id, false);
            // Presence (changelog 58): a later `hello` may add the capability (an update): a connection that had none starts with a snapshot then.
            self.core.presence_peer_hello(&self.device_id, false);
            return;
        }
        self.hello_accepted = true;
        let _ = self.core.db.touch(&self.device_id, unix_ms() as i64, Some(&self.remote.to_string()));
        self.core.connections.mark_open(&self.device_id, self.conn_id);
        // Media (changelog 57): a new connection starts with nothing known about the peer, and tells it this side's `media.status`.
        self.core.media_peer_hello(&self.device_id, true);
        // Presence (changelog 58): a new connection starts with nothing known about the peer, and sends it this PC's snapshot (battery, pause).
        self.core.presence_peer_hello(&self.device_id, true);
        log::info!(
            "phone connected: {:?} ({}, app {}, capabilities {:?}) device {} from {}",
            peer.name,
            peer.platform,
            peer.app_version,
            peer.capabilities.iter().take(16).map(|c| crate::encoding::truncate_chars(c, 32)).collect::<Vec<_>>(),
            short(&self.device_id),
            self.remote
        );
        self.core.log_connection("connected", &format!("{:?} from {}", peer.name, self.remote));
        self.core.emit(Event::StateChanged);
        // The queue goes out now that both `hello`s are accepted (protocol.md, "Queue").
        self.core.phone_connected_flush(&self.device_id);
    }

    async fn on_command(&mut self, cmd: ConnCommand) -> Option<End> {
        match cmd {
            ConnCommand::Close { code, reason } => Some(End::Close { code, reason: reason.into() }),
            ConnCommand::Ping(reply) => {
                let id = uuid_v7(unix_ms());
                let frame = json!({ "type": "ping", "id": id, "sentAt": unix_ms() });
                self.send_tracked(id, frame, reply).await
            }
            ConnCommand::Send { frame, reply } => {
                let frame = wire_form(frame, self.receiver.peer());
                match frame.get("id").and_then(|v| v.as_str()).map(str::to_string) {
                    Some(id) => self.send_tracked(id, frame, reply).await,
                    None => {
                        let _ = reply.send(SendOutcome::Failed("internal".into()));
                        None
                    }
                }
            }
            // The device name was changed in the settings: the phone learns it from a later `hello` (protocol.md, "Opening handshake":
            // a `hello` after the first one is handled the same way and is not acked).
            ConnCommand::Hello => {
                // What the PC lists may have changed ("Show phone notifications"): the receiver's own list and the `hello` come from the same place.
                self.receiver.set_own_capabilities(&self.core.capabilities());
                match self.send(&hello_frame(&self.core)).await {
                    Ok(()) => None,
                    Err(e) => Some(End::Error(e)),
                }
            }
            ConnCommand::Notif { frame, reply } => self.send_notif(frame, reply).await,
            ConnCommand::Media { frame } => self.send_media(frame).await,
            ConnCommand::Presence { frame } => self.send_presence(frame).await,
            ConnCommand::Ring { frame, reply } => self.send_capable(crate::presence::wire::RING_CAPABILITY, frame, reply).await,
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

    /// A `notif.*` message to the phone. Sent only if the phone's latest `hello` lists `notif` (else dropped). A `notif.action` (it has a `reply`) waits for its
    /// `ack` for 5 seconds and is never sent again.
    async fn send_notif(&mut self, frame: serde_json::Value, reply: Option<oneshot::Sender<SendOutcome>>) -> Option<End> {
        self.send_capable(crate::mirror::wire::CAPABILITY, frame, reply).await
    }

    /// A `presence` to the phone (changelog 58). Sent only if the phone's latest `hello` lists `presence` (else dropped, silently). Ephemeral: no ack, no queue,
    /// no re-send. Only the type is logged, and only as a count by the presence module (never a level or a pause).
    async fn send_presence(&mut self, frame: serde_json::Value) -> Option<End> {
        if !(self.receiver.is_open() && self.receiver.peer().is_some_and(|p| p.has(crate::presence::wire::CAPABILITY))) {
            return None;
        }
        match self.send(&frame).await {
            Ok(()) => None,
            Err(e) => Some(End::Error(e)),
        }
    }

    /// A message that goes only to a peer whose latest `hello` lists `capability`: a `notif.*` (`notif`) or a `ring.start`/`ring.stop` (`ring`). With a `reply` it is
    /// acked, the `ack` is waited for 5 seconds and **not** sent again, and no answer does not close the connection.
    async fn send_capable(
        &mut self,
        capability: &str,
        frame: serde_json::Value,
        reply: Option<oneshot::Sender<SendOutcome>>,
    ) -> Option<End> {
        let listed = self.receiver.is_open() && self.receiver.peer().is_some_and(|p| p.has(capability));
        if !listed {
            if let Some(reply) = reply {
                let _ = reply.send(SendOutcome::NotConnected);
            }
            return None;
        }
        let sent = Instant::now();
        if let Err(e) = self.send(&frame).await {
            if let Some(reply) = reply {
                let _ = reply.send(SendOutcome::NotConnected);
            }
            return Some(End::Error(e));
        }
        // The type only: nothing of the notification is logged.
        log::info!("{} sent to device {}", frame["type"].as_str().unwrap_or("message"), short(&self.device_id));
        if let (Some(reply), Some(id)) = (reply, frame.get("id").and_then(|v| v.as_str())) {
            self.actions.insert(id.to_string(), Pending { sent, reply });
        }
        None
    }

    /// A `media.*` message to the phone. Sent only if the phone's latest `hello` lists `media` (else dropped). Ephemeral: no ack, no queue, no re-send. The type is
    /// all that is logged, and only as a count (nothing of what plays is ever written down).
    async fn send_media(&mut self, frame: serde_json::Value) -> Option<End> {
        if !(self.receiver.is_open() && self.receiver.peer().is_some_and(|p| p.has(crate::media::wire::CAPABILITY))) {
            return None;
        }
        match self.send(&frame).await {
            Ok(()) => None,
            Err(e) => Some(End::Error(e)),
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
            End::Continue => ("the connection went on".to_string(), None),
        };
        for (_, p) in self.pending.drain().chain(self.actions.drain()) {
            let _ = p.reply.send(SendOutcome::Lost);
        }
        let current = self.core.connections.remove(&self.device_id, self.conn_id);
        // What the phone's media showed on this PC goes with the connection (a newer connection of the same phone keeps what it has).
        if current {
            self.core.media_connection_closed(&self.device_id);
            // What the phone said about its pause goes (nothing is paused as far as this side knows); its last battery stays for "last seen".
            self.core.presence_connection_closed(&self.device_id);
        }
        if self.hello_accepted {
            let _ = self.core.db.touch(&self.device_id, unix_ms() as i64, None);
            self.core.log_connection("disconnected", &format!("after {} s: {why}", self.opened.elapsed().as_secs()));
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
