//! The wire form of the `media.*` messages (protocol.md "Media controls", changelog 57): what a frame has to look like, and what the receiver ends up with
//! after the strings are cleaned and the numbers cut. Pure: no sockets, no Windows. `protocol/media.json` and the `media-*` cases of `messages.json` run
//! against it (`tests/vectors.rs`, `tests/media_vectors.rs`).
//!
//! Nothing here keeps anything: what is playing is shown or dropped, never stored.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};

use crate::encoding::sha256;
use crate::json::{uint, valid_id};
use crate::links::clean_link_text;
use crate::mirror::wire::{Access, hex32, is_hex32, png_size};

/// The capability of `hello.capabilities` that turns the `media.*` messages on. Every 0.2.0 build lists it, whatever its switches say.
pub const CAPABILITY: &str = "media";

/// A received frame over this many bytes is `error too_large` (48 KiB for a `media.image`: 24,576 bytes are 32,768 base64url characters).
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const MAX_IMAGE_FRAME_BYTES: usize = 48 * 1024;
/// A sender stays under this for a `media.state` (it cuts `album`, then `artist`, then `title`).
pub const MAX_STATE_SEND_BYTES: usize = 16 * 1024;

pub const MAX_APP: usize = 40;
pub const MAX_TITLE: usize = 150;
pub const MAX_ARTIST: usize = 100;
pub const MAX_ALBUM: usize = 100;
/// Only this many entries of `actions` are looked at.
pub const MAX_ACTIONS_LOOKED_AT: usize = 8;
pub const MAX_DURATION_MS: u64 = 86_400_000;
pub const MAX_POSITION_MS: u64 = 86_400_000;
pub const MAX_RATE_MILLI: u64 = 4000;
pub const MAX_SEEK_MS: u64 = 86_400_000;

/// A seek position from the PC's own window, as the phone may be told it: at most the duration of what plays (when known), and never over [`MAX_SEEK_MS`] (a
/// larger `seekMs` is `bad_request` on the phone).
pub fn clamp_seek(ms: u64, duration_ms: Option<u64>) -> u64 {
    ms.min(duration_ms.unwrap_or(MAX_SEEK_MS)).min(MAX_SEEK_MS)
}
pub const MAX_IMAGE_BYTES: usize = 24_576;
pub const ART_MAX_PIXELS: u32 = 256;
pub const ICON_MAX_PIXELS: u32 = 128;
pub const IMAGE_MIN_PIXELS: u32 = 16;

/// The `sid` of an app: the first 32 hex characters of SHA-256 of its identity (an Android package name, a Windows `SourceAppUserModelId`). The raw identity never
/// goes on the wire or into a log.
pub fn sid_of(identity: &str) -> String {
    hex32(&sha256(identity.as_bytes()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlayState {
    Playing,
    Paused,
    Buffering,
    Stopped,
}

impl PlayState {
    pub fn as_str(self) -> &'static str {
        match self {
            PlayState::Playing => "playing",
            PlayState::Paused => "paused",
            PlayState::Buffering => "buffering",
            PlayState::Stopped => "stopped",
        }
    }

    pub fn parse(s: &str) -> Option<PlayState> {
        Some(match s {
            "playing" => PlayState::Playing,
            "paused" => PlayState::Paused,
            "buffering" => PlayState::Buffering,
            "stopped" => PlayState::Stopped,
            _ => return None,
        })
    }
}

/// What a session offers, in the canonical order of the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek,
}

pub const ACTIONS: [Action; 6] = [Action::Play, Action::Pause, Action::Toggle, Action::Next, Action::Prev, Action::Seek];

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Play => "play",
            Action::Pause => "pause",
            Action::Toggle => "toggle",
            Action::Next => "next",
            Action::Prev => "prev",
            Action::Seek => "seek",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        ACTIONS.into_iter().find(|a| a.as_str() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub level: u8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub show: bool,
    pub share: bool,
    pub access: Option<Access>,
}

/// A `media.state` after cleaning: every string is within its limit, numbers are cut, `actions` are the known ones in the canonical order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub sid: String,
    pub app: String,
    pub state: PlayState,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub position_ms: Option<u64>,
    pub rate_milli: u64,
    pub actions: Vec<Action>,
    pub art_hash: Option<String>,
    pub icon_hash: Option<String>,
    pub volume: Option<Volume>,
}

