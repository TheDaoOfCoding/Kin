//! "Export logs" (milestone 1.4, the debug window): the log files and a redacted `status.json` (and the debug window's numbers) in one zip
//! at a place the user chose. The zip is for sending to the developer, so what identifies the person is taken out first.
//!
//! **What is redacted.** In the log lines and in every string of the JSON: the Windows user's profile path (`C:\Users\<name>` becomes
//! `%USERPROFILE%`), IPv4 addresses (`<ip>`), this PC's name (`<pc>`) and the names of every phone paired now or before (`<phone>`). In the JSON also the full
//! device IDs (cut to the first 8 characters) and the file names of the transfers (`names` becomes a count). **What is never in the logs to begin
//! with:** keys, tokens, the pairing URL, the text of a clipboard or a message, file names (see desktop/README.md, "Logs").
//!
//! The zip is written by hand with the "stored" method (no compression): five logs of 1 MB each are small, and it keeps a dependency out of
//! the installer. It is read back by .NET's `ZipFile` in `check-1-4.ps1` and by a reader in the tests.
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::Core;
use crate::encoding::unix_ms;

/// The CRC-32 of the zip format (IEEE 802.3, reflected, polynomial 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    !data.iter().fold(!0u32, |crc, b| table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8))
}

/// The MS-DOS date and time of a Unix time (UTC), as a zip entry stores them: `(time, date)`.
fn dos_time(unix_secs: u64) -> (u16, u16) {
    let days = (unix_secs / 86_400) as i64;
    let secs = unix_secs % 86_400;
    // days since 1970-01-01 -> civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    let year = (year - 1980).clamp(0, 127) as u16;
    let time = ((secs / 3600) as u16) << 11 | (((secs % 3600) / 60) as u16) << 5 | ((secs % 60) / 2) as u16;
    (time, year << 9 | (month as u16) << 5 | day as u16)
}

