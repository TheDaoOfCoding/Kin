//! The receiving side of the WebSocket control channel as a pure state machine (protocol.md, "Control channel").
//!
//! It takes one frame and says what to reply, what the app should do and whether to close the socket. It never
//! touches a socket or the UI, so the vectors in `protocol/messages.json` drive it directly.
use serde_json::{Map, Value, json};

use crate::json::{self, is_url_safe_id, non_empty_str, uint, valid_id};

pub const PROTOCOL: u64 = 1;
pub const MAX_FRAME_BYTES: usize = 1_048_576;
pub const HELLO_TIMEOUT_MS: u64 = 10_000;
/// Longest peer name kept for display.
pub const MAX_NAME_CHARS: usize = 64;

pub const CLOSE_NORMAL: u16 = 1000;
pub const CLOSE_PROTOCOL_ERROR: u16 = 1002;
pub const CLOSE_UNSUPPORTED_DATA: u16 = 1003;
pub const CLOSE_INVALID_PAYLOAD: u16 = 1007;
pub const CLOSE_TOO_BIG: u16 = 1009;
pub const CLOSE_REPLACED: u16 = 4001;
pub const CLOSE_UNSUPPORTED_VERSION: u16 = 4002;
pub const CLOSE_UNAUTHORIZED: u16 = 4003;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Pc,
    Phone,
}

/// A frame as it came off the wire. Text frames are raw bytes, because the UTF-8 check is part of the rules.
pub enum Frame<'a> {
    Text(&'a [u8]),
    Binary,
}

/// What the app does about a message, in the categories of `messages.json`.
#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    HandshakeOk,
    MarkDelivered {
        reference: String,
    },
    MarkFailed {
        reference: String,
        code: String,
    },
    ShowErrorOnly,
    ShowPingToast,
    /// A `probe` (changelog 60): the phone checks that the connection is alive. Acked at once (the reply), nothing else: nothing shown, nothing stored, the
    /// `id` is not remembered.
    AnswerProbe,
    /// A `text` message: put it on the clipboard and show it. (History comes in 1.4.) `sensitive`: no preview, never stored or logged.
    DeliverText {
        id: String,
        text: String,
        is_url: bool,
        sensitive: bool,
    },
    /// A `link` message (changelog 54): store it in the history and open it, or offer to. `url` is valid (checked as sent), `title` and
    /// `source_app` are already cleaned (`links::clean_link_text`). It never goes to the clipboard.
    DeliverLink {
        id: String,
        url: String,
        title: Option<String>,
        source_app: Option<String>,
    },
    /// An `offer` (PC to phone only): download the files. The PC never receives one (it answers `unsupported_type`), but the
    /// receiver implements both roles so the vectors can run on it.
    StartDownload {
        id: String,
        files: Vec<OfferedFile>,
    },
    /// A `complete` message (phone to PC): the phone has all the files of this offer.
    MarkTransferDone {
        transfer_id: String,
    },
    /// A `cancel` message, from either side.
    CancelTransfer {
        transfer_id: String,
        reason: String,
    },
    /// A `clipboard` message (PC to phone only): set the clipboard silently, no notification, no history. The PC never
    /// receives one (it answers `unsupported_type`), but the receiver implements both roles so the vectors can run on it.
    SetClipboard {
        id: String,
        text: String,
    },
    RemovePeerAndClose,
    IgnoreDuplicate,
    /// A repeated `offer` from the same PC (changelog 52): acked with the first response, nothing is downloaded again. The app carries on with
    /// an unfinished download, re-sends the `complete` of a finished one that was not acked, or the `cancel` of a canceled or failed one.
    /// (`ignore_duplicate` in the vectors.)
    RepeatedOffer {
        id: String,
        progress: Progress,
    },
    CloseOnly,
    /// A `notif.*` message (changelog 56) that passed validation. Only `notif.action` was acked (and remembered); the other six are ephemeral.
    Notif(crate::mirror::wire::NotifMsg),
    /// A `notif.*` message dropped silently (no reply, no error): the receiver's own `hello` or the sender's didn't list `notif`, or it was an icon that
    /// failed its checks. Counted.
    NotifDropped(NotifDrop),
    /// A `media.*` message (changelog 57) that passed validation. None is acked or remembered. The rules about who shows what (a viewer whose `show` is off, a source
    /// whose `share` is off) are the app's (`Core::media_message`).
    Media(crate::media::wire::Inbound),
    /// A `media.*` message dropped silently (no reply, no error): the receiver's own `hello` or the sender's didn't list `media`, or it was an image that failed its
    /// checks. Counted.
    MediaDropped,
    /// A `presence` or a `ring.status` (changelog 58) that passed validation. Neither is acked or remembered.
    Presence(crate::presence::wire::PresenceMsg),
    /// A `presence` or `ring.status` dropped silently (no reply, no error): the receiver's own `hello` or the sender's didn't list `presence`. Counted.
    PresenceDropped,
    /// A `ring.start` or `ring.stop` (PC to phone only, so only the vectors reach it on the PC's receiver): acked, and never queued or re-sent. `start`: whether
    /// it is a start (else a stop).
    RingCommand {
        id: String,
        start: bool,
    },
    /// An `alert` (changelog 59, PC to phone only, so only the vectors reach it on the PC's receiver): acked, de-duplicated. The phone shows a notification and stores a
    /// history row; it never touches the clipboard.
    DeliverAlert {
        id: String,
        alert: crate::alert::wire::AlertMsg,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifDrop {
    /// Step 6b: one of the two sides didn't list `notif`.
    Capability,
    /// A `notif.icon` whose hash, size or picture was wrong.
    BadIcon,
}

impl Action {
    /// The name used by `expect.action` in the vectors.
    pub fn vector_name(&self) -> Option<&'static str> {
        Some(match self {
            Action::None => return None,
            Action::HandshakeOk => "handshake_ok",
            Action::MarkDelivered { .. } => "mark_delivered",
            Action::MarkFailed { .. } => "mark_failed",
            Action::ShowErrorOnly => "show_error_only",
            Action::ShowPingToast => "show_ping_toast",
            Action::AnswerProbe => "answer_probe",
            Action::DeliverText { .. } => "deliver_text",
            Action::DeliverLink { .. } => "deliver_link",
            Action::SetClipboard { .. } => "set_clipboard",
            Action::StartDownload { .. } => "start_download",
            Action::MarkTransferDone { .. } => "mark_transfer_done",
            Action::CancelTransfer { .. } => "cancel_transfer",
            Action::RemovePeerAndClose => "remove_peer_and_close",
            Action::IgnoreDuplicate | Action::RepeatedOffer { .. } => "ignore_duplicate",
            Action::CloseOnly => "close_only",
            Action::Notif(m) => crate::mirror::wire::vector_name(m),
            Action::NotifDropped(_) => "notif_dropped",
            Action::Media(m) => crate::media::wire::vector_name(&m.msg),
            Action::MediaDropped => "media_dropped",
            Action::Presence(crate::presence::wire::PresenceMsg::Presence(_)) => "presence",
            Action::Presence(crate::presence::wire::PresenceMsg::RingStatus(_)) => "ring_status",
            Action::PresenceDropped => "presence_dropped",
            Action::RingCommand { start: true, .. } => "ring_start",
            Action::RingCommand { start: false, .. } => "ring_stop",
            Action::DeliverAlert { .. } => "deliver_alert",
        })
    }
}

