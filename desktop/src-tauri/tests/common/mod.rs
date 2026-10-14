//! Shared helpers for the integration tests: the vector files, a server in the test process on loopback, and a fake phone
//! (a rustls client with its own certificate that pins the PC's device ID).
#![allow(dead_code)]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kin_lib::cli::Config;
use kin_lib::clipboard::{MemoryClipboard, MonotonicClock};
use kin_lib::core::{Core, Event, Services};
use kin_lib::encoding::{device_id_from_spki, spki_of_certificate};
use kin_lib::identity::Identity;
use kin_lib::links::RecordingOpener;
use kin_lib::server::{self, ServerHandle};
use kin_lib::storage::Db;
use kin_lib::tls::crypto_provider;
use rustls::client::ResolvesClientCert;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::sign::{CertifiedKey, Signer, SigningKey};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureAlgorithm, SignatureScheme};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

// ----- the vector files -------------------------------------------------------------------------------------------

pub fn protocol_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol")
}

pub fn load(name: &str) -> Value {
    let path = protocol_dir().join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

pub fn fixture(name: &str) -> Vec<u8> {
    let path = protocol_dir().join("fixtures").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A key and certificate from `device-id.json` (test-only keys, public in the repo).
#[derive(Clone)]
pub struct TestKey {
    pub pkcs8: Vec<u8>,
    pub cert: Vec<u8>,
    pub device_id: String,
    pub short_id: String,
}

pub fn vector_keys() -> Vec<TestKey> {
    load("device-id.json")["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| TestKey {
            pkcs8: PrivatePkcs8KeyDer::from_pem_slice(k["privateKeyPkcs8Pem"].as_str().unwrap().as_bytes())
                .unwrap()
                .secret_pkcs8_der()
                .to_vec(),
            cert: CertificateDer::from_pem_slice(k["certificatePem"].as_str().unwrap().as_bytes()).unwrap().to_vec(),
            device_id: k["deviceId"].as_str().unwrap().to_string(),
            short_id: k["shortId"].as_str().unwrap().to_string(),
        })
        .collect()
}

pub fn identity_of(key: &TestKey) -> Identity {
    Identity::from_der(key.pkcs8.clone(), key.cert.clone()).unwrap()
}

// ----- a server in the test process -------------------------------------------------------------------------------

/// A core with an in-memory database that binds to a random loopback port. Nothing is exposed to the network. The data
/// directory is a temporary one, and the developer hook that writes `pair-url.txt` is on. The clipboard is in memory (the real
/// one is never touched) and the "browser" only records which links it was asked to open.
pub struct TestEnv {
    pub core: Arc<Core>,
    pub events: UnboundedReceiver<Event>,
    pub dir: tempfile::TempDir,
    pub clipboard: Arc<MemoryClipboard>,
    pub opener: Arc<RecordingOpener>,
}

pub fn test_core(identity: Identity, pc_name: &str) -> TestEnv {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(dir.path().to_path_buf());
    config.dev_bind = Some("127.0.0.1".parse().unwrap());
    config.dev_qr_address = Some("10.0.2.2".parse().unwrap());
    config.dev_port = Some(0);
    config.dev_print_pair_url = true;
    let (tx, events) = tokio::sync::mpsc::unbounded_channel();
    let clipboard = Arc::new(MemoryClipboard::default());
    let opener = Arc::new(RecordingOpener::default());
    let services = Services { clipboard: clipboard.clone(), opener: opener.clone(), clock: Arc::new(MonotonicClock::new()) };
    let core = Arc::new(Core::new(config, identity, Db::open_in_memory().unwrap(), pc_name.to_string(), tx, services));
    TestEnv { core, events, dir, clipboard, opener }
}

pub async fn start(core: &Arc<Core>) -> ServerHandle {
    server::start(core.clone()).await.unwrap()
}

// ----- the fake phone ---------------------------------------------------------------------------------------------

/// The phone side of the TLS handshake: accepts only the server whose SHA-256(SPKI) is the pinned device ID,
/// and ignores the host name and the dates.
#[derive(Debug)]
struct PinnedServer {
    pinned_device_id: String,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match spki_of_certificate(end_entity).map(device_id_from_spki) {
            Some(id) if id == self.pinned_device_id => Ok(ServerCertVerified::assertion()),
            _ => Err(TlsError::General("server key does not match the pinned device ID".into())),
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

/// Offers whatever certificate chain and key it was given, without the checks `with_client_auth_cert` makes, so tests can
/// present certificates outside the profile.
#[derive(Debug)]
struct Resolver(Option<Arc<CertifiedKey>>);

impl ResolvesClientCert for Resolver {
    fn resolve(&self, _root_hint_subjects: &[&[u8]], _sigschemes: &[SignatureScheme]) -> Option<Arc<CertifiedKey>> {
        self.0.clone()
    }

    fn has_certs(&self) -> bool {
        self.0.is_some()
    }
}

/// How a client signs the CertificateVerify message.
pub enum Signing {
    Honest,
    /// Flips a bit of every signature: a client that has the certificate but not the key.
    Corrupt,
}

#[derive(Debug)]
struct CorruptingKey(Arc<dyn SigningKey>);

#[derive(Debug)]
struct CorruptingSigner(Box<dyn Signer>);

impl SigningKey for CorruptingKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        self.0.choose_scheme(offered).map(|s| Box::new(CorruptingSigner(s)) as Box<dyn Signer>)
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        self.0.algorithm()
    }
}

impl Signer for CorruptingSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, TlsError> {
        let mut sig = self.0.sign(message)?;
        let last = sig.len() - 1;
        sig[last] ^= 1;
        Ok(sig)
    }

    fn scheme(&self) -> SignatureScheme {
        self.0.scheme()
    }
}

