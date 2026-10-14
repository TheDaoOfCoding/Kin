//! What this proves, on the machine it runs on (protocol.md: TLS 1.3 only, mutual TLS, keys pinned by SHA-256(SPKI)):
//!  1. rustls negotiates TLS 1.3 when only TLS 1.3 is enabled, and refuses a TLS 1.2-only client.
//!  2. The server can demand a client certificate, accept a self-signed one through a custom verifier, and read the
//!     client's SPKI hash from the handshake (the "device id").
//!  3. The client-side verifier really pins: it compares SHA-256(SPKI) of the server cert with an expected value, accepts a
//!     match and rejects a mismatch with its own error.
//!  4. DPAPI (CryptProtectData, current user scope) round-trips through the `windows` crate.
//!
//! Every claim is asserted. The program prints PASS or FAIL per check and exits non-zero if any check fails.
use std::io::{Read, Write};
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, ClientConnection, Connection, DigitallySignedStruct, DistinguishedName, Error, ServerConfig,
    ServerConnection, SignatureScheme, SupportedProtocolVersion, version,
};
use sha2::{Digest, Sha256};

// Accepts any client certificate. Authorization happens later by comparing the device id, as protocol.md says.
#[derive(Debug)]
struct AcceptAnyClient(WebPkiSupportedAlgorithms);
impl ClientCertVerifier for AcceptAnyClient {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not allowed".into()))
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(m, c, d, &self.0)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

const PIN_MISMATCH: &str = "server SPKI pin mismatch";

// Stands in for the phone. It ignores the host name and expiry and accepts only a server whose SHA-256(SPKI) equals the pin.
#[derive(Debug)]
struct PinnedServer {
    algs: WebPkiSupportedAlgorithms,
    expected_spki_sha256: String,
}
impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, Error> {
        if spki_sha256(end_entity) == self.expected_spki_sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General(PIN_MISMATCH.into()))
        }
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not allowed".into()))
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn spki_sha256(cert: &CertificateDer<'_>) -> String {
    let (_, c) = x509_parser::parse_x509_certificate(cert.as_ref()).unwrap();
    hex(&Sha256::digest(c.tbs_certificate.subject_pki.raw))
}

fn self_signed(name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = rcgen::KeyPair::generate().unwrap(); // ECDSA P-256 by default
    let cert = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap().self_signed(&key).unwrap();
    (cert.der().clone(), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())))
}

// Moves bytes between the two ends until neither has anything left to say. No sockets.
fn pump(a: &mut Connection, b: &mut Connection) -> Result<(), Error> {
    loop {
        let mut moved = false;
        for dir in 0..2 {
            let (from, to) = if dir == 0 { (&mut *a, &mut *b) } else { (&mut *b, &mut *a) };
            let mut buf = Vec::new();
            while from.wants_write() {
                from.write_tls(&mut buf).unwrap();
            }
            if !buf.is_empty() {
                moved = true;
                let mut rd = &buf[..];
                while !rd.is_empty() {
                    to.read_tls(&mut rd).unwrap();
                    to.process_new_packets()?;
                }
            }
        }
        if !moved {
            return Ok(());
        }
    }
}

// Counts failed checks, so main can exit non-zero.
#[derive(Default)]
struct Checks {
    failed: u32,
}
impl Checks {
    fn check(&mut self, name: &str, ok: bool) {
        println!("  {} {name}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            self.failed += 1;
        }
    }
}

