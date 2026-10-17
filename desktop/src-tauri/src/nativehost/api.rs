//! What a request from the browser extension means (docs/extension.md: "Messages", "Error codes" and "Pipe"). The host relays the request's bytes
//! over the pipe and Kin answers here. **Every limit of the contract is checked again**, because any program of the user can write to the pipe:
//! the URL rule, the 300-character title, the 4 MiB text, the 32 MiB image bytes, the 64 MiB download, and at most 30 `send` requests per 10 seconds.
//!
//! The pipe is narrow on purpose: `status`, `start`, `open` and `send` of a text, a link, a page or an image. Nothing here changes settings,
//! unpairs a device or reads the history. **Nothing is logged but counts and codes** (no URL, no text, no title, no name).
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};

use super::imagefetch::{self, FetchError, ImageFetcher};
use crate::clipboard;
use crate::core::{APP_VERSION, Core, lock};
use crate::encoding::{unix_ms, uuid_v7};
use crate::links;
use crate::queue::TextBody;
use crate::sync::{Kind, SendPlan, SendResult, Tap, plan_send};

/// The most `send` requests Kin accepts per window: a runaway program must not flood the phone.
pub const SENDS_PER_WINDOW: usize = 30;
pub const SEND_WINDOW: Duration = Duration::from_secs(10);
/// The most text a `send` of kind `text` may carry (UTF-8 bytes).
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// The most image bytes a `send` may carry (raw, before base64): 32 MiB.
pub const MAX_IMAGE_DATA_BYTES: usize = 32 * 1024 * 1024;
/// The most devices `status` lists.
const MAX_DEVICES: usize = 16;
/// What Kin says is the source of a link from the extension (the browser can't be told apart reliably).
pub const SOURCE_APP: &str = "Browser";

/// The window `open` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Main,
    Pairing,
}

/// Shows a window of Kin (the app wires it to the real windows; a test records the call).
pub type OpenHook = Arc<dyn Fn(OpenTarget) + Send + Sync>;

pub struct Api {
    core: Arc<Core>,
    fetcher: Arc<dyn ImageFetcher>,
    open: Option<OpenHook>,
    sends: Mutex<VecDeque<Instant>>,
}

