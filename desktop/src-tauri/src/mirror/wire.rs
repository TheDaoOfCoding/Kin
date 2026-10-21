//! The wire form of the `notif.*` messages (protocol.md "Notifications", changelog 56): what a frame has to look like, and what the receiver ends up
//! with after the strings are cleaned and cut. Pure: no sockets, no Windows. `protocol/notif.json` and the `notif-*` cases of `messages.json` run
//! against it (`tests/vectors.rs`).
//!
//! Nothing here keeps anything: a notification is shown or dropped, never stored.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};

use crate::encoding::sha256;
use crate::json::{uint, valid_id};
use crate::links::{clean_link_text, trim};

/// The capability of `hello.capabilities` that turns the `notif.*` messages on.
pub const CAPABILITY: &str = "notif";

/// A `notif.*` frame over this many bytes is `error too_large` (48 KiB for a `notif.icon`: a PNG of 32,768 bytes is 43,691 base64url characters).
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const MAX_ICON_FRAME_BYTES: usize = 48 * 1024;

pub const MAX_APP: usize = 40;
pub const MAX_TITLE: usize = 150;
pub const MAX_TEXT: usize = 500;
pub const MAX_BIG_TEXT: usize = 1000;
pub const MAX_SUB_TEXT: usize = 80;
pub const MAX_CONVERSATION: usize = 100;
pub const MAX_MESSAGE_SENDER: usize = 60;
pub const MAX_MESSAGE_TEXT: usize = 300;
pub const MAX_LINE: usize = 150;
pub const MAX_HINT: usize = 60;
pub const MAX_LABEL: usize = 30;
pub const MAX_MESSAGES: usize = 5;
pub const MAX_LINES: usize = 5;
pub const MAX_ACTIONS: usize = 3;
pub const MAX_SYNC_NIDS: usize = 200;
pub const MAX_ACTION_TEXT: usize = 2000;
/// The largest PNG of a `notif.icon`, in bytes, and the allowed edge in pixels.
pub const MAX_ICON_BYTES: usize = 32 * 1024;
pub const ICON_MIN_PIXELS: u32 = 24;
pub const ICON_MAX_PIXELS: u32 = 128;
/// The index of an action in the phone's list: 0 to 31.
pub const MAX_ACTION_INDEX: u64 = 31;

/// What a receiver of the six ephemeral messages and of `notif.action` ends up with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifMsg {
    Status(Status),
    /// The icon's hash and its PNG bytes (already checked: size, hash, a square PNG of 24 to 128 pixels).
    Icon {
        hash: String,
        png: Vec<u8>,
    },
    Post(Box<Post>),
    Remove {
        nid: String,
    },
    Sync {
        nids: Vec<String>,
    },
    Dismiss {
        nid: String,
    },
    Action(ActionRequest),
}

impl NotifMsg {
    /// What a receiver ends up with, as the vectors write it (`expect.notif` of `messages.json`).
    pub fn to_json(&self) -> Value {
        match self {
            NotifMsg::Status(s) => json!({
                "enabled": s.enabled,
                "access": if s.access == Access::Granted { "granted" } else { "missing" },
                "apps": s.apps,
                "otpHidden": s.otp_hidden,
            }),
            NotifMsg::Icon { hash, png } => json!({ "hash": hash, "bytes": png.len() }),
            NotifMsg::Post(p) => p.to_json(),
            NotifMsg::Remove { nid } | NotifMsg::Dismiss { nid } => json!({ "nid": nid }),
            NotifMsg::Sync { nids } => json!({ "nids": nids }),
            NotifMsg::Action(a) => {
                let mut v = json!({ "nid": a.nid, "i": a.i, "label": a.label });
                if let Some(t) = &a.text {
                    v["text"] = json!(t);
                }
                v
            }
        }
    }
}

