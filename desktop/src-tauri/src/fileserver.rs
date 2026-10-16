//! The HTTP side of the file transfers (protocol.md, "Files: phone -> PC" and "Files: PC -> phone"): `POST /v1/transfers`,
//! `PUT /v1/transfers/{id}/files/{fileId}?offset=N`, `DELETE /v1/transfers/{id}` and `GET /v1/transfers/{id}/files/{fileId}`.
//!
//! The order of checks is the one of the spec, and each handler keeps it: `401` (the device isn't paired) first, then the rest in
//! `transfers.rs` and `outgoing.rs`. A `PUT` never looks at `Content-Type`. The body of a `PUT` is a stream that goes straight to
//! disk, and the body of a `GET` is a stream read from disk in pieces, so no file is ever held in memory.
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, HeaderMap, RANGE};
use axum::http::{Response, StatusCode};
use hyper::body::{Bytes, Frame};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::core::{Core, short_transfer};
use crate::json::{MAX_SAFE_INT, parse_object, uint};
use crate::server::{ApiError, Limits, MAX_BODY_BYTES, Peer, is_json_content_type, read_limited};
use crate::transfers::{ActiveGuard, Entry, Failure, FileShared, FileSpec, PutParams, UploadRequest, parse_strict_u64};

/// Files are read from disk in pieces of this size, one piece ahead of the network.
const READ_PIECE: usize = 1 << 20;

fn valid_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn error_response(f: Failure) -> Response<Body> {
    let mut body = json!({ "error": f.error.code(), "message": f.message });
    if let Some(offset) = f.offset {
        body["offset"] = json!(offset);
    }
    Response::builder()
        .status(f.error.status())
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(body.to_string()))
        .expect("status and headers are valid")
}

fn plain(error: ApiError, message: &str) -> Response<Body> {
    error_response(Failure::new(error, message))
}

/// `401` unless the device is paired.
// The error is the response itself, built on the failure path and returned at once: its size doesn't matter.
#[allow(clippy::result_large_err)]
fn require_paired(core: &Core, peer: &Peer) -> Result<(), Response<Body>> {
    match core.db.is_paired(&peer.device_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(plain(ApiError::Unauthorized, "This device is not paired.")),
        Err(e) => {
            log::error!("checking the device failed: {e}");
            Err(error_response(Failure::internal()))
        }
    }
}

/// The IDs of a path with a `fileId`, or the `404` for something that can't exist.
#[allow(clippy::result_large_err)]
fn path_ids(path: Result<Path<(String, String)>, PathRejection>) -> Result<(String, String), Response<Body>> {
    match path {
        Ok(Path((t, f))) if valid_id(&t) && valid_id(&f) => Ok((t, f)),
        _ => Err(error_response(Failure::not_found())),
    }
}

// ----- POST /v1/transfers -----------------------------------------------------------------------------------------------

/// `POST /v1/transfers`: `401`, `413` (body over 1 MiB), `400` (Content-Type, JSON, fields), then everything else in
/// `Core::upload_post`.
pub async fn post_transfers(
    State(core): State<Arc<Core>>,
    Extension(peer): Extension<Peer>,
    limits: Option<Extension<Limits>>,
    headers: HeaderMap,
    body: Body,
) -> Response<Body> {
    if let Err(r) = require_paired(&core, &peer) {
        return r;
    }
    let body_timeout = limits.map_or(Limits::default().body_timeout, |Extension(l)| l.body_timeout);
    let bytes = match tokio::time::timeout(body_timeout, read_limited(body, MAX_BODY_BYTES)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(())) => return plain(ApiError::TooLarge, "The request body is over 1 MiB."),
        Err(_) => {
            let mut r = plain(ApiError::BadRequest, "The request body did not arrive in time.");
            r.headers_mut().insert(axum::http::header::CONNECTION, axum::http::HeaderValue::from_static("close"));
            return r;
        }
    };
    if !headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(is_json_content_type) {
        return plain(ApiError::BadRequest, "The request body must be JSON.");
    }
    let Some(obj) = std::str::from_utf8(&bytes).ok().and_then(parse_object) else {
        return plain(ApiError::BadRequest, "The request body is not valid JSON.");
    };
    let request = match parse_upload_request(&obj) {
        Ok(r) => r,
        Err(message) => return plain(ApiError::BadRequest, message),
    };
    match core.upload_post(&peer.device_id, &request).await {
        Ok(offsets) => {
            let files: Vec<Value> = offsets.into_iter().map(|(file_id, offset)| json!({ "fileId": file_id, "offset": offset })).collect();
            json_ok(&json!({ "id": request.id, "files": files }))
        }
        Err(f) => error_response(f),
    }
}

