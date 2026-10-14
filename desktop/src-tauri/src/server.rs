//! The HTTPS server: TLS 1.3 with a mandatory client certificate, `POST /v1/pair`, `GET /v1/ws` and `404` for the rest
//! (protocol.md, "Transport", "Pairing" and "Error codes").
//!
//! axum doesn't hand the client certificate to handlers, so the accept loop below does the TLS handshake itself,
//! computes the device ID from the leaf certificate and attaches it to every request of that connection as a
//! [`Peer`]. The device ID is never taken from a request or a message body.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

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
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

use crate::Result;
use crate::core::{Core, DEFAULT_PORT, PairFailure, PairRequest, short};
use crate::json::{non_empty_str, optional_str, parse_object, uint};
use crate::messages::PROTOCOL;
use crate::pairing::PairError;
use crate::{tls, ws};

/// A JSON body (pairing, and later transfers) may be at most 1 MiB.
pub const MAX_BODY_BYTES: usize = 1_048_576;
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections served at once. Anyone on the network can open one before authenticating, so there is a cap.
const MAX_CONNECTIONS: usize = 64;
const REQUEST_HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// Past the limit a body is read and thrown away up to this many bytes, so the client still gets to read its `413`.
const DRAIN_LIMIT: usize = 16 * MAX_BODY_BYTES;

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
async fn read_limited(mut body: Body, limit: usize) -> std::result::Result<Vec<u8>, ()> {
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
async fn pair(State(core): State<Arc<Core>>, Extension(peer): Extension<Peer>, headers: HeaderMap, body: Body) -> Response<Body> {
    let Ok(bytes) = read_limited(body, MAX_BODY_BYTES).await else {
        return ApiError::TooLarge.response("The request body is over 1 MiB.");
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
            json_response(StatusCode::OK, &json!({ "name": core.pc_name, "deviceId": core.identity.device_id, "protocol": PROTOCOL }))
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

/// Starts listening. The address is loopback in developer mode and every interface otherwise.
pub async fn start(core: Arc<Core>) -> Result<ServerHandle> {
    let tls_config = tls::server_config(&core.identity)?;
    let listener = bind(core.config.bind_ip(), core.config.dev_port.unwrap_or(DEFAULT_PORT)).await?;
    let local_addr = listener.local_addr()?;
    core.set_port(local_addr.port());
    log::info!("listening on {local_addr} (TLS 1.3, client certificate required)");

    let acceptor = TlsAcceptor::from(tls_config);
    let app = router(core.clone());
    let shutdown = Arc::new(Notify::new());
    let stop = shutdown.clone();
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = stop.notified() => break,
                accepted = listener.accept() => match accepted {
                    Ok((tcp, remote)) => match slots.clone().try_acquire_owned() {
                        Ok(slot) => {
                            let (acceptor, app) = (acceptor.clone(), app.clone());
                            tokio::spawn(async move {
                                serve_connection(acceptor, app, tcp, remote).await;
                                drop(slot);
                            });
                        }
                        Err(_) => log::warn!("too many connections: refusing {remote}"),
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

async fn serve_connection(acceptor: TlsAcceptor, app: Router, tcp: tokio::net::TcpStream, remote: SocketAddr) {
    let stream = match tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await {
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
    let app = app.layer(Extension(Peer { device_id, remote }));
    let service = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| app.clone().oneshot(req.map(Body::new)));
    let connection = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(REQUEST_HEADER_TIMEOUT)
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    if let Err(e) = connection.await {
        log::debug!("connection from {remote} ended with an error: {e}");
    }
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