/// A file of an `offer`, as offered. The name is raw: the receiver cleans it when it saves.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OfferedFile {
    pub file_id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

#[derive(Debug)]
pub struct Outcome {
    /// At most one message goes back: an `ack` or an `error`.
    pub reply: Option<Value>,
    pub action: Action,
    /// Close the WebSocket with this code after sending the reply.
    pub close: Option<u16>,
}

impl Outcome {
    fn close_only(code: u16) -> Self {
        Outcome { reply: None, action: Action::CloseOnly, close: Some(code) }
    }

    fn error(code: &str, message: &str, reference: Option<&str>, close: Option<u16>) -> Self {
        Outcome { reply: Some(error_message(code, message, reference)), action: Action::None, close }
    }
}

/// An error `code` the phone sent, as a word that is safe to log and to show: only the codes of the protocol's table (docs/protocol.md "Error codes"), anything else (text
/// from the phone, which could hold anything) is `other`. The same idea as `presence::reason_word` for a `ring.status` reason.
pub fn error_code_word(code: &str) -> &'static str {
    const KNOWN: [&str; 14] = [
        "bad_request",
        "unauthorized",
        "unsupported_version",
        "not_found",
        "offset_mismatch",
        "source_changed",
        "too_large",
        "disk_full",
        "unsupported_type",
        "stale",
        "action_failed",
        "disabled",
        "rate_limited",
        "internal",
    ];
    KNOWN.iter().find(|k| **k == code).copied().unwrap_or("other")
}

pub fn ack_message(reference: &str) -> Value {
    json!({ "type": "ack", "ref": reference })
}

