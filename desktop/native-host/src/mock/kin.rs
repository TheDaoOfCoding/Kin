//! The Kin side of docs/extension.md as a stand-in: answers the host's forwarded requests the way the TS mock host of the extension
//! (extension/tests/smoke/mock-host.ts) answers the extension's, so the same behavior can be checked through the real host.
//!
//! `<state>/scenario.json` picks the behavior (re-read for every request), `<state>/log.jsonl` gets one line per request received and one per
//! answer (image bytes are logged as size + sha256, never the bytes).

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::server::{Handler, Reply};

fn err(id: Option<&str>, ty: &str, code: &str, message: &str) -> Value {
    let mut m = json!({ "v": 1, "type": ty, "ok": false, "code": code, "message": message });
    if let Some(id) = id {
        m["ref"] = json!(id);
    }
    m
}

fn is_http_url(u: &Value) -> Option<&str> {
    let s = u.as_str()?;
    if s.len() > 8192 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")).then_some(s)
}

fn looks_like_image(b: &[u8]) -> bool {
    b.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) || b.starts_with(&[0xff, 0xd8]) || b.starts_with(b"GIF")
}

fn decode_chunked(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&b[..eol]).ok()?.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

/// What Kin does with an image URL: GET without cookies, check the bytes. Only plain `http`: the stand-in has no TLS. `Some(size)` when it's an image.
fn download_image(url: &str, referer: Option<&str>) -> Option<usize> {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("HTTP://"))?;
    let (authority, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let addr = if authority.contains(':') { authority.to_string() } else { format!("{authority}:80") };
    let sock = addr.to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&sock, Duration::from_secs(5)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nUser-Agent: Mozilla/5.0 (Kin stand-in)\r\n");
    if let Some(r) = referer {
        req.push_str(&format!("Referer: {r}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).ok()?;
    let mut raw = Vec::new();
    s.take(65 * 1024 * 1024).read_to_end(&mut raw).ok()?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    if !(200..300).contains(&status) {
        return None;
    }
    let mut body = raw[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        body = decode_chunked(&body)?;
    }
    (body.len() <= 64 * 1024 * 1024 && looks_like_image(&body)).then_some(body.len())
}

pub struct KinMock {
    state: PathBuf,
    sends: Mutex<VecDeque<Instant>>,
    log_lock: Mutex<()>,
}

impl KinMock {
    pub fn new(state_dir: &Path) -> Arc<KinMock> {
        Arc::new(KinMock { state: state_dir.to_path_buf(), sends: Mutex::new(VecDeque::new()), log_lock: Mutex::new(()) })
    }

    pub fn scenario(&self) -> Value {
        std::fs::read(self.state.join("scenario.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
    }

    fn log(&self, entry: Value) {
        // One write per line, under a lock: requests are served on several threads, and the smoke test reads the file while it is written.
        let _guard = self.log_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.state.join("log.jsonl")) {
            let _ = f.write_all(format!("{entry}\n").as_bytes());
        }
    }

    pub fn handler(self: &Arc<Self>) -> Handler {
        let me = self.clone();
        Arc::new(move |body| me.handle_bytes(&body))
    }

    fn handle_bytes(&self, body: &[u8]) -> Reply {
        let scenario = self.scenario();
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let req: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => {
                self.log(json!({ "error": "bad json" }));
                return Reply::Respond(err(None, "error", "BAD_REQUEST", "The request isn't valid JSON.").to_string().into_bytes());
            }
        };
        self.log(json!({ "at": at, "origin": req.get("origin"), "hostVersion": req.get("hostVersion"), "req": summarize(&req) }));
        if scenario["exitWithoutAnswer"] == json!(true) {
            return Reply::Close;
        }
        if let Some(ms) = scenario["delayMs"].as_u64() {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let mut response = self.handle(&req, &scenario);
        if scenario["badRef"] == json!(true) {
            response["ref"] = json!("someone-else");
        }
        self.log(json!({ "at": at, "res": response }));
        Reply::Respond(response.to_string().into_bytes())
    }

    fn handle(&self, req: &Value, s: &Value) -> Value {
        let id = req.get("id").and_then(Value::as_str).filter(|i| !i.is_empty() && i.chars().count() <= 64);
        let ty = req.get("type").and_then(Value::as_str);
        let (Some(id), Some(ty)) = (id, ty) else { return err(None, "error", "BAD_REQUEST", "The request has no id or type.") };
        if req.get("v") != Some(&json!(1)) {
            let mut m = err(Some(id), "error", "UNSUPPORTED_VERSION", "Update Kin or the extension.");
            m["supported"] = json!([1]);
            return m;
        }
        let ok = |ty: &str, extra: Value| {
            let mut m = json!({ "v": 1, "ref": id, "type": ty, "ok": true });
            if let (Some(o), Some(e)) = (m.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            m
        };
        let bad = |code: &str, message: &str| err(Some(id), ty, code, message);
        let devices = s["devices"].as_array().cloned().unwrap_or_default();
        let status_body = || {
            let mut m = Map::new();
            m.insert("running".into(), json!(true));
            m.insert("kinVersion".into(), json!("0.0.0-mock"));
            if let Some(v) = s.get("kinHostVersion") {
                m.insert("hostVersion".into(), v.clone());
            }
            m.insert("devices".into(), json!(devices));
            if let Some(d) = s.get("defaultDeviceId") {
                m.insert("defaultDeviceId".into(), d.clone());
            }
            Value::Object(m)
        };
        match ty {
            "status" => return ok("status", status_body()),
            "start" => return ok("start", status_body()),
            "open" => return ok("open", json!({})),
            "send" => {}
            _ => return bad("UNSUPPORTED_TYPE", "Unknown request type."),
        }

        // send: 30 per 10 seconds
        {
            let mut q = self.sends.lock().unwrap();
            let now = Instant::now();
            while q.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(10)) {
                q.pop_front();
            }
            if q.len() >= 30 {
                return bad("INTERNAL", "Too many requests, slow down");
            }
            q.push_back(now);
        }
        if let Some(e) = s.get("sendError") {
            return bad(e["code"].as_str().unwrap_or("INTERNAL"), e["message"].as_str().unwrap_or("Failed."));
        }
        if devices.is_empty() {
            return bad("NOT_PAIRED", "Pair your phone with Kin first.");
        }
        let wanted = req
            .get("deviceId")
            .and_then(Value::as_str)
            .or_else(|| s["defaultDeviceId"].as_str())
            .or_else(|| devices[0]["id"].as_str())
            .unwrap_or("");
        let Some(device) = devices.iter().find(|d| d["id"].as_str() == Some(wanted)) else { return bad("NOT_PAIRED", "That phone isn't paired.") };
        let online = device["online"] == json!(true);
        let name = device["name"].as_str().unwrap_or("Phone");
        let sent = || {
            let mut extra = json!({ "queued": !online, "deviceId": device["id"], "deviceName": name });
            if !online {
                extra["code"] = json!("DEVICE_OFFLINE_QUEUED");
                extra["message"] = json!(format!("{name} is offline. It will be sent when the phone is back."));
            }
            ok("send", extra)
        };
        let bad_url = || bad("BAD_URL", "Kin can only send web links (http or https).");

        match req.get("kind").and_then(Value::as_str) {
            Some("text") => match req.get("text").and_then(Value::as_str) {
                Some(t) if !t.is_empty() => {
                    if t.len() > 4 * 1024 * 1024 { bad("TOO_LARGE", "That text is too big.") } else { sent() }
                }
                _ => bad("BAD_REQUEST", "No text."),
            },
            Some("link" | "page") => {
                if req.get("url").and_then(is_http_url).is_none() {
                    return bad_url();
                }
                match req.get("title") {
                    None | Some(Value::Null) => sent(),
                    Some(Value::String(t)) if t.chars().count() <= 300 => sent(),
                    _ => bad("BAD_REQUEST", "Bad title."),
                }
            }
            Some("image") => {
                let (has_url, has_data) = (req.get("url").is_some(), req.get("data").is_some());
                if has_url == has_data {
                    return bad("BAD_REQUEST", "Exactly one of url and data.");
                }
                if has_url {
                    let Some(url) = req.get("url").and_then(is_http_url) else { return bad_url() };
                    return match download_image(url, req.get("pageUrl").and_then(Value::as_str)) {
                        Some(_) => sent(),
                        None => bad("IMAGE_NEEDS_BYTES", "The site wouldn't give Kin the picture."),
                    };
                }
                let (Some(data), Some(_), Some(_)) =
                    (req["data"].as_str(), req["mime"].as_str(), req["name"].as_str()) else { return bad("BAD_REQUEST", "Bad image.") };
                let Ok(bytes) = STANDARD.decode(data) else { return bad("BAD_REQUEST", "Bad image.") };
                if bytes.len() > 32 * 1024 * 1024 {
                    bad("TOO_LARGE", "That image is too big.")
                } else if !looks_like_image(&bytes) {
                    bad("NOT_AN_IMAGE", "That doesn't look like an image.")
                } else {
                    sent()
                }
            }
            _ => bad("BAD_REQUEST", "Unknown kind."),
        }
    }
}

fn summarize(req: &Value) -> Value {
    let mut copy = req.clone();
    if let Some(Value::String(data)) = copy.get("data").cloned() {
        let bytes = STANDARD.decode(&data).unwrap_or_default();
        copy["data"] = json!(format!("<{} bytes>", bytes.len()));
        copy["dataSha256"] = json!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
    copy
}
