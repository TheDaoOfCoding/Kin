//! Kin downloads an image URL itself (docs/extension.md, "Images: URL first, bytes as a fallback"): `GET`, **no cookies and no credentials**,
//! redirects followed up to 5 (only `http` and `https`, **never from https to http**), **30 seconds in total**, **at most 64 MiB** (the read
//! stops at the cap, it does not wait for the end), a normal browser's `User-Agent`, `Referer: <pageUrl>` when given. The result counts as an
//! image only if its **content** says so (the magic bytes of PNG, JPEG, GIF, WebP, BMP, TIFF, AVIF or HEIC; the contract also allows "the decoder opens it", which
//! Kin leaves out: the formats above are what a browser shows, and a byte check needs no COM);
//! `Content-Type` is never trusted. SVG is not accepted (it can carry script).
//!
//! The bytes never go back to the extension or to a page: they only go to the phone. The URL may point at the user's own network (they picked
//! that image on a page); that is allowed and nothing is returned to any page.
//!
//! Failures are told apart the way the contract needs: [`FetchError::TooLarge`] for a download that reached the cap (or whose `Content-Length` says
//! it would), and [`FetchError::NeedsBytes`] for every other failure, so the extension may fetch the image inside the page instead.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::links;

/// The most an image download may be: 64 MiB.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// The whole download, from connecting to the last byte.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_REDIRECTS: usize = 5;
/// A header line or the whole head may not be bigger than this.
const MAX_HEAD_LINE: usize = 16 * 1024;
const MAX_HEADERS: usize = 100;
/// A browser's `User-Agent` (a lot of image hosts refuse anything else).
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// It failed in a way the browser might get past: a status of 401, 403, 404, 429, 5xx or any other, a timeout, a connection error, or a result
    /// that isn't an image. The text is for the log.
    NeedsBytes(String),
    /// The download reached 64 MiB, or `Content-Length` says it would.
    TooLarge,
}

#[derive(Debug)]
pub struct Fetched {
    pub bytes: Vec<u8>,
    pub final_url: String,
}

pub type FetchFuture<'a> = Pin<Box<dyn Future<Output = Result<Fetched, FetchError>> + Send + 'a>>;

/// Gets the bytes of an image URL. Behind a trait so a test can say what the network answered.
pub trait ImageFetcher: Send + Sync {
    fn fetch<'a>(&'a self, url: &'a str, referer: Option<&'a str>) -> FetchFuture<'a>;
}

// ----- the image check ---------------------------------------------------------------------------------------------------

/// A picture format that the magic bytes name, with the extension a file of it gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sniffed {
    pub ext: &'static str,
}

/// The format of `bytes` by their magic bytes, or `None`.
pub fn sniff_bytes(b: &[u8]) -> Option<Sniffed> {
    let ext = if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        "png"
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        "gif"
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "webp"
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        "tif"
    } else if is_bmp(b) {
        "bmp"
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" {
        match &b[8..12] {
            b"avif" | b"avis" => "avif",
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"heif" => "heic",
            _ => return None,
        }
    } else {
        return None;
    };
    Some(Sniffed { ext })
}