fn json_ok(body: &Value) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(body.to_string()))
        .expect("status and headers are valid")
}

/// The fields of the body of `POST /v1/transfers`, all required. (The messages are examples: the vectors never compare them.)
fn parse_upload_request(obj: &serde_json::Map<String, Value>) -> Result<UploadRequest, &'static str> {
    let id = obj.get("id").and_then(Value::as_str).ok_or("Missing or invalid field: id.")?;
    if !valid_id(id) {
        return Err("Invalid transfer id.");
    }
    let files = obj.get("files").and_then(Value::as_array).ok_or("Missing or invalid field: files.")?;
    if files.is_empty() {
        return Err("files must contain at least one entry.");
    }
    let mut specs = Vec::with_capacity(files.len());
    let mut seen = std::collections::HashSet::new();
    for f in files {
        let f = f.as_object().ok_or("Missing or invalid field: files.")?;
        let file_id = f.get("fileId").and_then(Value::as_str).ok_or("Missing or invalid field: fileId.")?;
        if !valid_id(file_id) {
            return Err("Invalid fileId.");
        }
        if !seen.insert(file_id.to_string()) {
            return Err("Duplicate fileId.");
        }
        let name = f.get("name").and_then(Value::as_str).ok_or("Missing or invalid field: name.")?;
        let size = f.get("size").and_then(uint).ok_or("Missing or invalid field: size.")?;
        let mime = f.get("mime").and_then(Value::as_str).ok_or("Missing or invalid field: mime.")?;
        specs.push(FileSpec { file_id: file_id.to_string(), name: name.to_string(), size, mime: mime.to_string() });
    }
    Ok(UploadRequest { id: id.to_string(), files: specs })
}

// ----- PUT /v1/transfers/{id}/files/{fileId} ---------------------------------------------------------------------------

/// `offset` from the raw query: the text of the one `offset` parameter, or `None` (missing, or given twice).
fn offset_param(query: Option<&str>) -> Option<String> {
    let mut found = None;
    for pair in query?.split('&') {
        if let Some(v) = pair.strip_prefix("offset=") {
            if found.is_some() {
                return None;
            }
            found = Some(v.to_string());
        }
    }
    found
}

