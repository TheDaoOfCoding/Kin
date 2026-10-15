//! The HTTPS server: TLS 1.3 with a mandatory client certificate, `POST /v1/pair`, `GET /v1/ws`, the file endpoints of 1.3
//! (`fileserver.rs`) and `404` for the rest (protocol.md, "Transport", "Pairing", "Files" and "Error codes").
//!
//! axum doesn't hand the client certificate to handlers, so the accept loop below does the TLS handshake itself,
//! computes the device ID from the leaf certificate and attaches it to every request of that connection as a
//! [`Peer`]. The device ID is never taken from a request or a message body.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Extension, State};
use axum::http::header::{CONTENT_TYPE, HeaderMap};
use axum::http::{Request, Response, StatusCode};
use axum::routing::{MethodFilter, on};
use http_body_util::BodyExt;
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

use crate::Result;
use crate::core::{Core, DEFAULT_PORT, PairFailure, PairRequest, short};
use crate::json::{non_empty_str, optional_str, parse_object, uint};
use crate::messages::PROTOCOL;
use crate::pairing::PairError;
use crate::{fileserver, tls, ws};

/// A JSON body (pairing and `POST /v1/transfers`) may be at most 1 MiB.
pub const MAX_BODY_BYTES: usize = 1_048_576;
/// Past the limit a body is read and thrown away up to this many bytes, so the client still gets to read its `413`.
const DRAIN_LIMIT: usize = 16 * MAX_BODY_BYTES;

/// How long the server waits for a client, and how many connections it serves. Anyone on the network can open a connection
/// and complete the TLS handshake with a certificate of their own, before the server knows whether they are paired, so none of
/// these can be left unbounded.
///
/// The slots are three pools, 64 in all. A connection starts in `handshake_slots` and stays there only until its TLS
/// handshake is done (at most `tls_handshake_timeout`). Then the certificate says who it is: a paired device moves to
/// `paired_slots`, which nobody else can use, and anyone else moves to `unpaired_slots` (they can still reach `POST /v1/pair`).
/// So a flood of strangers can fill `unpaired_slots`, and a paired phone still gets through. One address may hold at most
/// `per_ip` connections while they are handshaking or unpaired. (A paired phone behind the same address as an attacker, such as
/// two programs on one PC, shares that count while it handshakes. A network that hides several devices behind one address is
/// not the case this is for.)
#[derive(Clone, Debug)]
pub struct Limits {
    /// From the TCP connection to the end of the TLS handshake.
    pub tls_handshake_timeout: Duration,
    /// For the request line and headers of each request, once the first byte of them has arrived.
    pub header_timeout: Duration,
    /// For a request body that is read whole (the JSON bodies, at most 1 MiB). A slow client gets `400 bad_request`.
    pub body_timeout: Duration,
    /// A connection that has moved no byte in either direction for this long is closed. A WebSocket doesn't count: it is
    /// handed over to its own task, with its own rules (10 minutes of silence), and this watchdog ends at the upgrade.
    pub idle_timeout: Duration,
    pub handshake_slots: usize,
    pub unpaired_slots: usize,
    pub paired_slots: usize,
    pub per_ip: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            tls_handshake_timeout: Duration::from_secs(10),
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(30),
            handshake_slots: 16,
            unpaired_slots: 24,
            paired_slots: 24,
            per_ip: 8,
        }
    }
}

/// Who is on the other end of a TLS connection: the device ID computed from its client certificate.
#[derive(Clone, Debug)]
pub struct Peer {
    pub device_id: String,
    pub remote: SocketAddr,
}

// ----- errors -----------------------------------------------------------------------------------------------------

/// The error codes of protocol.md and their HTTP statuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    BadRequest,
    Unauthorized,
    BadToken,
    TokenExpired,
    TokenUsed,
    UnsupportedVersion,
    NotFound,
    OffsetMismatch,
    SourceChanged,
    TooLarge,
    DiskFull,
    Internal,
}