/// How a frame that passed the capability check came out.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Ok(NotifMsg),
    /// `bad_request` with the frame's `id` as `ref`.
    Bad,
    /// Dropped silently and counted (a bad icon).
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Granted,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub enabled: bool,
    pub access: Access,
    pub apps: u64,
    pub otp_hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRequest {
    pub nid: String,
    pub i: u8,
    pub label: String,
    /// Present exactly when the action is a reply.
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLine {
    /// `None`: the user's own message ("You").
    pub sender: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub i: u8,
    pub hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneAction {
    pub i: u8,
    pub label: String,
}

/// A `notif.post` after cleaning: every string is within its limit and holds no control or bidi character, arrays are cut and filtered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub nid: String,
    pub app: String,
    pub icon_hash: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub big_text: Option<String>,
    pub sub_text: Option<String>,
    pub conversation: Option<String>,
    pub group: bool,
    pub messages: Vec<ChatLine>,
    pub lines: Vec<String>,
    pub importance: u8,
    pub silent: bool,
    pub sensitive: bool,
    pub redacted: bool,
    pub code: Option<String>,
    pub reply: Option<Reply>,
    pub actions: Vec<PhoneAction>,
}

impl Post {
    /// The normalized post of the vectors (`notif.json`, `expect.notif`): every field with its default, absent strings and empty arrays left out.
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "nid": self.nid,
            "app": self.app,
            "group": self.group,
            "importance": self.importance,
            "silent": self.silent,
            "sensitive": self.sensitive,
            "redacted": self.redacted,
        });
        let mut put = |key: &str, value: &Option<String>| {
            if let Some(s) = value {
                v[key] = json!(s);
            }
        };
        put("iconHash", &self.icon_hash);
        put("title", &self.title);
        put("text", &self.text);
        put("bigText", &self.big_text);
        put("subText", &self.sub_text);
        put("conversation", &self.conversation);
        put("code", &self.code);
        if !self.messages.is_empty() {
            v["messages"] = Value::Array(
                self.messages
                    .iter()
                    .map(|m| match &m.sender {
                        Some(s) => json!({ "sender": s, "text": m.text }),
                        None => json!({ "text": m.text }),
                    })
                    .collect(),
            );
        }
        if !self.lines.is_empty() {
            v["lines"] = json!(self.lines);
        }
        if let Some(r) = &self.reply {
            v["reply"] = match &r.hint {
                Some(h) => json!({ "i": r.i, "hint": h }),
                None => json!({ "i": r.i }),
            };
        }
        if !self.actions.is_empty() {
            v["actions"] = Value::Array(self.actions.iter().map(|a| json!({ "i": a.i, "label": a.label })).collect());
        }
        v
    }

    /// Whether the notification has anything of the phone's to show on the PC.
    pub fn has_content(&self) -> bool {
        self.title.is_some() || self.text.is_some() || self.big_text.is_some() || !self.messages.is_empty() || !self.lines.is_empty()
    }
}

/// 32 characters from `0-9a-f`: a `nid`, an icon `hash`, an `iconHash`.
pub fn is_hex32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The `nid` of a notification key: the first 32 hex characters of SHA-256 of its UTF-8 bytes (the phone's job; here for tests and tools).
pub fn nid_of_key(key: &str) -> String {
    hex32(&sha256(key.as_bytes()))
}

/// The first 32 hex characters of a hash.
pub fn hex32(hash: &[u8; 32]) -> String {
    hash[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// A one-time code the PC may offer to copy: 4 to 10 characters from `0-9A-Z-`. Checked as sent (no cleaning).
pub fn valid_code(s: &str) -> bool {
    (4..=10).contains(&s.len()) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'-'))
}

/// The width and height of a PNG from its signature and its IHDR chunk, which must be the first chunk. Nothing else is looked at (the shell's own
/// decoder gets the rest, `icons.rs`).
pub fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    if b.len() < 33 || b[..8] != SIGNATURE || &b[12..16] != b"IHDR" || b[8..12] != [0, 0, 0, 13] {
        return None;
    }
    let be = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    Some((be(16), be(20)))
}

/// The one failure of a field that is there with the wrong type or an out-of-range value: the whole message is `bad_request`.
struct Bad;

fn opt_str(obj: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, Bad> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(clean_link_text(s, max)),
        Some(_) => Err(Bad),
    }
}

fn opt_bool(obj: &Map<String, Value>, key: &str) -> Result<bool, Bad> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(Bad),
    }
}