impl State {
    /// The normalized state of the vectors (`expect.media`): `rateMilli` and `actions` always, the other optional fields only when present.
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "sid": self.sid,
            "app": self.app,
            "state": self.state.as_str(),
            "rateMilli": self.rate_milli,
            "actions": self.actions.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
        });
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(x) = value {
                v[key] = x;
            }
        };
        put("title", self.title.as_ref().map(|s| json!(s)));
        put("artist", self.artist.as_ref().map(|s| json!(s)));
        put("album", self.album.as_ref().map(|s| json!(s)));
        put("durationMs", self.duration_ms.map(|n| json!(n)));
        put("positionMs", self.position_ms.map(|n| json!(n)));
        put("artHash", self.art_hash.as_ref().map(|s| json!(s)));
        put("iconHash", self.icon_hash.as_ref().map(|s| json!(s)));
        put("volume", self.volume.map(|x| json!({ "level": x.level, "muted": x.muted })));
        v
    }

    /// The frame a source sends for this state (`media.state`), with the given `id`.
    pub fn to_frame(&self, id: &str) -> Value {
        let mut v = self.to_json();
        v["type"] = json!("media.state");
        v["id"] = json!(id);
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Art,
    Icon,
}

impl ImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageKind::Art => "art",
            ImageKind::Icon => "icon",
        }
    }

    pub fn max_pixels(self) -> u32 {
        match self {
            ImageKind::Art => ART_MAX_PIXELS,
            ImageKind::Icon => ICON_MAX_PIXELS,
        }
    }
}

/// A `media.image` that passed its checks (hash, magic bytes, size).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub hash: String,
    pub kind: ImageKind,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeOp {
    Level(u8),
    Delta(i64),
    Mute(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmdKind {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek(u64),
    Volume(VolumeOp),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    /// Absent only for a volume command.
    pub sid: Option<String>,
    pub kind: CmdKind,
}

impl Cmd {
    /// What a source must have in its `actions` for this command (any one of them), `None` for the volume.
    pub fn needs(&self) -> Option<&'static [Action]> {
        Some(match self.kind {
            CmdKind::Play => &[Action::Play, Action::Toggle],
            CmdKind::Pause => &[Action::Pause, Action::Toggle],
            CmdKind::Toggle => &[Action::Toggle, Action::Play, Action::Pause],
            CmdKind::Next => &[Action::Next],
            CmdKind::Prev => &[Action::Prev],
            CmdKind::Seek(_) => &[Action::Seek],
            CmdKind::Volume(_) => return None,
        })
    }

    /// The frame a viewer sends.
    pub fn to_frame(&self, id: &str) -> Value {
        let mut v = json!({ "type": "media.cmd", "id": id });
        if let Some(sid) = &self.sid {
            v["sid"] = json!(sid);
        }
        match self.kind {
            CmdKind::Play => v["cmd"] = json!("play"),
            CmdKind::Pause => v["cmd"] = json!("pause"),
            CmdKind::Toggle => v["cmd"] = json!("toggle"),
            CmdKind::Next => v["cmd"] = json!("next"),
            CmdKind::Prev => v["cmd"] = json!("prev"),
            CmdKind::Seek(ms) => {
                v["cmd"] = json!("seek");
                v["seekMs"] = json!(ms);
            }
            CmdKind::Volume(op) => {
                v["cmd"] = json!("volume");
                match op {
                    VolumeOp::Level(l) => v["level"] = json!(l),
                    VolumeOp::Delta(d) => v["delta"] = json!(d),
                    VolumeOp::Mute(m) => v["mute"] = json!(m),
                }
            }
        }
        v
    }
}

/// What a receiver of a `media.*` frame ends up with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaMsg {
    Status(Status),
    State(Box<State>),
    Image(Image),
    Clear,
    Volume(Volume),
    Cmd(Cmd),
}