impl ApiError {
    pub fn code(self) -> &'static str {
        match self {
            ApiError::BadRequest => "bad_request",
            ApiError::Unauthorized => "unauthorized",
            ApiError::BadToken => "bad_token",
            ApiError::TokenExpired => "token_expired",
            ApiError::TokenUsed => "token_used",
            ApiError::UnsupportedVersion => "unsupported_version",
            ApiError::NotFound => "not_found",
            ApiError::OffsetMismatch => "offset_mismatch",
            ApiError::SourceChanged => "source_changed",
            ApiError::TooLarge => "too_large",
            ApiError::DiskFull => "disk_full",
            ApiError::Internal => "internal",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            ApiError::BadRequest => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized | ApiError::BadToken | ApiError::TokenExpired | ApiError::TokenUsed => StatusCode::UNAUTHORIZED,
            ApiError::UnsupportedVersion => StatusCode::UPGRADE_REQUIRED,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::OffsetMismatch | ApiError::SourceChanged => StatusCode::CONFLICT,
            ApiError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::DiskFull => StatusCode::INSUFFICIENT_STORAGE,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Every non-2xx response is `{ "error", "message" }` as JSON.
    pub fn response(self, message: &str) -> Response<Body> {
        json_response(self.status(), &json!({ "error": self.code(), "message": message }))
    }
}

impl From<PairError> for ApiError {
    fn from(e: PairError) -> Self {
        match e {
            PairError::BadToken => ApiError::BadToken,
            PairError::TokenExpired => ApiError::TokenExpired,
            PairError::TokenUsed => ApiError::TokenUsed,
        }
    }
}

fn json_response(status: StatusCode, body: &Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(body.to_string()))
        .expect("status and headers are valid")
}

fn bad_request(message: &str) -> Response<Body> {
    ApiError::BadRequest.response(message)
}

fn pair_error_message(e: PairError) -> &'static str {
    match e {
        PairError::BadToken => "The pairing code is not valid.",
        PairError::TokenExpired => "The pairing code has expired. Show a new one on the PC.",
        PairError::TokenUsed => "The pairing code was already used.",
    }
}

// ----- request helpers --------------------------------------------------------------------------------------------

/// The media type must be `application/json` (ASCII case-insensitive), with no parameter or exactly `charset=utf-8`
/// (spaces or tabs around the `;` are fine). Anything else, including `charset="utf-8"`, is not JSON.
pub fn is_json_content_type(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("application/json") else {
        return false;
    };
    let rest = rest.trim_start_matches([' ', '\t']);
    match rest.strip_prefix(';') {
        None => rest.is_empty(),
        Some(param) => param.trim_start_matches([' ', '\t']) == "charset=utf-8",
    }
}

/// Reads a body up to `limit` bytes. A longer one is read to the end and thrown away (so the client can read its
/// `413`), up to a cap, and then reported as `Err`.
pub(crate) async fn read_limited(mut body: Body, limit: usize) -> std::result::Result<Vec<u8>, ()> {
    let mut out = Vec::new();
    let mut total = 0usize;
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else {
            return Err(());
        };
        if let Some(data) = frame.data_ref() {
            total += data.len();
            if total <= limit {
                out.extend_from_slice(data);
            } else if total > DRAIN_LIMIT {
                return Err(());
            }
        }
    }
    if total > limit { Err(()) } else { Ok(out) }
}

fn parse_pair_request(obj: &Map<String, Value>) -> std::result::Result<(PairRequest, u64), &'static str> {
    let token = obj.get("token").and_then(Value::as_str).ok_or("Missing or invalid field: token.")?;
    let name = non_empty_str(obj, "name").ok_or("Missing or invalid field: name.")?;
    let platform = non_empty_str(obj, "platform").ok_or("Missing or invalid field: platform.")?;
    let protocol = obj.get("protocol").and_then(uint).ok_or("Missing or invalid field: protocol.")?;
    let model = optional_str(obj, "model").map_err(|_| "Invalid field: model.")?;
    let app_version = optional_str(obj, "appVersion").map_err(|_| "Invalid field: appVersion.")?;
    // A `deviceId` in the body, if there is one, is ignored: the device ID comes from the TLS certificate only.
    Ok((
        PairRequest {
            token: token.to_string(),
            name: name.to_string(),
            platform: platform.to_string(),
            model: model.map(str::to_string),
            app_version: app_version.map(str::to_string),
        },
        protocol,
    ))
}

// ----- handlers ---------------------------------------------------------------------------------------------------

async fn not_found() -> Response<Body> {
    ApiError::NotFound.response("No such route.")
}