fn nid(v: Option<&Value>) -> Result<String, Bad> {
    match v.and_then(Value::as_str) {
        Some(s) if is_hex32(s) => Ok(s.to_string()),
        _ => Err(Bad),
    }
}

/// An index of an action: an integer from 0 to 31.
fn index(v: Option<&Value>) -> Result<u8, Bad> {
    v.and_then(uint).filter(|n| *n <= MAX_ACTION_INDEX).map(|n| n as u8).ok_or(Bad)
}

/// An optional array: absent and `null` are empty, any other non-array is invalid.
fn opt_array<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a [Value], Bad> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(Bad),
    }
}

/// A `notif.*` frame that passed validation steps 1 to 6b (the capability check and the size are the receiver's). `ty` is the type, `obj` the
/// parsed frame; the frame's `id` has to be valid.
pub fn parse(ty: &str, obj: &Map<String, Value>) -> Parsed {
    if valid_id(obj.get("id")).is_none() {
        return Parsed::Bad;
    }
    match parse_inner(ty, obj) {
        Ok(Some(msg)) => Parsed::Ok(msg),
        Ok(None) => Parsed::Drop,
        Err(Bad) => Parsed::Bad,
    }
}

fn parse_inner(ty: &str, obj: &Map<String, Value>) -> Result<Option<NotifMsg>, Bad> {
    Ok(Some(match ty {
        "notif.status" => {
            let enabled = obj.get("enabled").and_then(Value::as_bool).ok_or(Bad)?;
            let access = match obj.get("access").and_then(Value::as_str) {
                Some("granted") => Access::Granted,
                Some("missing") => Access::Missing,
                _ => return Err(Bad),
            };
            let apps = obj.get("apps").and_then(uint).ok_or(Bad)?;
            NotifMsg::Status(Status { enabled, access, apps, otp_hidden: opt_bool(obj, "otpHidden")? })
        }
        "notif.icon" => return icon(obj),
        "notif.remove" => NotifMsg::Remove { nid: nid(obj.get("nid"))? },
        "notif.dismiss" => NotifMsg::Dismiss { nid: nid(obj.get("nid"))? },
        "notif.sync" => {
            let list = obj.get("nids").and_then(Value::as_array).filter(|l| l.len() <= MAX_SYNC_NIDS).ok_or(Bad)?;
            NotifMsg::Sync { nids: list.iter().map(|v| nid(Some(v))).collect::<Result<_, _>>()? }
        }
        "notif.action" => {
            let n = nid(obj.get("nid"))?;
            let i = index(obj.get("i"))?;
            let label = obj.get("label").and_then(Value::as_str).ok_or(Bad)?;
            let text = match obj.get("text") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => {
                    // The receiver trims the reply (the White_Space set) and uses the trimmed text: 1 to 2,000 characters.
                    let t = trim(s);
                    if t.is_empty() || t.chars().count() > MAX_ACTION_TEXT {
                        return Err(Bad);
                    }
                    Some(t.to_string())
                }
                Some(_) => return Err(Bad),
            };
            NotifMsg::Action(ActionRequest { nid: n, i, label: clean_link_text(label, MAX_LABEL).unwrap_or_default(), text })
        }
        "notif.post" => NotifMsg::Post(Box::new(post(obj)?)),
        _ => return Err(Bad),
    }))
}

/// `notif.icon`: a bad `hash` or a missing `png` is `bad_request`; a `png` that is not base64url, too big, not a square PNG of 24 to 128 pixels or
/// that doesn't match the hash is dropped silently (`Ok(None)`).
fn icon(obj: &Map<String, Value>) -> Result<Option<NotifMsg>, Bad> {
    let hash = nid(obj.get("hash"))?;
    let png = obj.get("png").and_then(Value::as_str).ok_or(Bad)?;
    // 32,768 bytes are 43,691 characters: refuse a longer text before decoding it.
    if png.len() > MAX_ICON_BYTES.div_ceil(3) * 4 {
        return Ok(None);
    }
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(png) else {
        return Ok(None);
    };
    if bytes.len() > MAX_ICON_BYTES || hex32(&sha256(&bytes)) != hash {
        return Ok(None);
    }
    match png_size(&bytes) {
        Some((w, h)) if w == h && (ICON_MIN_PIXELS..=ICON_MAX_PIXELS).contains(&w) => Ok(Some(NotifMsg::Icon { hash, png: bytes })),
        _ => Ok(None),
    }
}

