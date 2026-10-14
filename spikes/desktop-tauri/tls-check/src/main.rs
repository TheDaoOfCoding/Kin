//! What this proves, on the machine it runs on (protocol.md: TLS 1.3 only, mutual TLS, keys pinned by SHA-256(SPKI)):
//!  1. rustls negotiates TLS 1.3 when only TLS 1.3 is enabled, and refuses a TLS 1.2-only client.
//!  2. The server can demand a client certificate, accept a self-signed one through a custom verifier, and read the
//!     client's SPKI hash from the handshake (the "device id").
//!  3. DPAPI (CryptProtectData, current user scope) round-trips through the `windows` crate.
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

// Stands in for the phone, which would pin the PC's SPKI hash. Here it accepts anything and the check happens in main.
#[derive(Debug)]
struct AcceptAnyServer(WebPkiSupportedAlgorithms);
impl ServerCertVerifier for AcceptAnyServer {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
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

fn main() {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let algs = provider.signature_verification_algorithms;
    println!("rustls crypto provider: aws-lc-rs (the rustls default)");

    let (pc_cert, pc_key) = self_signed("kin-pc");
    let (phone_cert, phone_key) = self_signed("kin-phone");

    let server_cfg = Arc::new(
        ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&version::TLS13])
            .unwrap()
            .with_client_cert_verifier(Arc::new(AcceptAnyClient(algs)))
            .with_single_cert(vec![pc_cert.clone()], pc_key)
            .unwrap(),
    );
    let client_cfg = |versions: &[&'static SupportedProtocolVersion]| {
        Arc::new(
            ClientConfig::builder_with_provider(provider.clone())
                .with_protocol_versions(versions)
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyServer(algs)))
                .with_client_auth_cert(vec![phone_cert.clone()], phone_key.clone_key())
                .unwrap(),
        )
    };
    let name = ServerName::try_from("kin-pc").unwrap();

    // 1 + 2: TLS 1.3 client, mutual auth.
    let mut client: Connection = ClientConnection::new(client_cfg(&[&version::TLS13]), name.clone()).unwrap().into();
    let mut server: Connection = ServerConnection::new(server_cfg.clone()).unwrap().into();
    pump(&mut client, &mut server).expect("TLS 1.3 handshake");
    println!("TLS 1.3 client vs 1.3-only server: negotiated {:?}, still handshaking = {}", server.protocol_version(), server.is_handshaking());
    let seen = server.peer_certificates().expect("client cert").first().unwrap().clone();
    println!("  SPKI sha256 the server read from the client cert: {}", spki_sha256(&seen));
    println!("  SPKI sha256 of the phone's own cert:              {}", spki_sha256(&phone_cert));
    println!("  SPKI sha256 the client read from the PC cert:     {}", spki_sha256(client.peer_certificates().unwrap().first().unwrap()));
    println!("  SPKI sha256 of the PC's own cert:                 {}", spki_sha256(&pc_cert));
    client.writer().write_all(b"hello over tls1.3").unwrap();
    pump(&mut client, &mut server).unwrap();
    let mut got = [0u8; 64];
    let n = server.reader().read(&mut got).unwrap();
    println!("  application data the server received: {:?}", std::str::from_utf8(&got[..n]).unwrap());

    // 1: a TLS 1.2-only client must be refused.
    let mut c12: Connection = ClientConnection::new(client_cfg(&[&version::TLS12]), name).unwrap().into();
    let mut s12: Connection = ServerConnection::new(server_cfg).unwrap().into();
    match pump(&mut c12, &mut s12) {
        Err(e) => println!("TLS 1.2-only client vs 1.3-only server: refused ({e})"),
        Ok(()) => println!("TLS 1.2-only client vs 1.3-only server: handshake SUCCEEDED (BAD)"),
    }

    dpapi_roundtrip();
}

fn dpapi_roundtrip() {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};
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
        println!(
            "DPAPI (current user): {} bytes -> {} bytes encrypted; round trip equal = {}; plaintext visible in the blob = {}",
            secret.len(),
            enc_bytes.len(),
            back == secret,
            enc_bytes.windows(secret.len()).any(|w| w == secret)
        );
    }
}