/// `POST /v1/pair`. Order of checks: 413, 400 (Content-Type, JSON, fields), 426, then the token.
async fn pair(
    State(core): State<Arc<Core>>,
    Extension(peer): Extension<Peer>,
    limits: Option<Extension<Limits>>,
    headers: HeaderMap,
    body: Body,
) -> Response<Body> {
    let body_timeout = limits.map_or(Limits::default().body_timeout, |Extension(l)| l.body_timeout);
    let bytes = match tokio::time::timeout(body_timeout, read_limited(body, MAX_BODY_BYTES)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(())) => return ApiError::TooLarge.response("The request body is over 1 MiB."),
        Err(_) => {
            // A client that sends a body this slowly is cut off. The status is `400` because protocol.md has no timeout code,
            // and the connection closes with it, so the half-read body can't be mistaken for the start of the next request.
            log::info!("pairing request from {}: the body did not arrive within {} s", peer.remote, body_timeout.as_secs_f32());
            let mut response = bad_request("The request body did not arrive in time.");
            response.headers_mut().insert(axum::http::header::CONNECTION, axum::http::HeaderValue::from_static("close"));
            return response;
        }
    };
    let is_json = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(is_json_content_type);
    if !is_json {
        return bad_request("The request body must be JSON.");
    }
    let Some(obj) = std::str::from_utf8(&bytes).ok().and_then(parse_object) else {
        return bad_request("The request body is not valid JSON.");
    };
    let (request, protocol) = match parse_pair_request(&obj) {
        Ok(r) => r,
        Err(message) => return bad_request(message),
    };
    if protocol != PROTOCOL {
        return ApiError::UnsupportedVersion.response(&format!("Protocol version {protocol} is not supported. Update the app."));
    }
    match core.pair(&peer.device_id, &request, peer.remote) {
        Ok(()) => {
            json_response(StatusCode::OK, &json!({ "name": core.pc_name(), "deviceId": core.identity.device_id, "protocol": PROTOCOL }))
        }
        Err(PairFailure::Rejected(e)) => ApiError::from(e).response(pair_error_message(e)),
        Err(PairFailure::Internal) => ApiError::Internal.response("Internal error. Check the logs."),
    }
}

/// `GET /v1/ws`: only for a paired device, otherwise `401 unauthorized` before anything is upgraded.
async fn websocket(
    State(core): State<Arc<Core>>,
    Extension(peer): Extension<Peer>,
    upgrade: std::result::Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response<Body> {
    match core.db.is_paired(&peer.device_id) {
        Ok(true) => {}
        Ok(false) => {
            log::info!("websocket refused: device {} from {} is not paired", short(&peer.device_id), peer.remote);
            return ApiError::Unauthorized.response("This device is not paired.");
        }
        Err(e) => {
            log::error!("checking the device failed: {e}");
            return ApiError::Internal.response("Internal error. Check the logs.");
        }
    }
    let Ok(upgrade) = upgrade else {
        return bad_request("Not a WebSocket upgrade request.");
    };
    // The library enforces the 1 MiB limit while reading, and `ws::run` turns its errors into the close codes.
    upgrade
        .max_message_size(crate::messages::MAX_FRAME_BYTES)
        .max_frame_size(crate::messages::MAX_FRAME_BYTES)
        .on_upgrade(move |socket| ws::run(core, socket, peer.device_id, peer.remote))
}

/// The routes. Unknown routes and methods are `404` for everyone, paired or not: only the endpoints that exist
/// check authorization. Each route handles one method, and every other method falls through to the same `404`.
pub fn router(core: Arc<Core>) -> Router {
    Router::new()
        .route("/v1/pair", on(MethodFilter::POST, pair).fallback(not_found))
        .route("/v1/ws", on(MethodFilter::GET, websocket).fallback(not_found))
        .route("/v1/transfers", on(MethodFilter::POST, fileserver::post_transfers).fallback(not_found))
        .route("/v1/transfers/{id}", on(MethodFilter::DELETE, fileserver::delete_transfer).fallback(not_found))
        .route(
            "/v1/transfers/{id}/files/{file_id}",
            on(MethodFilter::PUT, fileserver::put_file).on(MethodFilter::GET, fileserver::get_file).fallback(not_found),
        )
        .fallback(not_found)
        .with_state(core)
}

// ----- accept loop ------------------------------------------------------------------------------------------------

pub struct ServerHandle {
    pub local_addr: SocketAddr,
    shutdown: Arc<Notify>,
    task: JoinHandle<()>,
}

impl ServerHandle {
    /// Stops accepting connections. Open WebSockets are closed by `Core::shutdown`.
    pub async fn stop(self) {
        self.shutdown.notify_one();
        let _ = self.task.await;
    }
}

/// Binds the preferred port, or a random one if it's taken.
async fn bind(ip: std::net::IpAddr, preferred: u16) -> Result<TcpListener> {
    if preferred != 0 {
        match TcpListener::bind((ip, preferred)).await {
            Ok(l) => return Ok(l),
            Err(e) => log::warn!("port {preferred} is not available ({e}): using a random port"),
        }
    }
    Ok(TcpListener::bind((ip, 0)).await?)
}

/// Starts listening, with the default limits. The address is loopback in developer mode and every interface otherwise.
pub async fn start(core: Arc<Core>) -> Result<ServerHandle> {
    start_with(core, Limits::default()).await
}

/// [`start`] with other limits (the tests use short timeouts and few slots).
pub async fn start_with(core: Arc<Core>, limits: Limits) -> Result<ServerHandle> {
    let tls_config = tls::server_config(&core.identity)?;
    let listener = bind(core.config.bind_ip(), core.config.dev_port.unwrap_or(DEFAULT_PORT)).await?;
    let local_addr = listener.local_addr()?;
    core.set_port(local_addr.port());
    log::info!("listening on {local_addr} (TLS 1.3, client certificate required)");

    let acceptor = TlsAcceptor::from(tls_config);
    let app = router(core.clone());
    let shutdown = Arc::new(Notify::new());
    let stop = shutdown.clone();
    let slots = Arc::new(Slots::new(&limits));
    let refusals = Arc::new(RefusalLog::default());
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = stop.notified() => break,
                accepted = listener.accept() => match accepted {
                    Ok((tcp, remote)) => match slots.admit(remote.ip()) {
                        Some(admission) => {
                            let ctx = ConnectionContext { acceptor: acceptor.clone(), app: app.clone(), core: core.clone(), limits: limits.clone() };
                            tokio::spawn(serve_connection(ctx, tcp, remote, admission));
                        }
                        None => refusals.refused(remote),
                    },
                    Err(e) => {
                        log::warn!("accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
            }
        }
        log::info!("stopped listening");
    });
    Ok(ServerHandle { local_addr, shutdown, task })
}

