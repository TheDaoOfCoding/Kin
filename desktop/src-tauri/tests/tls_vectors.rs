//! The PC-side `tls[]` cases of `protocol/http.json`, with real TLS over loopback: a client certificate is mandatory, any chain
//! is accepted, the leaf must be inside the certificate profile, and the CertificateVerify signature is always checked.
//!
//! In TLS 1.3 the client finishes its handshake call before the server has checked its certificate, so a refused client can
//! still "connect". Every refusal below is therefore checked on the first read: an error or a close, and never an HTTP
//! response.
mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use common::*;
use kin_lib::tls::{P256ClientVerifier, certificate_profile_violations, peer_device_id};
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::danger::ClientCertVerifier;
use rustls::sign::{Signer, SigningKey};
use rustls::{SignatureAlgorithm, SignatureScheme};

struct Setup {
    _env: TestEnv,
    server: kin_lib::server::ServerHandle,
    pc: TestKey,
    phone: TestKey,
    keys: Vec<TestKey>,
}

impl Setup {
    async fn new() -> Setup {
        let keys = vector_keys();
        let env = test_core(identity_of(&keys[0]), "Desk PC");
        let server = start(&env.core).await;
        Setup { _env: env, server, pc: keys[0].clone(), phone: keys[1].clone(), keys }
    }

    fn addr(&self) -> SocketAddr {
        self.server.local_addr
    }