pub async fn put_file(
    State(core): State<Arc<Core>>,
    Extension(peer): Extension<Peer>,
    path: Result<Path<(String, String)>, PathRejection>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Response<Body> {
    if let Err(r) = require_paired(&core, &peer) {
        return r;
    }
    let (transfer_id, file_id) = match path_ids(path) {
        Ok(ids) => ids,
        Err(r) => return r,
    };
    // `Content-Type` is not looked at: the body is raw bytes. `Content-Length` has to be there (no chunked encoding).
    let content_length = headers.get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(parse_strict_u64_lenient);
    let params = PutParams { offset: offset_param(query.as_deref()), content_length };
    match core.upload_put(&peer.device_id, &transfer_id, &file_id, params, body).await {
        Ok(size) => json_ok(&json!({ "received": size })),
        Err(f) => error_response(f),
    }
}

/// `Content-Length` was checked by hyper (digits only), so a leading zero from a sloppy client is not worth a rule of its own.
fn parse_strict_u64_lenient(text: &str) -> Option<u64> {
    text.trim().parse::<u64>().ok().filter(|n| *n <= MAX_SAFE_INT).or_else(|| parse_strict_u64(text))
}

// ----- DELETE /v1/transfers/{id} ----------------------------------------------------------------------------------------

pub async fn delete_transfer(
    State(core): State<Arc<Core>>,
    Extension(peer): Extension<Peer>,
    path: Result<Path<String>, PathRejection>,
) -> Response<Body> {
    if let Err(r) = require_paired(&core, &peer) {
        return r;
    }
    let id = match path {
        Ok(Path(id)) if valid_id(&id) => id,
        _ => return error_response(Failure::not_found()),
    };
    match core.upload_delete(&peer.device_id, &id).await {
        Ok(()) => Response::builder().status(StatusCode::NO_CONTENT).body(Body::empty()).expect("status is valid"),
        Err(f) => error_response(f),
    }
}

// ----- GET /v1/transfers/{id}/files/{fileId} ---------------------------------------------------------------------------

/// `Range: bytes=N-`, the only form supported: `N` in ASCII digits, no sign, no leading zeros, at most 2^53 - 1. `Err` is
/// anything else, including a header that is there but isn't that.
fn parse_range(value: &str) -> Result<u64, ()> {
    let rest = value.get(..6).filter(|p| p.eq_ignore_ascii_case("bytes=")).map(|_| &value[6..]).ok_or(())?;
    let digits = rest.strip_suffix('-').ok_or(())?;
    parse_strict_u64(digits).ok_or(())
}

pub async fn get_file(
    State(core): State<Arc<Core>>,
    Extension(peer): Extension<Peer>,
    path: Result<Path<(String, String)>, PathRejection>,
    headers: HeaderMap,
) -> Response<Body> {
    if let Err(r) = require_paired(&core, &peer) {
        return r;
    }
    let (transfer_id, file_id) = match path_ids(path) {
        Ok(ids) => ids,
        Err(r) => return r,
    };
    let (entry, file) = match core.offer_file(&peer.device_id, &transfer_id, &file_id).await {
        Ok(found) => found,
        Err(f) => return error_response(f),
    };
    let size = file.size;
    let unsatisfiable = || {
        let mut r = plain(ApiError::BadRequest, "The range is not satisfiable.");
        *r.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
        r.headers_mut().insert(CONTENT_RANGE, format!("bytes */{size}").parse().expect("a valid header"));
        r
    };
    let (status, start) = match headers.get(RANGE) {
        None => (StatusCode::OK, 0),
        Some(value) => match value.to_str().ok().and_then(|v| parse_range(v).ok()) {
            Some(n) if n < size => (StatusCode::PARTIAL_CONTENT, n),
            _ => return unsatisfiable(),
        },
    };
    let path = file.data().path;
    // Opening the file and moving to the first byte are file calls: they run on the blocking pool, not on this async thread.
    let opened = tokio::task::spawn_blocking(move || {
        use std::io::{Seek, SeekFrom};
        let mut f = std::fs::File::open(&path).map_err(|e| (true, e))?;
        f.seek(SeekFrom::Start(start)).map_err(|e| (false, e))?;
        Ok::<_, (bool, std::io::Error)>(f)
    })
    .await
    .unwrap_or_else(|e| Err((false, std::io::Error::other(e))));
    let opened = match opened {
        Ok(f) => f,
        Err((true, e)) => {
            log::warn!("offer {}: opening the file failed: {e}", short_transfer(&transfer_id));
            return error_response(Failure::new(ApiError::SourceChanged, "The file changed after it was offered."));
        }
        Err((false, e)) => {
            log::error!("reading the offered file failed: {e}");
            return error_response(Failure::internal());
        }
    };
    let length = size - start;
    if start > 0 {
        log::info!("offer {}: file {} is served from byte {} of {}", short_transfer(&transfer_id), file.idx + 1, start, size);
    }
    let stream = FileBody::new(opened, start, length, entry, file);
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/octet-stream")
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, length.to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        response = response.header(CONTENT_RANGE, format!("bytes {start}-{}/{size}", size - 1));
    }
    response.body(Body::new(stream)).expect("status and headers are valid")
}

/// A piece of the file being read on the blocking pool: the file comes back with it.
type ReadTask = JoinHandle<std::io::Result<(std::fs::File, Vec<u8>)>>;

