//! The receiving side of the WebSocket control channel as a pure state machine (protocol.md, "Control channel").
//!
//! It takes one frame and says what to reply, what the app should do and whether to close the socket. It never
//! touches a socket or the UI, so the vectors in `protocol/messages.json` drive it directly.
use serde_json::{Map, Value, json};

use crate::json::{self, non_empty_str, uint, valid_id};

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
    /// A `text` message. Delivering it (clipboard, history) comes in milestone 1.2.
    DeliverText {
        text: String,
        is_url: bool,
    },
    RemovePeerAndClose,
    IgnoreDuplicate,
    CloseOnly,
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
            Action::DeliverText { .. } => "deliver_text",
            Action::RemovePeerAndClose => "remove_peer_and_close",
            Action::IgnoreDuplicate => "ignore_duplicate",
            Action::CloseOnly => "close_only",
        })
    }
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub name: String,
    pub platform: String,
    pub app_version: String,
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
}

/// Message types and which role may receive them. `complete` only goes phone -> PC, `clipboard` and `offer` only
/// PC -> phone.
fn receivable(role: Role, ty: &str) -> bool {
    match ty {
        "hello" | "ack" | "error" | "text" | "cancel" | "unpair" | "ping" => true,
        "complete" => role == Role::Pc,
        "clipboard" | "offer" => role == Role::Phone,
        _ => false,
    }
}

/// Types that exist in v1 but whose handlers come in a later milestone (1.2 for `clipboard`, 1.3 for the rest).
fn handled_in_later_milestone(ty: &str) -> bool {
    matches!(ty, "clipboard" | "offer" | "complete" | "cancel")
}

impl Receiver {
    pub fn new(role: Role) -> Self {
        Receiver { role, state: State::BeforeHello, peer: None }
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
        match ty {
            "hello" => self.hello(&obj, reference),
            "ack" => ack(&obj),
            "error" => error(&obj),
            "ping" | "unpair" | "text" => self.acked(ty, &obj, reference, dedup),
            t if handled_in_later_milestone(t) => {
                Outcome::error("unsupported_type", "This version of the app doesn't handle that yet.", reference, None)
            }
            _ => unreachable!("receivable() only lets known types through"),
        }
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

    /// `ping`, `unpair` and `text`: an `id`, an `ack`, and a repeat of a known ID gets the first answer again.
    fn acked(&mut self, ty: &str, obj: &Map<String, Value>, reference: Option<&str>, dedup: &mut dyn Dedup) -> Outcome {
        let Some(id) = reference else {
            return bad_fields(None);
        };
        let new_action = match ty {
            "ping" if obj.get("sentAt").and_then(uint).is_some() => Action::ShowPingToast,
            "unpair" => Action::RemovePeerAndClose,
            "text" => match (non_empty_str(obj, "text"), obj.get("isUrl").and_then(Value::as_bool)) {
                (Some(text), Some(is_url)) => Action::DeliverText { text: text.to_string(), is_url },
                _ => return bad_fields(reference),
            },
            _ => return bad_fields(reference),
        };
        let is_unpair = ty == "unpair";
        if let Some(first) = dedup.lookup(id) {
            let reply = serde_json::from_str(&first).unwrap_or_else(|_| ack_message(id));
            return Outcome { reply: Some(reply), action: Action::IgnoreDuplicate, close: is_unpair.then_some(CLOSE_NORMAL) };
        }
        let reply = ack_message(id);
        dedup.remember(id, &reply.to_string());
        Outcome { reply: Some(reply), action: new_action, close: is_unpair.then_some(CLOSE_NORMAL) }
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