fn post(obj: &Map<String, Value>) -> Result<Post, Bad> {
    let n = nid(obj.get("nid"))?;
    let app = match obj.get("app") {
        Some(Value::String(s)) => clean_link_text(s, MAX_APP).ok_or(Bad)?,
        _ => return Err(Bad),
    };
    let icon_hash = match obj.get("iconHash") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if is_hex32(s) => Some(s.clone()),
        Some(_) => return Err(Bad),
    };
    let mut messages = Vec::new();
    for m in opt_array(obj, "messages")? {
        let m = m.as_object().ok_or(Bad)?;
        let text = m.get("text").and_then(Value::as_str).ok_or(Bad)?;
        let sender = opt_str(m, "sender", MAX_MESSAGE_SENDER)?;
        if let Some(text) = clean_link_text(text, MAX_MESSAGE_TEXT) {
            messages.push(ChatLine { sender, text });
        }
    }
    // Entries with nothing to show are dropped first, then the newest five are kept.
    let skip = messages.len().saturating_sub(MAX_MESSAGES);
    messages.drain(..skip);
    let mut lines = Vec::new();
    for l in opt_array(obj, "lines")? {
        let l = l.as_str().ok_or(Bad)?;
        lines.extend(clean_link_text(l, MAX_LINE));
    }
    let skip = lines.len().saturating_sub(MAX_LINES);
    lines.drain(..skip);
    let importance = match obj.get("importance") {
        None | Some(Value::Null) => 3,
        Some(v) => uint(v).filter(|n| (2..=5).contains(n)).ok_or(Bad)? as u8,
    };
    // A code that isn't valid, of any type, is ignored: no button, the notification is still shown.
    let code = obj.get("code").and_then(Value::as_str).filter(|c| valid_code(c)).map(str::to_string);
    let reply = match obj.get("reply") {
        None | Some(Value::Null) => None,
        Some(Value::Object(r)) => Some(Reply { i: index(r.get("i"))?, hint: opt_str(r, "hint", MAX_HINT)? }),
        Some(_) => return Err(Bad),
    };
    let mut actions = Vec::new();
    for a in opt_array(obj, "actions")? {
        let a = a.as_object().ok_or(Bad)?;
        let i = index(a.get("i"))?;
        let label = a.get("label").and_then(Value::as_str).ok_or(Bad)?;
        if let Some(label) = clean_link_text(label, MAX_LABEL) {
            actions.push(PhoneAction { i, label });
        }
    }
    actions.truncate(MAX_ACTIONS);
    let post = Post {
        nid: n,
        app,
        icon_hash,
        title: opt_str(obj, "title", MAX_TITLE)?,
        text: opt_str(obj, "text", MAX_TEXT)?,
        big_text: opt_str(obj, "bigText", MAX_BIG_TEXT)?,
        sub_text: opt_str(obj, "subText", MAX_SUB_TEXT)?,
        conversation: opt_str(obj, "conversation", MAX_CONVERSATION)?,
        group: opt_bool(obj, "group")?,
        messages,
        lines,
        importance,
        silent: opt_bool(obj, "silent")?,
        sensitive: opt_bool(obj, "sensitive")?,
        redacted: opt_bool(obj, "redacted")?,
        code,
        reply,
        actions,
    };
    if !post.has_content() && !post.redacted {
        return Err(Bad);
    }
    Ok(post)
}

/// What an accepted message is called in the vectors (`expect.action`) and in the counters.
pub fn vector_name(msg: &NotifMsg) -> &'static str {
    match msg {
        NotifMsg::Status(_) => "notif_status",
        NotifMsg::Icon { .. } => "notif_icon",
        NotifMsg::Post(_) => "notif_post",
        NotifMsg::Remove { .. } => "notif_remove",
        NotifMsg::Sync { .. } => "notif_sync",
        NotifMsg::Dismiss { .. } => "notif_dismiss",
        NotifMsg::Action(_) => "notif_action",
    }
}

