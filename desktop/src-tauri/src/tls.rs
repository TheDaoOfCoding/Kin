//! TLS 1.3 with a mandatory client certificate, and the verifier that holds it to the certificate profile.
//!
//! The PC doesn't check the issuer, the dates or the extensions of a client certificate: trust is the device ID,
//! decided per request. But the handshake still has to prove that the client owns the key of the certificate it
//! presents (`verify_tls13_signature`), and the key has to be ECDSA P-256 (the profile in protocol.md, "Transport").
use std::sync::Arc;

use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, SignatureVerificationAlgorithm, UnixTime};
use rustls::server::NoServerSessionStorage;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme};

use crate::{Result, encoding, identity::Identity};

/// The only signature check this server runs: ECDSA P-256 with SHA-256. rustls maps `ecdsa_secp256r1_sha256` to
/// both the P-256 and the P-384 algorithm by default, so a P-384 client certificate would pass its default list.
static P256_ONLY: &[&dyn SignatureVerificationAlgorithm] = &[webpki::aws_lc_rs::ECDSA_P256_SHA256];
static P256_MAPPING: &[(SignatureScheme, &[&dyn SignatureVerificationAlgorithm])] = &[(SignatureScheme::ECDSA_NISTP256_SHA256, P256_ONLY)];
pub static P256_ALGORITHMS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms { all: P256_ONLY, mapping: P256_MAPPING };

/// DER of `SubjectPublicKeyInfo { id-ecPublicKey, prime256v1, BIT STRING }` up to the 65 bytes of the point.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01,
    0x07, 0x03, 0x42, 0x00,
];

/// 91 bytes: `id-ecPublicKey`, `prime256v1`, an uncompressed point.
pub fn is_p256_spki(spki: &[u8]) -> bool {
    spki.len() == 91 && spki.starts_with(&P256_SPKI_PREFIX) && spki[26] == 0x04
}

#[derive(Debug)]
pub struct P256ClientVerifier;

impl ClientCertVerifier for P256ClientVerifier {
    // No issuer hints: any self-signed certificate is welcome.
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    // `client_auth_mandatory` keeps its default of `true`: a client without a certificate fails the handshake.

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, Error> {
        let spki = encoding::spki_of_certificate(end_entity).ok_or(Error::InvalidCertificate(CertificateError::BadEncoding))?;
        if !is_p256_spki(spki) {
            return Err(Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure));
        }
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not allowed".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &P256_ALGORITHMS)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

pub fn crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// TLS 1.3 only, ALPN `http/1.1`, client certificate required, no session tickets (every connection proves its key).
pub fn server_config(identity: &Identity) -> Result<Arc<ServerConfig>> {
    let mut config = ServerConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(P256ClientVerifier))
        .with_single_cert(
            vec![CertificateDer::from(identity.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_der.clone())),
        )?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.session_storage = Arc::new(NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    Ok(Arc::new(config))
}

/// The device ID of the client, from the leaf certificate it presented in the handshake.
pub fn peer_device_id(chain: &[CertificateDer<'_>]) -> Option<String> {
    encoding::spki_of_certificate(chain.first()?).map(encoding::device_id_from_spki)
}

/// What the certificate profile of protocol.md says a certificate must look like, as a list of violations
/// (empty means it fits). rustls-webpki does the real checking in the handshake. This is for tests and diagnostics.
pub fn certificate_profile_violations(der: &[u8]) -> Vec<String> {
    use x509_parser::prelude::{FromDer, X509Certificate, X509Version};
    let mut out = Vec::new();
    let Ok((rest, cert)) = X509Certificate::from_der(der) else {
        return vec!["not a parseable X.509 certificate".into()];
    };
    if !rest.is_empty() {
        out.push("trailing bytes after the certificate".into());
    }
    if cert.version() != X509Version::V3 {
        out.push(format!("version {} instead of v3", cert.version().0 + 1));
    }
    if !is_p256_spki(cert.tbs_certificate.subject_pki.raw) {
        out.push("the key is not ECDSA P-256 in a 91-byte SPKI".into());
    }
    // 1.2.840.10045.4.3.2 = ecdsa-with-SHA256, with no parameters (RFC 5758).
    if cert.signature_algorithm.algorithm.to_id_string() != "1.2.840.10045.4.3.2" || cert.signature_algorithm.parameters.is_some() {
        out.push("signature algorithm is not ecdsa-with-SHA256 without parameters".into());
    }
    let serial = cert.raw_serial();
    if serial.len() > 20 || serial.first().is_none_or(|b| b & 0x80 != 0) {
        out.push("serial is not a positive integer of at most 20 bytes".into());
    }
    if cert.extensions().iter().any(|e| e.critical) {
        out.push("a critical extension".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_certificate_follows_the_profile() {
        let id = Identity::generate().unwrap();
        assert_eq!(certificate_profile_violations(&id.cert_der), Vec::<String>::new());
        let (_, cert) = x509_parser::parse_x509_certificate(&id.cert_der).unwrap();
        assert!(cert.extensions().is_empty(), "rcgen wrote an extension: {:?}", cert.extensions());
        assert_eq!(id.device_id.len(), 43);
        assert_eq!(id.short_id.len(), 8);
        assert!(is_p256_spki(encoding::spki_of_certificate(&id.cert_der).unwrap()));
    }

    #[test]
    fn spki_check_is_exact() {
        let id = Identity::generate().unwrap();
        let spki = encoding::spki_of_certificate(&id.cert_der).unwrap().to_vec();
        assert!(is_p256_spki(&spki));
        let mut short = spki.clone();
        short.pop();
        assert!(!is_p256_spki(&short));
        let mut compressed = spki.clone();
        compressed[26] = 0x02;
        assert!(!is_p256_spki(&compressed));
        assert!(!is_p256_spki(&[]));
    }
}