pub fn error_message(code: &str, message: &str, reference: Option<&str>) -> Value {
    let mut v = json!({ "type": "error", "code": code, "message": message });
    if let Some(r) = reference {
        v["ref"] = Value::String(r.to_string());
    }
    v
}

/// The IDs of processed messages and the first response to each, so a repeat gets the same answer again.
pub trait Dedup {
    fn lookup(&mut self, id: &str) -> Option<String>;
    fn remember(&mut self, id: &str, response: &str);
    /// What the receiver knows of an `offer`'s ID, for the one type with a rule of its own for a repeat (protocol.md, changelog 52). `files`
    /// are the files of the offer that just arrived. Only a phone receives offers, so the PC's database memory keeps the default (it never
    /// sees a repeated offer).
    fn offer_state(&mut self, _id: &str, _files: &[OfferedFile]) -> OfferState {
        OfferState::NotAnOffer
    }
    /// Called when an `offer` is processed for the first time: its ID is now an offer of this PC, with these files, not over.
    fn remember_offer(&mut self, _id: &str, _files: &[OfferedFile]) {}
}

/// How the download of an offer stands (what the receiving app has done with it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Not over: running, or waiting to resume.
    Unfinished,
    Done,
    Canceled,
    Failed,
}

/// What the receiver knows about the ID of an arriving `offer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferState {
    /// No offer has this ID. (If the ID is remembered at all it was another message, a `text` say: that is a reuse, `bad_request`.)
    NotAnOffer,
    /// The ID is an offer of another paired PC: a reuse, `bad_request`.
    OtherDevice,
    /// The ID is an offer this same paired PC made before: a **repeat**, whatever the state of its download. The phone answers with the first
    /// response and downloads nothing again; the state says what it re-sends (a `complete` that was not acked, the `cancel`). `same_files`:
    /// the offer lists the same `fileId`s with the same sizes as the first one; if not it is `bad_request`.
    Own { progress: Progress, same_files: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub name: String,
    pub platform: String,
    pub app_version: String,
    /// The `capabilities` of its latest accepted `hello` (protocol.md, "Versioning"): the strings of the array, empty when the value is absent,
    /// not an array, or has no strings.
    pub capabilities: Vec<String>,
}

