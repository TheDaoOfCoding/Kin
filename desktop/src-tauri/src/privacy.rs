//! Keyed tags for the log. A log line that has to say "this is the same text (or file) as that line" carries a tag, never the text or the name.
//!
//! **Why keyed.** The tag used to be the first 32 bits of the SHA-256 of the text. Anyone who has a log (the export is meant to be sent to a
//! developer) can hash a guess ("password", a name, a phone number) and compare, and 32 bits of an unkeyed hash is also cheap to search for a
//! short text. The tag is now the first 32 bits of an **HMAC-SHA256** under a key that is **random per installation**, kept in the data
//! directory protected by DPAPI (readable only by this Windows user on this PC), and **never exported**: not in the log, not in `status.json`,
//! not in the zip of "Export logs". Without the key a tag can't be checked against a guess.
//!
//! The tag still follows one text through a log (the same text gives the same tag for as long as the installation lives), and says nothing else.
use std::path::Path;
use std::sync::OnceLock;

use crate::encoding::{random_bytes, sha256};

/// The file in the data directory that holds the DPAPI-protected key. Not part of any export.
pub const KEY_FILE: &str = "log-key.dat";

const KEY_BYTES: usize = 32;

static KEY: OnceLock<[u8; KEY_BYTES]> = OnceLock::new();

/// HMAC-SHA256 (RFC 2104) of `data` under `key`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(BLOCK + data.len());
    inner.extend(k.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(data);
    let inner_hash = sha256(&inner);
    let mut outer = Vec::with_capacity(BLOCK + 32);
    outer.extend(k.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

/// Loads the per-installation key from `data_dir`, or makes it on the first run, and uses it for every tag of this process. Safe to call more
/// than once (the first key wins). A key that can't be read (the profile was copied to another Windows user) is replaced by a new one: tags
/// of older logs then don't match, which is the point of a key.
pub fn init(data_dir: &Path) {
    KEY.get_or_init(|| load_or_create(data_dir));
}

fn load_or_create(data_dir: &Path) -> [u8; KEY_BYTES] {
    let path = data_dir.join(KEY_FILE);
    if let Ok(blob) = std::fs::read(&path)
        && let Ok(plain) = crate::dpapi::unprotect(&blob)
        && let Ok(key) = <[u8; KEY_BYTES]>::try_from(plain.as_slice())
    {
        return key;
    }
    let key = random_bytes::<KEY_BYTES>();
    match crate::dpapi::protect(&key) {
        Ok(blob) => {
            let tmp = path.with_extension("tmp");
            if let Err(e) = std::fs::write(&tmp, blob).and_then(|()| std::fs::rename(&tmp, &path)) {
                log::warn!("saving the log key failed ({e}): tags in the log change at the next start");
            }
        }
        Err(e) => log::warn!("protecting the log key failed ({e}): tags in the log change at the next start"),
    }
    key
}

/// The key of this process: the installation's, or (a unit test, a tool that never called [`init`]) a random one for this run only.
fn key() -> &'static [u8; KEY_BYTES] {
    KEY.get_or_init(random_bytes::<KEY_BYTES>)
}

/// A keyed tag of `data` for a log line: 8 hex digits (the first 32 bits of the HMAC).
pub fn tag(data: &[u8]) -> String {
    hmac_sha256(key(), data)[..4].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hmac_sha256_matches_the_rfc_4231_test_vectors() {
        // Test case 1
        assert_eq!(hex(&hmac_sha256(&[0x0b; 20], b"Hi There")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        // Test case 2
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block size
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn a_tag_is_8_hex_digits_stable_and_not_the_unkeyed_hash() {
        let t = tag(b"my secret text");
        assert_eq!(t.len(), 8);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(t, tag(b"my secret text"), "the same text, the same tag");
        assert_ne!(t, tag(b"another text"));
        let unkeyed: String = sha256(b"my secret text")[..4].iter().map(|b| format!("{b:02x}")).collect();
        assert_ne!(t, unkeyed, "a tag can't be checked against a plain SHA-256 of a guess");
        // A different key gives a different tag for the same text.
        assert_ne!(hex(&hmac_sha256(&[1; 32], b"x")[..4]), hex(&hmac_sha256(&[2; 32], b"x")[..4]));
    }

    #[test]
    fn the_key_is_made_once_kept_protected_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path());
        let file = std::fs::read(dir.path().join(KEY_FILE)).unwrap();
        assert!(!file.windows(KEY_BYTES).any(|w| w == first), "the key is not in the file in the clear");
        assert_eq!(load_or_create(dir.path()), first, "the same key at the next start");
        std::fs::write(dir.path().join(KEY_FILE), b"not a DPAPI blob").unwrap();
        assert_ne!(load_or_create(dir.path()), first, "a key that can't be read is replaced");
    }
}
