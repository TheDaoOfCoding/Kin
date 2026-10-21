//! The developer clipboard sink (`--dev-clipboard-sink <dir>`, milestone 2B.6): a [`Clipboard`] that **never opens the real Windows clipboard**.
//!
//! Why: tests overwrote the real clipboard of the person at the keyboard three times in one week, however carefully a script saved and restored it
//! (a manual probe in the middle of a run is enough). So a test instance does not get a way to do it: with the sink on, `app.rs` does not even start the
//! clipboard listener window (`clipboard::WinClipboard`), and every read and write of Kin goes through the files in the sink directory instead.
//! [`open`] is the one place that decides, and its test proves the real backend is not asked for when a sink is set.
//!
//! **Kin writes** (the phone's text, a link, a received image, the auto-sent screenshot, the one-time code, `--dev-set-clipboard`), one file or two per write,
//! numbered from 1:
//!
//! * `write-<n>.txt`: the text, UTF-8, exactly as Kin wrote it (line ends included).
//! * `write-<n>.dib`: for an image, the exact `CF_DIBV5` bytes Kin would have put on the clipboard (124-byte header, then bottom-up upright BGRA pixels, EXIF
//!   rotation applied: `images::decode_for_sink` is the same decoder), and `write-<n>.png` with the original PNG bytes when the file is a PNG (the `PNG` format).
//! * `index.json`: `{"version":1,"writes":[...]}`, rewritten after every write. An entry has `n`, `kind` (`text`, `image` or `clear`), `file` (and `png`),
//!   `atMs`, `chars` or `width`/`height`/`source`, the list of `formats` Kin would have put on the clipboard (`name`, `bytes`, `dword` for the value formats)
//!   and the flags that matter to a test: `sensitive` (the write used the sensitive API), `quiet` (an instance beside the installed Kin), `excludedFromMonitor`,
//!   `excludedFromHistory` (`CanIncludeInClipboardHistory` = 0), `excludedFromCloud` (`CanUploadToCloudClipboard` = 0) and `testContent`.
//! * `current.json`: what a read of the clipboard sees now: `sequence`, `source` (`app` or `input`), `text` and the `formats` (what `Get-ClipboardFormats` of
//!   `desktop/clipboard-backup.ps1` lists for the real clipboard).
//!
//! **The user copies** (the PC to phone clipboard sync): a test writes `input.txt` (UTF-8; missing = a copy with no text) and optionally `input.formats` (one
//! opt-out format per line: `name` or `name=dword`), **then** changes `input.seq` (any text). Kin polls `input.seq` every 100 ms; a new value is one copy event
//! with the debounce, the exclusion rules and the loop guard of the real clipboard. The value that is already there at start is not an event.
//!
//! **Privacy:** these files hold the text of the clipboard: a dev directory of a test run, never a log (the log carries a length and `short_hash` only, as ever).
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::Result;
use crate::cli::Config;
use crate::clipboard::{
    CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD, ClipFormat, ClipSnapshot, Clipboard, DEV_TEST_CONTENT, EXCLUDE_FROM_MONITOR, Exclusion,
    INTERESTING_FORMATS, ImageOutcome, SENSITIVE_MARKS, text_marks,
};

/// How often Kin looks at `input.seq`.
pub const POLL_MS: u64 = 100;
pub const INPUT_TEXT: &str = "input.txt";
pub const INPUT_FORMATS: &str = "input.formats";
pub const INPUT_SEQ: &str = "input.seq";
pub const INDEX: &str = "index.json";
pub const CURRENT: &str = "current.json";

/// A clipboard backend and the receiver of its change events: what `app.rs` needs, whichever backend it is.
pub type Backend = (Arc<dyn Clipboard>, UnboundedReceiver<()>);

/// Chooses the clipboard of this run. **With a sink directory in the config the real clipboard is never asked for** (`real` is not called): that is the whole
/// point of the switch, and the unit test of this function holds it to that. `real` gets `quiet` (an instance beside the installed Kin).
pub fn open(config: &Config, real: impl FnOnce(bool) -> Result<Backend>) -> Result<Backend> {
    let quiet = config.instance_key().is_some();
    match &config.dev_clipboard_sink {
        Some(dir) => {
            let (sink, changes) = FileSinkClipboard::start(dir, quiet)?;
            log::warn!(
                "clipboard sink: the real Windows clipboard is NEVER opened in this run; writes go to files in {}, copy events come from {INPUT_SEQ} there",
                dir.display()
            );
            Ok((Arc::new(sink), changes))
        }
        None => real(quiet),
    }
}