impl MediaMsg {
    /// What a receiver ends up with, as the vectors write it (`expect.media` of `messages.json`).
    pub fn to_json(&self) -> Value {
        match self {
            MediaMsg::Status(s) => {
                let mut v = json!({ "show": s.show, "share": s.share });
                if let Some(a) = s.access {
                    v["access"] = json!(if a == Access::Granted { "granted" } else { "missing" });
                }
                v
            }
            MediaMsg::State(s) => s.to_json(),
            MediaMsg::Image(i) => json!({ "hash": i.hash, "kind": i.kind.as_str(), "bytes": i.data.len() }),
            MediaMsg::Clear => json!({}),
            MediaMsg::Volume(v) => json!({ "level": v.level, "muted": v.muted }),
            MediaMsg::Cmd(c) => {
                let mut v = json!({});
                if let Some(sid) = &c.sid {
                    v["sid"] = json!(sid);
                }
                let frame = c.to_frame("x");
                for key in ["cmd", "seekMs", "level", "delta", "mute"] {
                    if let Some(x) = frame.get(key) {
                        v[key] = x.clone();
                    }
                }
                v
            }
        }
    }
}

/// A `media.*` message that the receiver let through, with the `id` of its frame (an `error` answer says `ref` = this).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inbound {
    pub id: String,
    pub msg: MediaMsg,
}

/// How a frame that passed the capability check came out.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Ok(MediaMsg),
    /// `bad_request` with the frame's `id` as `ref`.
    Bad,
    /// Dropped silently and counted (a bad image).
    Drop,
}

/// Whether this type is one of the six `media.*` messages.
pub fn is_media(ty: &str) -> bool {
    matches!(ty, "media.status" | "media.state" | "media.image" | "media.clear" | "media.volume" | "media.cmd")
}

/// What an accepted message is called in the vectors (`expect.action`) and in the counters.
pub fn vector_name(msg: &MediaMsg) -> &'static str {
    match msg {
        MediaMsg::Status(_) => "media_status",
        MediaMsg::State(_) => "media_state",
        MediaMsg::Image(_) => "media_image",
        MediaMsg::Clear => "media_clear",
        MediaMsg::Volume(_) => "media_volume",
        MediaMsg::Cmd(_) => "media_cmd",
    }
}

/// The frame limit of a `media.*` type.
pub fn frame_limit(ty: &str) -> usize {
    if ty == "media.image" { MAX_IMAGE_FRAME_BYTES } else { MAX_FRAME_BYTES }
}

// ----- images ------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub format: Format,
    pub width: u32,
    pub height: u32,
}

/// The format and the size of a PNG or JPEG from its headers (a JPEG: the first frame header, SOF). Nothing is decoded.
pub fn image_info(b: &[u8]) -> Option<ImageInfo> {
    if let Some((width, height)) = png_size(b) {
        return Some(ImageInfo { format: Format::Png, width, height });
    }
    if b.len() < 4 || b[0] != 0xff || b[1] != 0xd8 {
        return None;
    }
    let mut pos = 2;
    while pos + 4 <= b.len() {
        if b[pos] != 0xff {
            return None;
        }
        while pos < b.len() && b[pos] == 0xff {
            pos += 1;
        }
        let marker = *b.get(pos)?;
        pos += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        if pos + 2 > b.len() {
            return None;
        }
        let len = u16::from_be_bytes([b[pos], b[pos + 1]]) as usize;
        if len < 2 {
            return None;
        }
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            if pos + 7 > b.len() {
                return None;
            }
            let height = u16::from_be_bytes([b[pos + 3], b[pos + 4]]) as u32;
            let width = u16::from_be_bytes([b[pos + 5], b[pos + 6]]) as u32;
            return Some(ImageInfo { format: Format::Jpeg, width, height });
        }
        pos += len;
    }
    None
}

/// Whether picture bytes of this `kind` pass the wire rules (size, format, both sides at least 16 and the longest at most the kind's limit).
pub fn image_fits(bytes: &[u8], kind: ImageKind) -> bool {
    if bytes.len() > MAX_IMAGE_BYTES {
        return false;
    }
    match image_info(bytes) {
        Some(i) => i.width >= IMAGE_MIN_PIXELS && i.height >= IMAGE_MIN_PIXELS && i.width.max(i.height) <= kind.max_pixels(),
        None => false,
    }
}