    /// The handshake works and application data flows: an unknown route answers 404.
    async fn expect_accepted(&self, config: Arc<rustls::ClientConfig>) {
        let mut stream = connect(self.addr(), config).await.expect("the TLS connection");
        let (_, conn) = stream.get_ref();
        assert_eq!(conn.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
        assert_eq!(conn.alpn_protocol(), Some(&b"http/1.1"[..]));
        let response = request(&mut stream, "GET", "/v1/nope", &[], &[]).await.expect("an HTTP response");
        assert_eq!(response.status, 404);
    }

    /// The server refuses the client, which shows on the first read. No HTTP response ever arrives. Returns the server's log
    /// lines about the handshake, so a test can check that the refusal came from the rule it is about.
    async fn expect_refused(&self, what: &str, config: Arc<rustls::ClientConfig>) -> Vec<String> {
        capture_logs();
        let before = captured_lines().len();
        match connect(self.addr(), config).await {
            Err(_) => {}
            Ok(mut stream) => {
                let outcome = request(&mut stream, "GET", "/v1/nope", &[], &[]).await;
                assert!(outcome.is_err(), "{what}: the server answered {outcome:?} instead of refusing the handshake");
            }
        }
        // The server logs why it refused. Wait for the line, so a test can tell the refusal came from the rule it is about.
        for _ in 0..50 {
            if captured_lines().len() > before {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let lines: Vec<String> = captured_lines().into_iter().skip(before).filter(|l| l.contains("TLS handshake")).collect();
        eprintln!("{what}: {lines:?}");
        lines
    }
}

#[tokio::test]
async fn pc_requires_a_client_certificate() {
    let s = Setup::new().await;
    let why = s.expect_refused("no client certificate", client_config(&s.pc.device_id, vec![], &[], Signing::Honest)).await;
    assert!(why.iter().any(|l| l.contains("peer sent no certificates")), "{why:?}");
}

#[tokio::test]
async fn pc_accepts_an_unpaired_self_signed_certificate() {
    let s = Setup::new().await;
    s.expect_accepted(phone_config(&s.pc.device_id, &s.phone)).await;
    // The follow-up of the vector: the handshake works, and it is the HTTP layer that says "not paired".
    let mut stream = connect(s.addr(), phone_config(&s.pc.device_id, &s.phone)).await.unwrap();
    let ws = request(
        &mut stream,
        "GET",
        "/v1/ws",
        &[
            ("Connection", "Upgrade"),
            ("Upgrade", "websocket"),
            ("Sec-WebSocket-Version", "13"),
            ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(ws.status, 401);
    assert_eq!(ws.json()["error"], "unauthorized");
}

#[tokio::test]
async fn pc_accepts_a_chain_from_an_unknown_issuer() {
    let s = Setup::new().await;
    // The leaf comes with an unrelated certificate behind it. Issuer, dates and extensions aren't checked.
    let chain = vec![s.phone.cert.clone(), s.keys[2].cert.clone()];
    s.expect_accepted(client_config(&s.pc.device_id, chain, &s.phone.pkcs8, Signing::Honest)).await;
}

#[tokio::test]
async fn pc_accepts_a_certificate_with_non_critical_extensions() {
    let s = Setup::new().await;
    let with_extension = &s.keys[3];
    assert!(!certificate_profile_violations(&with_extension.cert).iter().any(|v| v.contains("critical")));
    s.expect_accepted(phone_config(&s.pc.device_id, with_extension)).await;
    // And the minimal certificates with no extension at all (keys 1 to 3).
    for key in &s.keys[..3] {
        s.expect_accepted(phone_config(&s.pc.device_id, key)).await;
    }
}

#[tokio::test]
async fn pc_rejects_a_v1_client_certificate() {
    let s = Setup::new().await;
    let cert = fixture("client-v1.der");
    assert!(certificate_profile_violations(&cert).iter().any(|v| v.contains("v3")));
    let why = s.expect_refused("v1 certificate", client_config(&s.pc.device_id, vec![cert], &s.phone.pkcs8, Signing::Honest)).await;
    assert!(why.iter().any(|l| l.contains("UnsupportedCertVersion")), "{why:?}");
}

#[tokio::test]
async fn pc_rejects_a_client_certificate_with_an_unknown_critical_extension() {
    let s = Setup::new().await;
    let cert = fixture("client-unknown-critical-extension.der");
    assert!(certificate_profile_violations(&cert).iter().any(|v| v.contains("critical")));
    let why = s.expect_refused("critical extension", client_config(&s.pc.device_id, vec![cert], &s.phone.pkcs8, Signing::Honest)).await;
    assert!(why.iter().any(|l| l.contains("UnsupportedCriticalExtension")), "{why:?}");
}

/// A signing key that holds a P-384 key but says it signs `ecdsa_secp256r1_sha256`: the closest a P-384 client can get to
/// a handshake with this server.
#[derive(Debug)]
struct ClaimsP256(Arc<aws_lc_rs::signature::EcdsaKeyPair>);

#[derive(Debug)]
struct ClaimsP256Signer(Arc<aws_lc_rs::signature::EcdsaKeyPair>);

impl SigningKey for ClaimsP256 {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered.contains(&SignatureScheme::ECDSA_NISTP256_SHA256).then(|| Box::new(ClaimsP256Signer(self.0.clone())) as Box<dyn Signer>)
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }
}

impl Signer for ClaimsP256Signer {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        let rng = aws_lc_rs::rand::SystemRandom::new();
        Ok(self.0.sign(&rng, message).map_err(|_| rustls::Error::General("signing failed".into()))?.as_ref().to_vec())
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ECDSA_NISTP256_SHA256
    }
}

#[tokio::test]
async fn pc_rejects_a_client_certificate_that_is_not_p256() {
    let s = Setup::new().await;
    let cert = fixture("client-not-p256.der");
    let key = fixture("client-not-p256.key.der");
    assert!(certificate_profile_violations(&cert).iter().any(|v| v.contains("P-256")));

    // 1. An honest P-384 client: the server only offers `ecdsa_secp256r1_sha256`, so it can't sign and sends no certificate.
    let why = s.expect_refused("honest P-384 client", client_config(&s.pc.device_id, vec![cert.clone()], &key, Signing::Honest)).await;
    assert!(why.iter().any(|l| l.contains("peer sent no certificates")), "{why:?}");

    // 2. A client that signs with the P-384 key under the P-256 label. The verifier refuses the key itself
    //    (`ApplicationVerificationFailure` is its SPKI check), before the signature is looked at. (The one signature the
    //    default rustls list would accept, a P-384 key with SHA-256, can't be made here: aws-lc-rs has no such signing algorithm.)
    let pair = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(&aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1_SIGNING, &key).unwrap();
    let certified = Arc::new(rustls::sign::CertifiedKey::new(vec![CertificateDer::from(cert)], Arc::new(ClaimsP256(Arc::new(pair)))));
    let config = custom_client(&s.pc.device_id, certified);
    let why = s.expect_refused("P-384 key under the P-256 label", config).await;
    assert!(why.iter().any(|l| l.contains("ApplicationVerificationFailure")), "{why:?}");
}

fn custom_client(pinned: &str, certified: Arc<rustls::sign::CertifiedKey>) -> Arc<rustls::ClientConfig> {
    #[derive(Debug)]
    struct Fixed(Arc<rustls::sign::CertifiedKey>);
    impl rustls::client::ResolvesClientCert for Fixed {
        fn resolve(&self, _: &[&[u8]], _: &[SignatureScheme]) -> Option<Arc<rustls::sign::CertifiedKey>> {
            Some(self.0.clone())
        }
        fn has_certs(&self) -> bool {
            true
        }
    }
    // Reuse the pinned verifier of `client_config` by building a normal config, then swapping the resolver.
    let base = client_config(pinned, vec![], &[], Signing::Honest);
    let mut config = (*base).clone();
    config.client_auth_cert_resolver = Arc::new(Fixed(certified));
    Arc::new(config)
}

/// The SPKI check on its own: the verifier refuses the P-384 certificate before any signature is looked at.
#[test]
fn verifier_refuses_a_p384_certificate_before_the_signature() {
    let verifier = P256ClientVerifier;
    let p384 = CertificateDer::from(fixture("client-not-p256.der"));
    assert!(verifier.verify_client_cert(&p384, &[], UnixTime::now()).is_err());
    for key in vector_keys() {
        assert!(verifier.verify_client_cert(&CertificateDer::from(key.cert), &[], UnixTime::now()).is_ok());
    }
    // And it only offers one signature scheme.
    assert_eq!(verifier.supported_verify_schemes(), vec![SignatureScheme::ECDSA_NISTP256_SHA256]);
    assert!(verifier.client_auth_mandatory());
}

#[tokio::test]
async fn pc_rejects_a_replayed_certificate_without_the_private_key() {
    let s = Setup::new().await;
    // An attacker presents the paired phone's public certificate (key 2) but can only sign with a key of its own (key 3).
    let attacker_key = &s.keys[2];
    let why = s
        .expect_refused(
            "a replayed certificate",
            client_config(&s.pc.device_id, vec![s.phone.cert.clone()], &attacker_key.pkcs8, Signing::Honest),
        )
        .await;
    assert!(why.iter().any(|l| l.contains("BadSignature")), "{why:?}");
}

#[tokio::test]
async fn pc_rejects_an_invalid_certificateverify_signature() {
    let s = Setup::new().await;
    let why = s
        .expect_refused(
            "a corrupted signature",
            client_config(&s.pc.device_id, vec![s.phone.cert.clone()], &s.phone.pkcs8, Signing::Corrupt),
        )
        .await;
    assert!(why.iter().any(|l| l.contains("BadSignature")), "{why:?}");
}

/// A fixed, known-good control: the same certificate and key with an honest signature does connect.
#[tokio::test]
async fn control_the_same_client_connects_when_it_signs_honestly() {
    let s = Setup::new().await;
    s.expect_accepted(client_config(&s.pc.device_id, vec![s.phone.cert.clone()], &s.phone.pkcs8, Signing::Honest)).await;
}

/// The first real Android certificate is the test that matters (protocol/README.md, "Android: the certificate"). When the
/// integration agent adds `fixtures/android-client.der` (and `android-client.key.der`, a throwaway key), the PC has to
/// accept it. Until then there is nothing to test.
#[tokio::test]
async fn android_generated_certificate_is_accepted_when_the_fixture_exists() {
    let cert_path = protocol_dir().join("fixtures/android-client.der");
    if !cert_path.exists() {
        eprintln!("skipped: no protocol/fixtures/android-client.der yet");
        return;
    }
    let cert = std::fs::read(&cert_path).unwrap();
    assert_eq!(certificate_profile_violations(&cert), Vec::<String>::new(), "the Android certificate leaves the profile");
    assert!(P256ClientVerifier.verify_client_cert(&CertificateDer::from(cert.clone()), &[], UnixTime::now()).is_ok());
    // webpki parses it the way `verify_tls13_signature` will during a real handshake (version, extensions, encodings).
    webpki::EndEntityCert::try_from(&CertificateDer::from(cert.clone())).expect("rustls-webpki rejects the Android certificate");
    // The sidecar has the device ID the Android app computed for the same certificate.
    let sidecar: serde_json::Value = serde_json::from_slice(&fixture("android-client.json")).unwrap();
    assert_eq!(peer_device_id(&[CertificateDer::from(cert.clone())]).as_deref(), sidecar["deviceId"].as_str());
    let key_path = protocol_dir().join("fixtures/android-client.key.der");
    if key_path.exists() {
        let s = Setup::new().await;
        let key = std::fs::read(key_path).unwrap();
        s.expect_accepted(client_config(&s.pc.device_id, vec![cert], &key, Signing::Honest)).await;
    }
}