impl Peer {
    /// Whether the peer's latest `hello` listed this capability (case-sensitive).
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// What this PC says it can do in its own `hello`: `link`, `media` always (changelog 57: the switches of media travel in `media.status`, not in a new `hello`)
/// and `presence` always (changelog 58: the switches travel as what a snapshot contains). The PC lists nothing for `ring`. `notif` is added by the core while
/// "Show phone notifications" is on.
pub const CAPABILITIES: &[&str] = &["link", "media", "presence"];

/// The capabilities of a `hello`: an array of strings, anything else counts as none and entries that are not strings are skipped. A bad
/// value never fails a `hello`.
pub fn capabilities_of(obj: &Map<String, Value>) -> Vec<String> {
    match obj.get("capabilities") {
        Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    BeforeHello,
    Open,
}

pub struct Receiver {
    role: Role,
    state: State,
    peer: Option<Peer>,
    /// The capabilities of this side's own latest `hello` (what it said it can do), for step 6b of the validation order.
    own_capabilities: Vec<String>,
    /// The phone's ring budget is spent (see `set_ring_budget_spent`).
    ring_budget_spent: bool,
    /// The phone's alert bucket is empty (see `set_alert_budget_spent`).
    alert_budget_spent: bool,
}

/// Message types and which role may receive them. `complete` only goes phone -> PC, `clipboard` and `offer` only
/// PC -> phone.
fn receivable(role: Role, ty: &str) -> bool {
    // A `notif.*` message is received by the side it goes to: the phone sends five of them to the PC, the PC two to the phone.
    if let Some((phone_sends, _)) = crate::mirror::wire::kind(ty) {
        return phone_sends == (role == Role::Pc);
    }
    // A `media.*` message is received by both roles: each device is the source of one direction and the viewer of the other (changelog 57).
    if crate::media::wire::is_media(ty) {
        return true;
    }
    // `presence` goes both ways (changelog 58); `ring.start` and `ring.stop` are received by the phone, `ring.status` by the PC.
    match ty {
        "presence" => return true,
        "ring.start" | "ring.stop" => return role == Role::Phone,
        "ring.status" => return role == Role::Pc,
        // An `alert` goes PC to phone only (changelog 59).
        "alert" => return role == Role::Phone,
        _ => {}
    }
    match ty {
        "hello" | "ack" | "error" | "text" | "link" | "cancel" | "unpair" | "ping" | "probe" => true,
        "complete" => role == Role::Pc,
        "clipboard" | "offer" => role == Role::Phone,
        _ => false,
    }
}

impl Receiver {
    pub fn new(role: Role) -> Self {
        Receiver {
            role,
            state: State::BeforeHello,
            peer: None,
            own_capabilities: Vec::new(),
            ring_budget_spent: false,
            alert_budget_spent: false,
        }
    }

    /// What this side lists in its own `hello` (changelog 56: a `notif.*` message is dropped unless both sides list `notif`). The PC sets it from the
    /// setting every time it sends a `hello`.
    pub fn set_own_capabilities(&mut self, capabilities: &[&str]) {
        self.own_capabilities = capabilities.iter().map(|c| c.to_string()).collect();
    }

    pub fn is_open(&self) -> bool {
        self.state == State::Open
    }

    pub fn peer(&self) -> Option<&Peer> {
        self.peer.as_ref()
    }

    /// `Some(1002)` once a socket has had no accepted `hello` for 10 seconds. No error message goes first.
    pub fn hello_timeout(&self, elapsed_since_open_ms: u64) -> Option<u16> {
        (self.state == State::BeforeHello && elapsed_since_open_ms >= HELLO_TIMEOUT_MS).then_some(CLOSE_PROTOCOL_ERROR)
    }

    /// Validation step 1: size, UTF-8 and text frame. These close the socket without a reply.
    pub fn handle_frame(&mut self, frame: Frame<'_>, dedup: &mut dyn Dedup) -> Outcome {
        match frame {
            Frame::Binary => Outcome::close_only(CLOSE_UNSUPPORTED_DATA),
            Frame::Text(bytes) if bytes.len() > MAX_FRAME_BYTES => Outcome::close_only(CLOSE_TOO_BIG),
            Frame::Text(bytes) => match std::str::from_utf8(bytes) {
                Ok(text) => self.handle_text(text, dedup),
                Err(_) => Outcome::close_only(CLOSE_INVALID_PAYLOAD),
            },
        }
    }

    /// Validation steps 2 to 8 for one text frame.
    pub fn handle_text(&mut self, text: &str, dedup: &mut dyn Dedup) -> Outcome {
        let Some(obj) = json::parse_object(text) else {
            return Outcome::error("bad_request", "The message is not a valid JSON object.", None, None);
        };
        let reference = valid_id(obj.get("id"));
        let Some(ty) = obj.get("type").and_then(Value::as_str) else {
            return Outcome::error("bad_request", "The message has no type.", reference, None);
        };
        if self.state == State::BeforeHello && ty != "hello" {
            return Outcome::error("bad_request", "Expected hello first.", reference, Some(CLOSE_PROTOCOL_ERROR));
        }
        if !receivable(self.role, ty) {
            return Outcome::error("unsupported_type", "Unknown message type.", reference, None);
        }
        if ty == "hello"
            && let Some(p) = obj.get("protocol").and_then(uint)
            && p != PROTOCOL
        {
            return Outcome::error(
                "unsupported_version",
                &format!("Protocol version {p} is not supported. Update the app."),
                reference,
                Some(CLOSE_UNSUPPORTED_VERSION),
            );
        }
        if crate::mirror::wire::kind(ty).is_some() {
            return self.notif(ty, &obj, text.len(), reference, dedup);
        }
        if crate::media::wire::is_media(ty) {
            return self.media(ty, &obj, text.len(), reference);
        }
        if crate::presence::wire::is_presence_type(ty) {
            return self.presence(ty, &obj, text.len(), reference);
        }
        if crate::presence::wire::is_ring_command(ty) {
            return self.ring_command(ty, text.len(), reference, dedup);
        }
        if ty == "alert" {
            return self.alert(&obj, text.len(), reference, dedup);
        }
        match ty {
            "hello" => self.hello(&obj, reference),
            "ack" => ack(&obj),
            "error" => error(&obj),
            "probe" => probe(reference),
            "ping" | "unpair" | "text" | "link" | "clipboard" | "offer" | "complete" | "cancel" => self.acked(ty, &obj, reference, dedup),
            _ => unreachable!("receivable() only lets known types through"),
        }
    }

    /// A `notif.*` message: step 6b (dropped silently unless both sides list `notif`), the 32 KiB rule, then the fields (`mirror::wire`). The six ephemeral
    /// types skip the duplicate check; `notif.action` is acked and remembered like any other acked message.
    fn notif(&mut self, ty: &str, obj: &Map<String, Value>, frame_bytes: usize, reference: Option<&str>, dedup: &mut dyn Dedup) -> Outcome {
        use crate::mirror::wire;
        let capable =
            self.own_capabilities.iter().any(|c| c == wire::CAPABILITY) && self.peer.as_ref().is_some_and(|p| p.has(wire::CAPABILITY));
        if !capable {
            return Outcome { reply: None, action: Action::NotifDropped(NotifDrop::Capability), close: None };
        }
        if frame_bytes > wire::frame_limit(ty) {
            let message = format!("A {ty} message can be at most {} KiB.", wire::frame_limit(ty) / 1024);
            return Outcome::error("too_large", &message, reference, None);
        }
        let msg = match wire::parse(ty, obj) {
            wire::Parsed::Bad => return bad_fields(reference),
            wire::Parsed::Drop => return Outcome { reply: None, action: Action::NotifDropped(NotifDrop::BadIcon), close: None },
            wire::Parsed::Ok(msg) => msg,
        };
        let acked = wire::kind(ty).is_some_and(|(_, acked)| acked);
        let Some(id) = reference.filter(|_| acked) else {
            return Outcome { reply: None, action: Action::Notif(msg), close: None };
        };
        if let Some(first) = dedup.lookup(id) {
            let reply = serde_json::from_str(&first).unwrap_or_else(|_| ack_message(id));
            return Outcome { reply: Some(reply), action: Action::IgnoreDuplicate, close: None };
        }
        let reply = ack_message(id);
        dedup.remember(id, &reply.to_string());
        Outcome { reply: Some(reply), action: Action::Notif(msg), close: None }
    }

    /// A `media.*` message (validation step 6c): dropped silently unless both sides list `media`, then the 32 KiB rule (48 KiB for an image), then the fields
    /// (`media::wire`). None is acked, remembered or de-duplicated: a repeated `id` is processed again.
    fn media(&mut self, ty: &str, obj: &Map<String, Value>, frame_bytes: usize, reference: Option<&str>) -> Outcome {
        use crate::media::wire;
        let capable =
            self.own_capabilities.iter().any(|c| c == wire::CAPABILITY) && self.peer.as_ref().is_some_and(|p| p.has(wire::CAPABILITY));
        if !capable {
            return Outcome { reply: None, action: Action::MediaDropped, close: None };
        }
        if frame_bytes > wire::frame_limit(ty) {
            let message = format!("A {ty} message can be at most {} KiB.", wire::frame_limit(ty) / 1024);
            return Outcome::error("too_large", &message, reference, None);
        }
        match wire::parse(ty, obj) {
            wire::Parsed::Bad => bad_fields(reference),
            wire::Parsed::Drop => Outcome { reply: None, action: Action::MediaDropped, close: None },
            wire::Parsed::Ok(msg) => {
                let id = reference.unwrap_or_default().to_string();
                Outcome { reply: None, action: Action::Media(wire::Inbound { id, msg }), close: None }
            }
        }
    }

    /// A `presence` or `ring.status` (validation step 6d, changelog 58): dropped silently unless both sides list `presence`, then the 4 KiB rule (2 KiB for
    /// `ring.status`), then the fields. Neither is acked, remembered or de-duplicated: a repeated `id` is processed again.
    fn presence(&mut self, ty: &str, obj: &Map<String, Value>, frame_bytes: usize, reference: Option<&str>) -> Outcome {
        use crate::presence::wire;
        let capable =
            self.own_capabilities.iter().any(|c| c == wire::CAPABILITY) && self.peer.as_ref().is_some_and(|p| p.has(wire::CAPABILITY));
        if !capable {
            return Outcome { reply: None, action: Action::PresenceDropped, close: None };
        }
        if frame_bytes > wire::frame_limit(ty) {
            let message = format!("A {ty} message can be at most {} KiB.", wire::frame_limit(ty) / 1024);
            return Outcome::error("too_large", &message, reference, None);
        }
        match wire::parse(ty, obj) {
            wire::Parsed::Bad => bad_fields(reference),
            wire::Parsed::Ok(msg) => Outcome { reply: None, action: Action::Presence(msg), close: None },
        }
    }

    /// A `ring.start` or `ring.stop` (the phone's side of the rules, kept here so the vectors can run on it): never dropped, because it is acked. The 2 KiB rule, the
    /// `id`, the duplicate check (a repeat gets the first answer and rings nothing again), then for a start: `disabled` if this side's own `hello` doesn't list
    /// `ring`, `rate_limited` if its budget is spent. An error answer is not remembered. A stop is always acked, also with the switch off.
    fn ring_command(&mut self, ty: &str, frame_bytes: usize, reference: Option<&str>, dedup: &mut dyn Dedup) -> Outcome {
        use crate::presence::wire;
        if frame_bytes > wire::MAX_RING_FRAME_BYTES {
            return Outcome::error("too_large", "A ring message can be at most 2 KiB.", reference, None);
        }
        let Some(id) = reference else {
            return bad_fields(None);
        };
        let start = ty == "ring.start";
        if let Some(first) = dedup.lookup(id) {
            let reply = serde_json::from_str(&first).unwrap_or_else(|_| ack_message(id));
            return Outcome { reply: Some(reply), action: Action::IgnoreDuplicate, close: None };
        }
        if start {
            if !self.own_capabilities.iter().any(|c| c == wire::RING_CAPABILITY) {
                return Outcome::error("disabled", "Ringing is turned off on this phone.", reference, None);
            }
            if self.ring_budget_spent {
                return Outcome::error("rate_limited", "Too many ring requests.", reference, None);
            }
        }
        let reply = ack_message(id);
        dedup.remember(id, &reply.to_string());
        Outcome { reply: Some(reply), action: Action::RingCommand { id: id.to_string(), start }, close: None }
    }

    /// For the vectors: the phone's ring budget is spent (3 starts in the last 60 s, or inside the lock-out). The app (Android) keeps the real budget.
    pub fn set_ring_budget_spent(&mut self, spent: bool) {
        self.ring_budget_spent = spent;
    }

    /// For the vectors: the phone's alert bucket is empty (10 per 60 s). The app (Android) keeps the real bucket.
    pub fn set_alert_budget_spent(&mut self, spent: bool) {
        self.alert_budget_spent = spent;
    }

    /// An `alert` (changelog 59, the phone's side of the rules, kept here so the vectors can run on it): acked, so never dropped. The 4 KiB rule, the fields, the
    /// duplicate check (a repeat gets the first answer), then the bucket: `rate_limited` with `ref`, and the `id` is not remembered.
    fn alert(&mut self, obj: &Map<String, Value>, frame_bytes: usize, reference: Option<&str>, dedup: &mut dyn Dedup) -> Outcome {
        use crate::alert::wire;
        if frame_bytes > wire::MAX_FRAME_BYTES {
            return Outcome::error("too_large", "An alert can be at most 4 KiB.", reference, None);
        }
        let (Some(id), Some(alert)) = (reference, wire::parse(obj)) else {
            return bad_fields(reference);
        };
        if let Some(first) = dedup.lookup(id) {
            let reply = serde_json::from_str(&first).unwrap_or_else(|_| ack_message(id));
            return Outcome { reply: Some(reply), action: Action::IgnoreDuplicate, close: None };
        }
        if self.alert_budget_spent {
            return Outcome::error("rate_limited", "Too many alerts.", reference, None);
        }
        let reply = ack_message(id);
        dedup.remember(id, &reply.to_string());
        Outcome { reply: Some(reply), action: Action::DeliverAlert { id: id.to_string(), alert }, close: None }
    }

    fn hello(&mut self, obj: &Map<String, Value>, reference: Option<&str>) -> Outcome {
        let fields = (|| {
            valid_id(obj.get("id"))?;
            uint(obj.get("protocol")?)?;
            let name = non_empty_str(obj, "name")?;
            let platform = non_empty_str(obj, "platform")?;
            let app_version = obj.get("appVersion")?.as_str()?;
            Some(Peer {
                name: crate::encoding::truncate_chars(name, MAX_NAME_CHARS).to_string(),
                platform: platform.to_string(),
                app_version: app_version.to_string(),
                capabilities: capabilities_of(obj),
            })
        })();
        match fields {
            Some(peer) => {
                self.peer = Some(peer);
                self.state = State::Open;
                Outcome { reply: None, action: Action::HandshakeOk, close: None }
            }
            None => bad_fields(reference),
        }
    }

    /// `ping`, `unpair`, `text`, `link` and `clipboard`: an `id`, an `ack`, and a repeat of a known ID gets the first answer again.
    fn acked(&mut self, ty: &str, obj: &Map<String, Value>, reference: Option<&str>, dedup: &mut dyn Dedup) -> Outcome {
        let Some(id) = reference else {
            return bad_fields(None);
        };
        let new_action = match ty {
            "ping" if obj.get("sentAt").and_then(uint).is_some() => Action::ShowPingToast,
            "unpair" => Action::RemovePeerAndClose,
            "text" => {
                // `sensitive` is optional: absent and `null` are false, anything but a boolean is invalid.
                let sensitive = match obj.get("sensitive") {
                    None | Some(Value::Null) => Some(false),
                    Some(Value::Bool(b)) => Some(*b),
                    Some(_) => None,
                };
                match (non_empty_str(obj, "text"), obj.get("isUrl").and_then(Value::as_bool), sensitive) {
                    (Some(text), Some(is_url), Some(sensitive)) => {
                        Action::DeliverText { id: id.to_string(), text: text.to_string(), is_url, sensitive }
                    }
                    _ => return bad_fields(reference),
                }
            }
            "link" => match parse_link(obj) {
                Some((url, title, source_app)) => Action::DeliverLink { id: id.to_string(), url, title, source_app },
                None => return bad_fields(reference),
            },
            "offer" => match parse_offer(obj, id) {
                // More files than an offer may hold: rejected like any other offer the receiver can't take (`error`, no ack, nothing
                // remembered). The PC splits a bigger send into several offers, so a phone never sees one.
                Some(files) if files.len() > MAX_OFFER_FILES => {
                    return Outcome::error("too_large", "An offer can have at most 800 files.", reference, None);
                }
                Some(files) => Action::StartDownload { id: id.to_string(), files },
                None => return bad_fields(reference),
            },
            "complete" => match transfer_id_of(obj) {
                Some(transfer_id) => Action::MarkTransferDone { transfer_id },
                None => return bad_fields(reference),
            },
            "cancel" => match (transfer_id_of(obj), obj.get("reason").and_then(Value::as_str)) {
                (Some(transfer_id), Some(reason)) => Action::CancelTransfer { transfer_id, reason: reason.to_string() },
                _ => return bad_fields(reference),
            },
            "clipboard" => match non_empty_str(obj, "text") {
                Some(text) => Action::SetClipboard { id: id.to_string(), text: text.to_string() },
                None => return bad_fields(reference),
            },
            _ => return bad_fields(reference),
        };
        let is_unpair = ty == "unpair";
        let first = dedup.lookup(id);
        if let Action::StartDownload { files, .. } = &new_action {
            // An offer whose ID is known (changelog 52). **The same paired PC repeating its own offer is a repeat, whatever the state of the
            // download:** the phone answers with the first response (the stored ack), downloads nothing again, and the app re-sends what the
            // state calls for. **Any other reuse is `bad_request`** with `ref` = the offer's `id`: the ID of a `text`, a `clipboard` or any other
            // message, the ID of another PC's offer, and a repeat that lists other files than the first one.
            match dedup.offer_state(id, files) {
                OfferState::OtherDevice => return Outcome::error("bad_request", "That ID belongs to another device.", reference, None),
                OfferState::Own { same_files: false, .. } => {
                    return Outcome::error("bad_request", "That ID was used for other files.", reference, None);
                }
                OfferState::Own { progress, .. } => {
                    let reply = first.and_then(|f| serde_json::from_str(&f).ok()).unwrap_or_else(|| ack_message(id));
                    return Outcome { reply: Some(reply), action: Action::RepeatedOffer { id: id.to_string(), progress }, close: None };
                }
                OfferState::NotAnOffer if first.is_some() => {
                    return Outcome::error("bad_request", "That ID was used before.", reference, None);
                }
                OfferState::NotAnOffer => {}
            }
        } else if let Some(first) = first {
            let reply = serde_json::from_str(&first).unwrap_or_else(|_| ack_message(id));
            return Outcome { reply: Some(reply), action: Action::IgnoreDuplicate, close: is_unpair.then_some(CLOSE_NORMAL) };
        }
        let reply = ack_message(id);
        dedup.remember(id, &reply.to_string());
        if let Action::StartDownload { files, .. } = &new_action {
            dedup.remember_offer(id, files);
        }
        Outcome { reply: Some(reply), action: new_action, close: is_unpair.then_some(CLOSE_NORMAL) }
    }
}

/// The most files one `offer` may carry (protocol.md, "Files: PC -> phone"). The message has to stay under 1 MiB: a file takes about 100
/// bytes plus its name, and a name of 255 characters is at most 765 bytes in UTF-8, so 800 files are under 700 KB even with the longest
/// names. A receiver rejects an offer over it with `error` `too_large`.
pub const MAX_OFFER_FILES: usize = 800;

/// The fields of a `link` (protocol.md, "Links: `link`"): `url` is required and valid **as sent** (no trimming); `title` and `sourceApp` are optional
/// strings (`null` is absent, any other type is invalid) and are cleaned and cut here.
fn parse_link(obj: &Map<String, Value>) -> Option<(String, Option<String>, Option<String>)> {
    let url = obj.get("url")?.as_str().filter(|u| crate::links::is_link_url(u))?;
    let text = |key: &str, max: usize| match obj.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(crate::links::clean_link_text(s, max)),
        Some(_) => None,
    };
    let title = text("title", crate::links::MAX_LINK_TITLE_CHARS)?;
    let source_app = text("sourceApp", crate::links::MAX_LINK_SOURCE_CHARS)?;
    Some((url.to_string(), title, source_app))
}

/// `transferId` of `complete` and `cancel`: 1 to 64 characters of `A-Z a-z 0-9 - _`, as it goes in URLs.
fn transfer_id_of(obj: &Map<String, Value>) -> Option<String> {
    let t = obj.get("transferId")?.as_str()?;
    is_url_safe_id(t).then(|| t.to_string())
}

/// The fields of an `offer`: the transfer ID (the message `id`, which also ends up in URLs), `files` with at least one entry, each
/// with a URL-safe unique `fileId`, a `name` (any string, even empty), an integer `size` and a `mime`, and the integer `totalSize`,
/// which is only informational.
fn parse_offer(obj: &Map<String, Value>, id: &str) -> Option<Vec<OfferedFile>> {
    if !is_url_safe_id(id) {
        return None;
    }
    uint(obj.get("totalSize")?)?;
    let files = obj.get("files")?.as_array().filter(|f| !f.is_empty())?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let f = f.as_object()?;
        let file_id = f.get("fileId")?.as_str().filter(|s| is_url_safe_id(s))?;
        if !seen.insert(file_id) {
            return None;
        }
        out.push(OfferedFile {
            file_id: file_id.to_string(),
            name: f.get("name")?.as_str()?.to_string(),
            size: uint(f.get("size")?)?,
            mime: f.get("mime")?.as_str()?.to_string(),
        });
    }
    Some(out)
}