/// The `media.image` frame for these bytes, with the given `id` (the `hash` is worked out here).
pub fn image_frame(id: &str, kind: ImageKind, data: &[u8]) -> Value {
    json!({ "type": "media.image", "id": id, "hash": hash_of(data), "kind": kind.as_str(), "data": URL_SAFE_NO_PAD.encode(data) })
}

/// The `hash` of picture bytes: the first 32 hex characters of SHA-256.
pub fn hash_of(data: &[u8]) -> String {
    hex32(&sha256(data))
}

// ----- parsing -----------------------------------------------------------------------------------------------------------

/// The one failure of a field that is there with the wrong type or an out-of-range value: the whole message is `bad_request`.
struct Bad;

fn present<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.get(key).filter(|v| !v.is_null())
}

fn opt_str(obj: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, Bad> {
    match present(obj, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(clean_link_text(s, max)),
        Some(_) => Err(Bad),
    }
}

fn int(v: &Value, lo: u64, hi: u64) -> Result<u64, Bad> {
    uint(v).filter(|n| (lo..=hi).contains(n)).ok_or(Bad)
}

fn opt_int(obj: &Map<String, Value>, key: &str, lo: u64, hi: u64) -> Result<Option<u64>, Bad> {
    present(obj, key).map(|v| int(v, lo, hi)).transpose()
}

fn boolean(v: Option<&Value>) -> Result<bool, Bad> {
    v.and_then(Value::as_bool).ok_or(Bad)
}

fn hash(v: Option<&Value>) -> Result<String, Bad> {
    match v.and_then(Value::as_str) {
        Some(s) if is_hex32(s) => Ok(s.to_string()),
        _ => Err(Bad),
    }
}

fn opt_hash(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, Bad> {
    present(obj, key).map(|v| hash(Some(v))).transpose()
}

fn volume(v: &Value) -> Result<Volume, Bad> {
    let o = v.as_object().ok_or(Bad)?;
    Ok(Volume { level: int(o.get("level").ok_or(Bad)?, 0, 100)? as u8, muted: boolean(o.get("muted"))? })
}

/// A `media.*` frame that passed validation steps 1 to 6c (the capability check and the size are the receiver's). `ty` is the type, `obj` the parsed frame; the
/// frame's `id` has to be valid.
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

fn parse_inner(ty: &str, obj: &Map<String, Value>) -> Result<Option<MediaMsg>, Bad> {
    Ok(Some(match ty {
        "media.status" => {
            let access = match present(obj, "access") {
                None => None,
                Some(Value::String(s)) if s == "granted" => Some(Access::Granted),
                Some(Value::String(s)) if s == "missing" => Some(Access::Missing),
                Some(_) => return Err(Bad),
            };
            MediaMsg::Status(Status { show: boolean(obj.get("show"))?, share: boolean(obj.get("share"))?, access })
        }
        "media.clear" => MediaMsg::Clear,
        "media.volume" => {
            MediaMsg::Volume(Volume { level: int(obj.get("level").ok_or(Bad)?, 0, 100)? as u8, muted: boolean(obj.get("muted"))? })
        }
        "media.state" => MediaMsg::State(Box::new(state(obj)?)),
        "media.image" => return image(obj),
        "media.cmd" => MediaMsg::Cmd(cmd(obj)?),
        _ => return Err(Bad),
    }))
}

fn state(obj: &Map<String, Value>) -> Result<State, Bad> {
    let sid = hash(obj.get("sid"))?;
    let app = match obj.get("app") {
        Some(Value::String(s)) => clean_link_text(s, MAX_APP).ok_or(Bad)?,
        _ => return Err(Bad),
    };
    let play = obj.get("state").and_then(Value::as_str).and_then(PlayState::parse).ok_or(Bad)?;
    let title = opt_str(obj, "title", MAX_TITLE)?;
    let artist = opt_str(obj, "artist", MAX_ARTIST)?;
    let album = opt_str(obj, "album", MAX_ALBUM)?;
    let duration_ms = opt_int(obj, "durationMs", 1, MAX_DURATION_MS)?;
    let position_ms = opt_int(obj, "positionMs", 0, MAX_POSITION_MS)?.map(|p| duration_ms.map_or(p, |d| p.min(d)));
    let rate_milli = present(obj, "rateMilli").map(|v| int(v, 0, u64::MAX)).transpose()?.map_or(1000, |r| r.min(MAX_RATE_MILLI));
    let mut actions = Vec::new();
    match present(obj, "actions") {
        None => {}
        Some(Value::Array(list)) => {
            let first = &list[..list.len().min(MAX_ACTIONS_LOOKED_AT)];
            let mut named = Vec::new();
            for entry in first {
                named.push(entry.as_str().ok_or(Bad)?);
            }
            actions = ACTIONS.into_iter().filter(|a| named.contains(&a.as_str())).collect();
        }
        Some(_) => return Err(Bad),
    }
    if duration_ms.is_none() {
        actions.retain(|a| *a != Action::Seek);
    }
    Ok(State {
        sid,
        app,
        state: play,
        title,
        artist,
        album,
        duration_ms,
        position_ms,
        rate_milli,
        actions,
        art_hash: opt_hash(obj, "artHash")?,
        icon_hash: opt_hash(obj, "iconHash")?,
        volume: present(obj, "volume").map(volume).transpose()?,
    })
}

/// `media.image`: a bad `hash`, a `kind` that is not `art` or `icon` or a `data` that is not a string is `bad_request`; a `data` that is not base64url, is too big,
/// is not a PNG or JPEG of the right size or does not match the hash is dropped silently (`Ok(None)`).
fn image(obj: &Map<String, Value>) -> Result<Option<MediaMsg>, Bad> {
    let hash_text = hash(obj.get("hash"))?;
    let kind = match obj.get("kind").and_then(Value::as_str) {
        Some("art") => ImageKind::Art,
        Some("icon") => ImageKind::Icon,
        _ => return Err(Bad),
    };
    let data = obj.get("data").and_then(Value::as_str).ok_or(Bad)?;
    if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 + 4 {
        return Ok(None);
    }
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(data) else {
        return Ok(None);
    };
    if hash_of(&bytes) != hash_text || !image_fits(&bytes, kind) {
        return Ok(None);
    }
    Ok(Some(MediaMsg::Image(Image { hash: hash_text, kind, data: bytes })))
}

fn cmd(obj: &Map<String, Value>) -> Result<Cmd, Bad> {
    let name = obj.get("cmd").and_then(Value::as_str).ok_or(Bad)?;
    if !matches!(name, "play" | "pause" | "toggle" | "next" | "prev" | "seek" | "volume") {
        return Err(Bad);
    }
    let sid = match present(obj, "sid") {
        Some(v) => Some(hash(Some(v))?),
        None if name == "volume" => None,
        None => return Err(Bad),
    };
    let has = |k: &str| present(obj, k).is_some();
    if name == "seek" {
        if !has("seekMs") {
            return Err(Bad);
        }
    } else if has("seekMs") {
        return Err(Bad);
    }
    let kind = match name {
        "play" => CmdKind::Play,
        "pause" => CmdKind::Pause,
        "toggle" => CmdKind::Toggle,
        "next" => CmdKind::Next,
        "prev" => CmdKind::Prev,
        "seek" => CmdKind::Seek(int(present(obj, "seekMs").ok_or(Bad)?, 0, MAX_SEEK_MS)?),
        _ => {
            let given: Vec<&str> = ["level", "delta", "mute"].into_iter().filter(|k| has(k)).collect();
            if given.len() != 1 {
                return Err(Bad);
            }
            CmdKind::Volume(match given[0] {
                "level" => VolumeOp::Level(int(&obj["level"], 0, 100)? as u8),
                "delta" => {
                    let d = obj["delta"].as_i64().filter(|d| (-100..=100).contains(d) && *d != 0).ok_or(Bad)?;
                    VolumeOp::Delta(d)
                }
                _ => VolumeOp::Mute(boolean(obj.get("mute"))?),
            })
        }
    };
    if name != "volume" && (has("level") || has("delta") || has("mute")) {
        return Err(Bad);
    }
    Ok(Cmd { sid, kind })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_seek_from_the_pc_is_cut_to_the_duration_and_to_a_day() {
        use super::clamp_seek;
        assert_eq!(clamp_seek(90_000, Some(337_000)), 90_000);
        assert_eq!(clamp_seek(400_000, Some(337_000)), 337_000);
        assert_eq!(clamp_seek(400_000, None), 400_000);
        assert_eq!(clamp_seek(u64::MAX, None), 86_400_000);
        assert_eq!(clamp_seek(u64::MAX, Some(u64::MAX)), 86_400_000);
        assert_eq!(clamp_seek(5, Some(0)), 0);
    }

    use super::*;

    const SID: &str = "be0ca287362158a5ad91beed9ebd9263";

    fn obj(v: Value) -> Map<String, Value> {
        let mut o = v.as_object().unwrap().clone();
        o.entry("id").or_insert(json!("id-1"));
        o
    }

    fn state_of(v: Value) -> Option<State> {
        match parse("media.state", &obj(v)) {
            Parsed::Ok(MediaMsg::State(s)) => Some(*s),
            _ => None,
        }
    }

    #[test]
    fn the_sid_of_an_identity_is_the_first_32_hex_of_its_hash() {
        assert_eq!(sid_of("com.spotify.music"), SID);
        assert_eq!(sid_of("Spotify.exe"), "b527fa0aeeb2e7be74407eb00708f43c");
        assert!(is_hex32(&sid_of("")));
    }

    #[test]
    fn a_minimal_state_has_the_defaults() {
        let s = state_of(json!({ "sid": SID, "app": " Spotify ", "state": "paused" })).unwrap();
        assert_eq!((s.rate_milli, s.actions.len(), s.title, s.position_ms), (1000, 0, None, None));
        assert_eq!(s.app, "Spotify");
    }

    #[test]
    fn actions_are_canonical_known_deduplicated_and_seek_needs_a_duration() {
        let s = state_of(
            json!({ "sid": SID, "app": "A", "state": "playing", "durationMs": 10, "actions": ["seek", "next", "x", "play", "play"] }),
        )
        .unwrap();
        assert_eq!(s.actions, [Action::Play, Action::Next, Action::Seek]);
        let s = state_of(json!({ "sid": SID, "app": "A", "state": "playing", "actions": ["seek", "play"] })).unwrap();
        assert_eq!(s.actions, [Action::Play]);
        // Only the first 8 entries are looked at: a ninth of the wrong type is not an error.
        let s = state_of(json!({ "sid": SID, "app": "A", "state": "playing", "actions": ["a", "b", "c", "d", "e", "f", "g", "h", 5] }))
            .unwrap();
        assert!(s.actions.is_empty());
        assert!(state_of(json!({ "sid": SID, "app": "A", "state": "playing", "actions": ["play", 5] })).is_none());
    }

    #[test]
    fn numbers_are_cut_or_refused_as_the_spec_says() {
        let base = |extra: Value| {
            let mut v = json!({ "sid": SID, "app": "A", "state": "playing" });
            for (k, val) in extra.as_object().unwrap() {
                v[k] = val.clone();
            }
            v
        };
        assert_eq!(state_of(base(json!({ "rateMilli": 99999 }))).unwrap().rate_milli, 4000);
        assert_eq!(state_of(base(json!({ "durationMs": 1000, "positionMs": 5000 }))).unwrap().position_ms, Some(1000));
        for bad in [
            json!({ "durationMs": 0 }),
            json!({ "durationMs": 86_400_001u64 }),
            json!({ "positionMs": -1 }),
            json!({ "positionMs": 86_400_001u64 }),
            json!({ "rateMilli": -1 }),
            json!({ "rateMilli": 1.5 }),
            json!({ "volume": { "level": 101, "muted": false } }),
            json!({ "volume": { "level": 5 } }),
            json!({ "artHash": "ABC" }),
            json!({ "state": "Playing" }),
            json!({ "title": 5 }),
        ] {
            assert!(state_of(base(bad.clone())).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_command_has_exactly_what_its_kind_needs() {
        let cmd_of = |v: Value| match parse("media.cmd", &obj(v)) {
            Parsed::Ok(MediaMsg::Cmd(c)) => Some(c),
            _ => None,
        };
        assert_eq!(cmd_of(json!({ "cmd": "volume", "delta": -5 })).unwrap().kind, CmdKind::Volume(VolumeOp::Delta(-5)));
        assert!(cmd_of(json!({ "cmd": "volume", "delta": 0 })).is_none());
        assert!(cmd_of(json!({ "cmd": "volume", "delta": 101 })).is_none());
        assert!(cmd_of(json!({ "cmd": "volume", "level": 5, "mute": true })).is_none());
        assert!(cmd_of(json!({ "cmd": "volume" })).is_none());
        assert!(cmd_of(json!({ "cmd": "play" })).is_none(), "no sid");
        assert!(cmd_of(json!({ "cmd": "play", "sid": SID, "level": 5 })).is_none());
        assert!(cmd_of(json!({ "cmd": "seek", "sid": SID })).is_none());
        assert!(cmd_of(json!({ "cmd": "play", "sid": SID, "seekMs": 0 })).is_none());
        assert_eq!(cmd_of(json!({ "cmd": "seek", "sid": SID, "seekMs": 0 })).unwrap().kind, CmdKind::Seek(0));
        // A command that is built goes back through the parser as the same command.
        for c in [
            Cmd { sid: Some(SID.into()), kind: CmdKind::Next },
            Cmd { sid: None, kind: CmdKind::Volume(VolumeOp::Mute(true)) },
            Cmd { sid: Some(SID.into()), kind: CmdKind::Seek(1234) },
        ] {
            let frame = c.to_frame("id-1");
            assert_eq!(cmd_of(frame.clone()), Some(c), "{frame}");
        }
    }

    #[test]
    fn an_image_is_judged_by_its_headers() {
        let png = |w: u32, h: u32| {
            let mut b = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
            b.extend_from_slice(b"IHDR");
            b.extend_from_slice(&w.to_be_bytes());
            b.extend_from_slice(&h.to_be_bytes());
            b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
            b
        };
        assert!(image_fits(&png(256, 256), ImageKind::Art));
        assert!(!image_fits(&png(257, 256), ImageKind::Art));
        assert!(!image_fits(&png(129, 128), ImageKind::Icon));
        assert!(image_fits(&png(16, 16), ImageKind::Icon));
        assert!(!image_fits(&png(15, 16), ImageKind::Art));
        assert!(!image_fits(b"GIF89a\0\x01\0\x01\0\0\0;", ImageKind::Art));
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 16];
        jpeg.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        jpeg.extend_from_slice(&[0xff, 0xc0, 0, 11, 8, 1, 0, 1, 0, 1, 1, 0x11, 0, 0xff, 0xd9]);
        assert_eq!(image_info(&jpeg), Some(ImageInfo { format: Format::Jpeg, width: 256, height: 256 }));
        assert!(image_fits(&jpeg, ImageKind::Art) && !image_fits(&jpeg, ImageKind::Icon));
        assert_eq!(image_info(&jpeg[..5]), None);
    }

    #[test]
    fn a_bad_image_is_dropped_and_a_bad_shape_is_a_bad_request() {
        let png = {
            let mut b = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
            b.extend_from_slice(b"IHDR");
            b.extend_from_slice(&16u32.to_be_bytes());
            b.extend_from_slice(&16u32.to_be_bytes());
            b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
            b
        };
        let ok = image_frame("i1", ImageKind::Art, &png);
        assert!(matches!(parse("media.image", ok.as_object().unwrap()), Parsed::Ok(MediaMsg::Image(_))));
        let mut wrong_hash = ok.clone();
        wrong_hash["hash"] = json!("0".repeat(32));
        assert_eq!(parse("media.image", wrong_hash.as_object().unwrap()), Parsed::Drop);
        let mut not_a_picture = ok.clone();
        not_a_picture["data"] = json!("!!!");
        assert_eq!(parse("media.image", not_a_picture.as_object().unwrap()), Parsed::Drop);
        let mut bad_kind = ok.clone();
        bad_kind["kind"] = json!("cover");
        assert_eq!(parse("media.image", bad_kind.as_object().unwrap()), Parsed::Bad);
    }
}