/// One format on the clipboard, as the sink lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Listed {
    name: String,
    bytes: usize,
    dword: Option<u32>,
}

impl Listed {
    fn json(&self) -> Value {
        let mut v = json!({ "name": self.name, "bytes": self.bytes });
        if let Some(d) = self.dword {
            v["dword"] = json!(d);
        }
        v
    }
}

struct State {
    snapshot: ClipSnapshot,
    listing: Vec<Listed>,
    /// `app` (Kin wrote it) or `input` (a test "copied" it).
    source: &'static str,
    sequence: u32,
    writes: Vec<Value>,
    next: u64,
    input_seen: Option<String>,
}

struct Inner {
    dir: PathBuf,
    quiet: bool,
    state: Mutex<State>,
    changes: UnboundedSender<()>,
    stopped: AtomicBool,
}

/// The clipboard that lives in a directory. See the module comment.
pub struct FileSinkClipboard {
    inner: Arc<Inner>,
}

fn io(context: &str, e: std::io::Error) -> Box<dyn std::error::Error + Send + Sync> {
    format!("clipboard sink: {context}: {e}").into()
}

/// Written next to the target and renamed over it, so a reader never sees half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    // A reader that has the target open (a script, an indexer) can make a rename fail for a moment.
    let mut result = std::fs::rename(&tmp, path);
    for _ in 0..10 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        result = std::fs::rename(&tmp, path);
    }
    result
}

fn is_value_format(name: &str) -> bool {
    [CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD].iter().any(|k| k.eq_ignore_ascii_case(name))
}

/// The text of `CF_UNICODETEXT` for a text: UTF-16 and a NUL.
fn unicode_text_bytes(text: &str) -> usize {
    text.encode_utf16().count() * 2 + 2
}

/// The `ClipFormat`s Kin's rules look at, out of a listing: only the opt-out formats, and a value only for the two that carry one.
fn clip_formats(listing: &[Listed]) -> Vec<ClipFormat> {
    listing
        .iter()
        .filter_map(|l| {
            let known = INTERESTING_FORMATS.iter().find(|k| k.eq_ignore_ascii_case(&l.name))?;
            Some(ClipFormat { name: (*known).to_string(), dword: if is_value_format(known) { l.dword } else { None } })
        })
        .collect()
}

fn marks_listed(marks: &[(&str, u32)]) -> Vec<Listed> {
    marks.iter().map(|(name, value)| Listed { name: (*name).to_string(), bytes: 4, dword: Some(*value) }).collect()
}

/// The flags of an index entry, read off the formats.
fn flags(listing: &[Listed], sensitive: bool, quiet: bool) -> Value {
    let dword_of = |name: &str| listing.iter().find(|l| l.name.eq_ignore_ascii_case(name)).and_then(|l| l.dword);
    let has = |name: &str| listing.iter().any(|l| l.name.eq_ignore_ascii_case(name));
    json!({
        "sensitive": sensitive,
        "quiet": quiet,
        "excludedFromMonitor": has(EXCLUDE_FROM_MONITOR),
        "excludedFromHistory": dword_of(CAN_INCLUDE_IN_HISTORY) == Some(0),
        "excludedFromCloud": dword_of(CAN_UPLOAD_TO_CLOUD) == Some(0),
        "testContent": has(DEV_TEST_CONTENT),
    })
}