/// `probe` (changelog 60): only an `id`, acked at once. It skips the duplicate check: never looked up, never remembered (no database write).
fn probe(reference: Option<&str>) -> Outcome {
    match reference {
        Some(id) => Outcome { reply: Some(ack_message(id)), action: Action::AnswerProbe, close: None },
        None => bad_fields(None),
    }
}

fn bad_fields(reference: Option<&str>) -> Outcome {
    Outcome::error("bad_request", "A required field is missing or has the wrong type.", reference, None)
}

fn ack(obj: &Map<String, Value>) -> Outcome {
    match valid_id(obj.get("ref")) {
        Some(r) => Outcome { reply: None, action: Action::MarkDelivered { reference: r.to_string() }, close: None },
        None => bad_fields(None),
    }
}

fn error(obj: &Map<String, Value>) -> Outcome {
    let reference = match obj.get("ref") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => return bad_fields(None),
    };
    let (Some(code), Some(_message)) = (obj.get("code").and_then(Value::as_str), obj.get("message").and_then(Value::as_str)) else {
        return bad_fields(None);
    };
    let action = match reference {
        Some(r) => Action::MarkFailed { reference: r.to_string(), code: code.to_string() },
        None => Action::ShowErrorOnly,
    };
    Outcome { reply: None, action, close: None }
}