/// Which role sends this type, and whether it needs an `ack`: `(sender is the phone, acked)`. `None` for a type that is not a `notif.*` message.
pub fn kind(ty: &str) -> Option<(bool, bool)> {
    Some(match ty {
        "notif.status" | "notif.icon" | "notif.post" | "notif.remove" | "notif.sync" => (true, false),
        "notif.dismiss" => (false, false),
        "notif.action" => (false, true),
        _ => return None,
    })
}

/// The frame limit of a `notif.*` type.
pub fn frame_limit(ty: &str) -> usize {
    if ty == "notif.icon" { MAX_ICON_FRAME_BYTES } else { MAX_FRAME_BYTES }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NID: &str = "54892d1f759b11f6762ff1bd98cda7df";

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn post_of(v: Value) -> Option<Post> {
        let mut o = obj(v);
        o.entry("id").or_insert(json!("id-1"));
        match parse("notif.post", &o) {
            Parsed::Ok(NotifMsg::Post(p)) => Some(*p),
            _ => None,
        }
    }

    #[test]
    fn the_nid_of_a_key_matches_the_design_examples() {
        assert_eq!(nid_of_key("0|com.example.chat|42|null|10123"), NID);
        assert_eq!(nid_of_key("0|com.whatsapp|1|null|10123"), "36460390475e793c9a9e06dd5e2d421f");
        assert!(is_hex32(NID) && !is_hex32(&NID.to_uppercase()) && !is_hex32(&NID[1..]));
    }

    #[test]
    fn a_minimal_post_has_the_defaults_and_a_cleaned_app() {
        let p = post_of(json!({ "nid": NID, "app": "  Chat\u{202e} App ", "title": "Alice\nB" })).unwrap();
        assert_eq!(p.app, "Chat_ App");
        assert_eq!(p.title.as_deref(), Some("Alice B"));
        assert_eq!((p.importance, p.group, p.silent, p.sensitive, p.redacted), (3, false, false, false, false));
    }

    #[test]
    fn a_post_with_nothing_to_show_is_refused_unless_it_is_redacted() {
        assert!(post_of(json!({ "nid": NID, "app": "Chat" })).is_none());
        assert!(
            post_of(json!({ "nid": NID, "app": "Chat", "subText": "s", "conversation": "c" })).is_none(),
            "subText and conversation are not content"
        );
        assert!(
            post_of(json!({ "nid": NID, "app": "Chat", "title": "\n", "text": "  ", "messages": [{ "text": " " }], "lines": [" "] }))
                .is_none()
        );
        assert!(post_of(json!({ "nid": NID, "app": "Chat", "redacted": true })).is_some());
    }

    #[test]
    fn arrays_drop_empty_entries_then_keep_the_newest_five_or_first_three() {
        let p = post_of(json!({ "nid": NID, "app": "A", "lines": ["1", "2", "3", "4", "", "5", "6", "7"] })).unwrap();
        assert_eq!(p.lines, ["3", "4", "5", "6", "7"]);
        let m: Vec<Value> = (1..=7).map(|i| json!({ "sender": "S", "text": i.to_string() })).collect();
        let p = post_of(json!({ "nid": NID, "app": "A", "messages": m })).unwrap();
        assert_eq!(p.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["3", "4", "5", "6", "7"]);
        let a: Vec<Value> = (0..5).map(|i| json!({ "i": i, "label": format!("A{i}") })).collect();
        let p = post_of(json!({ "nid": NID, "app": "A", "title": "t", "actions": a })).unwrap();
        assert_eq!(p.actions.iter().map(|a| a.i).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn a_code_is_checked_as_sent_and_a_bad_one_is_ignored_never_an_error() {
        for (code, ok) in [
            ("123456", true),
            ("AB12-CD", true),
            ("123", false),
            ("12345678901", false),
            ("abc123", false),
            ("12 34", false),
            (" 123456", false),
        ] {
            let p = post_of(json!({ "nid": NID, "app": "A", "title": "t", "code": code })).unwrap();
            assert_eq!(p.code.is_some(), ok, "{code:?}");
            assert_eq!(valid_code(code), ok, "{code:?}");
        }
        let p = post_of(json!({ "nid": NID, "app": "A", "title": "t", "code": 123456 })).unwrap();
        assert_eq!(p.code, None, "a code of the wrong type is ignored too");
    }

    #[test]
    fn wrong_types_and_ranges_are_bad_requests() {
        let base = |extra: Value| {
            let mut v = json!({ "nid": NID, "app": "A", "title": "t" });
            for (k, val) in extra.as_object().unwrap() {
                v[k] = val.clone();
            }
            v
        };
        for bad in [
            json!({ "importance": 1 }),
            json!({ "importance": 6 }),
            json!({ "importance": "3" }),
            json!({ "importance": 3.5 }),
            json!({ "silent": "true" }),
            json!({ "sensitive": 1 }),
            json!({ "reply": { "i": 32 } }),
            json!({ "reply": { "i": -1 } }),
            json!({ "reply": { "hint": "h" } }),
            json!({ "actions": [{ "i": 0 }] }),
            json!({ "actions": ["a"] }),
            json!({ "messages": "hi" }),
            json!({ "messages": [{ "text": 5 }] }),
            json!({ "lines": ["a", 5] }),
            json!({ "iconHash": "ABCDEF0123456789ABCDEF0123456789" }),
            json!({ "title": 5 }),
        ] {
            assert!(post_of(base(bad.clone())).is_none(), "{bad}");
        }
        assert!(post_of(json!({ "nid": NID, "title": "t" })).is_none(), "no app");
        assert!(post_of(json!({ "nid": NID, "app": "\n", "title": "t" })).is_none(), "an app that cleans to nothing");
        assert!(post_of(json!({ "nid": NID.to_uppercase(), "app": "A", "title": "t" })).is_none());
    }

    #[test]
    fn an_action_trims_its_reply_and_a_reply_needs_text_that_is_not_empty() {
        let act = |v: Value| {
            let mut o = obj(v);
            o.insert("id".into(), json!("x"));
            parse("notif.action", &o)
        };
        let ok = act(json!({ "nid": NID, "i": 1, "label": "Reply", "text": "  hi \n" }));
        assert_eq!(
            ok,
            Parsed::Ok(NotifMsg::Action(ActionRequest { nid: NID.into(), i: 1, label: "Reply".into(), text: Some("hi".into()) }))
        );
        assert_eq!(act(json!({ "nid": NID, "i": 1, "label": "x", "text": " \n" })), Parsed::Bad);
        assert_eq!(act(json!({ "nid": NID, "i": 1, "label": "x", "text": "a".repeat(2001) })), Parsed::Bad);
        assert!(matches!(act(json!({ "nid": NID, "i": 1, "label": "x", "text": "a".repeat(2000) })), Parsed::Ok(_)));
        assert!(matches!(
            act(json!({ "nid": NID, "i": 1, "label": "x", "text": null })),
            Parsed::Ok(NotifMsg::Action(ActionRequest { text: None, .. }))
        ));
        assert_eq!(act(json!({ "nid": NID, "i": 32, "label": "x" })), Parsed::Bad);
    }

    #[test]
    fn a_sync_of_more_than_200_nids_or_with_a_bad_one_is_refused() {
        let sync = |nids: Vec<String>| parse("notif.sync", &obj(json!({ "id": "x", "nids": nids })));
        assert!(matches!(sync(vec![]), Parsed::Ok(NotifMsg::Sync { .. })));
        assert!(matches!(sync(vec![NID.into(); 200]), Parsed::Ok(_)));
        assert_eq!(sync(vec![NID.into(); 201]), Parsed::Bad);
        assert_eq!(sync(vec![NID.into(), "nope".into()]), Parsed::Bad);
    }

    #[test]
    fn png_size_reads_only_a_signature_and_an_ihdr() {
        let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&24u32.to_be_bytes());
        png.extend_from_slice(&48u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(png_size(&png), Some((24, 48)));
        assert_eq!(png_size(&png[..32]), None);
        let mut other = png.clone();
        other[1] = b'X';
        assert_eq!(png_size(&other), None);
        assert_eq!(png_size(b"this is not a png, it is text that is long enough to look like data"), None);
    }
}