impl FileSinkClipboard {
    /// Opens (creates) the sink directory and starts the poller of `input.seq`. The receiver gets one `()` per change, like `WM_CLIPBOARDUPDATE`.
    pub fn start(dir: &Path, quiet: bool) -> Result<(FileSinkClipboard, UnboundedReceiver<()>)> {
        std::fs::create_dir_all(dir).map_err(|e| io("creating the directory", e))?;
        let (writes, next) = match std::fs::read(dir.join(INDEX)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            Some(index) => {
                let writes = index["writes"].as_array().cloned().unwrap_or_default();
                let next = writes.iter().filter_map(|w| w["n"].as_u64()).max().unwrap_or(0) + 1;
                (writes, next)
            }
            None => (Vec::new(), 1),
        };
        // The value that is there now is not a copy event: it is what an earlier run left.
        let input_seen = read_seq(dir);
        let (tx, rx) = unbounded_channel();
        let inner = Arc::new(Inner {
            dir: dir.to_path_buf(),
            quiet,
            state: Mutex::new(State {
                snapshot: ClipSnapshot::default(),
                listing: Vec::new(),
                source: "app",
                sequence: 0,
                writes,
                next,
                input_seen,
            }),
            changes: tx,
            stopped: AtomicBool::new(false),
        });
        {
            // Both files exist from the start, so a script can always read them.
            let state = inner.lock();
            inner.publish(&state)?;
            inner.write_index(&state)?;
        }
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        std::thread::Builder::new().name("clipboard-sink-input".into()).stack_size(256 * 1024).spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(POLL_MS));
                let Some(inner) = weak.upgrade() else { break };
                if inner.stopped.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(e) = inner.poll_input() {
                    log::warn!("{e}");
                }
            }
        })?;
        Ok((FileSinkClipboard { inner }, rx))
    }

    /// The directory of the sink.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }
}

impl Drop for FileSinkClipboard {
    fn drop(&mut self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
    }
}