/// The body of a `GET`: the file, read in pieces of 1 MiB on the blocking pool, with the next piece being read while the
/// current one goes out. It counts what it hands over (for the speed and the progress) and stops when the transfer is canceled.
struct FileBody {
    file: Option<std::fs::File>,
    remaining: u64,
    pending: Option<ReadTask>,
    entry: Arc<Entry>,
    shared: Arc<FileShared>,
    /// Where in the file this body started, and how much it has handed over.
    start: u64,
    sent: u64,
    /// Counts this body as moving the file until it is dropped (finished, canceled or abandoned by the phone).
    _active: ActiveGuard,
}

impl FileBody {
    /// `file` is already at byte `start`.
    fn new(file: std::fs::File, start: u64, length: u64, entry: Arc<Entry>, shared: Arc<FileShared>) -> FileBody {
        let _active = shared.begin();
        let mut body = FileBody { file: Some(file), remaining: length, pending: None, entry, shared, start, sent: 0, _active };
        body.shared.done.store(start, std::sync::atomic::Ordering::Relaxed);
        body.read_ahead();
        body
    }

    /// Starts reading the next piece, if there is one.
    fn read_ahead(&mut self) {
        let Some(mut file) = self.file.take() else {
            return;
        };
        if self.remaining == 0 {
            self.file = Some(file);
            return;
        }
        let want = self.remaining.min(READ_PIECE as u64) as usize;
        self.pending = Some(tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut buf = vec![0u8; want];
            file.read_exact(&mut buf)?;
            Ok((file, buf))
        }));
    }
}

impl hyper::body::Body for FileBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
        use std::sync::atomic::Ordering;
        if self.entry.canceled.load(Ordering::Relaxed) {
            return Poll::Ready(Some(Err(std::io::Error::other("the transfer was canceled"))));
        }
        let Some(task) = self.pending.as_mut() else {
            return Poll::Ready(None);
        };
        match Pin::new(task).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => {
                self.pending = None;
                Poll::Ready(Some(Err(std::io::Error::other(e))))
            }
            Poll::Ready(Ok(Err(e))) => {
                self.pending = None;
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(Ok(Ok((file, buf)))) => {
                self.pending = None;
                self.file = Some(file);
                let n = buf.len() as u64;
                self.remaining -= n;
                self.sent += n;
                let done = self.start + self.sent;
                self.shared.done.store(done, Ordering::Relaxed);
                self.entry.note_moved(n);
                self.read_ahead();
                Poll::Ready(Some(Ok(Frame::data(Bytes::from(buf)))))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0 && self.pending.is_none()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        hyper::body::SizeHint::with_exact(self.remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_header_is_one_open_ended_range_in_ascii_digits() {
        assert_eq!(parse_range("bytes=0-"), Ok(0));
        assert_eq!(parse_range("bytes=1048576-"), Ok(1_048_576));
        assert_eq!(parse_range("BYTES=5-"), Ok(5));
        for bad in [
            "bytes=+5-",
            "bytes=007-",
            "bytes=-5",
            "bytes=0-9",
            "bytes=9007199254740992-",
            "bytes=-",
            "bytes=",
            "bytes=abc-",
            "bytes=5",
            "items=5-",
            "bytes=5-,6-",
            "bytes=\u{663}-",
            "",
        ] {
            assert!(parse_range(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ids_are_urlsafe() {
        assert!(valid_id("0193a5c2-8000-7a11-8b22-0123456789ab") && valid_id("f_1"));
        assert!(!valid_id("") && !valid_id("a/b") && !valid_id("a b") && !valid_id(&"x".repeat(65)) && !valid_id("caf\u{e9}"));
    }

    #[test]
    fn the_offset_parameter_must_be_there_once() {
        assert_eq!(offset_param(Some("offset=5")).as_deref(), Some("5"));
        assert_eq!(offset_param(Some("x=1&offset=0")).as_deref(), Some("0"));
        assert_eq!(offset_param(Some("offset=")).as_deref(), Some(""));
        assert_eq!(offset_param(Some("offset=1&offset=2")), None);
        assert_eq!(offset_param(Some("other=1")), None);
        assert_eq!(offset_param(None), None);
    }
}