/// What a client does after its connection ends (`closeCodes[]`). The PC never reconnects, but it logs what the
/// phone is expected to do, which makes a log easier to read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClientAction {
    ReconnectWithBackoff,
    ReconnectOnUserAction,
    StopUpdateApp,
    StopPairAgain,
}

impl ClientAction {
    pub fn vector_name(self) -> &'static str {
        match self {
            ClientAction::ReconnectWithBackoff => "reconnect_with_backoff",
            ClientAction::ReconnectOnUserAction => "reconnect_on_user_action",
            ClientAction::StopUpdateApp => "stop_update_app",
            ClientAction::StopPairAgain => "stop_pair_again",
        }
    }
}

/// `close_code` is `None` for a network error with no close frame. `upgrade_status` is the HTTP status that refused
/// the upgrade.
pub fn client_action(close_code: Option<u16>, upgrade_status: Option<u16>) -> ClientAction {
    if upgrade_status == Some(401) {
        return ClientAction::StopPairAgain;
    }
    match close_code {
        Some(CLOSE_REPLACED) => ClientAction::ReconnectOnUserAction,
        Some(CLOSE_UNSUPPORTED_VERSION) => ClientAction::StopUpdateApp,
        Some(CLOSE_UNAUTHORIZED) => ClientAction::StopPairAgain,
        _ => ClientAction::ReconnectWithBackoff,
    }
}

/// Only a connection that stayed open for 60 s after its `hello` was accepted resets the reconnect backoff.
pub fn resets_backoff(hello_accepted_for_ms: Option<u64>) -> bool {
    hello_accepted_for_ms.is_some_and(|ms| ms >= 60_000)
}