fn main() {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let algs = provider.signature_verification_algorithms;
    println!("rustls crypto provider: aws-lc-rs (the rustls default)");
    let mut t = Checks::default();

    let (pc_cert, pc_key) = self_signed("kin-pc");
    let (phone_cert, phone_key) = self_signed("kin-phone");
    let pc_spki = spki_sha256(&pc_cert);
    let phone_spki = spki_sha256(&phone_cert);

    let server_cfg = Arc::new(
        ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&version::TLS13])
            .unwrap()
            .with_client_cert_verifier(Arc::new(AcceptAnyClient(algs)))
            .with_single_cert(vec![pc_cert.clone()], pc_key)
            .unwrap(),
    );
    let client_cfg = |versions: &[&'static SupportedProtocolVersion], pin: &str| {
        Arc::new(
            ClientConfig::builder_with_provider(provider.clone())
                .with_protocol_versions(versions)
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedServer { algs, expected_spki_sha256: pin.to_string() }))
                .with_client_auth_cert(vec![phone_cert.clone()], phone_key.clone_key())
                .unwrap(),
        )
    };
    let name = ServerName::try_from("kin-pc").unwrap();

    // 1 + 2 + 3: TLS 1.3 client with the right pin, mutual auth.
    println!("TLS 1.3 client, pinned to the PC's SPKI hash, vs 1.3-only server:");
    let mut client: Connection = ClientConnection::new(client_cfg(&[&version::TLS13], &pc_spki), name.clone()).unwrap().into();
    let mut server: Connection = ServerConnection::new(server_cfg.clone()).unwrap().into();
    let handshake = pump(&mut client, &mut server);
    t.check("handshake with the right pin succeeds", handshake.is_ok());
    t.check("handshake finished on both sides", !client.is_handshaking() && !server.is_handshaking());
    t.check("negotiated TLS 1.3", server.protocol_version() == Some(rustls::ProtocolVersion::TLSv1_3));
    let seen_by_server = server.peer_certificates().map(|c| spki_sha256(&c[0]));
    println!("    SPKI sha256 the server read from the client cert: {}", seen_by_server.as_deref().unwrap_or("none"));
    println!("    SPKI sha256 of the phone's own cert:              {phone_spki}");
    t.check("server read the phone's SPKI hash from the client cert", seen_by_server.as_deref() == Some(phone_spki.as_str()));
    let seen_by_client = client.peer_certificates().map(|c| spki_sha256(&c[0]));
    println!("    SPKI sha256 the client read from the PC cert:     {}", seen_by_client.as_deref().unwrap_or("none"));
    println!("    SPKI sha256 of the PC's own cert:                 {pc_spki}");
    t.check("client read the PC's SPKI hash from the server cert", seen_by_client.as_deref() == Some(pc_spki.as_str()));
    client.writer().write_all(b"hello over tls1.3").unwrap();
    let sent = pump(&mut client, &mut server);
    let mut got = [0u8; 64];
    let n = server.reader().read(&mut got).unwrap_or(0);
    t.check("application data arrives intact", sent.is_ok() && &got[..n] == b"hello over tls1.3");

    // 3: a wrong pin must make the handshake fail with the verifier's error. The phone's hash is a valid hash, just not the PC's.
    println!("TLS 1.3 client, pinned to the WRONG hash, vs 1.3-only server:");
    let mut cw: Connection = ClientConnection::new(client_cfg(&[&version::TLS13], &phone_spki), name.clone()).unwrap().into();
    let mut sw: Connection = ServerConnection::new(server_cfg.clone()).unwrap().into();
    let wrong = pump(&mut cw, &mut sw);
    match &wrong {
        Err(e) => println!("    handshake error: {e}"),
        Ok(()) => println!("    handshake succeeded"),
    }
    t.check(
        "handshake fails with the verifier's pin mismatch error",
        matches!(&wrong, Err(Error::General(m)) if m == PIN_MISMATCH),
    );
    t.check("the failed client never finished its handshake", cw.is_handshaking());

    // 1: a TLS 1.2-only client must be refused. It has the right pin, so only the version can be the reason.
    println!("TLS 1.2-only client, right pin, vs 1.3-only server:");
    let mut c12: Connection = ClientConnection::new(client_cfg(&[&version::TLS12], &pc_spki), name).unwrap().into();
    let mut s12: Connection = ServerConnection::new(server_cfg).unwrap().into();
    let refused = pump(&mut c12, &mut s12);
    match &refused {
        Err(e) => println!("    refused: {e}"),
        Ok(()) => println!("    handshake succeeded"),
    }
    t.check("handshake is refused", refused.is_err());
    t.check("refusal is not the pin check", !matches!(&refused, Err(Error::General(m)) if m == PIN_MISMATCH));

    dpapi_roundtrip(&mut t);

    if t.failed > 0 {
        println!("\n{} check(s) FAILED", t.failed);
        std::process::exit(1);
    }
    println!("\nAll checks passed");
}

fn dpapi_roundtrip(t: &mut Checks) {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};
    println!("DPAPI (current user):");
    let secret = b"pretend this is a P-256 private key";
    let input = CRYPT_INTEGER_BLOB { cbData: secret.len() as u32, pbData: secret.as_ptr() as *mut u8 };
    let mut enc = CRYPT_INTEGER_BLOB::default();
    let mut dec = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(&input, None, None, None, None, 0, &mut enc).expect("CryptProtectData");
        let enc_bytes = std::slice::from_raw_parts(enc.pbData, enc.cbData as usize).to_vec();
        CryptUnprotectData(&enc, None, None, None, None, 0, &mut dec).expect("CryptUnprotectData");
        let back = std::slice::from_raw_parts(dec.pbData, dec.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(enc.pbData as _)));
        let _ = LocalFree(Some(HLOCAL(dec.pbData as _)));
        println!("    {} bytes -> {} bytes encrypted", secret.len(), enc_bytes.len());
        t.check("round trip returns the original bytes", back == secret);
        t.check("plaintext is not visible in the encrypted blob", !enc_bytes.windows(secret.len()).any(|w| w == secret));
    }
}