/// A zip with the given files stored as they are. Names are UTF-8 (flag bit 11), paths use `/`.
pub fn build_zip(files: &[(String, Vec<u8>)], unix_secs: u64) -> Vec<u8> {
    let (time, date) = dos_time(unix_secs);
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name = name.as_bytes();
        // local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        // central directory entry
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra length
        central.extend_from_slice(&0u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let central_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk of the central directory
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// A reader for the zips this file writes (stored entries only): the names and contents, or why it isn't one. Used by the tests.
pub fn read_zip(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| zip.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| zip.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let eocd = zip.len().checked_sub(22).filter(|&i| u32_at(i) == Some(0x0605_4b50)).ok_or("no end of central directory")?;
    let count = u16_at(eocd + 10).ok_or("short")? as usize;
    let mut at = u32_at(eocd + 16).ok_or("short")? as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32_at(at) != Some(0x0201_4b50) {
            return Err("bad central directory entry".into());
        }
        let (method, crc) = (u16_at(at + 10).ok_or("short")?, u32_at(at + 16).ok_or("short")?);
        let size = u32_at(at + 24).ok_or("short")? as usize;
        let (name_len, extra_len, comment_len) =
            (u16_at(at + 28).ok_or("short")? as usize, u16_at(at + 30).ok_or("short")? as usize, u16_at(at + 32).ok_or("short")? as usize);
        let local = u32_at(at + 42).ok_or("short")? as usize;
        let name = String::from_utf8(zip.get(at + 46..at + 46 + name_len).ok_or("short")?.to_vec()).map_err(|e| e.to_string())?;
        if method != 0 {
            return Err(format!("{name}: not stored"));
        }
        if u32_at(local) != Some(0x0403_4b50) {
            return Err(format!("{name}: bad local header"));
        }
        let (lname, lextra) = (u16_at(local + 26).ok_or("short")? as usize, u16_at(local + 28).ok_or("short")? as usize);
        let data = zip.get(local + 30 + lname + lextra..local + 30 + lname + lextra + size).ok_or("short data")?.to_vec();
        if crc32(&data) != crc {
            return Err(format!("{name}: wrong CRC"));
        }
        out.push((name, data));
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// What is taken out of an exported text (see the top of this file).
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    /// `C:\Users\<name>`: replaced by `%USERPROFILE%`.
    pub home: Option<String>,
    /// The same folder in its 8.3 short form (`C:\Users\ANNABE~1`), which Windows hands out as `%TEMP%` and some tools write instead of the long one.
    pub home_short: Option<String>,
    /// This PC's name as the phone sees it, and the computer's own name when that differs: replaced by `<pc>`.
    pub pc_names: Vec<String>,
    pub phone_names: Vec<String>,
    /// The folder that received files go to, wherever it is (a second drive, a share): replaced by `<received>`. It is tried before the
    /// profile path, so a received folder inside the profile is `<received>` too.
    pub received: Option<String>,
}

impl Redactor {
    /// Redacts one string: paths, addresses and names.
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some(dir) = self.received.as_deref().map(|d| d.trim_end_matches(['\\', '/'])).filter(|d| !d.is_empty()) {
            out = replace_ignore_case(&out, dir, "<received>");
            out = replace_ignore_case(&out, &dir.replace('\\', "/"), "<received>");
        }
        for home in [self.home.as_deref(), self.home_short.as_deref()].into_iter().flatten().filter(|h| !h.is_empty()) {
            out = replace_ignore_case(&out, home, "%USERPROFILE%");
            // The same path with forward slashes, as some libraries write it.
            out = replace_ignore_case(&out, &home.replace('\\', "/"), "%USERPROFILE%");
        }
        // Whatever else looks like a profile folder (another user, a form that wasn't predicted) loses the user's name as well.
        out = redact_user_folders(&out);
        // Longer names first, so "Pixel 8 Pro" doesn't leave "Pro" behind a replaced "Pixel 8".
        let mut names: Vec<&str> = self.phone_names.iter().map(String::as_str).filter(|n| n.chars().count() >= 2).collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        for name in names {
            out = replace_ignore_case(&out, name, "<phone>");
        }
        for name in self.pc_names.iter().filter(|n| n.chars().count() >= 2) {
            out = replace_ignore_case(&out, name, "<pc>");
        }
        // IPv6 first: an IPv4-mapped address (`::ffff:192.168.1.5`) is one address, not an address with an `<ip>` inside it.
        redact_ipv4(&redact_ipv6(&out))
    }

    /// Redacts every string in a JSON value, cuts device IDs to their first 8 characters and turns lists of file names into counts.
    pub fn apply_json(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.apply(s)),
            Value::Array(a) => Value::Array(a.iter().map(|v| self.apply_json(v)).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, v)| {
                        let v = match (k.as_str(), v) {
                            ("deviceId", Value::String(s)) => Value::String(s.chars().take(8).collect()),
                            ("names", Value::Array(a)) => serde_json::json!(a.len()),
                            _ => self.apply_json(v),
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// `C:\Users\<anything>` and `C:/Users/<anything>` (any drive, any case) become `C:\Users\<user>`: the name is the part up to the next
/// separator, a quote or a blank, which covers long names without spaces and the 8.3 form (`ANNABE~1`). (The current user's own folder was
/// replaced by `%USERPROFILE%` before this, spaces and all.)
pub fn redact_user_folders(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        // `X:` + separator + `Users` + separator
        if bytes[i].is_ascii_alphabetic()
            && bytes.get(i + 1) == Some(&b':')
            && matches!(bytes.get(i + 2), Some(b'\\' | b'/'))
            && text.get(i + 3..i + 8).is_some_and(|s| s.eq_ignore_ascii_case("users"))
            && matches!(bytes.get(i + 8), Some(b'\\' | b'/'))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
        {
            let name_start = i + 9;
            let name_len =
                text[name_start..].find(['\\', '/', '"', '\'', ' ', '\t', ',', ';', ')', ']', '>']).unwrap_or(text.len() - name_start);
            if name_len > 0 && !text[name_start..].starts_with("<user>") {
                out.push_str(&text[i..name_start]);
                out.push_str("<user>");
                i = name_start + name_len;
                continue;
            }
        }
        let ch = text[i..].chars().next().expect("a character");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `{ "networks": [{ "name": …, "category": … }] }` without the names: `{ "networks": [{ "category": … }] }`. Anything else is kept as it is
/// (an error text from Windows has no network name in it).
pub fn network_categories_only(network: &Value) -> Value {
    let mut out = network.clone();
    if let Some(list) = out.get_mut("networks").and_then(Value::as_array_mut) {
        for item in list {
            if let Some(obj) = item.as_object_mut() {
                obj.retain(|k, _| k == "category");
            }
        }
    }
    out
}

/// The 8.3 short form of a path, if Windows has one (`GetShortPathNameW`).
pub fn short_path(path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    use windows::core::HSTRING;
    let long = HSTRING::from(path);
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `long` is a valid, NUL-terminated wide string for the call, and `buffer` is a writable slice of the length it is told.
    let n = unsafe { GetShortPathNameW(&long, Some(&mut buffer)) } as usize;
    (n > 0 && n < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..n])).filter(|s| !s.eq_ignore_ascii_case(path))
}

fn replace_ignore_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let (lower, needle_lower) = (text.to_lowercase(), needle.to_lowercase());
    // Lower-casing can change a string's length (a few non-ASCII letters); then the positions don't line up and the exact match is used.
    if lower.len() != text.len() || needle_lower.len() != needle.len() {
        return text.replace(needle, with);
    }
    let mut out = String::new();
    let mut last = 0;
    for (i, _) in lower.match_indices(&needle_lower) {
        if i < last {
            continue;
        }
        out.push_str(&text[last..i]);
        out.push_str(with);
        last = i + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

/// `192.168.1.5` and `192.168.1.5:50123` become `<ip>`. A version such as `0.1.0` has three numbers and is left alone.
pub fn redact_ipv4(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.')) {
            // four groups of 1 to 3 digits separated by dots
            let mut j = i;
            let mut groups = 0;
            loop {
                let start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() && j - start < 3 {
                    j += 1;
                }
                if j == start {
                    break;
                }
                groups += 1;
                if groups == 4 || j >= bytes.len() || bytes[j] != b'.' || !bytes.get(j + 1).is_some_and(u8::is_ascii_digit) {
                    break;
                }
                j += 1;
            }
            let followed_by_digit = bytes.get(j).is_some_and(u8::is_ascii_digit)
                || (bytes.get(j) == Some(&b'.') && bytes.get(j + 1).is_some_and(u8::is_ascii_digit));
            if groups == 4 && !followed_by_digit {
                // an optional :port
                if bytes.get(j) == Some(&b':') {
                    let mut k = j + 1;
                    while k < bytes.len() && bytes[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k > j + 1 {
                        j = k;
                    }
                }
                out.push_str("<ip>");
                i = j;
                continue;
            }
        }
        let ch = text[i..].chars().next().expect("a character");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// IPv6 addresses (`fe80::1%12`, `2001:db8::5`, `[::1]:47821`, `::ffff:10.0.0.5`) become `<ip>`. A candidate is a run of hex digits, `:` and `.`
/// that starts and ends at a word boundary and that `Ipv6Addr` accepts, so a time (`06:35:39`), a MAC address (six groups, no `::`) and a Rust
/// path (`std::fmt`, which starts in the middle of a word) are left alone.
pub fn redact_ipv6(text: &str) -> String {
    let bytes = text.as_bytes();
    let is_run = |b: u8| b.is_ascii_hexdigit() || b == b':' || b == b'.';
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if is_run(bytes[i]) && (i == 0 || !is_word(bytes[i - 1])) {
            let mut j = i;
            while j < bytes.len() && is_run(bytes[j]) {
                j += 1;
            }
            // A sentence's full stop or a colon that ends a line is not part of the address (`::` at the end is).
            let mut end = j;
            while end > i && bytes[end - 1] == b'.' {
                end -= 1;
            }
            if end > i + 1 && bytes[end - 1] == b':' && bytes[end - 2] != b':' {
                end -= 1;
            }
            let candidate = &text[i..end];
            let mut after = end;
            if bytes.get(after) == Some(&b'%') {
                // a zone (`%12`, `%eth0`)
                let mut k = after + 1;
                while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                    k += 1;
                }
                after = k;
            }
            let valid = candidate.contains(':')
                && candidate.bytes().any(|b| b.is_ascii_hexdigit())
                && candidate.parse::<std::net::Ipv6Addr>().is_ok()
                && !bytes.get(after).is_some_and(|&b| is_word(b));
            if valid {
                // `[addr]` and `[addr]:port`
                if out.ends_with('[') && bytes.get(after) == Some(&b']') {
                    out.pop();
                    after += 1;
                    if bytes.get(after) == Some(&b':') && bytes.get(after + 1).is_some_and(u8::is_ascii_digit) {
                        after += 1;
                        while bytes.get(after).is_some_and(u8::is_ascii_digit) {
                            after += 1;
                        }
                    }
                }
                out.push_str("<ip>");
                i = after;
            } else {
                out.push_str(&text[i..j]);
                i = j;
            }
            continue;
        }
        let ch = text[i..].chars().next().expect("a character");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The profile folder of the Windows user, `C:\Users\<name>`.
pub fn user_profile() -> Option<String> {
    std::env::var("USERPROFILE").ok().filter(|p| !p.is_empty())
}

/// What an export contained, for the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: usize,
}

impl Core {
    fn redactor(&self) -> Redactor {
        // Every phone name this PC has known, not only the phones paired now: the logs still hold the name of a phone that was unpaired (or
        // renamed) since the line was written.
        let mut phone_names: Vec<String> = self.snapshot().devices.into_iter().map(|d| d.name).collect();
        match self.db.known_device_names() {
            Ok(known) => phone_names.extend(known),
            Err(e) => log::warn!("reading the known device names for the export failed: {e}"),
        }
        phone_names.sort();
        phone_names.dedup();
        // The name this PC shows (the device name setting) and the computer's own name: both identify the person's PC.
        let mut pc_names = vec![self.pc_name()];
        if pc_names[0] != self.default_pc_name() {
            pc_names.push(self.default_pc_name().to_string());
        }
        let home = user_profile();
        let home_short = home.as_deref().and_then(short_path);
        let received = Some(self.received_dir().to_string_lossy().into_owned());
        Redactor { home, home_short, pc_names, phone_names, received }
    }

    /// The files of an export: the log files with the user's paths, addresses and names taken out, `status.json` and `debug.json`
    /// (redacted), and a short `README.txt` that says what was taken out.
    pub fn export_files(&self, network: Value) -> Vec<(String, Vec<u8>)> {
        let redactor = self.redactor();
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        files.push(("README.txt".into(), README.as_bytes().to_vec()));
        let logs = self.config.data_dir.join("logs");
        let mut names: Vec<String> = std::fs::read_dir(&logs)
            .map(|r| {
                r.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| n.starts_with("kin") && n.ends_with(".log"))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        for name in names {
            if let Ok(bytes) = std::fs::read(logs.join(&name)) {
                let text = String::from_utf8_lossy(&bytes);
                let redacted: String = text.lines().map(|l| redactor.apply(l)).collect::<Vec<_>>().join("\n");
                files.push((format!("logs/{name}"), format!("{redacted}\n").into_bytes()));
            }
        }
        let mut status = redactor.apply_json(&self.status_json());
        // The copy that leaves this PC has no battery level of the phone (the file in the data directory keeps its numbers for a test run).
        crate::presence::redact_status_for_export(&mut status);
        files.push(("status.json".into(), serde_json::to_vec_pretty(&status).unwrap_or_default()));
        // The name of a Wi-Fi network is the person's own (a home, a workplace, a street): only the category (Private, Public, Domain) is exported.
        let debug = redactor.apply_json(&self.debug_view(network_categories_only(&network)));
        files.push(("debug.json".into(), serde_json::to_vec_pretty(&debug).unwrap_or_default()));
        files
    }

    /// Writes the export zip to `dest` (a file path the user chose). The file is written next to its final name and then renamed.
    pub fn export_logs(&self, dest: &Path, network: Value) -> Result<ExportReport, String> {
        let files = self.export_files(network);
        let zip = build_zip(&files, unix_ms() / 1000);
        let tmp = dest.with_extension("zip.tmp");
        std::fs::write(&tmp, &zip).map_err(|e| format!("Could not write the file: {e}"))?;
        std::fs::rename(&tmp, dest).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Could not write the file: {e}")
        })?;
        log::info!("logs exported: {} file(s), {} bytes", files.len(), zip.len());
        Ok(ExportReport { path: dest.to_path_buf(), files: files.len(), bytes: zip.len() })
    }
}

const README: &str = "Kin Relay: exported logs\r\n\r\nlogs\\kin*.log   the log files (the newest is kin.log)\r\nstatus.json     what the app was doing when this was exported\r\ndebug.json      the success rate, latency, recent errors and connection history of the debug window\r\n\r\nTaken out before this file was made: your Windows profile path in its long and its short (8.3) form and the folder of any other user under C:\\Users (%USERPROFILE%, the name becomes <user>), IP addresses, IPv4 and IPv6 (<ip>), the folder that received files go to (<received>), the name of this PC (<pc>) and of every phone paired now or before (<phone>), the names of Wi-Fi networks (only Private, Public or Domain stays), device IDs (cut to 8 characters) and file names (counts only). The phone's battery level and Do Not Disturb state, and whether this PC is in Focus, are left out of status.json too.\r\nNever in a log to begin with: keys, pairing tokens, the pairing link, the text of the clipboard or of a message, and file names.\r\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn the_zip_reads_back_with_names_and_contents() {
        let files = vec![
            ("logs/kin.log".to_string(), b"line one\nline two\n".to_vec()),
            ("status.json".to_string(), b"{}".to_vec()),
            ("empty.txt".to_string(), Vec::new()),
            ("logs/\u{e9}.log".to_string(), "caf\u{e9}".as_bytes().to_vec()),
        ];
        let zip = build_zip(&files, 1_760_443_200);
        assert_eq!(read_zip(&zip).unwrap(), files);
        let mut bad = zip.clone();
        bad[45] ^= 0xFF; // inside the first file's data (30 bytes of header and a 12 byte name come first)
        assert!(read_zip(&bad).unwrap_err().contains("CRC"), "a flipped byte is noticed");
    }

    #[test]
    fn dos_time_is_the_utc_time_of_day_and_date() {
        // 2025-10-14 12:00:00 UTC = 1760443200
        let (time, date) = dos_time(1_760_443_200);
        assert_eq!((time >> 11, (time >> 5) & 63, (time & 31) * 2), (12, 0, 0));
        assert_eq!(((date >> 9) + 1980, (date >> 5) & 15, date & 31), (2025, 10, 14));
        let (_, leap) = dos_time(1_709_164_800); // 2024-02-29
        assert_eq!((leap >> 5 & 15, leap & 31), (2, 29));
    }

    #[test]
    fn addresses_paths_and_names_are_taken_out() {
        let r = Redactor {
            home: Some("C:\\Users\\Anna Berg".into()),
            home_short: Some("C:\\Users\\ANNABE~1".into()),
            pc_names: vec!["ANNA-PC".into()],
            phone_names: vec!["Anna's Pixel 8".into(), "Pixel".into()],
            received: None,
        };
        assert_eq!(r.apply("data dir C:\\Users\\Anna Berg\\AppData\\Roaming\\Kin"), "data dir %USERPROFILE%\\AppData\\Roaming\\Kin");
        assert_eq!(r.apply("c:/users/anna berg/x"), "%USERPROFILE%/x");
        assert_eq!(r.apply("phone connected: \"Anna's Pixel 8\" from 192.168.1.20:50123"), "phone connected: \"<phone>\" from <ip>");
        assert_eq!(r.apply("Kin on anna-pc"), "Kin on <pc>");
        assert_eq!(r.apply("version 0.1.0 and 10.0.2.2"), "version 0.1.0 and <ip>", "a version is not an address");
        assert_eq!(r.apply("1.2.3.4.5"), "1.2.3.4.5", "five numbers are not an address");
        assert_eq!(redact_ipv4("a 8.8.8.8, b 127.0.0.1:47821."), "a <ip>, b <ip>.");
        assert_eq!(redact_ipv4("id 1234567.89"), "id 1234567.89");
    }

    #[test]
    fn the_short_8_3_profile_path_and_any_other_user_folder_lose_the_name_too() {
        let r = Redactor {
            home: Some("C:\\Users\\Maria Lindqvist".into()),
            home_short: Some("C:\\Users\\MARIAL~1".into()),
            pc_names: vec![],
            phone_names: vec![],
            received: None,
        };
        assert_eq!(r.apply("temp C:\\Users\\MARIAL~1\\AppData\\Local\\Temp\\x"), "temp %USERPROFILE%\\AppData\\Local\\Temp\\x");
        assert_eq!(r.apply("temp c:/users/marial~1/AppData"), "temp %USERPROFILE%/AppData");
        assert_eq!(r.apply("C:\\Users\\Maria Lindqvist\\Downloads\\Kin"), "%USERPROFILE%\\Downloads\\Kin");
        // Not predicted: another user, another drive, a form the redactor was not given.
        assert_eq!(r.apply("opened D:\\Users\\Bob\\Documents\\a.txt"), "opened D:\\Users\\<user>\\Documents\\a.txt");
        assert_eq!(r.apply("C:\\USERS\\ANNABE~1\\x and \"C:\\Users\\Eve\" ok"), "C:\\USERS\\<user>\\x and \"C:\\Users\\<user>\" ok");
        assert_eq!(redact_user_folders("C:\\Users\\<user>\\x"), "C:\\Users\\<user>\\x", "idempotent");
        assert_eq!(redact_user_folders("no path here, only Users\\Bob and C:Users"), "no path here, only Users\\Bob and C:Users");
        // The real profile folder has a short form that is found by Windows (when the folder has one) and never equals the long one.
        if let Some(home) = user_profile()
            && let Some(short) = short_path(&home)
        {
            assert_ne!(short.to_lowercase(), home.to_lowercase());
            assert!(!short.contains(' '));
        }
    }

    #[test]
    fn the_names_of_wifi_networks_are_not_exported_only_their_category() {
        let net = serde_json::json!({ "networks": [{ "name": "Anna's Home WiFi", "category": "Private" }, { "name": "Cafe", "category": "Public" }] });
        let out = network_categories_only(&net);
        assert_eq!(out, serde_json::json!({ "networks": [{ "category": "Private" }, { "category": "Public" }] }));
        assert!(!out.to_string().contains("Home WiFi") && !out.to_string().contains("Cafe"));
        let error = serde_json::json!({ "networks": [], "error": "COM failed" });
        assert_eq!(network_categories_only(&error), error);
    }

    #[test]
    fn json_keeps_its_shape_but_loses_ids_names_and_paths() {
        let r = Redactor {
            home: Some("C:\\Users\\x".into()),
            home_short: None,
            pc_names: vec!["BOX".into()],
            phone_names: vec!["Pixel 8".into()],
            received: None,
        };
        let v = serde_json::json!({
            "deviceId": "YoJqfu2V_Y-CgWunLoXVix4RiuqUOB_OnaL68etn9O4",
            "devices": [{ "deviceId": "ABCDEFGHIJKLMNOP", "name": "Pixel 8", "connected": true }],
            "transfers": { "recent": [{ "names": ["secret.pdf", "plan.docx"], "files": 2 }], "receivedFolder": "C:\\Users\\x\\Downloads\\Kin" },
            "n": 5
        });
        let out = r.apply_json(&v);
        assert_eq!(out["deviceId"], "YoJqfu2V");
        assert_eq!(out["devices"][0]["deviceId"], "ABCDEFGH");
        assert_eq!(out["devices"][0]["name"], "<phone>");
        assert_eq!(out["transfers"]["recent"][0]["names"], 2, "file names become a count");
        assert_eq!(out["transfers"]["receivedFolder"], "%USERPROFILE%\\Downloads\\Kin");
        assert_eq!((out["n"].clone(), out["devices"][0]["connected"].clone()), (serde_json::json!(5), serde_json::json!(true)));
        assert!(!out.to_string().contains("secret.pdf"));
    }

    #[test]
    fn phone_names_are_found_in_any_case() {
        let r = Redactor { phone_names: vec!["Anna's Pixel 8".into(), "Pixel".into()], ..Redactor::default() };
        assert_eq!(r.apply("phone connected: \"ANNA'S PIXEL 8\" and \"anna's pixel 8\""), "phone connected: \"<phone>\" and \"<phone>\"");
        assert_eq!(r.apply("the pixel said hello"), "the <phone> said hello");
        // A name of one letter is not used (it would redact half the log).
        let short = Redactor { phone_names: vec!["P".into()], ..Redactor::default() };
        assert_eq!(short.apply("a p b"), "a p b");
    }

    #[test]
    fn ipv6_addresses_are_taken_out_and_times_macs_and_paths_are_not() {
        let r = Redactor::default();
        assert_eq!(r.apply("from fe80::1c2b:3dff:fe4a:5b6c%12 ok"), "from <ip> ok");
        assert_eq!(r.apply("from [2001:db8::5]:47821 (listening on [::1]:47821)"), "from <ip> (listening on <ip>)");
        assert_eq!(r.apply("addr 2001:0DB8:85A3:0000:0000:8A2E:0370:7334."), "addr <ip>.");
        assert_eq!(r.apply("peer ::ffff:192.168.1.5 and ::1: refused"), "peer <ip> and <ip>: refused");
        assert_eq!(r.apply("fd00::abcd, FE80::1"), "<ip>, <ip>");
        // Not addresses
        assert_eq!(r.apply("2026-10-15T06:35:39.123456Z INFO ok"), "2026-10-15T06:35:39.123456Z INFO ok");
        assert_eq!(
            r.apply("mac aa:bb:cc:dd:ee:ff, ratio 3:4, std::fmt::Display, a::b::c, ::"),
            "mac aa:bb:cc:dd:ee:ff, ratio 3:4, std::fmt::Display, a::b::c, ::"
        );
        assert_eq!(r.apply("12:34:56 and 1:2:3:4:5:6:7"), "12:34:56 and 1:2:3:4:5:6:7");
        assert_eq!(redact_ipv6("x"), "x");
        assert_eq!(redact_ipv6("caf\u{e9} fe80::1 \u{e9}"), "caf\u{e9} <ip> \u{e9}");
    }

    #[test]
    fn a_received_folder_anywhere_becomes_received() {
        let r =
            Redactor { home: Some(r"C:\Users\Anna".into()), received: Some(r"D:\Shared Files\Kin Inbox\".into()), ..Redactor::default() };
        assert_eq!(r.apply(r"received D:\Shared Files\Kin Inbox\a\b.txt"), r"received <received>\a\b.txt");
        assert_eq!(r.apply("d:/shared files/kin inbox/x"), "<received>/x");
        // Inside the profile it is `<received>` as well (tried first), and the rest of the profile is still the profile.
        let inside =
            Redactor { home: Some(r"C:\Users\Anna".into()), received: Some(r"C:\Users\Anna\Downloads\Kin".into()), ..Redactor::default() };
        assert_eq!(inside.apply(r"C:\Users\Anna\Downloads\Kin\f and C:\Users\Anna\Documents"), r"<received>\f and %USERPROFILE%\Documents");
        let v = serde_json::json!({ "transfers": { "receivedFolder": r"D:\Shared Files\Kin Inbox" } });
        assert_eq!(r.apply_json(&v)["transfers"]["receivedFolder"], "<received>");
        // An empty setting redacts nothing.
        let none = Redactor { received: Some(String::new()), ..Redactor::default() };
        assert_eq!(none.apply("anything"), "anything");
    }
}
