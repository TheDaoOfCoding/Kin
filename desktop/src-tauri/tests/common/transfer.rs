//! The fake phone's side of the file transfers over real TLS: a data pattern that can be generated and checked at any offset (so a 2 GB
//! file never has to be held in memory or even written by the test), and a small HTTP/1.1 client that streams a body out or in.
#![allow(dead_code)]
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use super::{HttpResponse, World};

const BLOCK: usize = 1 << 20;

/// Bytes of a file that can be made, and checked, at any offset: a 1 MiB block of pseudo-random data, repeated, with the number of the
/// block in its first 8 bytes (so a byte in the wrong place is noticed, and a seed makes two files differ).
#[derive(Clone)]
pub struct Pattern {
    base: Arc<Vec<u8>>,
}

impl Pattern {
    pub fn new(seed: u64) -> Pattern {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut base = vec![0u8; BLOCK];
        for chunk in base.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let bytes = x.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Pattern { base: Arc::new(base) }
    }

    /// Writes the bytes at `abs..abs + out.len()` of the file into `out`.
    pub fn fill(&self, mut abs: u64, mut out: &mut [u8]) {
        while !out.is_empty() {
            let in_block = (abs % BLOCK as u64) as usize;
            let n = out.len().min(BLOCK - in_block);
            out[..n].copy_from_slice(&self.base[in_block..in_block + n]);
            let index = (abs / BLOCK as u64).to_le_bytes();
            let patched = (in_block + n).min(8).saturating_sub(in_block);
            if in_block < 8 {
                out[..patched].copy_from_slice(&index[in_block..in_block + patched]);
            }
            abs += n as u64;
            out = &mut out[n..];
        }
    }

    pub fn bytes(&self, abs: u64, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        self.fill(abs, &mut v);
        v
    }

    /// Whether `data` is what the file holds at `abs`.
    pub fn matches(&self, abs: u64, data: &[u8]) -> bool {
        self.bytes(abs, data.len()) == data
    }

    /// Checks a file on disk against the pattern, 1 MiB at a time. Returns the first offset that differs.
    pub fn check_file(&self, path: &std::path::Path, size: u64) -> Result<(), String> {
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len != size {
            return Err(format!("the file has {len} bytes, not {size}"));
        }
        let mut buf = vec![0u8; BLOCK];
        let mut pos = 0u64;
        while pos < size {
            let n = (size - pos).min(BLOCK as u64) as usize;
            file.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
            if !self.matches(pos, &buf[..n]) {
                return Err(format!("the bytes differ in the block at {pos}"));
            }
            pos += n as u64;
        }
        Ok(())
    }

    /// Writes the pattern as a file of `size` bytes.
    pub fn write_file(&self, path: &std::path::Path, size: u64) {
        use std::io::Write;
        let mut file = std::io::BufWriter::with_capacity(BLOCK, std::fs::File::create(path).unwrap());
        let mut pos = 0u64;
        let mut buf = vec![0u8; BLOCK];
        while pos < size {
            let n = (size - pos).min(BLOCK as u64) as usize;
            self.fill(pos, &mut buf[..n]);
            file.write_all(&buf[..n]).unwrap();
            pos += n as u64;
        }
        file.flush().unwrap();
    }
}

/// What a `GET` brought.
#[derive(Debug)]
pub struct Download {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Body bytes read.
    pub received: u64,
    /// Every byte read matched the pattern at its place.
    pub intact: bool,
    pub elapsed: Duration,
}

impl Download {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// One TLS connection of the phone to the PC, used for HTTP requests.
pub struct Http {
    pub stream: TlsStream<TcpStream>,
}

pub async fn http(w: &World) -> Http {
    Http { stream: w.tls().await }
}

impl Http {
    /// A JSON request. `POST /v1/transfers` and the like.
    pub async fn json(&mut self, method: &str, path: &str, body: &Value) -> HttpResponse {
        let bytes = body.to_string().into_bytes();
        super::request(&mut self.stream, method, path, &[("Content-Type", "application/json; charset=utf-8")], &bytes).await.unwrap()
    }

    pub async fn simple(&mut self, method: &str, path: &str) -> HttpResponse {
        super::request(&mut self.stream, method, path, &[], b"").await.unwrap()
    }

    /// `POST /v1/transfers` for files given as `(fileId, name, size)`. The mime type is `application/octet-stream`.
    pub async fn post_transfer(&mut self, id: &str, files: &[(&str, &str, u64)]) -> HttpResponse {
        let files: Vec<Value> =
            files.iter().map(|(f, n, s)| json!({ "fileId": f, "name": n, "size": s, "mime": "application/octet-stream" })).collect();
        self.json("POST", "/v1/transfers", &json!({ "id": id, "files": files })).await
    }