/// The value of `input.seq`. An empty file is a writer caught half way: not a value.
fn read_seq(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(INPUT_SEQ)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `current.json` for the state.
    fn publish(&self, state: &State) -> Result<()> {
        let current = json!({
            "sequence": state.sequence,
            "source": state.source,
            "text": state.snapshot.text,
            "formats": state.listing.iter().map(Listed::json).collect::<Vec<_>>(),
        });
        write_atomic(&self.dir.join(CURRENT), serde_json::to_vec_pretty(&current)?.as_slice())
            .map_err(|e| io("writing current.json", e))?;
        Ok(())
    }

    fn write_index(&self, state: &State) -> Result<()> {
        let index = json!({ "version": 1, "writes": state.writes });
        write_atomic(&self.dir.join(INDEX), serde_json::to_vec_pretty(&index)?.as_slice()).map_err(|e| io("writing index.json", e))?;
        Ok(())
    }

    /// Makes `listing` (and the text, if any) what the clipboard holds, bumps the counter, writes `current.json` and tells the listener.
    fn replace(&self, state: &mut State, text: Option<String>, listing: Vec<Listed>, source: &'static str) -> Result<()> {
        state.snapshot = ClipSnapshot { text, formats: clip_formats(&listing) };
        state.listing = listing;
        state.source = source;
        state.sequence = state.sequence.wrapping_add(1);
        self.publish(state)?;
        let _ = self.changes.send(());
        Ok(())
    }

    /// Records a write in the index; returns its number.
    fn record(&self, state: &mut State, mut entry: Value, listing: &[Listed], sensitive: bool) -> Result<u64> {
        let n = state.next;
        state.next += 1;
        entry["n"] = json!(n);
        entry["atMs"] = json!(crate::encoding::unix_ms());
        entry["formats"] = json!(listing.iter().map(Listed::json).collect::<Vec<_>>());
        entry["flags"] = flags(listing, sensitive, self.quiet);
        state.writes.push(entry);
        Ok(n)
    }

    fn write_text(&self, text: &str, marks: &[(&str, u32)], sensitive: bool) -> Result<()> {
        let mut state = self.lock();
        let mut listing = vec![Listed { name: "CF_UNICODETEXT".into(), bytes: unicode_text_bytes(text), dword: None }];
        listing.extend(marks_listed(marks));
        // The number is taken first so the file name and the entry agree.
        let n = state.next;
        let file = format!("write-{n}.txt");
        std::fs::write(self.dir.join(&file), text.as_bytes()).map_err(|e| io("writing the text", e))?;
        let recorded =
            self.record(&mut state, json!({ "kind": "text", "file": file, "chars": text.chars().count() }), &listing, sensitive)?;
        debug_assert_eq!(n, recorded);
        self.write_index(&state)?;
        self.replace(&mut state, Some(text.to_string()), listing, "app")
    }

    fn write_image(&self, path: &Path, sensitive: bool) -> Result<ImageOutcome> {
        let image = match crate::images::decode_for_sink(path) {
            Ok(image) => image,
            Err(why) => return Ok(ImageOutcome::Skipped(why)),
        };
        let mut state = self.lock();
        let n = state.next;
        let mut listing = vec![Listed { name: "CF_DIBV5".into(), bytes: image.dib.len(), dword: None }];
        let mut entry = json!({ "kind": "image", "file": format!("write-{n}.dib"), "width": image.width, "height": image.height, "source": path.display().to_string() });
        std::fs::write(self.dir.join(format!("write-{n}.dib")), &image.dib).map_err(|e| io("writing the picture", e))?;
        if let Some(png) = &image.png {
            std::fs::write(self.dir.join(format!("write-{n}.png")), png).map_err(|e| io("writing the PNG", e))?;
            entry["png"] = json!(format!("write-{n}.png"));
            listing.push(Listed { name: "PNG".into(), bytes: png.len(), dword: None });
        }
        // CF_HDROP: a 20-byte header and the path as UTF-16 with two NULs.
        listing.push(Listed {
            name: "CF_HDROP".into(),
            bytes: 20 + (path.as_os_str().to_string_lossy().encode_utf16().count() + 2) * 2,
            dword: None,
        });
        listing.extend(marks_listed(&crate::images::image_marks(self.quiet, sensitive)));
        let recorded = self.record(&mut state, entry, &listing, sensitive)?;
        debug_assert_eq!(n, recorded);
        self.write_index(&state)?;
        self.replace(&mut state, None, listing, "app")?;
        Ok(ImageOutcome::Set { width: image.width, height: image.height })
    }

    fn clear_if_text_unchanged(&self, expected: &str, sequence: Option<u32>) -> Result<bool> {
        let mut state = self.lock();
        if state.snapshot.text.as_deref() != Some(expected) || sequence.is_some_and(|s| s != state.sequence) {
            return Ok(false);
        }
        self.record(&mut state, json!({ "kind": "clear" }), &[], false)?;
        self.write_index(&state)?;
        self.replace(&mut state, None, Vec::new(), "app")?;
        Ok(true)
    }

    /// A new value in `input.seq` is the user copying what is in `input.txt` / `input.formats`.
    fn poll_input(&self) -> Result<()> {
        let Some(seq) = read_seq(&self.dir) else { return Ok(()) };
        let mut state = self.lock();
        if state.input_seen.as_deref() == Some(seq.as_str()) {
            return Ok(());
        }
        state.input_seen = Some(seq);
        let text = std::fs::read(self.dir.join(INPUT_TEXT)).ok().map(|b| {
            let s = String::from_utf8_lossy(&b).into_owned();
            s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s)
        });
        let mut listing = Vec::new();
        if let Some(t) = &text {
            listing.push(Listed { name: "CF_UNICODETEXT".into(), bytes: unicode_text_bytes(t), dword: None });
        }
        if let Ok(formats) = std::fs::read_to_string(self.dir.join(INPUT_FORMATS)) {
            for line in formats.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let (name, dword) = match line.split_once('=') {
                    Some((n, v)) => (n.trim(), v.trim().parse::<u32>().ok()),
                    None => (line, None),
                };
                listing.push(Listed { name: name.to_string(), bytes: 4, dword });
            }
        }
        self.replace(&mut state, text, listing, "input")
    }
}

impl Clipboard for FileSinkClipboard {
    fn read(&self) -> Result<ClipSnapshot> {
        Ok(self.inner.lock().snapshot.clone())
    }

    fn write_text(&self, text: &str, mark: Option<Exclusion>) -> Result<()> {
        self.inner.write_text(text, &text_marks(mark, self.inner.quiet), false)
    }

    fn write_text_sensitive(&self, text: &str) -> Result<()> {
        self.inner.write_text(text, &SENSITIVE_MARKS, true)
    }

