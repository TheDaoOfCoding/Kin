//! The PC's identity: a P-256 key and a self-signed certificate that follow the certificate profile of protocol.md.
//! The key is kept DPAPI-encrypted in the data directory. The certificate is public and kept next to it.
use std::fs;
use std::path::Path;

use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256, PublicKeyData};

use crate::{Result, dpapi, encoding};

const KEY_FILE: &str = "identity.key";
const CERT_FILE: &str = "identity.crt";

pub struct Identity {
    /// PKCS#8. Never logged, never leaves this process.
    pub key_der: Vec<u8>,
    pub cert_der: Vec<u8>,
    pub device_id: String,
    pub short_id: String,
}

/// An identity and whether this call created the key.
pub struct Loaded {
    pub identity: Identity,
    pub created: bool,
}

/// The certificate of the profile: v3, `ecdsa-with-SHA256`, CN=Kin, no extensions at all.
///
/// Each setting below is what keeps rcgen from writing an extension, because rustls-webpki, which verifies the TLS
/// signature, refuses unknown critical extensions: `is_ca` stays `NoCa`, no key usages (rcgen always marks them
/// critical), no name constraints, no subject alternative names, no custom extensions. The serial stays `None`, so
/// rcgen makes a positive 20-byte one.
fn make_certificate(key: &KeyPair) -> Result<Vec<u8>> {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, "Kin");
    params.is_ca = IsCa::NoCa;
    Ok(params.self_signed(key)?.der().to_vec())
}

impl Identity {
    /// An identity from a key (PKCS#8) and the certificate over it. Tests use this with the vector keys.
    pub fn from_der(key_der: Vec<u8>, cert_der: Vec<u8>) -> Result<Identity> {
        let spki = encoding::spki_of_certificate(&cert_der).ok_or("the certificate has no readable public key")?;
        Ok(Identity { device_id: encoding::device_id_from_spki(spki), short_id: encoding::short_id_from_spki(spki), key_der, cert_der })
    }

    pub fn generate() -> Result<Identity> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        Identity::from_der(key.serialize_der(), make_certificate(&key)?)
    }

    /// Loads the identity from `dir`, or creates it on the first run. A missing or mismatching certificate is made
    /// again from the same key, which keeps the device ID. A key that can't be decrypted (a profile copied to
    /// another Windows user, say) is replaced by a new identity, which paired phones will notice.
    pub fn load_or_create(dir: &Path) -> Result<Loaded> {
        fs::create_dir_all(dir)?;
        let (key_path, cert_path) = (dir.join(KEY_FILE), dir.join(CERT_FILE));
        if let Ok(blob) = fs::read(&key_path) {
            match dpapi::unprotect(&blob).ok().and_then(|der| KeyPair::try_from(der.as_slice()).ok().map(|k| (der, k))) {
                Some((key_der, key)) => {
                    let cert = fs::read(&cert_path)
                        .ok()
                        .filter(|c| encoding::spki_of_certificate(c).is_some_and(|spki| spki == key.subject_public_key_info().as_slice()));
                    let cert_der = match cert {
                        Some(c) => c,
                        None => {
                            let c = make_certificate(&key)?;
                            write_atomic(&cert_path, &c)?;
                            c
                        }
                    };
                    return Ok(Loaded { identity: Identity::from_der(key_der, cert_der)?, created: false });
                }
                None => log::warn!("the saved private key can't be decrypted: creating a new identity"),
            }
        }
        let identity = Identity::generate()?;
        write_atomic(&key_path, &dpapi::protect(&identity.key_der)?)?;
        write_atomic(&cert_path, &identity.cert_der)?;
        Ok(Loaded { identity, created: true })
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