// ----- slots ------------------------------------------------------------------------------------------------------

/// The connection slots (see [`Limits`]).
struct Slots {
    handshake: Arc<Semaphore>,
    unpaired: Arc<Semaphore>,
    paired: Arc<Semaphore>,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
    max_per_ip: usize,
}

/// What a connection holds. Freed when dropped.
struct Admission {
    slots: Arc<Slots>,
    ip: IpAddr,
    counted_for_ip: bool,
    handshake: Option<OwnedSemaphorePermit>,
    unpaired: Option<OwnedSemaphorePermit>,
    paired: Option<OwnedSemaphorePermit>,
}

impl Slots {
    fn new(limits: &Limits) -> Slots {
        Slots {
            handshake: Arc::new(Semaphore::new(limits.handshake_slots)),
            unpaired: Arc::new(Semaphore::new(limits.unpaired_slots)),
            paired: Arc::new(Semaphore::new(limits.paired_slots)),
            per_ip: Mutex::new(HashMap::new()),
            max_per_ip: limits.per_ip,
        }
    }

    /// A new connection takes a handshake slot and counts against its address. `None` if either is full.
    fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<Admission> {
        {
            let mut per_ip = self.per_ip.lock().unwrap_or_else(|p| p.into_inner());
            let n = per_ip.entry(ip).or_insert(0);
            if *n >= self.max_per_ip {
                return None;
            }
            *n += 1;
        }
        // On `None` the admission is dropped, which gives the address count back.
        let mut admission = Admission { slots: self.clone(), ip, counted_for_ip: true, handshake: None, unpaired: None, paired: None };
        admission.handshake = Some(self.handshake.clone().try_acquire_owned().ok()?);
        Some(admission)
    }
}

impl Admission {
    /// The handshake is done and the certificate says whether the device is paired. A paired device takes a slot of the pool
    /// that strangers can't use, and the address count is given back. Anyone else takes a slot of the unpaired pool, and
    /// `false` means it is full and the connection has to go. (If the paired pool is full, which would take more connections
    /// than a phone ever opens, the device is treated like anyone else.)
    fn settle(&mut self, is_paired: bool) -> bool {
        if is_paired && let Ok(permit) = self.slots.paired.clone().try_acquire_owned() {
            self.paired = Some(permit);
            self.handshake = None;
            self.release_ip();
            return true;
        }
        match self.slots.unpaired.clone().try_acquire_owned() {
            Ok(permit) => {
                self.unpaired = Some(permit);
                self.handshake = None;
                true
            }
            Err(_) => false,
        }
    }