pub fn signing_key(pkcs8: &[u8]) -> Arc<dyn SigningKey> {
    rustls::crypto::aws_lc_rs::sign::any_supported_type(&PrivatePkcs8KeyDer::from(pkcs8.to_vec()).into()).unwrap()
}

/// A client configuration for a phone. `chain` is the certificate chain it presents (empty: none), `key` the PKCS#8 key it
/// signs with, and `pinned` the device ID of the PC it expects.
pub fn client_config(pinned: &str, chain: Vec<Vec<u8>>, key: &[u8], signing: Signing) -> Arc<ClientConfig> {
    let provider = crypto_provider();
    let certified = (!chain.is_empty()).then(|| {
        let key: Arc<dyn SigningKey> = match signing {
            Signing::Honest => signing_key(key),
            Signing::Corrupt => Arc::new(CorruptingKey(signing_key(key))),
        };
        Arc::new(CertifiedKey::new(chain.into_iter().map(CertificateDer::from).collect(), key))
    });
    let verifier = PinnedServer { pinned_device_id: pinned.to_string(), algs: provider.signature_verification_algorithms };
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_cert_resolver(Arc::new(Resolver(certified)));
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

pub fn phone_config(pinned: &str, phone: &TestKey) -> Arc<ClientConfig> {
    client_config(pinned, vec![phone.cert.clone()], &phone.pkcs8, Signing::Honest)
}

/// Connects. In TLS 1.3 this can succeed even when the server is about to refuse the client certificate: the failure shows
/// up on the first read.
pub async fn connect(addr: SocketAddr, config: Arc<ClientConfig>) -> std::io::Result<TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(addr).await?;
    TlsConnector::from(config).connect(ServerName::try_from("kin").unwrap(), tcp).await
}

// ----- a minimal HTTP/1.1 client ----------------------------------------------------------------------------------

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", String::from_utf8_lossy(&self.body)))
    }
}

pub async fn send_request<S>(stream: &mut S, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> std::io::Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: kin\r\n");
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if !body.is_empty() || method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

pub async fn read_response<S>(stream: &mut S) -> std::io::Result<HttpResponse>
where
    S: AsyncReadExt + Unpin,
{
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the connection closed before a response"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
    let mut body = buf[head_end + 4..].to_vec();
    let length = headers.iter().find(|(n, _)| n.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok());
    if let (Some(length), true) = (length, status != 101) {
        while body.len() < length {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    }
    Ok(HttpResponse { status, headers, body })
}

pub async fn request<S>(stream: &mut S, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> std::io::Result<HttpResponse>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    send_request(stream, method, path, headers, body).await?;
    read_response(stream).await
}

pub fn pair_body(token: &str, name: &str) -> Vec<u8> {
    serde_json::json!({ "token": token, "name": name, "platform": "android", "model": "Test", "appVersion": "0.1.0", "protocol": 1 })
        .to_string()
        .into_bytes()
}

/// Starts a pairing the way a developer would (the URL goes to `pair-url.txt`) and returns the token from that URL.
pub fn start_pairing_and_read_token(env: &TestEnv) -> String {
    let view = env.core.start_pairing();
    assert_eq!(view.status, "waiting");
    let url = std::fs::read_to_string(env.dir.path().join("pair-url.txt")).unwrap();
    url.trim().split('&').find_map(|p| p.strip_prefix("tk=")).expect("the URL has a tk parameter").to_string()
}

// ----- capturing the server's log lines, to see why a handshake was refused ---------------------------------------

static CAPTURED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        CAPTURED.lock().unwrap().push(format!("{}: {}", record.level(), record.args()));
    }

    fn flush(&self) {}
}

/// Installs a logger that keeps every line in memory. Only the first call in a test process has any effect.
pub fn capture_logs() {
    if log::set_logger(&Capture).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

pub fn captured_lines() -> Vec<String> {
    CAPTURED.lock().unwrap().clone()
}