fn ok(id: &str, ty: &str, extra: Value) -> Value {
    let mut m = json!({ "v": 1, "ref": id, "type": ty, "ok": true });
    if let (Some(o), Some(e)) = (m.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    m
}

fn fail(id: Option<&str>, ty: &str, code: &str, message: &str) -> Value {
    // The message is shown to the user as it is: at most 300 characters.
    let mut m = json!({ "v": 1, "type": ty, "ok": false, "code": code, "message": crate::encoding::truncate_chars(message, 300) });
    if let Some(id) = id {
        m["ref"] = json!(id);
    }
    m
}

/// A request id is 1 to 64 characters.
fn valid_request_id(v: Option<&Value>) -> Option<&str> {
    v?.as_str().filter(|s| !s.is_empty() && s.chars().count() <= 64)
}

impl Api {
    pub fn new(core: Arc<Core>, fetcher: Arc<dyn ImageFetcher>, open: Option<OpenHook>) -> Api {
        Api { core, fetcher, open, sends: Mutex::new(VecDeque::new()) }
    }

    /// Answers one request (the bytes of a JSON object) with the bytes of one JSON object, at most 1 MiB.
    pub async fn handle(&self, body: &[u8]) -> Vec<u8> {
        let response = self.respond(body).await;
        let bytes = response.to_string().into_bytes();
        if bytes.len() <= super::framing::MAX_RESPONSE {
            bytes
        } else {
            fail(response["ref"].as_str(), "error", "INTERNAL", "The answer was too big.").to_string().into_bytes()
        }
    }

    async fn respond(&self, body: &[u8]) -> Value {
        // The JSON rules of protocol.md: a generic tree first (a repeated key: the last one wins, which is what `Value` does), then the fields.
        let Ok(Value::Object(req)) = serde_json::from_slice::<Value>(body) else {
            return fail(None, "error", "BAD_REQUEST", "The request isn't a valid JSON object.");
        };
        let (Some(id), Some(ty)) = (valid_request_id(req.get("id")), req.get("type").and_then(Value::as_str)) else {
            return fail(None, "error", "BAD_REQUEST", "The request has no usable id or type.");
        };
        if req.get("v") != Some(&json!(1)) {
            let mut m = fail(Some(id), "error", "UNSUPPORTED_VERSION", "Update Kin or the extension.");
            m["supported"] = json!([1]);
            return m;
        }
        match ty {
            "status" | "start" => ok(id, ty, self.status_body()),
            "open" => self.open(id, &req),
            "send" => self.send(id, &req).await,
            _ => fail(Some(id), ty, "UNSUPPORTED_TYPE", "Unknown request type."),
        }
    }

    /// `status` and `start` (the host does the starting): what Kin is doing. No device address, no key.
    fn status_body(&self) -> Value {
        let snapshot = self.core.snapshot();
        let devices: Vec<Value> = snapshot
            .devices
            .iter()
            .take(MAX_DEVICES)
            .map(|d| {
                let queued = self.core.db.queued_items(&d.device_id).map_or(0, |q| q.len());
                json!({
                    "id": d.device_id,
                    "name": crate::encoding::truncate_chars(&d.name, 64),
                    "online": d.connected,
                    "queuedCount": queued,
                })
            })
            .collect();
        let mut body = json!({ "running": true, "kinVersion": APP_VERSION, "devices": devices });
        if let Some(default) = self.core.queue_target() {
            body["defaultDeviceId"] = json!(default);
        }
        body
    }

    fn open(&self, id: &str, req: &Map<String, Value>) -> Value {
        let target = match req.get("target") {
            None | Some(Value::Null) => OpenTarget::Main,
            Some(Value::String(t)) if t == "main" => OpenTarget::Main,
            Some(Value::String(t)) if t == "pairing" => OpenTarget::Pairing,
            Some(_) => return fail(Some(id), "open", "BAD_REQUEST", "The target must be \"main\" or \"pairing\"."),
        };
        match &self.open {
            Some(hook) => {
                hook(target);
                log::info!("extension: asked to open a window ({target:?})");
                ok(id, "open", json!({}))
            }
            None => fail(Some(id), "open", "UNSUPPORTED_TYPE", "Kin can't open a window now."),
        }
    }

    /// At most 30 `send` requests per 10 seconds, counted for every request that reaches `send`.
    fn allow_send(&self) -> bool {
        let mut sends = lock(&self.sends);
        let now = Instant::now();
        while sends.front().is_some_and(|t| now.duration_since(*t) > SEND_WINDOW) {
            sends.pop_front();
        }
        if sends.len() >= SENDS_PER_WINDOW {
            return false;
        }
        sends.push_back(now);
        true
    }

    async fn send(&self, id: &str, req: &Map<String, Value>) -> Value {
        let bad = |code: &str, message: &str| fail(Some(id), "send", code, message);
        if !self.allow_send() {
            log::warn!("extension: too many send requests, one was refused");
            return bad("INTERNAL", "Too many requests, slow down");
        }
        // Which phone: the one named, else the default. None paired, or the name isn't one of them: NOT_PAIRED.
        let wanted = match req.get("deviceId") {
            None | Some(Value::Null) => None,
            Some(Value::String(d)) => Some(d.as_str()),
            Some(_) => return bad("BAD_REQUEST", "deviceId must be a string."),
        };
        let Some(device_id) = self.core.queue_target_for(wanted) else {
            return bad("NOT_PAIRED", if wanted.is_some() { "That phone isn't paired." } else { "Pair your phone with Kin first." });
        };
        let device_name = self.core.db.get_device(&device_id).ok().flatten().map_or_else(|| "Your phone".to_string(), |d| d.name);
        let target = Target { device_id, device_name };
        let Some(kind) = req.get("kind").and_then(Value::as_str) else {
            return bad("BAD_REQUEST", "The request has no kind.");
        };
        match kind {
            "text" => self.send_text(id, req, &target).await,
            "link" | "page" => self.send_link(id, kind, req, &target).await,
            "image" => self.send_image(id, req, &target).await,
            _ => bad("BAD_REQUEST", "Unknown kind."),
        }
    }

    async fn send_text(&self, id: &str, req: &Map<String, Value>, target: &Target) -> Value {
        let bad = |code: &str, message: &str| fail(Some(id), "send", code, message);
        let Some(text) = req.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
            return bad("BAD_REQUEST", "There is no text to send.");
        };
        if text.len() > MAX_TEXT_BYTES {
            return bad("TOO_LARGE", "That text is too big to send.");
        }
        // The text of a page is never `sensitive` (a browser can't know), and goes in its wire form (LF). Over 128 KiB it is a `.txt` file.
        let wire = clipboard::to_wire(text);
        let (result, transfer) = match plan_send(Kind::Text, false, wire.len()) {
            SendPlan::File => {
                log::info!("extension: text of {} bytes goes as a .txt file", wire.len());
                self.core.send_temp_file(Some(&target.device_id), crate::outgoing::TEXT_FILE_NAME, wire.as_bytes()).await
            }
            _ => {
                let message_id = uuid_v7(unix_ms());
                let body = TextBody::Text { text: &wire, is_url: links::is_http_url(&wire) };
                (self.core.queue_text_item(Some(&target.device_id), &message_id, body, Tap::now()).await, None::<String>)
            }
        };
        log::info!("extension: send text ({} bytes): {}", wire.len(), result.word());
        self.sent(id, &result, target, transfer)
    }

    async fn send_link(&self, id: &str, kind: &str, req: &Map<String, Value>, target: &Target) -> Value {
        let bad = |code: &str, message: &str| fail(Some(id), "send", code, message);
        let Some(url) = req.get("url").and_then(Value::as_str) else {
            return bad("BAD_REQUEST", "There is no URL to send.");
        };
        // The rule of protocol.md for `link.url`: http or https, no whitespace or control character, at most 8 KiB, checked as it is (not
        // trimmed). A URL that passes is never rewritten: `t=`, `list=` and fragments stay.
        if !links::is_link_url(url) {
            return bad("BAD_URL", "Kin can only send web links (http or https).");
        }
        let title = match req.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(t)) => links::clean_link_text(t, links::MAX_LINK_TITLE_CHARS),
            Some(_) => return bad("BAD_REQUEST", "The title must be a string."),
        };
        let message_id = uuid_v7(unix_ms());
        let result = self.core.queue_link(Some(&target.device_id), &message_id, url, title.as_deref(), Some(SOURCE_APP), Tap::now()).await;
        log::info!("extension: send {kind} ({} characters): {}", url.chars().count(), result.word());
        self.sent(id, &result, target, None)
    }

    async fn send_image(&self, id: &str, req: &Map<String, Value>, target: &Target) -> Value {
        let bad = |code: &str, message: &str| fail(Some(id), "send", code, message);
        let present = |key: &str| req.get(key).is_some_and(|v| !v.is_null());
        if present("url") == present("data") {
            return bad("BAD_REQUEST", "An image has exactly one of url and data.");
        }
        let name = match req.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(n)) => Some(n.as_str()),
            Some(_) => return bad("BAD_REQUEST", "The name must be a string."),
        };
        let (bytes, name_source): (Vec<u8>, String) = if present("url") {
            let Some(url) = req["url"].as_str() else {
                return bad("BAD_REQUEST", "The url must be a string.");
            };
            if !links::is_link_url(url) {
                return bad("BAD_URL", "Kin can only send web links (http or https).");
            }
            let page_url = match req.get("pageUrl") {
                None | Some(Value::Null) => None,
                Some(Value::String(p)) => Some(p.as_str()),
                Some(_) => return bad("BAD_REQUEST", "The pageUrl must be a string."),
            };
            match self.fetcher.fetch(url, page_url).await {
                Ok(fetched) => (fetched.bytes, fetched.final_url),
                Err(FetchError::TooLarge) => return bad("TOO_LARGE", "That image is too big to send."),
                Err(FetchError::NeedsBytes(why)) => {
                    log::info!("extension: the image download failed ({why}); the extension may fetch it itself");
                    return bad("IMAGE_NEEDS_BYTES", "The site wouldn't give Kin the picture.");
                }
            }
        } else {
            let Some(data) = req["data"].as_str() else {
                return bad("BAD_REQUEST", "The data must be a string.");
            };
            if !req.get("mime").is_some_and(Value::is_string) {
                return bad("BAD_REQUEST", "The image has no mime.");
            }
            // Base64 makes 32 MiB of bytes 42.7 MiB of text: more than that is over the limit without decoding it.
            if data.len() > MAX_IMAGE_DATA_BYTES / 3 * 4 + 4 {
                return bad("TOO_LARGE", "That image is too big to send.");
            }
            let Ok(bytes) = STANDARD.decode(data) else {
                return bad("BAD_REQUEST", "The image data isn't valid base64.");
            };
            if bytes.len() > MAX_IMAGE_DATA_BYTES {
                return bad("TOO_LARGE", "That image is too big to send.");
            }
            (bytes, String::new())
        };
        // Whatever the type said: it is an image only if its bytes are.
        let Some(sniffed) = imagefetch::sniff_bytes(&bytes) else {
            return if present("url") {
                log::info!("extension: the download wasn't an image; the extension may fetch it itself");
                bad("IMAGE_NEEDS_BYTES", "The site wouldn't give Kin the picture.")
            } else {
                bad("NOT_AN_IMAGE", "That doesn't look like an image.")
            };
        };
        let file_name = imagefetch::image_file_name(name, &name_source, sniffed.ext);
        let size = bytes.len();
        let (result, transfer) = self.core.send_temp_file(Some(&target.device_id), &file_name, &bytes).await;
        log::info!("extension: send image ({size} bytes, {}): {}", sniffed.ext, result.word());
        self.sent(id, &result, target, transfer)
    }

    /// The answer to a `send`: sent (acked), queued (the phone is offline or didn't answer: it is in Kin's queue), or a failure.
    fn sent(&self, id: &str, result: &SendResult, target: &Target, transfer: Option<String>) -> Value {
        let name = &target.device_name;
        let mut extra = json!({ "deviceId": target.device_id, "deviceName": name });
        if let Some(t) = transfer {
            extra["transferId"] = json!(t);
        }
        match result {
            SendResult::Acked { .. } => {
                extra["queued"] = json!(false);
                ok(id, "send", extra)
            }
            SendResult::PhoneOffline | SendResult::NoReply => {
                extra["queued"] = json!(true);
                extra["code"] = json!("DEVICE_OFFLINE_QUEUED");
                extra["message"] = json!(format!("{name} is offline. It will be sent when the phone is back."));
                ok(id, "send", extra)
            }
            SendResult::NoPhonePaired => fail(Some(id), "send", "NOT_PAIRED", "Pair your phone with Kin first."),
            SendResult::TooLarge { .. } => fail(Some(id), "send", "TOO_LARGE", "That is too big to send."),
            SendResult::Rejected(code) if code == "too_large" => {
                fail(Some(id), "send", "TOO_LARGE", &format!("{name} says that is too big."))
            }
            SendResult::Rejected(code) if code == "disk_full" => {
                fail(Some(id), "send", "INTERNAL", &format!("{name} doesn't have enough free space."))
            }
            SendResult::Rejected(_) => fail(Some(id), "send", "INTERNAL", &format!("{name} couldn't take it.")),
        }
    }
}

struct Target {
    device_id: String,
    device_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_id_is_one_to_sixty_four_characters() {
        assert_eq!(valid_request_id(Some(&json!("a"))), Some("a"));
        assert_eq!(valid_request_id(Some(&json!("x".repeat(64)))), Some(&*"x".repeat(64)));
        assert_eq!(valid_request_id(Some(&json!("\u{1F600}".repeat(64)))).map(str::len), Some(256), "characters, not bytes");
        for bad in [json!(""), json!("x".repeat(65)), json!(5), json!(null), json!(["a"])] {
            assert_eq!(valid_request_id(Some(&bad)), None, "{bad}");
        }
        assert_eq!(valid_request_id(None), None);
    }

    #[test]
    fn a_failure_has_a_message_of_at_most_300_characters_and_a_ref_only_when_the_id_is_known() {
        let m = fail(Some("r"), "send", "INTERNAL", &"x".repeat(1000));
        assert_eq!(m["message"].as_str().unwrap().chars().count(), 300);
        assert_eq!((m["ok"].clone(), m["ref"].clone(), m["v"].clone()), (json!(false), json!("r"), json!(1)));
        assert!(fail(None, "error", "BAD_REQUEST", "x").get("ref").is_none());
    }
}
