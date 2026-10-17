//! A fake phone for manual checks of a running Kin, with no Android involved. It pairs from a pairing URL, opens the WebSocket,
//! says `hello`, acks every `ping`, `text` and `clipboard` it gets, can send pings and `text` messages of its own, and (1.3) uploads
//! files to the PC and downloads the files the PC offers.
//!
//! ```text
//! cargo run --example fake_phone -- "<kinrelay://pair?...>" --host 127.0.0.1 [--name "Fake Phone"] [--seconds 30] [--ping-every 5]
//!     [--send-text "some text" [--send-text-after 2]] [--no-ack]
//!     [--upload <file>]... [--upload-generated <bytes>]... [--upload-after 3] [--upload-cut <bytes>]
//!     [--save-dir <folder>] [--download-cut <bytes>] [--no-download] [--refuse-offer <code>]
//!     [--offline-after <s> [--offline-for <s>]] [--identity-file <path>] [--skip-pairing]
//! ```
//!
//! * `--send-text <text>` sends one `text` message `--send-text-after` seconds (default 2) after the `hello`, with `isUrl` set when the
//!   whole text is one http(s) link, and prints how long its `ack` took. Repeat the flag to send several, one per second.
//! * (2B.1) The phone says `"capabilities": ["link"]` in its `hello` (`--old-phone` leaves it out, like a 0.1.x phone). `--send-link <url>` (repeat it;
//!   `--send-link-title <title>` gives each a title) sends a `link` message the way `--send-text` sends a text. A `link` from the PC is acked and printed as
//!   `received link id=<id> url=<url> title=<title> sourceApp=<app>`.
//! * `--no-ack` doesn't ack `text` and `clipboard` from the PC (to see the PC's 5 s ack timeout and its notice).
//! * What it receives is printed on one line each, for scripts to look for: `received clipboard id=<id> text=<text>` and
//!   `received text id=<id> isUrl=<bool> text=<text>`. (This is a developer tool: it prints the text. Kin itself never logs it.)
//!
//! **Files, phone to PC.** `--upload <file>` (repeat it for several files) and `--upload-generated <bytes>` (a file of that many bytes of a
//! fixed pattern, never written to disk) are sent as one transfer `--upload-after` seconds (default 3) after the `hello`, each file
//! with `PUT` after one `POST /v1/transfers`. With `--upload-cut <bytes>` the connection of the first file that is bigger than that is dropped after that many
//! bytes and the transfer resumes: a new connection, a new `POST`, and a `PUT` from the offset the PC reports. It prints one line per
//! file for scripts: `uploaded <name> bytes=<n> sha256=<hex> seconds=<s> mbps=<MB/s>` (the hash is of the whole file, to compare
//! with the received one), and `upload done: <transfer id>`.
//!
//! **Files, PC to phone.** When the PC sends an `offer`, the phone acks it (or, with `--refuse-offer <code>`, answers with an `error`
//! of that code, such as `disk_full`) and downloads every file into `--save-dir` (default `fake-phone-downloads` in the temp folder),
//! as `<name>.part` and then `<name>`, resuming with `Range` from what is already there. With `--download-cut <bytes>` the first GET is
//! dropped after that many bytes to show the resume. It prints `downloaded <name> bytes=<n> sha256=<hex> seconds=<s> mbps=<MB/s>`
//! for each file, then sends `complete`. `--no-download` acks and leaves the offer alone (to watch it expire or be canceled). A
//! `cancel` from the PC is acked and printed. A file that can't be downloaded (404, 409) cancels the whole offer with reason `error`.
//!
//! **Going away and coming back (1.4, the queue).** `--offline-after <s>` closes the WebSocket that many seconds after the `hello`, stays away
//! for `--offline-for <s>` (default 5) and then connects again with the same key and a new `hello`. It prints `going offline at=<unix ms>` and
//! `reconnected at=<unix ms>`, and **one line for every message the PC sends**, `arrival type=<type> id=<id> at=<unix ms>
//! ms_since_hello=<n>`: what a script reads to see that what the PC queued while the phone was away arrives, in order, within seconds
//! of the reconnect.
//!
//! `--identity-file <path>` keeps the phone's key in that file (made on the first run), so a later run is the same phone, and `--skip-pairing`
//! connects without pairing (the phone is paired already): together they test what happens across a restart of the PC.
//!
//! `--host` replaces the host of the first address in the URL (in developer mode that is `10.0.2.2`, the emulator's alias for
//! the PC, which the PC itself can't reach). The port is the one in the URL. The phone's key is made fresh on every run.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use kin_lib::encoding::{device_id_from_spki, spki_of_certificate, unix_ms, uuid_v7};
use kin_lib::identity::Identity;
use kin_lib::tls::crypto_provider;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug)]
struct Pinned {
    device_id: String,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match spki_of_certificate(end_entity).map(device_id_from_spki) {
            Some(id) if id == self.device_id => Ok(ServerCertVerified::assertion()),
            _ => Err(TlsError::General("the PC's key is not the one in the pairing URL".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is not allowed".into()))
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(m, c, d, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// The phone's `hello`. A 2B.1 phone lists the `link` capability; `--old-phone` is a 0.1.x phone, which doesn't.
fn hello_of(name: &str, id: &str, old_phone: bool) -> Value {
    let mut hello = json!({ "type": "hello", "id": id, "protocol": 1, "name": name, "platform": "android", "appVersion": "0.0.0" });
    if !old_phone {
        hello["capabilities"] = json!(["link"]);
    }
    hello
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Every value of a flag that can be given more than once.
fn all_args(args: &[String], name: &str) -> Vec<String> {
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1)).cloned().collect()
}

/// The same rule as `isUrl` in protocol.md, for this tool: the whole text, trimmed, is one http(s) URL.
fn is_url(text: &str) -> bool {
    kin_lib::links::is_http_url(text)
}

fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|p| p.strip_prefix(name)?.strip_prefix('='))
}

fn say(msg: &str) {
    println!("[fake phone] {msg}");
}

/// A way to open TLS connections to the PC.
#[derive(Clone)]
struct Net {
    connector: TlsConnector,
    host: String,
    port: u16,
}

impl Net {
    async fn connect(&self) -> TlsStream<TcpStream> {
        let tcp = TcpStream::connect((self.host.as_str(), self.port)).await.expect("connect");
        self.connector.connect(ServerName::try_from("kin").unwrap(), tcp).await.expect("TLS handshake")
    }
}

// ----- a minimal HTTP/1.1 client ------------------------------------------------------------------------------------------

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
    /// Body bytes that came in with the head.
    rest: Vec<u8>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

async fn read_head(tls: &mut TlsStream<TcpStream>) -> std::io::Result<Head> {
    let mut buf = Vec::new();
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let mut chunk = [0u8; 4096];
        let n = tls.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the connection closed before a response"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let text = String::from_utf8_lossy(&buf[..end]).to_string();
    let mut lines = text.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
    Ok(Head { status, headers, rest: buf[end + 4..].to_vec() })
}

/// The body of a response that has a `Content-Length`, read whole (for the small JSON ones).
async fn read_body(tls: &mut TlsStream<TcpStream>, head: &mut Head) -> std::io::Result<Vec<u8>> {
    let length: usize = head.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = std::mem::take(&mut head.rest);
    while body.len() < length {
        let mut chunk = vec![0u8; length - body.len()];
        let n = tls.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Ok(body)
}

async fn json_request(tls: &mut TlsStream<TcpStream>, method: &str, path: &str, body: &Value) -> std::io::Result<(u16, Value)> {
    let body = body.to_string();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: kin\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(request.as_bytes()).await?;
    let mut head = read_head(tls).await?;
    let bytes = read_body(tls, &mut head).await?;
    Ok((head.status, serde_json::from_slice(&bytes).unwrap_or(Value::Null)))
}

// ----- uploading ----------------------------------------------------------------------------------------------------------

#[derive(Clone)]
enum Source {
    File(PathBuf),
    /// A file of a fixed pattern that is never written anywhere.
    Generated,
}

#[derive(Clone)]
struct Upload {
    name: String,
    size: u64,
    source: Source,
}

fn fill(source: &Source, abs: u64, buf: &mut [u8]) -> std::io::Result<()> {
    match source {
        Source::File(path) => {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(path)?;
            f.seek(SeekFrom::Start(abs))?;
            f.read_exact(buf)
        }
        Source::Generated => {
            for (i, b) in buf.iter_mut().enumerate() {
                let p = abs + i as u64;
                *b = (p.wrapping_mul(2_654_435_761) >> 13) as u8 ^ (p >> 20) as u8;
            }
            Ok(())
        }
    }
}

fn hash_of(u: &Upload) -> String {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut pos = 0u64;
    while pos < u.size {
        let n = (u.size - pos).min(buf.len() as u64) as usize;
        fill(&u.source, pos, &mut buf[..n]).expect("the file can be read");
        hasher.update(&buf[..n]);
        pos += n as u64;
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

async fn post_transfer(tls: &mut TlsStream<TcpStream>, id: &str, files: &[Value]) -> std::io::Result<(u16, Value)> {
    json_request(tls, "POST", "/v1/transfers", &json!({ "id": id, "files": files })).await
}

/// One transfer: a `POST`, then a `PUT` for every file from the offset the PC reports. With `cut` the first connection is dropped after
/// that many bytes of the first file, and everything resumes on a new one.
async fn upload(net: &Net, uploads: Vec<Upload>, cut: Option<u64>) {
    // The hashes first: a connection that sits idle for 30 s (while a big file is hashed) is closed by the PC.
    let hashes: Vec<String> = uploads.iter().map(hash_of).collect();
    let id = uuid_v7(unix_ms());
    let files: Vec<Value> = uploads
        .iter()
        .enumerate()
        .map(|(i, u)| json!({ "fileId": format!("f{}", i + 1), "name": u.name, "size": u.size, "mime": "application/octet-stream" }))
        .collect();
    let mut tls = net.connect().await;
    let (status, answer) = post_transfer(&mut tls, &id, &files).await.expect("POST /v1/transfers");
    if status != 200 {
        say(&format!("upload refused: {status} {answer}"));
        return;
    }
    let offsets =
        |answer: &Value| -> Vec<u64> { answer["files"].as_array().unwrap().iter().map(|f| f["offset"].as_u64().unwrap()).collect() };
    let mut offsets_now = offsets(&answer);
    // The cut is for the first file that is bigger than it.
    let cut_file = cut.and_then(|c| uploads.iter().position(|u| u.size > c));
    let mut cut = cut;
    for (i, u) in uploads.iter().enumerate() {
        let started = Instant::now();
        let sha = hashes[i].clone();
        loop {
            let offset = offsets_now[i];
            let length = u.size - offset;
            let head = format!(
                "PUT /v1/transfers/{id}/files/f{}?offset={offset} HTTP/1.1\r\nHost: kin\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\n\r\n",
                i + 1
            );
            if let Err(e) = tls.write_all(head.as_bytes()).await {
                say(&format!("upload of {} failed: {e}", u.name));
                return;
            }
            let to_send = if Some(i) == cut_file { cut.map_or(length, |c| c.min(length)) } else { length };
            let mut buf = vec![0u8; 1 << 20];
            let mut sent = 0u64;
            while sent < to_send {
                let n = (to_send - sent).min(buf.len() as u64) as usize;
                fill(&u.source, offset + sent, &mut buf[..n]).expect("the file can be read");
                if tls.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                sent += n as u64;
            }
            if Some(i) == cut_file && cut.is_some_and(|c| c < length) {
                say(&format!("dropping the connection after {} bytes of {}, then resuming", sent, u.name));
                cut = None;
                drop(tls);
                tls = net.connect().await;
                let (status, answer) = post_transfer(&mut tls, &id, &files).await.expect("POST /v1/transfers (resume)");
                assert_eq!(status, 200, "the resume POST: {answer}");
                offsets_now = offsets(&answer);
                say(&format!("the PC has {} bytes of {}", offsets_now[i], u.name));
                continue;
            }
            let _ = tls.flush().await;
            let mut head = match read_head(&mut tls).await {
                Ok(h) => h,
                Err(e) => {
                    say(&format!("upload of {} failed: {e}", u.name));
                    return;
                }
            };
            let body = read_body(&mut tls, &mut head).await.unwrap_or_default();
            if head.status == 409 {
                // Stale offset: the PC says where it is.
                let at = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["offset"].as_u64()).unwrap_or(0);
                say(&format!("offset mismatch on {}: the PC has {at}, starting again from there", u.name));
                offsets_now[i] = at;
                tls = net.connect().await;
                continue;
            }
            if head.status != 200 {
                say(&format!("upload of {} failed: {} {}", u.name, head.status, String::from_utf8_lossy(&body)));
                return;
            }
            let seconds = started.elapsed().as_secs_f64();
            say(&format!(
                "uploaded {} bytes={} sha256={sha} seconds={seconds:.2} mbps={:.0}",
                u.name,
                u.size,
                u.size as f64 / 1e6 / seconds.max(1e-9)
            ));
            break;
        }
    }
    say(&format!("upload done: {id}"));
}

// ----- downloading --------------------------------------------------------------------------------------------------------

/// Downloads every file of an offer into `save_dir`. Returns the message to send afterwards: `complete`, or `cancel` with reason
/// `error` when a file can't be had.
async fn download(net: &Net, offer: &Value, save_dir: &Path, cut: Option<u64>) -> Value {
    let transfer = offer["id"].as_str().unwrap().to_string();
    std::fs::create_dir_all(save_dir).expect("the save folder");
    let mut cut = cut;
    for file in offer["files"].as_array().unwrap() {
        let (file_id, size) = (file["fileId"].as_str().unwrap(), file["size"].as_u64().unwrap());
        let existing: Vec<String> = std::fs::read_dir(save_dir)
            .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let name = kin_lib::filenames::clean_file_name(file["name"].as_str().unwrap(), &existing);
        let part = save_dir.join(format!("{name}.part"));
        let started = Instant::now();
        let mut attempts = 0;
        loop {
            let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0).min(size);
            if have == size && std::fs::metadata(&part).is_ok() {
                break;
            }
            attempts += 1;
            if attempts > 5 {
                say(&format!("giving up on {name}"));
                return json!({ "type": "cancel", "id": uuid_v7(unix_ms()), "transferId": transfer, "reason": "error" });
            }
            let mut tls = net.connect().await;
            let range = if have > 0 { format!("Range: bytes={have}-\r\n") } else { String::new() };
            tls.write_all(format!("GET /v1/transfers/{transfer}/files/{file_id} HTTP/1.1\r\nHost: kin\r\n{range}\r\n").as_bytes())
                .await
                .unwrap();
            let mut head = read_head(&mut tls).await.expect("a response");
            if head.status == 404 || head.status == 409 {
                say(&format!("{name}: {} from the PC, the offer is over", head.status));
                return json!({ "type": "cancel", "id": uuid_v7(unix_ms()), "transferId": transfer, "reason": "error" });
            }
            if head.status != 200 && head.status != 206 {
                let body = read_body(&mut tls, &mut head).await.unwrap_or_default();
                say(&format!("{name}: {} {}", head.status, String::from_utf8_lossy(&body)));
                continue;
            }
            // A 200 to a Range request means the whole file again.
            let from = if head.status == 206 { have } else { 0 };
            let mut out = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&part).unwrap();
            out.set_len(from).unwrap();
            std::io::Seek::seek(&mut out, std::io::SeekFrom::Start(from)).unwrap();
            let length: u64 = head.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
            let mut received = 0u64;
            let mut chunk = vec![0u8; 1 << 20];
            let mut pending = std::mem::take(&mut head.rest);
            loop {
                if !pending.is_empty() {
                    std::io::Write::write_all(&mut out, &pending).unwrap();
                    received += pending.len() as u64;
                    pending.clear();
                }
                if received >= length {
                    break;
                }
                if let Some(c) = cut
                    && received >= c
                {
                    say(&format!("dropping the connection after {received} bytes of {name}, then resuming"));
                    cut = None;
                    break;
                }
                match tls.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => pending.extend_from_slice(&chunk[..n]),
                }
            }
        }
        let final_path = save_dir.join(&name);
        std::fs::rename(&part, &final_path).expect("rename .part to the final name");
        let seconds = started.elapsed().as_secs_f64();
        let mut hasher = Sha256::new();
        let mut f = std::fs::File::open(&final_path).unwrap();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = std::io::Read::read(&mut f, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let sha: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        say(&format!("downloaded {name} bytes={size} sha256={sha} seconds={seconds:.2} mbps={:.0}", size as f64 / 1e6 / seconds.max(1e-9)));
    }
    json!({ "type": "complete", "id": uuid_v7(unix_ms()), "transferId": transfer })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = args.first().expect("usage: fake_phone <pairing URL> [--host H] [--name N] [--seconds S] [--ping-every S]");
    let query = url.split_once('?').expect("not a pairing URL").1;
    let skip_pairing = args.iter().any(|a| a == "--skip-pairing");
    let (token, pc_id) = (param(query, "tk").unwrap_or(""), param(query, "id").expect("no id"));
    if token.is_empty() && !skip_pairing {
        panic!("no tk in the URL (use --skip-pairing for a phone that is paired already)");
    }
    let first = param(query, "a").expect("no a").split(',').next().unwrap();
    let port: u16 = first.rsplit_once(':').unwrap().1.parse().unwrap();
    let host = arg(&args, "--host").unwrap_or_else(|| first.rsplit_once(':').unwrap().0.to_string());
    let name = arg(&args, "--name").unwrap_or_else(|| "Fake Phone".to_string());
    let seconds: u64 = arg(&args, "--seconds").map_or(30, |s| s.parse().unwrap());
    let ping_every: Option<u64> = arg(&args, "--ping-every").map(|s| s.parse().unwrap());
    let old_phone = args.iter().any(|a| a == "--old-phone");
    let link_title = arg(&args, "--send-link-title");
    // What the phone sends on its own: texts and links, one a second.
    let texts: Vec<(&'static str, String)> = all_args(&args, "--send-text")
        .into_iter()
        .map(|t| ("text", t))
        .chain(all_args(&args, "--send-link").into_iter().map(|u| ("link", u)))
        .collect();
    let send_text_after: u64 = arg(&args, "--send-text-after").map_or(2, |s| s.parse().unwrap());
    let no_ack = args.iter().any(|a| a == "--no-ack");
    let mut uploads: Vec<Upload> = all_args(&args, "--upload")
        .into_iter()
        .map(|p| {
            let path = PathBuf::from(&p);
            let size = std::fs::metadata(&path).unwrap_or_else(|e| panic!("{p}: {e}")).len();
            Upload { name: path.file_name().unwrap().to_string_lossy().into_owned(), size, source: Source::File(path) }
        })
        .collect();
    for (i, size) in all_args(&args, "--upload-generated").into_iter().enumerate() {
        uploads.push(Upload {
            name: format!("generated-{}.bin", i + 1),
            size: size.parse().expect("a number of bytes"),
            source: Source::Generated,
        });
    }
    let upload_after: u64 = arg(&args, "--upload-after").map_or(3, |s| s.parse().unwrap());
    let upload_cut: Option<u64> = arg(&args, "--upload-cut").map(|s| s.parse().unwrap());
    let save_dir = arg(&args, "--save-dir").map_or_else(|| std::env::temp_dir().join("fake-phone-downloads"), PathBuf::from);
    let download_cut: Option<u64> = arg(&args, "--download-cut").map(|s| s.parse().unwrap());
    let no_download = args.iter().any(|a| a == "--no-download");
    let refuse_offer = arg(&args, "--refuse-offer");
    let offline_after: Option<u64> = arg(&args, "--offline-after").map(|s| s.parse().unwrap());
    let offline_for: u64 = arg(&args, "--offline-for").map_or(5, |s| s.parse().unwrap());

    let me = match arg(&args, "--identity-file") {
        Some(path) => match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            Some(v) => {
                let hex = |k: &str| -> Vec<u8> {
                    let s = v[k].as_str().expect("the identity file has the key and the certificate");
                    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
                };
                Identity::from_der(hex("key"), hex("cert")).expect("the identity file is valid")
            }
            None => {
                let me = Identity::generate().unwrap();
                let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
                std::fs::write(&path, json!({ "key": hex(&me.key_der), "cert": hex(&me.cert_der) }).to_string())
                    .expect("write the identity file");
                me
            }
        },
        None => Identity::generate().unwrap(),
    };
    say(&format!("my device ID is {}", me.device_id));
    let provider = crypto_provider();
    let verifier = Pinned { device_id: pc_id.to_string(), algs: provider.signature_verification_algorithms };
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_auth_cert(
            vec![CertificateDer::from(me.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(me.key_der.clone())),
        )
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let net = Net { connector: TlsConnector::from(Arc::new(config)), host: host.clone(), port };

    // Pair.
    if !skip_pairing {
        let mut tls = net.connect().await;
        let body = json!({ "token": token, "name": name, "platform": "android", "model": "Fake", "appVersion": "0.0.0", "protocol": 1 })
            .to_string();
        let request = format!(
            "POST /v1/pair HTTP/1.1\r\nHost: kin\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        tls.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        let _ = tls.read_to_string(&mut response).await;
        let status = response.lines().next().unwrap_or("");
        say(&format!("pair: {status} {}", response.rsplit("\r\n\r\n").next().unwrap_or("")));
        assert!(status.contains(" 200"), "pairing failed");
    }

    // WebSocket.
    let (mut ws, _) =
        tokio_tungstenite::client_async(format!("ws://{host}:{port}/v1/ws"), net.connect().await).await.expect("WebSocket upgrade");
    let hello = hello_of(&name, "fake-hello", old_phone);
    ws.send(Message::Text(hello.to_string().into())).await.unwrap();
    say("connected, hello sent");
    let mut hello_sent_at = Instant::now();
    let mut offline_at = offline_after.map(|s| Instant::now() + Duration::from_secs(s));
    let mut hellos = 1;

    // Messages from the download tasks (a `complete` or `cancel`) go out on the WebSocket from here.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut next_ping = ping_every.map(|s| Instant::now() + Duration::from_secs(s));
    let mut sent: Option<(String, Instant)> = None;
    let mut counter = 0;
    let mut texts = texts.into_iter().enumerate();
    let mut next_text = Instant::now() + Duration::from_secs(send_text_after);
    let mut text_sent: Option<(String, Instant)> = None;
    let mut pending_text = texts.next();
    let mut upload_at = (!uploads.is_empty()).then(|| Instant::now() + Duration::from_secs(upload_after));
    loop {
        let mut wake = next_ping.map_or(deadline, |p| p.min(deadline));
        if pending_text.is_some() {
            wake = wake.min(next_text);
        }
        if let Some(at) = upload_at {
            wake = wake.min(at);
        }
        if let Some(at) = offline_at {
            wake = wake.min(at);
        }
        tokio::select! {
            () = tokio::time::sleep_until(wake.into()) => {
                if Instant::now() >= deadline { break; }
                if offline_at.is_some_and(|at| Instant::now() >= at) {
                    // The phone goes away (Wi-Fi off) and comes back.
                    offline_at = None;
                    say(&format!("going offline at={}", unix_ms()));
                    let _ = ws.close(None).await;
                    tokio::time::sleep(Duration::from_secs(offline_for)).await;
                    let (new_ws, _) = tokio_tungstenite::client_async(format!("ws://{host}:{port}/v1/ws"), net.connect().await).await.expect("WebSocket upgrade after the reconnect");
                    ws = new_ws;
                    hellos += 1;
                    let hello = hello_of(&name, &format!("fake-hello-{hellos}"), old_phone);
                    ws.send(Message::Text(hello.to_string().into())).await.unwrap();
                    hello_sent_at = Instant::now();
                    say(&format!("reconnected at={} (hello sent)", unix_ms()));
                    continue;
                }
                if upload_at.is_some_and(|at| Instant::now() >= at) {
                    upload_at = None;
                    let (net, uploads) = (net.clone(), std::mem::take(&mut uploads));
                    tokio::spawn(async move { upload(&net, uploads, upload_cut).await });
                    continue;
                }
                if let Some((i, (kind, text))) = pending_text.take().filter(|_| Instant::now() >= next_text) {
                    let id = format!("fake-{kind}-{}", i + 1);
                    let frame = if kind == "link" {
                        let mut f = json!({ "type": "link", "id": id, "url": text, "sourceApp": "Fake Phone" });
                        if let Some(t) = &link_title {
                            f["title"] = json!(t);
                        }
                        f
                    } else {
                        json!({ "type": "text", "id": id, "text": text, "isUrl": is_url(&text) })
                    };
                    ws.send(Message::Text(frame.to_string().into())).await.unwrap();
                    say(&format!("sent {kind} id={id} isUrl={}", is_url(&text)));
                    text_sent = Some((id, Instant::now()));
                    pending_text = texts.next();
                    next_text = Instant::now() + Duration::from_secs(1);
                    continue;
                }
                if (pending_text.is_some() || upload_at.is_some()) && next_ping.is_none_or(|p| Instant::now() < p) { continue; }
                counter += 1;
                let id = format!("fake-ping-{counter}");
                ws.send(Message::Text(json!({ "type": "ping", "id": id, "sentAt": kin_lib::encoding::unix_ms() }).to_string().into())).await.unwrap();
                sent = Some((id, Instant::now()));
                next_ping = ping_every.map(|s| Instant::now() + Duration::from_secs(s));
            }
            Some(message) = out_rx.recv() => {
                say(&format!("sending {}", message["type"]));
                ws.send(Message::Text(message.to_string().into())).await.unwrap();
            }
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["type"] != "ack" && v["type"] != "hello" {
                        say(&format!("arrival type={} id={} at={} ms_since_hello={}", v["type"].as_str().unwrap_or("?"), v["id"].as_str().unwrap_or("-"), unix_ms(), hello_sent_at.elapsed().as_millis()));
                    }
                    match v["type"].as_str() {
                        Some("ping") => {
                            say(&format!("ping from the PC ({}): acking", v["id"]));
                            ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                        }
                        Some("ack") => {
                            if let Some((id, at)) = sent.take().filter(|(id, _)| v["ref"] == *id) {
                                say(&format!("ack for {id} after {} ms", at.elapsed().as_millis()));
                            } else if let Some((id, at)) = text_sent.take().filter(|(id, _)| v["ref"] == *id) {
                                say(&format!("ack for text {id} after {} ms", at.elapsed().as_millis()));
                            }
                        }
                        Some("link") => {
                            say(&format!(
                                "received link id={} url={} title={} sourceApp={}",
                                v["id"],
                                v["url"].as_str().unwrap_or(""),
                                v["title"].as_str().unwrap_or(""),
                                v["sourceApp"].as_str().unwrap_or("")
                            ));
                            if no_ack {
                                say("not acking (--no-ack)");
                            } else {
                                ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                            }
                        }
                        Some(kind @ ("clipboard" | "text")) => {
                            let text = v["text"].as_str().unwrap_or("");
                            if kind == "text" {
                                say(&format!("received text id={} isUrl={} sensitive={} text={text}", v["id"], v["isUrl"], v["sensitive"] == json!(true)));
                            } else {
                                say(&format!("received clipboard id={} text={text}", v["id"]));
                            }
                            if no_ack {
                                say("not acking (--no-ack)");
                            } else {
                                ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                            }
                        }
                        Some("offer") => {
                            let files = v["files"].as_array().map_or(0, Vec::len);
                            say(&format!("received offer id={} files={files} totalSize={}", v["id"], v["totalSize"]));
                            if let Some(code) = &refuse_offer {
                                say(&format!("refusing the offer with {code}"));
                                ws.send(Message::Text(json!({ "type": "error", "ref": v["id"], "code": code, "message": "refused by the fake phone" }).to_string().into())).await.unwrap();
                            } else {
                                ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                                if no_download {
                                    say("not downloading (--no-download)");
                                } else {
                                    let (net, save_dir, out_tx) = (net.clone(), save_dir.clone(), out_tx.clone());
                                    tokio::spawn(async move {
                                        let after = download(&net, &v, &save_dir, download_cut).await;
                                        let _ = out_tx.send(after);
                                    });
                                }
                            }
                        }
                        Some("cancel") => {
                            say(&format!("the PC canceled transfer {} ({})", v["transferId"], v["reason"]));
                            ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                        }
                        Some("unpair") => {
                            say("the PC unpaired this phone");
                            ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                        }
                        _ => say(&format!("received {v}")),
                    }
                }
                Some(Ok(Message::Close(frame))) => { say(&format!("closed by the PC: {frame:?}")); return; }
                Some(Ok(_)) => {}
                other => { say(&format!("connection ended: {other:?}")); return; }
            }
        }
    }
    say("done, closing");
    let _ = ws.close(None).await;
}