    fn release_ip(&mut self) {
        if std::mem::take(&mut self.counted_for_ip) {
            let mut per_ip = self.slots.per_ip.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(n) = per_ip.get_mut(&self.ip) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    per_ip.remove(&self.ip);
                }
            }
        }
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.release_ip();
    }
}

/// Says "too many connections" in the log, at most once every 10 seconds with a count, so a flood can't fill the log files.
#[derive(Default)]
struct RefusalLog {
    /// When it was last logged, and how many connections were refused since.
    state: Mutex<(Option<Instant>, u64)>,
}

impl RefusalLog {
    fn refused(&self, remote: SocketAddr) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.1 += 1;
        if state.0.is_none_or(|t| t.elapsed() >= Duration::from_secs(10)) {
            log::warn!("too many connections: refused {remote} ({} refused since the last time this was logged)", state.1);
            *state = (Some(Instant::now()), 0);
        }
    }
}

// ----- one connection ---------------------------------------------------------------------------------------------

/// Wraps a connection and notes when a byte last moved in either direction, for the idle timeout.
struct Activity<S> {
    inner: S,
    started: Instant,
    last_ms: Arc<AtomicU64>,
}

impl<S> Activity<S> {
    fn touch(&self) {
        self.last_ms.store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Activity<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.touch();
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Activity<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, data);
        if matches!(poll, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct ConnectionContext {
    acceptor: TlsAcceptor,
    app: Router,
    core: Arc<Core>,
    limits: Limits,
}

async fn serve_connection(ctx: ConnectionContext, tcp: tokio::net::TcpStream, remote: SocketAddr, mut admission: Admission) {
    let ConnectionContext { acceptor, app, core, limits } = ctx;
    let stream = match tokio::time::timeout(limits.tls_handshake_timeout, acceptor.accept(tcp)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            log::info!("TLS handshake with {remote} failed: {e}");
            return;
        }
        Err(_) => {
            log::info!("TLS handshake with {remote} timed out");
            return;
        }
    };
    let Some(device_id) = stream.get_ref().1.peer_certificates().and_then(tls::peer_device_id) else {
        // The verifier makes a client certificate mandatory, so this shouldn't happen.
        log::warn!("TLS connection from {remote} has no client certificate");
        return;
    };
    if !admission.settle(core.db.is_paired(&device_id).unwrap_or(false)) {
        log::debug!("refusing {remote}: too many connections from unpaired devices");
        return;
    }
    let app = app.layer(Extension(Peer { device_id, remote })).layer(Extension(limits.clone()));
    let service = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| app.clone().oneshot(req.map(Body::new)));
    let activity = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let io = Activity { inner: stream, started, last_ms: activity.clone() };
    let connection = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_timeout)
        .serve_connection(TokioIo::new(io), service)
        .with_upgrades();
    tokio::pin!(connection);
    // The idle watchdog: no byte either way for `idle_timeout` closes the connection. It only runs while hyper is serving
    // requests: a WebSocket upgrade ends `connection`, and the socket goes on in `ws::run` under its own rules.
    let idle = async {
        loop {
            let idle_for = Duration::from_millis((started.elapsed().as_millis() as u64).saturating_sub(activity.load(Ordering::Relaxed)));
            if idle_for >= limits.idle_timeout {
                return;
            }
            tokio::time::sleep(limits.idle_timeout - idle_for).await;
        }
    };
    tokio::select! {
        result = &mut connection => {
            if let Err(e) = result {
                log::debug!("connection from {remote} ended with an error: {e}");
            }
        }
        () = idle => log::info!("closing the connection from {remote}: idle for {} s", limits.idle_timeout.as_secs_f32()),
    }
    drop(admission);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_rule() {
        for ok in [
            "application/json",
            "application/json; charset=utf-8",
            "Application/JSON; Charset=UTF-8",
            "application/json;charset=utf-8",
            "application/json ; charset=utf-8",
            "application/json\t;\tcharset=UTF-8",
        ] {
            assert!(is_json_content_type(ok), "{ok}");
        }
        for bad in [
            "",
            "text/plain",
            "text/json",
            "application/vnd.api+json",
            "application/json-patch+json",
            "application/jsonx",
            "application/json; charset=utf-16",
            "application/json; charset=\"utf-8\"",
            "application/json; boundary=x",
            "application/json;",
            "application/json; charset=utf-8; boundary=x",
            "application/json charset=utf-8",
        ] {
            assert!(!is_json_content_type(bad), "{bad}");
        }
    }
}