    fn clear_if_text(&self, expected: &str) -> Result<bool> {
        self.inner.clear_if_text_unchanged(expected, None)
    }

    fn sequence_number(&self) -> Option<u32> {
        Some(self.inner.lock().sequence)
    }

    fn clear_if_text_unchanged(&self, expected: &str, sequence: Option<u32>) -> Result<bool> {
        self.inner.clear_if_text_unchanged(expected, sequence)
    }

    fn write_image_file_with(&self, path: &Path, sensitive: bool) -> Result<ImageOutcome> {
        self.inner.write_image(path, sensitive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{exclusion, exclusion_in_instance};

    fn read_json(dir: &Path, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(name)).unwrap()).unwrap()
    }

    fn names(entry: &Value) -> Vec<String> {
        entry["formats"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for b in bytes {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    /// A valid 1 x 1 RGBA PNG (one stored zlib block), made here so the test needs no image file.
    fn png_1x1() -> Vec<u8> {
        let scanline = [0u8, 0, 0, 255, 255]; // filter 0, then blue with full alpha
        let (mut a, mut b) = (1u32, 0u32);
        for byte in scanline {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        let mut zlib = vec![0x78, 0x01, 0x01, 0x05, 0x00, 0xfa, 0xff];
        zlib.extend_from_slice(&scanline);
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
        png.extend(chunk(b"IDAT", &zlib));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn a_text_write_goes_to_files_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let (sink, mut changes) = FileSinkClipboard::start(dir.path(), false).unwrap();
        sink.write_text("from the phone\r\nsecond line", None).unwrap();
        assert!(changes.try_recv().is_ok(), "one change event per write");
        assert_eq!(std::fs::read_to_string(dir.path().join("write-1.txt")).unwrap(), "from the phone\r\nsecond line");
        assert_eq!(sink.read().unwrap().text.as_deref(), Some("from the phone\r\nsecond line"));
        let index = read_json(dir.path(), INDEX);
        let entry = &index["writes"][0];
        assert_eq!((entry["n"].as_u64(), entry["kind"].as_str(), entry["file"].as_str()), (Some(1), Some("text"), Some("write-1.txt")));
        assert_eq!(names(entry), ["CF_UNICODETEXT"]);
        assert_eq!(entry["flags"]["sensitive"], false);
        let current = read_json(dir.path(), CURRENT);
        assert_eq!(current["text"], "from the phone\r\nsecond line");
        assert_eq!(current["source"], "app");
        // A second write is number 2 and the first file stays.
        sink.write_text("again", None).unwrap();
        assert!(dir.path().join("write-1.txt").exists() && dir.path().join("write-2.txt").exists());
        assert_eq!(read_json(dir.path(), INDEX)["writes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_quiet_sink_records_the_test_content_marks_and_the_instance_reads_its_own_text_as_ordinary() {
        let dir = tempfile::tempdir().unwrap();
        let (sink, _changes) = FileSinkClipboard::start(dir.path(), true).unwrap();
        sink.write_text("hello", None).unwrap();
        let entry = read_json(dir.path(), INDEX)["writes"][0].clone();
        assert_eq!(names(&entry), ["CF_UNICODETEXT", EXCLUDE_FROM_MONITOR, DEV_TEST_CONTENT]);
        assert_eq!(
            (entry["flags"]["quiet"].as_bool(), entry["flags"]["excludedFromMonitor"].as_bool(), entry["flags"]["testContent"].as_bool()),
            (Some(true), Some(true), Some(true))
        );
        let formats = sink.read().unwrap().formats;
        assert!(exclusion(&formats).is_some(), "an installed Kin would skip it");
        assert_eq!(exclusion_in_instance(&formats, true), None, "the instance that wrote it reads it as ordinary text");
    }

    #[test]
    fn a_developer_mark_and_the_sensitive_formats_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (sink, _changes) = FileSinkClipboard::start(dir.path(), false).unwrap();
        sink.write_text("secret", Some(Exclusion::NotInHistory)).unwrap();
        sink.write_text_sensitive("kinrelay://pair?x").unwrap();
        let writes = read_json(dir.path(), INDEX)["writes"].clone();
        assert_eq!(writes[0]["flags"]["excludedFromHistory"], true);
        assert_eq!(writes[0]["flags"]["sensitive"], false);
        assert_eq!(names(&writes[1]), ["CF_UNICODETEXT", EXCLUDE_FROM_MONITOR, CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD]);
        let f = &writes[1]["flags"];
        assert_eq!(
            (
                f["sensitive"].as_bool(),
                f["excludedFromMonitor"].as_bool(),
                f["excludedFromHistory"].as_bool(),
                f["excludedFromCloud"].as_bool()
            ),
            (Some(true), Some(true), Some(true), Some(true))
        );
        assert_eq!(
            sink.read().unwrap().formats,
            [
                ClipFormat::named(EXCLUDE_FROM_MONITOR),
                ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 0),
                ClipFormat::with_dword(CAN_UPLOAD_TO_CLOUD, 0),
            ]
        );
    }

    #[test]
    fn an_image_write_records_the_dib_the_png_and_the_marks() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("shot.png");
        let png = png_1x1();
        std::fs::write(&picture, &png).unwrap();
        let sink_dir = dir.path().join("sink");
        let (sink, _changes) = FileSinkClipboard::start(&sink_dir, true).unwrap();
        let outcome = sink.write_image_file_with(&picture, true).unwrap();
        assert_eq!(outcome, ImageOutcome::Set { width: 1, height: 1 });
        let entry = read_json(&sink_dir, INDEX)["writes"][0].clone();
        assert_eq!(entry["kind"], "image");
        assert_eq!(
            names(&entry),
            ["CF_DIBV5", "PNG", "CF_HDROP", EXCLUDE_FROM_MONITOR, DEV_TEST_CONTENT, CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD]
        );
        assert_eq!(
            (
                entry["flags"]["sensitive"].as_bool(),
                entry["flags"]["excludedFromHistory"].as_bool(),
                entry["flags"]["excludedFromCloud"].as_bool()
            ),
            (Some(true), Some(true), Some(true))
        );
        let dib = std::fs::read(sink_dir.join("write-1.dib")).unwrap();
        assert_eq!(dib.len(), 124 + 4, "a V5 header and one BGRA pixel");
        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 124);
        assert_eq!(std::fs::read(sink_dir.join("write-1.png")).unwrap(), png);
        // An image is no text, and the real clipboard rule for the history marks sees it.
        let now = sink.read().unwrap();
        assert_eq!(now.text, None);
        assert!(exclusion(&now.formats).is_some());
    }

    #[test]
    fn a_file_that_is_not_an_image_is_skipped_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "not a picture").unwrap();
        let sink_dir = dir.path().join("sink");
        let (sink, mut changes) = FileSinkClipboard::start(&sink_dir, false).unwrap();
        assert!(matches!(sink.write_image_file_with(&notes, false).unwrap(), ImageOutcome::Skipped(_)));
        assert!(changes.try_recv().is_err());
        assert!(read_json(&sink_dir, INDEX)["writes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn clearing_works_only_for_the_exact_text_and_an_unchanged_counter() {
        let dir = tempfile::tempdir().unwrap();
        let (sink, _changes) = FileSinkClipboard::start(dir.path(), false).unwrap();
        sink.write_text_sensitive("482913").unwrap();
        let seq = sink.sequence_number();
        assert!(!sink.clear_if_text_unchanged("other", seq).unwrap());
        assert!(!sink.clear_if_text_unchanged("482913", seq.map(|s| s + 1)).unwrap(), "something was copied since");
        assert_eq!(sink.read().unwrap().text.as_deref(), Some("482913"));
        assert!(sink.clear_if_text_unchanged("482913", seq).unwrap());
        assert_eq!(sink.read().unwrap().text, None);
        assert_eq!(read_json(dir.path(), INDEX)["writes"][1]["kind"], "clear");
        assert!(!sink.clear_if_text("482913").unwrap(), "nothing left to clear");
    }

    #[tokio::test]
    async fn a_new_value_of_input_seq_is_one_copy_event_with_the_formats_of_input_formats() {
        let dir = tempfile::tempdir().unwrap();
        let (sink, mut changes) = FileSinkClipboard::start(dir.path(), true).unwrap();
        std::fs::write(dir.path().join(INPUT_TEXT), "copied by the user").unwrap();
        std::fs::write(dir.path().join(INPUT_FORMATS), format!("{EXCLUDE_FROM_MONITOR}\n{CAN_INCLUDE_IN_HISTORY}=0\n")).unwrap();
        // The text and the formats alone are not a copy: the counter is.
        tokio::time::sleep(Duration::from_millis(3 * POLL_MS)).await;
        assert!(changes.try_recv().is_err());
        assert_eq!(sink.read().unwrap().text, None);
        std::fs::write(dir.path().join(INPUT_SEQ), "1").unwrap();
        tokio::time::timeout(Duration::from_secs(3), changes.recv()).await.expect("a change event").unwrap();
        let now = sink.read().unwrap();
        assert_eq!(now.text.as_deref(), Some("copied by the user"));
        assert_eq!(now.formats, [ClipFormat::named(EXCLUDE_FROM_MONITOR), ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 0)]);
        let current = read_json(dir.path(), CURRENT);
        assert_eq!(current["source"], "input");
        // The same value again is no new copy; another value is.
        std::fs::write(dir.path().join(INPUT_SEQ), "1").unwrap();
        tokio::time::sleep(Duration::from_millis(3 * POLL_MS)).await;
        assert!(changes.try_recv().is_err());
        std::fs::remove_file(dir.path().join(INPUT_TEXT)).unwrap();
        std::fs::remove_file(dir.path().join(INPUT_FORMATS)).unwrap();
        std::fs::write(dir.path().join(INPUT_SEQ), "2").unwrap();
        tokio::time::timeout(Duration::from_secs(3), changes.recv()).await.expect("a second change event").unwrap();
        assert_eq!(sink.read().unwrap().text, None, "a copy with no text");
        // The user's copies are not Kin's writes.
        assert!(read_json(dir.path(), INDEX)["writes"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_input_seq_that_is_already_there_at_start_is_not_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(INPUT_TEXT), "left by an earlier run").unwrap();
        std::fs::write(dir.path().join(INPUT_SEQ), "7").unwrap();
        let (sink, mut changes) = FileSinkClipboard::start(dir.path(), false).unwrap();
        tokio::time::sleep(Duration::from_millis(3 * POLL_MS)).await;
        assert!(changes.try_recv().is_err());
        assert_eq!(sink.read().unwrap().text, None);
    }

    #[test]
    fn a_restart_goes_on_numbering_after_the_earlier_writes() {
        let dir = tempfile::tempdir().unwrap();
        {
            let (sink, _changes) = FileSinkClipboard::start(dir.path(), false).unwrap();
            sink.write_text("one", None).unwrap();
        }
        let (sink, _changes) = FileSinkClipboard::start(dir.path(), false).unwrap();
        sink.write_text("two", None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("write-1.txt")).unwrap(), "one", "the earlier file is not overwritten");
        assert_eq!(std::fs::read_to_string(dir.path().join("write-2.txt")).unwrap(), "two");
    }

    fn config_with_sink(sink: Option<&Path>) -> Config {
        let mut config = Config::new(PathBuf::from(r"C:\tmp\kin-test-data"));
        config.dev_clipboard_sink = sink.map(Path::to_path_buf);
        config
    }

    #[tokio::test]
    async fn with_a_sink_the_real_clipboard_is_never_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_with_sink(Some(dir.path()));
        let (clipboard, _changes) = open(&config, |_| panic!("the real clipboard backend must not be created when a sink is set")).unwrap();
        clipboard.write_text("kept off the real clipboard", None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("write-1.txt")).unwrap(), "kept off the real clipboard");
    }

    #[test]
    fn without_a_sink_the_real_backend_is_used_and_told_whether_it_is_quiet() {
        // `quiet` is true for any instance that has a key: this config has no installed data directory, so its directory is "another instance".
        let config = config_with_sink(None);
        let asked = std::cell::Cell::new(None);
        let result = open(&config, |quiet| {
            asked.set(Some(quiet));
            Err("no real clipboard in this test".into())
        });
        assert!(result.is_err());
        assert_eq!(asked.get(), Some(true));
    }
}