/// "BM" alone would be any text file that starts with those letters: the file header's reserved fields are zero and the next header has one of the
/// known sizes.
fn is_bmp(b: &[u8]) -> bool {
    b.len() >= 26
        && &b[..2] == b"BM"
        && b[6..10] == [0, 0, 0, 0]
        && matches!(u32::from_le_bytes([b[14], b[15], b[16], b[17]]), 12 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// Whether the file extension `ext` (lower case, no dot) is one that a file of the sniffed type may have.
pub fn ext_fits(sniffed: &str, ext: &str) -> bool {
    match sniffed {
        "jpg" => matches!(ext, "jpg" | "jpeg" | "jpe" | "jfif"),
        "tif" => matches!(ext, "tif" | "tiff"),
        "heic" => matches!(ext, "heic" | "heif"),
        other => ext == other,
    }
}

/// The name an image is sent under: `name` if given, else the last segment of the URL's path (percent-decoded), else `image`; with the extension
/// of the detected type (a different short extension is replaced, none is added), cleaned like every file name (path, bidi and control characters,
/// reserved names, at most 200 bytes).
pub fn image_file_name(name: Option<&str>, url: &str, ext: &str) -> String {
    let from_url = || {
        let path = url.split_once("://").map_or(url, |(_, r)| r).split(['?', '#']).next().unwrap_or("");
        let after_host = path.split_once('/').map_or("", |(_, p)| p);
        percent_decode(after_host.rsplit('/').next().unwrap_or(""))
    };
    let chosen =
        name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).or_else(|| Some(from_url()).filter(|n| !n.trim().is_empty()));
    let chosen = chosen.unwrap_or_else(|| "image".to_string());
    // Only the last part counts (`clean` does that again); then the extension.
    let base = chosen.rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let (stem, old_ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), e.to_ascii_lowercase()),
        _ => (base.clone(), String::new()),
    };
    let with_ext = if !old_ext.is_empty() && ext_fits(ext, &old_ext) {
        base
    } else if !old_ext.is_empty() && old_ext.len() <= 5 && old_ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        format!("{stem}.{ext}")
    } else {
        format!("{base}.{ext}")
    };
    crate::filenames::clean_file_name(&with_ext, &[])
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ----- URLs --------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedUrl {
    pub https: bool,
    /// Without brackets.
    pub host: String,
    pub port: u16,
    /// The `Host` header: the host (with brackets for IPv6) and the port when it isn't the default.
    pub host_header: String,
    /// The request target: the path and the query (never the fragment, never user information).
    pub target: String,
}

impl ParsedUrl {
    /// Parses an http or https URL that has passed `links::is_link_url` (which refuses user information: no credentials are ever sent).
    pub fn parse(url: &str) -> Option<ParsedUrl> {
        if !links::is_link_url(url) {
            return None;
        }
        let (scheme, rest) = url.split_once("://")?;
        let https = scheme.eq_ignore_ascii_case("https");
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let host_port = authority; // no user information, no backslash, no escape: `links::is_link_url` refused them
        let (host, port) = match host_port.strip_prefix('[') {
            Some(v6) => {
                let (inside, after) = v6.split_once(']')?;
                (inside.to_string(), after.strip_prefix(':').map(str::to_string))
            }
            None => match host_port.split_once(':') {
                Some((h, p)) => (h.to_string(), Some(p.to_string())),
                None => (host_port.to_string(), None),
            },
        };
        let default_port = if https { 443 } else { 80 };
        let port = match port {
            Some(p) => p.parse::<u16>().ok().filter(|p| *p != 0)?,
            None => default_port,
        };
        let tail = tail.split('#').next().unwrap_or("");
        let target = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('/') {
            tail.to_string()
        } else {
            format!("/{tail}") // `https://host?x=1`
        };
        let shown_host = if host.contains(':') { format!("[{host}]") } else { host.clone() };
        let host_header = if port == default_port { shown_host } else { format!("{shown_host}:{port}") };
        Some(ParsedUrl { https, host, port, host_header, target })
    }

    fn to_url(&self) -> String {
        format!("{}://{}{}", if self.https { "https" } else { "http" }, self.host_header, self.target)
    }

    /// The URL a `Location` header names, relative to this one. `None` for anything that isn't an http or https URL after resolving.
    pub fn resolve(&self, location: &str) -> Option<ParsedUrl> {
        let location = location.trim();
        // A scheme of its own (`mailto:`, `javascript:`, `data:`, `ftp:`) is not a relative reference: only http and https go on.
        let has_scheme = location.split_once(':').is_some_and(|(s, _)| {
            s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        });
        if has_scheme
            && !location.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
            && !location.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        {
            return None;
        }
        let absolute = if let Some(rest) = location.strip_prefix("//") {
            format!("{}://{rest}", if self.https { "https" } else { "http" })
        } else if location.contains("://") {
            location.to_string()
        } else if location.starts_with('/') {
            format!("{}://{}{location}", if self.https { "https" } else { "http" }, self.host_header)
        } else if location.starts_with('?') {
            let path = self.target.split('?').next().unwrap_or("/");
            format!("{}://{}{path}{location}", if self.https { "https" } else { "http" }, self.host_header)
        } else {
            let dir = &self.target[..self.target.rfind('/').map_or(0, |i| i + 1)];
            format!(
                "{}://{}{}{location}",
                if self.https { "https" } else { "http" },
                self.host_header,
                if dir.is_empty() { "/" } else { dir }
            )
        };
        ParsedUrl::parse(&absolute)
    }
}

