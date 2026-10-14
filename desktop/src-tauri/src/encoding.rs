//! The encodings of protocol.md: base64url, base32, device and short IDs, pairing tokens, message IDs.
use sha2::{Digest, Sha256};

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub const DEVICE_ID_LEN: usize = 43;
pub const TOKEN_LEN: usize = 22;
pub const SHORT_ID_LEN: usize = 8;

/// RFC 4648 §5 without padding.
pub fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(B64URL[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// RFC 4648 §6, uppercase, without padding.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0);
    for b in bytes {
        buf = buf << 8 | *b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(B32[(buf >> (bits - 5) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[(buf << (5 - bits) & 31) as usize] as char);
    }
    out
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn device_id_from_hash(hash: &[u8; 32]) -> String {
    base64url(hash)
}

/// The first 8 characters of the base32 form of the hash. The UI may add a hyphen (see [`format_short_id`]).
pub fn short_id_from_hash(hash: &[u8; 32]) -> String {
    base32(hash)[..SHORT_ID_LEN].to_string()
}

pub fn device_id_from_spki(spki: &[u8]) -> String {
    device_id_from_hash(&sha256(spki))
}

pub fn short_id_from_spki(spki: &[u8]) -> String {
    short_id_from_hash(&sha256(spki))
}

pub fn format_short_id(short_id: &str) -> String {
    match short_id.split_at_checked(4) {
        Some((a, b)) if !b.is_empty() => format!("{a}-{b}"),
        _ => short_id.to_string(),
    }
}

/// The SubjectPublicKeyInfo exactly as it appears in the certificate.
pub fn spki_of_certificate(cert_der: &[u8]) -> Option<&[u8]> {
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der).ok()?;
    Some(cert.tbs_certificate.subject_pki.raw)
}

/// Length and alphabet only, as the "validity check" of protocol.md says.
pub fn is_base64url_of_len(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| B64URL.contains(&b))
}

pub fn is_valid_device_id(s: &str) -> bool {
    is_base64url_of_len(s, DEVICE_ID_LEN)
}

pub fn is_valid_token(s: &str) -> bool {
    is_base64url_of_len(s, TOKEN_LEN)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("the OS random generator is available");
    buf
}

/// 16 random bytes as 22 base64url characters.
pub fn new_token() -> String {
    base64url(&random_bytes::<16>())
}

/// UUIDv7, lowercase and hyphenated: 48 bits of Unix milliseconds, then random bits.
pub fn uuid_v7(unix_ms: u64) -> String {
    let mut b = random_bytes::<16>();
    b[..6].copy_from_slice(&unix_ms.to_be_bytes()[2..]);
    b[6] = 0x70 | (b[6] & 0x0f);
    b[8] = 0x80 | (b[8] & 0x3f);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Percent-encodes UTF-8 bytes, leaving only `A-Z a-z 0-9 - . _ ~`, with uppercase hex.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Cuts to at most `max` code points (not bytes, not UTF-16 units).
pub fn truncate_chars(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(i, _)| &s[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_and_base32_known_values() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(b"fo"), "MZXQ");
    }

    #[test]
    fn uuid_v7_has_version_and_variant() {
        let id = uuid_v7(0x0193_a5c2_7b1e);
        assert_eq!(id.len(), 36);
        assert!(id.starts_with("0193a5c2-7b1e-7"));
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn truncates_on_code_points() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("😀😀😀", 2), "😀😀");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }
}