    /// `PUT` of the bytes from `offset` to the end of a file of `size` bytes that holds `pattern`. With `cut_after` the body stops after
    /// that many bytes and `None` comes back (the caller drops the connection, as a phone does when the Wi-Fi goes). Otherwise the
    /// answer and how long it took.
    pub async fn put(
        &mut self,
        transfer: &str,
        file: &str,
        offset: u64,
        size: u64,
        pattern: &Pattern,
        cut_after: Option<u64>,
    ) -> Option<(HttpResponse, Duration)> {
        let length = size - offset;
        let head = format!(
            "PUT /v1/transfers/{transfer}/files/{file}?offset={offset} HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\n\r\n"
        );
        let started = Instant::now();
        self.stream.write_all(head.as_bytes()).await.unwrap();
        let to_send = cut_after.map_or(length, |c| c.min(length));
        let mut buf = vec![0u8; BLOCK];
        let mut sent = 0u64;
        while sent < to_send {
            let n = (to_send - sent).min(BLOCK as u64) as usize;
            pattern.fill(offset + sent, &mut buf[..n]);
            if self.stream.write_all(&buf[..n]).await.is_err() {
                break;
            }
            sent += n as u64;
        }
        if cut_after.is_some_and(|c| c < length) {
            let _ = self.stream.flush().await;
            return None;
        }
        // The PC answers an error at once, before the body is read, and closes the connection. A phone that is still sending then
        // gets a write error. The answer can be lost with the connection, so a test that wants to see an error answer keeps its
        // body small.
        let _ = self.stream.flush().await;
        let response = super::read_response(&mut self.stream).await.ok()?;
        Some((response, started.elapsed()))
    }

    /// `PUT` of a whole file given as bytes (offset 0). The answer, or `None` if the connection went away.
    pub async fn put_bytes(&mut self, transfer: &str, file: &str, bytes: &[u8]) -> Option<HttpResponse> {
        let head = format!(
            "PUT /v1/transfers/{transfer}/files/{file}?offset=0 HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
            bytes.len()
        );
        self.stream.write_all(head.as_bytes()).await.ok()?;
        self.stream.write_all(bytes).await.ok()?;
        self.stream.flush().await.ok()?;
        super::read_response(&mut self.stream).await.ok()
    }

    /// `GET` of a file, with `Range: bytes=<from>-` when `from` is given. The body is read as it comes and checked against `pattern`
    /// (the file starts at `from`). With `stop_after` it stops reading after that many bytes (the caller drops the connection).
    pub async fn get(
        &mut self,
        transfer: &str,
        file: &str,
        from: Option<u64>,
        pattern: Option<&Pattern>,
        stop_after: Option<u64>,
    ) -> Download {
        let range = from.map_or(String::new(), |n| format!("Range: bytes={n}-\r\n"));
        let request = format!("GET /v1/transfers/{transfer}/files/{file} HTTP/1.1\r\nHost: kin\r\n{range}\r\n");
        let started = Instant::now();
        self.stream.write_all(request.as_bytes()).await.unwrap();
        self.stream.flush().await.unwrap();
        let mut buf = Vec::new();
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            let mut chunk = [0u8; 4096];
            let n = self.stream.read(&mut chunk).await.unwrap();
            assert!(n > 0, "the connection closed before the response head");
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let status: u16 = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        let headers: Vec<(String, String)> =
            lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
        let length: u64 = headers.iter().find(|(n, _)| n.eq_ignore_ascii_case("content-length")).map_or(0, |(_, v)| v.parse().unwrap());
        let mut body: Vec<u8> = buf[head_end + 4..].to_vec();
        let (mut received, mut intact) = (0u64, true);
        let base = from.unwrap_or(0);
        let mut chunk = vec![0u8; BLOCK];
        let ok_status = status == 200 || status == 206;
        loop {
            if !body.is_empty() {
                if ok_status && let Some(p) = pattern {
                    intact &= p.matches(base + received, &body);
                }
                received += body.len() as u64;
                body.clear();
            }
            if received >= length || stop_after.is_some_and(|s| received >= s) {
                break;
            }
            let n = self.stream.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        // An error response has a JSON body that was read above (it is short): the caller can look at `received` only.
        Download { status, headers, received, intact, elapsed: started.elapsed() }
    }
}

/// A 1 x 1 PNG: a real picture, for the tests that need the decoder to say "image".
pub const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63,
    0xFC, 0xCF, 0xC0, 0x50, 0x0F, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xA9, 0x8C, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

/// Megabytes per second.
pub fn mb_per_s(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / 1e6 / elapsed.as_secs_f64().max(1e-9)
}

/// Waits (up to 10 s) for a condition to become true.
pub async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}