// ----- the client --------------------------------------------------------------------------------------------------------

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// The real fetcher. `timeout` and `max_bytes` are the contract's numbers unless a test says otherwise.
pub struct HttpFetcher {
    tls: TlsConnector,
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl Default for HttpFetcher {
    fn default() -> Self {
        HttpFetcher::new()
    }
}

impl HttpFetcher {
    pub fn new() -> HttpFetcher {
        HttpFetcher::with_roots(rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() })
    }

    /// A fetcher that trusts these roots (the tests trust a throwaway one).
    pub fn with_roots(roots: rustls::RootCertStore) -> HttpFetcher {
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the default protocol versions are valid")
            .with_root_certificates(roots)
            .with_no_client_auth();
        HttpFetcher { tls: TlsConnector::from(Arc::new(config)), timeout: TOTAL_TIMEOUT, max_bytes: MAX_IMAGE_BYTES }
    }

    async fn connect(&self, url: &ParsedUrl) -> Result<Box<dyn Stream>, FetchError> {
        let tcp = TcpStream::connect((url.host.as_str(), url.port))
            .await
            .map_err(|e| FetchError::NeedsBytes(format!("connecting failed: {e}")))?;
        let _ = tcp.set_nodelay(true);
        if !url.https {
            return Ok(Box::new(tcp));
        }
        let name = ServerName::try_from(url.host.clone()).map_err(|e| FetchError::NeedsBytes(format!("not a valid host name: {e}")))?;
        let tls = self.tls.connect(name, tcp).await.map_err(|e| FetchError::NeedsBytes(format!("the TLS handshake failed: {e}")))?;
        Ok(Box::new(tls))
    }

    async fn fetch_inner(&self, url: &str, referer: Option<&str>) -> Result<Fetched, FetchError> {
        let mut current = ParsedUrl::parse(url).ok_or_else(|| FetchError::NeedsBytes("not an http(s) URL".into()))?;
        let referer = referer.filter(|r| links::is_link_url(r));
        for hop in 0..=MAX_REDIRECTS {
            let stream = self.connect(&current).await?;
            let mut reader = BufReader::new(stream);
            let mut request = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {USER_AGENT}\r\nAccept: image/avif,image/webp,image/png,image/jpeg,image/*;q=0.8,*/*;q=0.5\r\nAccept-Encoding: identity\r\nConnection: close\r\n",
                current.target, current.host_header
            );
            if let Some(r) = referer {
                // No fragment in a Referer.
                request.push_str(&format!("Referer: {}\r\n", r.split('#').next().unwrap_or(r)));
            }
            request.push_str("\r\n");
            reader
                .get_mut()
                .write_all(request.as_bytes())
                .await
                .map_err(|e| FetchError::NeedsBytes(format!("sending the request failed: {e}")))?;
            let head = read_head(&mut reader).await?;
            match head.status {
                301 | 302 | 303 | 307 | 308 => {
                    if hop == MAX_REDIRECTS {
                        return Err(FetchError::NeedsBytes("too many redirects".into()));
                    }
                    let location = head.header("location").ok_or_else(|| FetchError::NeedsBytes("a redirect with no Location".into()))?;
                    let next = current
                        .resolve(location)
                        .ok_or_else(|| FetchError::NeedsBytes("a redirect to something that isn't an http(s) URL".into()))?;
                    if current.https && !next.https {
                        return Err(FetchError::NeedsBytes("a redirect from https to http".into()));
                    }
                    current = next;
                }
                200 => {
                    if let Some(enc) = head.header("content-encoding")
                        && !enc.trim().eq_ignore_ascii_case("identity")
                    {
                        return Err(FetchError::NeedsBytes("the server compressed the answer although it was not asked to".into()));
                    }
                    let bytes = read_body(&mut reader, &head, self.max_bytes).await?;
                    return Ok(Fetched { bytes, final_url: current.to_url() });
                }
                other => return Err(FetchError::NeedsBytes(format!("the server answered {other}"))),
            }
        }
        Err(FetchError::NeedsBytes("too many redirects".into()))
    }
}

impl ImageFetcher for HttpFetcher {
    fn fetch<'a>(&'a self, url: &'a str, referer: Option<&'a str>) -> FetchFuture<'a> {
        Box::pin(async move {
            match tokio::time::timeout(self.timeout, self.fetch_inner(url, referer)).await {
                Ok(result) => result,
                Err(_) => Err(FetchError::NeedsBytes("the download took longer than 30 seconds".into())),
            }
        })
    }
}

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

async fn read_line<R: AsyncBufReadExt + Unpin>(r: &mut R) -> Result<String, FetchError> {
    let mut line = Vec::new();
    let n = r
        .take(MAX_HEAD_LINE as u64)
        .read_until(b'\n', &mut line)
        .await
        .map_err(|e| FetchError::NeedsBytes(format!("reading failed: {e}")))?;
    if n == 0 {
        return Err(FetchError::NeedsBytes("the connection closed".into()));
    }
    if !line.ends_with(b"\n") {
        return Err(FetchError::NeedsBytes("a header line that is too long".into()));
    }
    Ok(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string())
}

async fn read_head<R: AsyncBufReadExt + Unpin>(r: &mut R) -> Result<Head, FetchError> {
    let status_line = read_line(r).await?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    let status = parts.next().and_then(|s| s.parse::<u16>().ok());
    let (true, Some(status)) = (version.starts_with("HTTP/1."), status) else {
        return Err(FetchError::NeedsBytes("not an HTTP/1.x answer".into()));
    };
    let mut headers = Vec::new();
    loop {
        let line = read_line(r).await?;
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(FetchError::NeedsBytes("too many headers".into()));
        }
        if let Some((n, v)) = line.split_once(':') {
            headers.push((n.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok(Head { status, headers })
}

/// Reads exactly `n` bytes into `out`, but never lets `out` grow past `cap`.
async fn read_exact_capped<R: AsyncReadExt + Unpin>(r: &mut R, mut n: usize, out: &mut Vec<u8>, cap: usize) -> Result<(), FetchError> {
    let mut buf = vec![0u8; 64 * 1024];
    while n > 0 {
        let want = n.min(buf.len());
        let got = r.read(&mut buf[..want]).await.map_err(|e| FetchError::NeedsBytes(format!("reading failed: {e}")))?;
        if got == 0 {
            return Err(FetchError::NeedsBytes("the connection closed before the end".into()));
        }
        if out.len() + got > cap {
            return Err(FetchError::TooLarge);
        }
        out.extend_from_slice(&buf[..got]);
        n -= got;
    }
    Ok(())
}

async fn read_body<R: AsyncBufReadExt + Unpin>(r: &mut R, head: &Head, cap: usize) -> Result<Vec<u8>, FetchError> {
    let mut out = Vec::new();
    let chunked = head.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        loop {
            let line = read_line(r).await?;
            let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| FetchError::NeedsBytes("a bad chunk size".into()))?;
            if size == 0 {
                // Trailers, up to the blank line.
                loop {
                    if read_line(r).await?.is_empty() {
                        break;
                    }
                }
                return Ok(out);
            }
            if out.len().saturating_add(size) > cap {
                return Err(FetchError::TooLarge);
            }
            read_exact_capped(r, size, &mut out, cap).await?;
            if !read_line(r).await?.is_empty() {
                return Err(FetchError::NeedsBytes("a chunk without its line end".into()));
            }
        }
    }
    if let Some(len) = head.header("content-length") {
        let len: usize = len.trim().parse().map_err(|_| FetchError::NeedsBytes("a bad Content-Length".into()))?;
        if len > cap {
            return Err(FetchError::TooLarge);
        }
        read_exact_capped(r, len, &mut out, cap).await?;
        return Ok(out);
    }
    // No length: to the end of the connection, but not past the cap.
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let got = r.read(&mut buf).await.map_err(|e| FetchError::NeedsBytes(format!("reading failed: {e}")))?;
        if got == 0 {
            return Ok(out);
        }
        if out.len() + got > cap {
            return Err(FetchError::TooLarge);
        }
        out.extend_from_slice(&buf[..got]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 1, 2, 3, 4];

    /// A one-shot HTTP server: for each connection it reads the request head and answers with the next scripted answer (raw bytes). It records the
    /// request heads, so a test can see what Kin sent.
    async fn server(answers: Vec<Vec<u8>>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            for answer in answers {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if sock.read(&mut b).await.unwrap_or(0) == 0 {
                        break;
                    }
                    head.push(b[0]);
                }
                seen2.lock().unwrap().push(String::from_utf8_lossy(&head).into_owned());
                let _ = sock.write_all(&answer).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn ok_with(content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut v = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        v.extend_from_slice(body);
        v
    }

    fn fetcher() -> HttpFetcher {
        HttpFetcher::new()
    }

    #[tokio::test]
    async fn a_plain_image_comes_back_and_the_request_has_no_cookies_and_a_browser_user_agent_and_the_referer() {
        let (base, seen) = server(vec![ok_with("image/png", PNG)]).await;
        let got = fetcher().fetch(&format!("{base}/a.png?x=1#frag"), Some("https://example.com/post#top")).await.unwrap();
        assert_eq!(got.bytes, PNG);
        let head = seen.lock().unwrap()[0].clone();
        assert!(head.starts_with("GET /a.png?x=1 HTTP/1.1\r\n"), "no fragment in the request target: {head}");
        assert!(head.contains("User-Agent: Mozilla/5.0"), "{head}");
        assert!(head.contains("Referer: https://example.com/post\r\n"), "the referer, without its fragment: {head}");
        let lower = head.to_ascii_lowercase();
        assert!(!lower.contains("cookie") && !lower.contains("authorization"), "no cookies and no credentials: {head}");
    }

    #[tokio::test]
    async fn a_url_with_user_information_is_refused_and_nothing_is_sent() {
        let (base, seen) = server(vec![ok_with("image/png", PNG)]).await;
        let url = base.replace("http://", "http://user:secret@");
        assert!(fetcher().fetch(&url, None).await.is_err());
        assert!(seen.lock().unwrap().is_empty(), "no request was made");
    }

    #[tokio::test]
    async fn statuses_that_a_browser_might_get_past_say_needs_bytes() {
        for status in [401u16, 403, 404, 429, 500, 503, 418] {
            let (base, _) = server(vec![format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\n\r\n").into_bytes()]).await;
            let err = fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap_err();
            assert!(matches!(err, FetchError::NeedsBytes(_)), "{status}: {err:?}");
        }
    }

    #[tokio::test]
    async fn redirects_are_followed_up_to_five_and_relative_ones_resolve() {
        // 1: absolute path, 2: relative, 3: protocol-relative is not used here; the last one answers.
        let (base, _) = server(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /b/c.png\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 301 Moved\r\nLocation: d.png\r\nContent-Length: 0\r\n\r\n".to_vec(),
            ok_with("image/png", PNG),
        ])
        .await;
        let got = fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap();
        assert_eq!(got.bytes, PNG);
        assert!(got.final_url.ends_with("/b/d.png"), "{}", got.final_url);
    }

    #[tokio::test]
    async fn the_sixth_redirect_is_one_too_many() {
        let hop = b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\n\r\n".to_vec();
        let (base, seen) = server(vec![hop.clone(); 7]).await;
        let err = fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap_err();
        assert_eq!(err, FetchError::NeedsBytes("too many redirects".into()));
        assert_eq!(seen.lock().unwrap().len(), 6, "the first request and five redirects, no more");
    }

    #[tokio::test]
    async fn a_redirect_to_another_scheme_is_refused() {
        for location in ["ftp://example.com/a.png", "file:///C:/Windows/notepad.exe", "javascript:alert(1)", "data:image/png;base64,AAAA"] {
            let (base, _) =
                server(vec![format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n").into_bytes()]).await;
            let err = fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap_err();
            assert!(matches!(err, FetchError::NeedsBytes(_)), "{location}: {err:?}");
        }
    }

    /// A TLS server with a throwaway certificate for 127.0.0.1, and a fetcher that trusts exactly that certificate.
    async fn tls_server(answers: Vec<Vec<u8>>) -> (String, HttpFetcher, Arc<std::sync::Mutex<Vec<String>>>) {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap().self_signed(&key).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let server_config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der()).unwrap())
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            for answer in answers {
                let Ok((sock, _)) = listener.accept().await else { return };
                let Ok(mut tls) = acceptor.accept(sock).await else { continue };
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if tls.read(&mut b).await.unwrap_or(0) == 0 {
                        break;
                    }
                    head.push(b[0]);
                }
                seen2.lock().unwrap().push(String::from_utf8_lossy(&head).into_owned());
                let _ = tls.write_all(&answer).await;
                let _ = tls.shutdown().await;
            }
        });
        (format!("https://{addr}"), HttpFetcher::with_roots(roots), seen)
    }

    #[tokio::test]
    async fn https_works_with_a_trusted_certificate_and_not_with_an_untrusted_one() {
        let (base, f, seen) = tls_server(vec![ok_with("image/png", PNG)]).await;
        assert_eq!(f.fetch(&format!("{base}/a.png"), None).await.unwrap().bytes, PNG);
        assert!(seen.lock().unwrap()[0].starts_with("GET /a.png HTTP/1.1"));
        // The default roots (the web's) don't know that certificate: the handshake fails, which says needs-bytes and sends nothing.
        let (base, _, seen) = tls_server(vec![ok_with("image/png", PNG)]).await;
        let err = fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap_err();
        assert!(matches!(err, FetchError::NeedsBytes(ref m) if m.contains("TLS")), "{err:?}");
        assert!(seen.lock().unwrap().is_empty(), "no request went over an unverified connection");
    }

    #[tokio::test]
    async fn a_redirect_from_https_to_http_is_refused_and_the_http_server_is_never_asked() {
        let (plain, plain_seen) = server(vec![ok_with("image/png", PNG)]).await;
        let (base, f, _) =
            tls_server(vec![format!("HTTP/1.1 302 Found\r\nLocation: {plain}/b.png\r\nContent-Length: 0\r\n\r\n").into_bytes()]).await;
        let err = f.fetch(&format!("{base}/a.png"), None).await.unwrap_err();
        assert_eq!(err, FetchError::NeedsBytes("a redirect from https to http".into()));
        assert!(plain_seen.lock().unwrap().is_empty(), "the http URL was never requested");
    }

    #[tokio::test]
    async fn a_redirect_from_http_to_https_is_followed() {
        let (secure, f, _) = tls_server(vec![ok_with("image/png", PNG)]).await;
        let (plain, _) =
            server(vec![format!("HTTP/1.1 301 Moved\r\nLocation: {secure}/b.png\r\nContent-Length: 0\r\n\r\n").into_bytes()]).await;
        assert_eq!(f.fetch(&format!("{plain}/a.png"), None).await.unwrap().bytes, PNG);
    }

    #[tokio::test]
    async fn a_content_length_over_the_cap_is_too_large_before_the_body_is_read() {
        let answer = b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999\r\n\r\n".to_vec();
        let (base, _) = server(vec![answer]).await;
        assert_eq!(fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap_err(), FetchError::TooLarge);
    }

    #[tokio::test]
    async fn a_download_with_no_length_stops_at_the_cap() {
        let mut f = fetcher();
        f.max_bytes = 1000;
        let mut answer = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        answer.extend(std::iter::repeat_n(b'x', 5000));
        let (base, _) = server(vec![answer]).await;
        assert_eq!(f.fetch(&format!("{base}/a.png"), None).await.unwrap_err(), FetchError::TooLarge);
    }

    #[tokio::test]
    async fn a_chunked_download_stops_at_the_cap_and_a_small_one_decodes() {
        let chunk = |n: usize| {
            let mut v = format!("{n:x}\r\n").into_bytes();
            v.extend(std::iter::repeat_n(b'x', n));
            v.extend_from_slice(b"\r\n");
            v
        };
        let mut f = fetcher();
        f.max_bytes = 1000;
        let mut big = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for _ in 0..4 {
            big.extend(chunk(400));
        }
        big.extend_from_slice(b"0\r\n\r\n");
        let (base, _) = server(vec![big]).await;
        assert_eq!(f.fetch(&format!("{base}/a.png"), None).await.unwrap_err(), FetchError::TooLarge);
        // A small chunked body is the PNG, whole.
        let mut small = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        small.extend_from_slice(format!("{:x}\r\n", PNG.len()).as_bytes());
        small.extend_from_slice(PNG);
        small.extend_from_slice(b"\r\n0\r\nX-Trailer: 1\r\n\r\n");
        let (base, _) = server(vec![small]).await;
        assert_eq!(fetcher().fetch(&format!("{base}/a.png"), None).await.unwrap().bytes, PNG);
    }

    #[tokio::test]
    async fn exactly_the_cap_is_fine() {
        let mut f = fetcher();
        f.max_bytes = PNG.len();
        let (base, _) = server(vec![ok_with("image/png", PNG)]).await;
        assert_eq!(f.fetch(&format!("{base}/a.png"), None).await.unwrap().bytes, PNG);
    }

    #[tokio::test]
    async fn a_slow_server_hits_the_total_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await; // never answers
        });
        let mut f = fetcher();
        f.timeout = Duration::from_millis(300);
        let err = f.fetch(&format!("http://{addr}/a.png"), None).await.unwrap_err();
        assert!(matches!(err, FetchError::NeedsBytes(ref m) if m.contains("longer")), "{err:?}");
    }

    #[tokio::test]
    async fn a_connection_error_and_a_non_http_answer_say_needs_bytes() {
        // Nothing listens on this port.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(matches!(fetcher().fetch(&format!("http://127.0.0.1:{port}/a.png"), None).await, Err(FetchError::NeedsBytes(_))));
        let (base, _) = server(vec![b"hello, not http\r\n\r\n".to_vec()]).await;
        assert!(matches!(fetcher().fetch(&format!("{base}/a.png"), None).await, Err(FetchError::NeedsBytes(_))));
    }

    #[tokio::test]
    async fn a_compressed_answer_that_was_not_asked_for_needs_bytes() {
        let answer = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 3\r\n\r\nabc".to_vec();
        let (base, _) = server(vec![answer]).await;
        assert!(matches!(fetcher().fetch(&format!("{base}/a.png"), None).await, Err(FetchError::NeedsBytes(_))));
    }

    #[tokio::test]
    async fn only_http_and_https_urls_can_be_fetched() {
        for bad in [
            "file:///C:/a.png",
            "ftp://example.com/a.png",
            "data:image/png;base64,AAAA",
            "javascript:1",
            "blob:https://x/1",
            " http://example.com/a",
        ] {
            assert!(matches!(fetcher().fetch(bad, None).await, Err(FetchError::NeedsBytes(_))), "{bad}");
        }
    }

    #[test]
    fn the_image_check_is_by_content_not_by_name_or_type() {
        assert_eq!(sniff_bytes(PNG), Some(Sniffed { ext: "png" }));
        assert_eq!(sniff_bytes(&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10]), Some(Sniffed { ext: "jpg" }));
        assert_eq!(sniff_bytes(b"GIF89a\x01\0\x01\0"), Some(Sniffed { ext: "gif" }));
        assert_eq!(sniff_bytes(b"RIFF\x24\0\0\0WEBPVP8 "), Some(Sniffed { ext: "webp" }));
        assert_eq!(sniff_bytes(b"II*\0\x08\0\0\0"), Some(Sniffed { ext: "tif" }));
        assert_eq!(sniff_bytes(b"\0\0\0\x1cftypavif\0\0\0\0"), Some(Sniffed { ext: "avif" }));
        assert_eq!(sniff_bytes(b"\0\0\0\x18ftypheic\0\0\0\0"), Some(Sniffed { ext: "heic" }));
        let mut bmp = vec![0u8; 30];
        bmp[0] = b'B';
        bmp[1] = b'M';
        bmp[14] = 40;
        assert_eq!(sniff_bytes(&bmp), Some(Sniffed { ext: "bmp" }));
        // Not images: an HTML sign-in page, JSON, SVG (script), text that starts with "BM", an MP4, empty.
        for not in [
            &b"<!DOCTYPE html><html>"[..],
            b"{\"error\":1}",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            b"BM is a company",
            b"\0\0\0\x18ftypmp42\0\0\0\0",
            b"",
        ] {
            assert_eq!(sniff_bytes(not), None, "{:?}", String::from_utf8_lossy(not));
        }
    }

    #[test]
    fn the_file_name_comes_from_the_name_then_the_url_then_image_and_gets_the_detected_extension() {
        assert_eq!(image_file_name(Some("photo.jpg"), "https://e.com/x/y.png", "jpg"), "photo.jpg");
        assert_eq!(image_file_name(Some("photo.jpeg"), "https://e.com/x", "jpg"), "photo.jpeg", "an extension that fits is kept");
        assert_eq!(image_file_name(None, "https://e.com/dir/cat%20pic.png?size=big#x", "png"), "cat pic.png");
        assert_eq!(image_file_name(None, "https://e.com/dir/get", "webp"), "get.webp", "no extension: the detected one is added");
        assert_eq!(image_file_name(None, "https://e.com/", "png"), "image.png");
        assert_eq!(image_file_name(None, "https://e.com", "png"), "image.png");
        assert_eq!(image_file_name(Some("  "), "https://e.com/a.gif", "gif"), "a.gif");
        assert_eq!(image_file_name(Some("shot.php"), "https://e.com/x", "png"), "shot.png", "a wrong short extension is replaced");
        assert_eq!(image_file_name(Some("report.exe"), "https://e.com/x", "png"), "report.png", "an executable name never survives");
        assert_eq!(image_file_name(Some("a.b.c.d.e.f.longerextension"), "https://e.com/x", "png"), "a.b.c.d.e.f.longerextension.png");
    }

    #[test]
    fn the_file_name_is_cleaned_like_every_file_name() {
        assert_eq!(image_file_name(Some("..\\..\\evil.png"), "https://e.com/x", "png"), "evil.png", "no path");
        assert_eq!(image_file_name(Some("a<b>:c.png"), "https://e.com/x", "png"), "a_b__c.png");
        assert_eq!(image_file_name(Some("photo\u{202e}gpj.exe"), "https://e.com/x", "png"), "photo_gpj.png", "bidi characters replaced");
        assert_eq!(image_file_name(Some("CON.png"), "https://e.com/x", "png"), "_CON.png", "reserved names");
        let long = image_file_name(Some(&format!("{}.png", "a".repeat(400))), "https://e.com/x", "png");
        assert!(long.len() <= 200 && long.ends_with(".png"), "{}", long.len());
        assert_eq!(
            image_file_name(Some("%2e%2e%2f"), "https://e.com/x", "png"),
            "%2e%2e%2f.png",
            "a name is not percent-decoded; only a URL segment is"
        );
        assert_eq!(image_file_name(None, "https://e.com/%2e%2e%2fsecret.png", "png"), "secret.png", "a decoded slash can't make a path");
    }

    #[test]
    fn urls_are_parsed_the_way_the_request_needs() {
        assert!(ParsedUrl::parse("https://user:pw@Example.com:8443/a/b?c=d#e").is_none(), "user information is refused (changelog 55)");
        let u = ParsedUrl::parse("https://Example.com:8443/a/b?c=d#e").unwrap();
        assert_eq!(
            (u.https, u.host.as_str(), u.port, u.host_header.as_str(), u.target.as_str()),
            (true, "Example.com", 8443, "Example.com:8443", "/a/b?c=d")
        );
        let u = ParsedUrl::parse("http://[::1]:8080").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.host_header.as_str(), u.target.as_str()), ("::1", 8080, "[::1]:8080", "/"));
        let u = ParsedUrl::parse("http://example.com?x=1").unwrap();
        assert_eq!((u.port, u.host_header.as_str(), u.target.as_str()), (80, "example.com", "/?x=1"));
        assert!(ParsedUrl::parse("http://example.com:0/").is_none());
        assert!(ParsedUrl::parse("ftp://example.com/").is_none());
        let base = ParsedUrl::parse("https://example.com/dir/page?x=1").unwrap();
        assert_eq!(base.resolve("/root").unwrap().target, "/root");
        assert_eq!(base.resolve("other.png").unwrap().target, "/dir/other.png");
        assert_eq!(base.resolve("//cdn.example.net/i.png").unwrap().host, "cdn.example.net");
        assert_eq!(base.resolve("?y=2").unwrap().target, "/dir/page?y=2");
        assert!(base.resolve("mailto:a@b.c").is_none());
    }

    /// Needs the internet: run by hand with `cargo test fetches_a_real_https_image -- --ignored`.
    #[tokio::test]
    #[ignore = "needs the internet"]
    async fn fetches_a_real_https_image() {
        let got = fetcher().fetch("https://www.google.com/images/branding/googlelogo/2x/googlelogo_color_92x30dp.png", None).await.unwrap();
        assert_eq!(sniff_bytes(&got.bytes), Some(Sniffed { ext: "png" }));
    }
}
