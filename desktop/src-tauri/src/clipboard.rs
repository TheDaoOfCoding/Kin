//! The Windows clipboard, for automatic sync (PC to phone) and for receiving text from the phone (windows.md,
//! "Sending to the phone" and "Receiving from the phone").
//!
//! The decisions are plain functions and small state machines with no Windows in them, so they have unit tests:
//!
//! * [`exclusion`]: content that a password manager (or any app) marked "don't record this" is never synced,
//! * [`LoopGuard`]: the text the phone just put on the clipboard is not sent back,
//! * [`Debouncer`]: a burst of clipboard changes is one sync, 300 ms after the last one,
//! * [`preview`], [`to_windows_newlines`], [`text_hash`]: text helpers.
//!
//! The Win32 side is [`WinClipboard`]: a hidden message-only window that listens with `AddClipboardFormatListener`, and
//! reads and writes `CF_UNICODETEXT`. Everything above it talks to the [`Clipboard`] trait, so the integration tests use
//! [`MemoryClipboard`] and never touch the real clipboard.
//!
//! **Privacy:** the text of the clipboard never goes into a log. Log lines carry a length and [`short_hash`] only.
use std::sync::Mutex;
use std::time::Instant;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::Result;
use crate::encoding::sha256;

/// A burst of clipboard changes is sent once, this long after the last change.
pub const DEBOUNCE_MS: u64 = 300;
/// The echo of a clipboard write of ours is recognised for this long. A write can cause a couple of change events, and the
/// debounce adds 300 ms, so this is generous. After it, the same text copied again counts as a new copy.
pub const LOOP_GUARD_TTL_MS: u64 = 10_000;

// ----- formats that mean "don't sync this" ------------------------------------------------------------------------

/// Presence alone excludes the content (Microsoft's "clipboard history and cloud" opt-out for password managers).
pub const EXCLUDE_FROM_MONITOR: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// A DWORD. 0 means: keep this out of the clipboard history.
pub const CAN_INCLUDE_IN_HISTORY: &str = "CanIncludeInClipboardHistory";
/// A DWORD. 0 means: don't upload this to the cloud clipboard.
pub const CAN_UPLOAD_TO_CLOUD: &str = "CanUploadToCloudClipboard";
/// The older opt-out that KeePass and others still set. Presence alone excludes. (Not in the milestone text; skipping
/// more is the safe direction.)
pub const VIEWER_IGNORE: &str = "Clipboard Viewer Ignore";

/// Written (with `EXCLUDE_FROM_MONITOR`) by a Kin that runs **beside the installed one** (its own data directory, `instance.rs`) on every text
/// and image it puts on the real clipboard. The installed Kin (0.1.0 too: it skips content that carries `EXCLUDE_FROM_MONITOR`) then leaves
/// the content alone and nothing of a test reaches the user's phone, Windows' clipboard history or the cloud clipboard. The instance that
/// wrote it knows its own mark: **only an instance beside the installed one** reads content that carries this format as ordinary content
/// (`exclusion_in_instance`), so its sync, its hotkey and its echo guard behave as they do for any copy. In the installed Kin the format
/// means nothing, and the content stays excluded.
pub const DEV_TEST_CONTENT: &str = "KinRelayDevTestContent";

/// The registered clipboard formats that matter, and nothing else is read from the clipboard besides the text.
const INTERESTING_FORMATS: [&str; 5] = [EXCLUDE_FROM_MONITOR, CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD, VIEWER_IGNORE, DEV_TEST_CONTENT];

/// A clipboard format that is on the clipboard now. `dword` is the first four bytes of its data as a little-endian number,
/// read only for the two formats that carry a value. `None` means it carried no readable data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipFormat {
    pub name: String,
    pub dword: Option<u32>,
}

impl ClipFormat {
    pub fn named(name: &str) -> Self {
        ClipFormat { name: name.to_string(), dword: None }
    }

    pub fn with_dword(name: &str, value: u32) -> Self {
        ClipFormat { name: name.to_string(), dword: Some(value) }
    }
}

/// Why a clipboard change is not synced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exclusion {
    ExcludeFromMonitor,
    NotInHistory,
    NotToCloud,
    ViewerIgnore,
}

impl Exclusion {
    /// The format that said so, for the log.
    pub fn format_name(self) -> &'static str {
        match self {
            Exclusion::ExcludeFromMonitor => EXCLUDE_FROM_MONITOR,
            Exclusion::NotInHistory => CAN_INCLUDE_IN_HISTORY,
            Exclusion::NotToCloud => CAN_UPLOAD_TO_CLOUD,
            Exclusion::ViewerIgnore => VIEWER_IGNORE,
        }
    }
}

fn find<'a>(formats: &'a [ClipFormat], name: &str) -> Option<&'a ClipFormat> {
    // Windows compares registered format names without regard to case.
    formats.iter().find(|f| f.name.eq_ignore_ascii_case(name))
}

/// Whether the content must be left alone. Presence of `ExcludeClipboardContentFromMonitorProcessing` (or the older
/// `Clipboard Viewer Ignore`) does it, and so does a value of 0 in `CanIncludeInClipboardHistory` or
/// `CanUploadToCloudClipboard`. A value format with no readable value counts as 0: when in doubt, don't sync.
pub fn exclusion(formats: &[ClipFormat]) -> Option<Exclusion> {
    if find(formats, EXCLUDE_FROM_MONITOR).is_some() {
        return Some(Exclusion::ExcludeFromMonitor);
    }
    let zero = |name: &str| find(formats, name).is_some_and(|f| f.dword.unwrap_or(0) == 0);
    if zero(CAN_INCLUDE_IN_HISTORY) {
        return Some(Exclusion::NotInHistory);
    }
    if zero(CAN_UPLOAD_TO_CLOUD) {
        return Some(Exclusion::NotToCloud);
    }
    if find(formats, VIEWER_IGNORE).is_some() {
        return Some(Exclusion::ViewerIgnore);
    }
    None
}

/// [`exclusion`] as an instance sees it. An instance beside the installed Kin (`beside_installed`) treats content that carries
/// [`DEV_TEST_CONTENT`] (what it wrote itself) as ordinary content, whatever other opt-out formats come with it.
pub fn exclusion_in_instance(formats: &[ClipFormat], beside_installed: bool) -> Option<Exclusion> {
    if beside_installed && find(formats, DEV_TEST_CONTENT).is_some() {
        return None;
    }
    exclusion(formats)
}

// ----- text helpers -----------------------------------------------------------------------------------------------

pub fn text_hash(text: &str) -> [u8; 32] {
    sha256(text.as_bytes())
}

/// What a log line says to let one text be followed through the log: 8 hex digits of an **HMAC-SHA256 under a per-installation key** that is
/// kept protected and never exported (`privacy.rs`). It is not a plain hash of the text, so a guess can't be checked against it.
pub fn short_hash(text: &str) -> String {
    crate::privacy::tag(text.as_bytes())
}

/// The wire form of a text (protocol.md, "Line ends"): LF only. Every CRLF becomes LF in one left-to-right pass, and a lone CR stays.
/// The loop guard hashes this form, so the clipboard form (CRLF) and what the phone sent (LF) are the same text to it.
pub fn to_wire(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Text that lands on the Windows clipboard should use `\r\n` line ends: Notepad before Windows 11 and older apps show a
/// text with bare `\n` as one long line. A `\n` that already follows a `\r` is left alone.
pub fn to_windows_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut previous = '\0';
    for c in text.chars() {
        if c == '\n' && previous != '\r' {
            out.push('\r');
        }
        out.push(c);
        previous = c;
    }
    out
}

/// What a toast shows of a text: whitespace runs (new lines too) become one space, other control characters go, and it is
/// cut at `max_chars` code points with an ellipsis.
pub fn preview(text: &str, max_chars: usize) -> String {
    let mut cleaned = String::new();
    let mut pending_space = false;
    // A text can be a megabyte long. Only the start of it can ever show.
    for c in text.chars().take(10_000) {
        if c.is_whitespace() {
            pending_space = !cleaned.is_empty();
        } else if !c.is_control() {
            if pending_space {
                cleaned.push(' ');
                pending_space = false;
            }
            cleaned.push(c);
        }
    }
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let mut cut: String = cleaned.chars().take(max_chars).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

// ----- time -------------------------------------------------------------------------------------------------------

/// Milliseconds on a monotonic clock. A trait so tests can move time by hand.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct MonotonicClock(Instant);

impl MonotonicClock {
    pub fn new() -> Self {
        MonotonicClock(Instant::now())
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}

/// A clock that only moves when told to. For tests.
#[derive(Default)]
pub struct ManualClock(std::sync::atomic::AtomicU64);

impl ManualClock {
    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

// ----- the debounce -----------------------------------------------------------------------------------------------

/// Trailing-edge debounce: every event pushes the deadline to `now + quiet`, and the burst is reported once the deadline
/// passes with no further event. It knows nothing about clocks: the caller passes the time in milliseconds.
#[derive(Debug)]
pub struct Debouncer {
    quiet_ms: u64,
    /// `(first event of the burst, deadline)`
    pending: Option<(u64, u64)>,
}

impl Debouncer {
    pub fn new(quiet_ms: u64) -> Self {
        Debouncer { quiet_ms, pending: None }
    }

    pub fn event(&mut self, now_ms: u64) {
        let first = self.pending.map_or(now_ms, |(first, _)| first);
        self.pending = Some((first, now_ms + self.quiet_ms));
    }

    /// When the burst will be reported if nothing else happens. `None` when no event is waiting.
    pub fn deadline(&self) -> Option<u64> {
        self.pending.map(|(_, deadline)| deadline)
    }

    /// `Some(time of the first event of the burst)` once the quiet period is over. That is when the user acted, which is
    /// where "tap to ack" starts.
    pub fn poll(&mut self, now_ms: u64) -> Option<u64> {
        match self.pending {
            Some((first, deadline)) if now_ms >= deadline => {
                self.pending = None;
                Some(first)
            }
            _ => None,
        }
    }
}

// ----- loop prevention --------------------------------------------------------------------------------------------

/// Remembers the hash of the text that was last put on the clipboard on behalf of the phone, so the change event that write
/// causes is not sent straight back. It is checked against what the clipboard holds when the change is processed.
#[derive(Debug, Default)]
pub struct LoopGuard {
    last: Option<([u8; 32], u64)>,
}

impl LoopGuard {
    /// Call before writing `text` to the clipboard.
    pub fn note_set(&mut self, text: &str, now_ms: u64) {
        self.last = Some((text_hash(text), now_ms));
    }

    /// The write failed, so there will be no echo.
    pub fn forget(&mut self) {
        self.last = None;
    }

    /// `true` if the clipboard holds what we put there (so the change is our own echo and must be ignored). Several change
    /// events for one write are all recognised, for [`LOOP_GUARD_TTL_MS`]. Different text, or too much time, clears the
    /// memory: from then on the same text counts as a new copy.
    pub fn is_echo(&mut self, text: &str, now_ms: u64) -> bool {
        let Some((hash, set_at)) = self.last else {
            return false;
        };
        if now_ms.saturating_sub(set_at) > LOOP_GUARD_TTL_MS || hash != text_hash(text) {
            self.last = None;
            return false;
        }
        true
    }
}

// ----- the clipboard itself ---------------------------------------------------------------------------------------

/// What is on the clipboard: its text (if it has any) and which of the opt-out formats are present.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipSnapshot {
    pub text: Option<String>,
    pub formats: Vec<ClipFormat>,
}

/// Reads and writes text. The calls can block for a moment (another app may hold the clipboard open), so async code calls
/// them from `spawn_blocking`.
pub trait Clipboard: Send + Sync {
    fn read(&self) -> Result<ClipSnapshot>;
    /// Replaces the clipboard with `text`. `mark` also adds an opt-out format, which is how a developer hook pretends to be a
    /// password manager. The app itself never passes it.
    fn write_text(&self, text: &str, mark: Option<Exclusion>) -> Result<()>;
    /// Replaces the clipboard with `text` that the sender marked sensitive: along with the text it writes the formats that keep it out
    /// of the clipboard history and the cloud clipboard (`ExcludeClipboardContentFromMonitorProcessing`,
    /// `CanIncludeInClipboardHistory` = 0, `CanUploadToCloudClipboard` = 0).
    fn write_text_sensitive(&self, text: &str) -> Result<()>;
    /// Empties the clipboard, but only if its text is exactly `expected`, checked and done in one go (nothing can be copied in between).
    /// `true` if it was emptied. Used to take the pairing link off the clipboard once it has been used: a clipboard that holds
    /// anything else is the user's and is left alone. The default is for clipboards that can't.
    fn clear_if_text(&self, _expected: &str) -> Result<bool> {
        Err("this clipboard can't be cleared".into())
    }
    /// Puts the image in this file on the clipboard (see `images.rs`). The default is for clipboards that can't.
    fn write_image_file(&self, _path: &std::path::Path) -> Result<ImageOutcome> {
        Ok(ImageOutcome::Skipped("this clipboard can't hold images".to_string()))
    }
}

/// What `write_image_file` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageOutcome {
    Set {
        width: u32,
        height: u32,
    },
    /// Not put on the clipboard, and why (too big, not an image the system can decode).
    Skipped(String),
}

/// A clipboard in memory, with the change events a real one gives. For tests and developer tooling: nothing here touches
/// the real clipboard.
#[derive(Default)]
pub struct MemoryClipboard {
    state: Mutex<ClipSnapshot>,
    listeners: Mutex<Vec<UnboundedSender<()>>>,
    writes: Mutex<Vec<String>>,
    sensitive_writes: Mutex<Vec<String>>,
    images: Mutex<Vec<std::path::PathBuf>>,
}

impl MemoryClipboard {
    /// Change events, one per write, like `WM_CLIPBOARDUPDATE`.
    pub fn changes(&self) -> UnboundedReceiver<()> {
        let (tx, rx) = unbounded_channel();
        self.listeners.lock().unwrap_or_else(|p| p.into_inner()).push(tx);
        rx
    }

    fn notify(&self) {
        for tx in self.listeners.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            let _ = tx.send(());
        }
    }

    /// The user copies something: text, plus whatever opt-out formats the source app adds.
    pub fn user_copy(&self, text: &str, formats: Vec<ClipFormat>) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = ClipSnapshot { text: Some(text.to_string()), formats };
        self.notify();
    }

    /// The user copies something that isn't text (an image, files).
    pub fn user_copy_non_text(&self) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = ClipSnapshot::default();
        self.notify();
    }

    pub fn text(&self) -> Option<String> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).text.clone()
    }

    /// The texts the app itself wrote (not the user), in order. (Sensitive ones are in `app_sensitive_writes`.)
    pub fn app_writes(&self) -> Vec<String> {
        self.writes.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The sensitive texts the app wrote, in order.
    pub fn app_sensitive_writes(&self) -> Vec<String> {
        self.sensitive_writes.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The image files the app put on the clipboard, in order.
    pub fn app_images(&self) -> Vec<std::path::PathBuf> {
        self.images.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Clipboard for MemoryClipboard {
    fn read(&self) -> Result<ClipSnapshot> {
        Ok(self.state.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    fn write_text(&self, text: &str, mark: Option<Exclusion>) -> Result<()> {
        let formats = mark.map(|m| vec![ClipFormat::with_dword(m.format_name(), 0)]).unwrap_or_default();
        self.writes.lock().unwrap_or_else(|p| p.into_inner()).push(text.to_string());
        self.user_copy(text, formats);
        Ok(())
    }

    fn write_text_sensitive(&self, text: &str) -> Result<()> {
        let formats = vec![
            ClipFormat::named(EXCLUDE_FROM_MONITOR),
            ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 0),
            ClipFormat::with_dword(CAN_UPLOAD_TO_CLOUD, 0),
        ];
        self.sensitive_writes.lock().unwrap_or_else(|p| p.into_inner()).push(text.to_string());
        self.user_copy(text, formats);
        Ok(())
    }

    fn clear_if_text(&self, expected: &str) -> Result<bool> {
        {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.text.as_deref() != Some(expected) {
                return Ok(false);
            }
            *state = ClipSnapshot::default();
        }
        self.notify();
        Ok(true)
    }

    fn write_image_file(&self, path: &std::path::Path) -> Result<ImageOutcome> {
        self.images.lock().unwrap_or_else(|p| p.into_inner()).push(path.to_path_buf());
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = ClipSnapshot::default();
        self.notify();
        Ok(ImageOutcome::Set { width: 1, height: 1 })
    }
}

// ----- Win32 ------------------------------------------------------------------------------------------------------

pub use win::WinClipboard;

mod win {
    use std::cell::RefCell;
    use std::time::Duration;

    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, PostMessageW, PostQuitMessage,
        RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_DESTROY, WNDCLASSW,
    };
    use windows::core::{HSTRING, PCWSTR, w};

    use super::{CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD, ClipFormat, ClipSnapshot, Clipboard, Exclusion, INTERESTING_FORMATS};
    use crate::Result;

    const CF_UNICODETEXT: u32 = 13;
    /// Registered formats have ids from here up.
    const FIRST_REGISTERED_FORMAT: u32 = 0xC000;
    const OPEN_ATTEMPTS: usize = 10;
    const OPEN_RETRY: Duration = Duration::from_millis(20);
    /// A clipboard text above this is not read at all (it would go as a `.txt` file over 128 KiB, but 64 MiB of text in memory is the limit).
    const MAX_READ_BYTES: usize = 64 * 1024 * 1024;

    thread_local! {
        /// The window procedure runs on the listener thread, which is where this is set.
        static CHANGES: RefCell<Option<UnboundedSender<()>>> = const { RefCell::new(None) };
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_CLIPBOARDUPDATE => {
                CHANGES.with(|c| {
                    if let Some(tx) = c.borrow().as_ref() {
                        let _ = tx.send(());
                    }
                });
                LRESULT(0)
            }
            WM_CLOSE => {
                // SAFETY: `hwnd` is this window, on its own thread.
                let _ = unsafe { DestroyWindow(hwnd) };
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: as above.
                unsafe {
                    let _ = RemoveClipboardFormatListener(hwnd);
                    PostQuitMessage(0);
                }
                LRESULT(0)
            }
            // SAFETY: the default procedure for every other message.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// The real clipboard. A hidden message-only window on its own thread receives `WM_CLIPBOARDUPDATE`, and owns the
    /// clipboard while the app writes to it.
    pub struct WinClipboard {
        /// The window handle as a number, so the struct is `Send` and `Sync`.
        hwnd: isize,
        /// Every text and image the app writes carries [`EXCLUDE_FROM_MONITOR`] and [`DEV_TEST_CONTENT`] (see there). On in an instance that
        /// runs beside the installed Kin.
        quiet: bool,
    }

    impl WinClipboard {
        /// Creates the window and starts listening. The receiver gets one `()` per change event (a single copy usually causes
        /// two or three: the debounce is what turns them into one).
        pub fn start() -> Result<(WinClipboard, UnboundedReceiver<()>)> {
            let (tx, rx) = unbounded_channel();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<isize, String>>();
            std::thread::Builder::new().name("clipboard-listener".into()).stack_size(256 * 1024).spawn(move || {
                CHANGES.with(|c| *c.borrow_mut() = Some(tx));
                // SAFETY: plain Win32 calls on this thread. The window lives until `WM_CLOSE` and the loop ends with it.
                unsafe {
                    let hinstance = match GetModuleHandleW(PCWSTR::null()) {
                        Ok(h) => HINSTANCE(h.0),
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("GetModuleHandle: {e}")));
                            return;
                        }
                    };
                    let class = w!("KinClipboardListener");
                    // A second registration (another listener in the same process, as in tests) fails with "already exists",
                    // which is fine: the class is there.
                    RegisterClassW(&WNDCLASSW {
                        lpfnWndProc: Some(window_proc),
                        hInstance: hinstance,
                        lpszClassName: class,
                        ..Default::default()
                    });
                    let hwnd = match CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        w!("Kin clipboard listener"),
                        WINDOW_STYLE(0),
                        0,
                        0,
                        0,
                        0,
                        Some(HWND_MESSAGE),
                        None,
                        Some(hinstance),
                        None,
                    ) {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("CreateWindowEx: {e}")));
                            return;
                        }
                    };
                    if let Err(e) = AddClipboardFormatListener(hwnd) {
                        let _ = ready_tx.send(Err(format!("AddClipboardFormatListener: {e}")));
                        let _ = DestroyWindow(hwnd);
                        return;
                    }
                    let _ = ready_tx.send(Ok(hwnd.0 as isize));
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })?;
            let hwnd = ready_rx.recv().map_err(|_| "the clipboard listener thread ended at once")??;
            Ok((WinClipboard { hwnd, quiet: false }, rx))
        }

        /// Writes made through this clipboard are invisible to the installed Kin and to Windows' history (see [`DEV_TEST_CONTENT`]).
        pub fn quiet(mut self, on: bool) -> WinClipboard {
            self.quiet = on;
            self
        }

        fn hwnd(&self) -> HWND {
            HWND(self.hwnd as *mut _)
        }

        /// The text, and each of `marks` as a registered format that holds a DWORD.
        fn write_text_marked(&self, text: &str, marks: &[(&str, u32)]) -> Result<()> {
            let mut bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            bytes.extend_from_slice(&[0, 0]);
            let _open = Open::new(self.hwnd())?;
            // SAFETY: the clipboard is open, and this window owns it from here on.
            unsafe { EmptyClipboard() }?;
            put(CF_UNICODETEXT, &bytes)?;
            for (name, value) in marks {
                put(register(name), &value.to_le_bytes())?;
            }
            Ok(())
        }
    }

    impl Drop for WinClipboard {
        fn drop(&mut self) {
            // SAFETY: posts a message to our own window. If the window is gone already this just fails.
            unsafe {
                let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// The clipboard, open. Closed again when dropped.
    struct Open;

    impl Open {
        /// Another app can have the clipboard open for a moment, so this tries a few times.
        fn new(owner: HWND) -> Result<Open> {
            let mut last = None;
            for attempt in 0..OPEN_ATTEMPTS {
                // SAFETY: `owner` is a live window of this process.
                match unsafe { OpenClipboard(Some(owner)) } {
                    Ok(()) => return Ok(Open),
                    Err(e) => last = Some(e),
                }
                if attempt + 1 < OPEN_ATTEMPTS {
                    std::thread::sleep(OPEN_RETRY);
                }
            }
            Err(format!("the clipboard is in use by another app ({})", last.map(|e| e.to_string()).unwrap_or_default()).into())
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: closes what `new` opened.
            let _ = unsafe { CloseClipboard() };
        }
    }

    fn format_name(format: u32) -> Option<String> {
        let mut buf = [0u16; 128];
        // SAFETY: `buf` is a valid buffer of the length passed.
        let n = unsafe { GetClipboardFormatNameW(format, &mut buf) };
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }

    /// The bytes of a global memory block that the clipboard hands out (the clipboard has to be open).
    fn global_bytes(format: u32, limit: usize) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open, so the handle stays valid. The block is locked while it is read.
        unsafe {
            let handle = GetClipboardData(format).ok()?;
            let global = HGLOBAL(handle.0);
            let size = GlobalSize(global);
            if size == 0 || size > limit {
                return None;
            }
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            let _ = GlobalUnlock(global);
            Some(bytes)
        }
    }

    fn text_of(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).collect();
        // A lone surrogate becomes U+FFFD, as protocol.md says a sender does.
        String::from_utf16_lossy(&units)
    }

    fn register(name: &str) -> u32 {
        // SAFETY: the string is NUL-terminated.
        unsafe { RegisterClipboardFormatW(&HSTRING::from(name)) }
    }

    fn put(format: u32, bytes: &[u8]) -> Result<()> {
        // SAFETY: the block is allocated, filled while locked, and handed to the clipboard, which owns it on success.
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
            let ptr = GlobalLock(global) as *mut u8;
            if ptr.is_null() {
                let _ = GlobalFree(Some(global));
                return Err("GlobalLock failed".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = GlobalUnlock(global);
            if let Err(e) = SetClipboardData(format, Some(HANDLE(global.0))) {
                let _ = GlobalFree(Some(global));
                return Err(format!("SetClipboardData failed: {e}").into());
            }
        }
        Ok(())
    }

    impl Clipboard for WinClipboard {
        fn read(&self) -> Result<ClipSnapshot> {
            let _open = Open::new(self.hwnd())?;
            // SAFETY: the clipboard is open for the rest of this function.
            let has_text = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_ok();
            let text = if has_text { global_bytes(CF_UNICODETEXT, MAX_READ_BYTES).map(|b| text_of(&b)) } else { None };
            let mut formats = Vec::new();
            let mut format = 0u32;
            loop {
                // SAFETY: the clipboard is open.
                format = unsafe { EnumClipboardFormats(format) };
                if format == 0 {
                    break;
                }
                if format < FIRST_REGISTERED_FORMAT {
                    continue;
                }
                let Some(name) = format_name(format) else { continue };
                let Some(known) = INTERESTING_FORMATS.iter().find(|k| k.eq_ignore_ascii_case(&name)) else { continue };
                let carries_value = [CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD].iter().any(|k| k.eq_ignore_ascii_case(known));
                let dword = if carries_value {
                    global_bytes(format, 1024).and_then(|b| b.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
                } else {
                    None
                };
                formats.push(ClipFormat { name: (*known).to_string(), dword });
            }
            Ok(ClipSnapshot { text, formats })
        }

        fn write_text(&self, text: &str, mark: Option<Exclusion>) -> Result<()> {
            // A mark that a developer hook asks for is written as it is; with none, a quiet clipboard marks the text as test content.
            let marks: Vec<(&str, u32)> = match mark {
                Some(m) => vec![(m.format_name(), if matches!(m, Exclusion::NotInHistory | Exclusion::NotToCloud) { 0 } else { 1 })],
                None if self.quiet => vec![(super::EXCLUDE_FROM_MONITOR, 1), (super::DEV_TEST_CONTENT, 1)],
                None => Vec::new(),
            };
            self.write_text_marked(text, &marks)
        }

        fn write_text_sensitive(&self, text: &str) -> Result<()> {
            self.write_text_marked(text, &[(super::EXCLUDE_FROM_MONITOR, 1), (CAN_INCLUDE_IN_HISTORY, 0), (CAN_UPLOAD_TO_CLOUD, 0)])
        }

        fn clear_if_text(&self, expected: &str) -> Result<bool> {
            // One open of the clipboard for the look and the clearing, so a copy by the user can't land in between.
            let _open = Open::new(self.hwnd())?;
            // SAFETY: the clipboard is open for the rest of this function.
            let has_text = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_ok();
            let text = if has_text { global_bytes(CF_UNICODETEXT, MAX_READ_BYTES).map(|b| text_of(&b)) } else { None };
            if text.as_deref() != Some(expected) {
                return Ok(false);
            }
            // SAFETY: the clipboard is open, and this window owns it from here on.
            unsafe { EmptyClipboard() }?;
            Ok(true)
        }

        fn write_image_file(&self, path: &std::path::Path) -> Result<super::ImageOutcome> {
            crate::images::write_clipboard_image(self.hwnd(), path, self.quiet)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> ClipFormat {
        ClipFormat::named(name)
    }

    // ----- password managers and other opt-outs ----------------------------------------------------------------------

    #[test]
    fn plain_text_is_not_excluded() {
        assert_eq!(exclusion(&[]), None);
        assert_eq!(exclusion(&[f("SomeOtherFormat"), f("HTML Format")]), None);
    }

    #[test]
    fn the_exclude_format_excludes_by_being_there() {
        assert_eq!(exclusion(&[f(EXCLUDE_FROM_MONITOR)]), Some(Exclusion::ExcludeFromMonitor));
        // Whatever its data is, even a 1.
        assert_eq!(exclusion(&[ClipFormat::with_dword(EXCLUDE_FROM_MONITOR, 1)]), Some(Exclusion::ExcludeFromMonitor));
        assert_eq!(exclusion(&[f("HTML Format"), f(EXCLUDE_FROM_MONITOR)]), Some(Exclusion::ExcludeFromMonitor));
    }

    #[test]
    fn history_and_cloud_formats_exclude_only_with_a_value_of_zero() {
        assert_eq!(exclusion(&[ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 0)]), Some(Exclusion::NotInHistory));
        assert_eq!(exclusion(&[ClipFormat::with_dword(CAN_UPLOAD_TO_CLOUD, 0)]), Some(Exclusion::NotToCloud));
        assert_eq!(exclusion(&[ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 1)]), None, "1 means: allowed");
        assert_eq!(exclusion(&[ClipFormat::with_dword(CAN_UPLOAD_TO_CLOUD, 1)]), None);
        assert_eq!(exclusion(&[ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 2)]), None, "any non-zero value allows it");
        assert_eq!(
            exclusion(&[ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 1), ClipFormat::with_dword(CAN_UPLOAD_TO_CLOUD, 0)]),
            Some(Exclusion::NotToCloud),
            "each format is judged on its own"
        );
    }

    #[test]
    fn a_value_format_without_a_readable_value_counts_as_zero() {
        assert_eq!(exclusion(&[f(CAN_INCLUDE_IN_HISTORY)]), Some(Exclusion::NotInHistory));
        assert_eq!(exclusion(&[f(CAN_UPLOAD_TO_CLOUD)]), Some(Exclusion::NotToCloud));
    }

    #[test]
    fn format_names_match_without_regard_to_case() {
        assert_eq!(exclusion(&[f("excludeclipboardcontentfrommonitorprocessing")]), Some(Exclusion::ExcludeFromMonitor));
        // The mark of a test copy beside the installed Kin changes nothing for the plain rule: the installed Kin must skip it.
        assert_eq!(exclusion(&[f(EXCLUDE_FROM_MONITOR), f(DEV_TEST_CONTENT)]), Some(Exclusion::ExcludeFromMonitor));
        assert_eq!(exclusion(&[f(DEV_TEST_CONTENT)]), None, "the test mark alone excludes nothing");
        assert_eq!(exclusion(&[ClipFormat::with_dword("CANINCLUDEINCLIPBOARDHISTORY", 0)]), Some(Exclusion::NotInHistory));
    }

    #[test]
    fn the_older_viewer_ignore_format_excludes_too() {
        assert_eq!(exclusion(&[f(VIEWER_IGNORE)]), Some(Exclusion::ViewerIgnore));
    }

    #[test]
    fn the_exclusion_names_the_format_for_the_log() {
        assert_eq!(Exclusion::ExcludeFromMonitor.format_name(), "ExcludeClipboardContentFromMonitorProcessing");
        assert_eq!(Exclusion::NotInHistory.format_name(), "CanIncludeInClipboardHistory");
        assert_eq!(Exclusion::NotToCloud.format_name(), "CanUploadToCloudClipboard");
    }

    // ----- loop prevention -------------------------------------------------------------------------------------------

    #[test]
    fn the_text_the_phone_set_is_an_echo_and_other_text_is_not() {
        let mut g = LoopGuard::default();
        assert!(!g.is_echo("hello", 0), "nothing was set yet");
        g.note_set("from the phone", 1_000);
        assert!(g.is_echo("from the phone", 1_400));
        assert!(!g.is_echo("something else", 1_500), "a different text is a real copy");
    }

    #[test]
    fn one_write_can_cause_several_change_events() {
        let mut g = LoopGuard::default();
        g.note_set("from the phone", 0);
        for t in [100, 350, 2_000] {
            assert!(g.is_echo("from the phone", t), "event at {t} ms");
        }
    }

    #[test]
    fn a_real_copy_clears_the_memory_so_the_same_text_copied_later_is_sent() {
        let mut g = LoopGuard::default();
        g.note_set("A", 0);
        assert!(g.is_echo("A", 100));
        assert!(!g.is_echo("B", 200), "the user copied B");
        assert!(!g.is_echo("A", 300), "and then A again: that is a new copy now");
    }

    #[test]
    fn the_memory_expires() {
        let mut g = LoopGuard::default();
        g.note_set("A", 0);
        assert!(g.is_echo("A", LOOP_GUARD_TTL_MS));
        assert!(!g.is_echo("A", LOOP_GUARD_TTL_MS + 1), "too late to be the echo of that write");
        assert!(!g.is_echo("A", 5), "and it stays forgotten");
    }

    #[test]
    fn a_failed_write_leaves_no_memory() {
        let mut g = LoopGuard::default();
        g.note_set("A", 0);
        g.forget();
        assert!(!g.is_echo("A", 10));
    }

    #[test]
    fn a_new_write_replaces_the_memory() {
        let mut g = LoopGuard::default();
        g.note_set("A", 0);
        g.note_set("B", 10);
        assert!(!g.is_echo("A", 20));
        g.note_set("B", 30);
        assert!(g.is_echo("B", 40));
    }

    // ----- the debounce, with a clock that moves by hand --------------------------------------------------------------

    #[test]
    fn a_single_change_is_reported_300_ms_later() {
        let clock = ManualClock::default();
        let mut d = Debouncer::new(DEBOUNCE_MS);
        assert_eq!(d.poll(clock.now_ms()), None);
        assert_eq!(d.deadline(), None);
        d.event(clock.now_ms());
        assert_eq!(d.deadline(), Some(300));
        clock.advance(299);
        assert_eq!(d.poll(clock.now_ms()), None, "299 ms: still waiting");
        clock.advance(1);
        assert_eq!(d.poll(clock.now_ms()), Some(0), "300 ms: reported, with the time of the first event");
        assert_eq!(d.poll(clock.now_ms()), None, "and only once");
        assert_eq!(d.deadline(), None);
    }

    #[test]
    fn a_burst_is_one_report_after_the_last_change() {
        let clock = ManualClock::default();
        let mut d = Debouncer::new(DEBOUNCE_MS);
        d.event(clock.now_ms()); // 0
        clock.advance(100);
        d.event(clock.now_ms()); // 100
        clock.advance(150);
        d.event(clock.now_ms()); // 250
        clock.advance(299);
        assert_eq!(d.poll(clock.now_ms()), None, "549 ms: 299 ms after the last change");
        clock.advance(1);
        assert_eq!(d.poll(clock.now_ms()), Some(0), "550 ms: one report for the whole burst, dated at its first change");
        assert_eq!(d.poll(clock.now_ms() + 10_000), None);
    }

    #[test]
    fn a_change_after_a_report_starts_a_new_burst() {
        let mut d = Debouncer::new(DEBOUNCE_MS);
        d.event(0);
        assert_eq!(d.poll(300), Some(0));
        d.event(1_000);
        assert_eq!(d.poll(1_299), None);
        assert_eq!(d.poll(1_300), Some(1_000));
    }

    #[test]
    fn changes_that_keep_coming_keep_it_waiting() {
        let mut d = Debouncer::new(DEBOUNCE_MS);
        for t in (0..3_000).step_by(100) {
            d.event(t);
            assert_eq!(d.poll(t), None, "at {t} ms");
        }
        assert_eq!(d.poll(2_900 + 300), Some(0));
    }

    // ----- text helpers ----------------------------------------------------------------------------------------------

    #[test]
    fn the_hash_follows_the_text_and_gives_nothing_away() {
        assert_eq!(text_hash("a"), text_hash("a"));
        assert_ne!(text_hash("a"), text_hash("b"));
        assert_eq!(short_hash("hello").len(), 8);
        assert_eq!(short_hash("hello"), short_hash("hello"));
        assert_ne!(short_hash("hello"), "2cf24dba", "the tag is keyed: it is not the first digits of the plain SHA-256 of the text");
    }

    #[test]
    fn new_lines_become_windows_new_lines_once() {
        assert_eq!(to_windows_newlines("a\nb"), "a\r\nb");
        assert_eq!(to_windows_newlines("a\r\nb"), "a\r\nb");
        assert_eq!(to_windows_newlines("a\n\nb\n"), "a\r\n\r\nb\r\n");
        assert_eq!(to_windows_newlines("no new lines"), "no new lines");
        assert_eq!(to_windows_newlines("日本\n😀"), "日本\r\n😀");
        assert_eq!(to_windows_newlines(""), "");
    }

    #[test]
    fn the_preview_is_one_short_line() {
        assert_eq!(preview("hello", 100), "hello");
        assert_eq!(preview("  line one\r\n\r\n  line two\tend ", 100), "line one line two end");
        assert_eq!(preview("a\u{0}b\u{1b}c", 100), "abc");
        assert_eq!(preview("", 10), "");
        assert_eq!(preview("abcdefghij", 10), "abcdefghij", "exactly the limit: no ellipsis");
        assert_eq!(preview("abcdefghijk", 10), "abcdefghij…");
        assert_eq!(preview("日本語のテキスト", 3), "日本語…", "counts code points, not bytes");
        assert_eq!(preview("😀😀😀😀", 2), "😀😀…");
    }

    #[test]
    fn the_preview_never_ends_in_a_dangling_space() {
        assert_eq!(preview("abcd efgh", 5), "abcd…", "the cut lands on the space");
        assert_eq!(preview("abc  def", 4), "abc…");
    }

    // ----- the in-memory clipboard the other tests use ----------------------------------------------------------------

    #[tokio::test]
    async fn the_memory_clipboard_sends_a_change_event_per_write() {
        let c = MemoryClipboard::default();
        let mut changes = c.changes();
        c.user_copy("hi", vec![f(EXCLUDE_FROM_MONITOR)]);
        assert!(changes.try_recv().is_ok());
        assert_eq!(c.read().unwrap(), ClipSnapshot { text: Some("hi".into()), formats: vec![f(EXCLUDE_FROM_MONITOR)] });
        c.write_text("from the app", Some(Exclusion::NotInHistory)).unwrap();
        assert!(changes.try_recv().is_ok());
        assert_eq!(c.app_writes(), ["from the app"]);
        assert_eq!(exclusion(&c.read().unwrap().formats), Some(Exclusion::NotInHistory));
        c.user_copy_non_text();
        assert_eq!(c.read().unwrap().text, None);
        assert_eq!(c.app_writes().len(), 1, "a user copy is not an app write");
    }

    /// The real clipboard. It's the user's, so this test is `#[ignore]`d and has to be asked for:
    /// `cargo test real_clipboard -- --ignored --test-threads=1`. It saves the text that is on the clipboard (and does nothing
    /// if there is more than plain text there, such as an image, files or rich text, which it couldn't put back), uses it,
    /// and puts the text back. An empty clipboard stays empty. **Every write is quiet** (`DEV_TEST_CONTENT` and the opt-out format of password
    /// managers): a Kin that is installed on this PC, with its automatic sync on, skips it and sends none of the test text to a phone.
    #[test]
    #[ignore = "touches the real clipboard"]
    fn real_clipboard_round_trip_and_password_manager_formats() {
        let (clipboard, mut changes) = WinClipboard::start().unwrap();
        let clipboard = clipboard.quiet(true);
        let other = other_formats();
        if !other.is_empty() {
            eprintln!("skipped: the clipboard holds more than plain text ({other:?}), which this test couldn't restore exactly");
            return;
        }
        let restore = clipboard.read().unwrap().text.unwrap_or_default();

        // Plain text, with a new line and characters outside the BMP.
        let text = "kin clipboard test\r\nsecond line 日本 😀";
        clipboard.write_text(text, None).unwrap();
        let got = clipboard.read().unwrap();
        assert_eq!(got.text.as_deref(), Some(text));
        assert_eq!(exclusion(&got.formats), Some(Exclusion::ExcludeFromMonitor), "an installed Kin skips it: {:?}", got.formats);
        assert_eq!(exclusion_in_instance(&got.formats, true), None, "the instance that wrote it reads it as ordinary text");
        assert!(changes.blocking_recv().is_some(), "a change event for our own write");

        // Each opt-out format is seen when it is there.
        for mark in [Exclusion::ExcludeFromMonitor, Exclusion::NotInHistory, Exclusion::NotToCloud, Exclusion::ViewerIgnore] {
            clipboard.write_text("secret", Some(mark)).unwrap();
            let got = clipboard.read().unwrap();
            assert_eq!(got.text.as_deref(), Some("secret"));
            assert_eq!(exclusion(&got.formats), Some(mark), "{mark:?}: {:?}", got.formats);
        }

        // Back to what the user had (quiet too: an installed Kin must not send the user's own text to the phone a second time).
        clipboard.write_text(&restore, None).unwrap();
    }

    /// `clear_if_text` on the real clipboard (the pairing link is taken off it once the phone has paired). It replaces whatever is on the
    /// clipboard, so only a run that saved every format first (`clipboard-backup.ps1`) and restores them afterwards may use it:
    /// `Save-Clipboard; cargo test real_clipboard_clear -- --ignored; Restore-Clipboard`.
    #[test]
    #[ignore = "replaces the real clipboard: run it between Save-Clipboard and Restore-Clipboard"]
    fn real_clipboard_clear_if_text_clears_only_the_exact_text() {
        let (clipboard, _changes) = WinClipboard::start().unwrap();
        let link = "kinrelay://pair?v=1&a=10.0.2.2:1&id=x&tk=TEST&n=PC";
        clipboard.write_text_sensitive(link).unwrap();
        assert_eq!(clipboard.read().unwrap().text.as_deref(), Some(link));
        // Anything but exactly the link: nothing happens.
        assert!(!clipboard.clear_if_text("kinrelay://pair?v=1&a=10.0.2.2:1&id=x&tk=OTHER&n=PC").unwrap());
        assert!(!clipboard.clear_if_text(&format!("{link} ")).unwrap());
        assert_eq!(clipboard.read().unwrap().text.as_deref(), Some(link), "left alone");
        // Exactly the link: emptied, with its opt-out formats.
        assert!(clipboard.clear_if_text(link).unwrap());
        let after = clipboard.read().unwrap();
        assert_eq!(after.text, None);
        assert!(after.formats.is_empty(), "{:?}", after.formats);
        // An empty clipboard: nothing to do.
        assert!(!clipboard.clear_if_text(link).unwrap());
        // Other text: not touched. (Marked as a password manager would, so that a Kin that is running on this PC with its automatic sync on
        // does not send the test text to a phone.)
        clipboard.write_text("Dinner at 8?", Some(Exclusion::ExcludeFromMonitor)).unwrap();
        assert!(!clipboard.clear_if_text(link).unwrap());
        assert_eq!(clipboard.read().unwrap().text.as_deref(), Some("Dinner at 8?"));
    }

    /// Puts the image in the file named by `KIN_TEST_IMAGE` on the real clipboard, the way the app does for a received image, and
    /// prints what came of it. It **does not restore** the clipboard: `desktop/check-1-3.ps1` saves every format first
    /// (`clipboard-backup.ps1`), reads the result back with PowerShell, and restores them.
    #[test]
    #[ignore = "replaces the real clipboard: only the script check-1-3.ps1 runs it"]
    fn real_clipboard_image_for_the_script() {
        let path = std::env::var("KIN_TEST_IMAGE").expect("KIN_TEST_IMAGE is the path of the image to put on the clipboard");
        let (clipboard, _changes) = WinClipboard::start().unwrap();
        let clipboard = clipboard.quiet(true);
        let outcome = clipboard.write_image_file(std::path::Path::new(&path)).unwrap();
        println!("IMAGE_OUTCOME {outcome:?}");
    }

    /// The formats on the clipboard that are not just plain text (or nothing): an image, files, rich text. Plain text shows up as
    /// `CF_TEXT`, `CF_OEMTEXT`, `CF_UNICODETEXT` and `CF_LOCALE` (Windows makes the first, second and last from the third), and
    /// an app that uses OLE (Windows PowerShell's `Set-Clipboard`) adds its bookkeeping formats.
    fn other_formats() -> Vec<String> {
        use windows::Win32::System::DataExchange::{CloseClipboard, EnumClipboardFormats, GetClipboardFormatNameW, OpenClipboard};
        const HARMLESS_IDS: [u32; 4] = [1, 7, 13, 16];
        const HARMLESS_NAMES: [&str; 3] = ["DataObject", "Ole Private Data", "OleClipboardPersistOnFlush"];
        let mut other = Vec::new();
        // SAFETY: opens the clipboard, lists its formats and closes it.
        unsafe {
            if OpenClipboard(None).is_err() {
                return vec!["(the clipboard is busy)".to_string()];
            }
            let mut format = 0;
            loop {
                format = EnumClipboardFormats(format);
                if format == 0 {
                    break;
                }
                if HARMLESS_IDS.contains(&format) {
                    continue;
                }
                let mut buf = [0u16; 128];
                let n = GetClipboardFormatNameW(format, &mut buf);
                let name = if n > 0 { String::from_utf16_lossy(&buf[..n as usize]) } else { format!("format {format}") };
                if !HARMLESS_NAMES.iter().any(|h| h.eq_ignore_ascii_case(&name)) {
                    other.push(name);
                }
            }
            let _ = CloseClipboard();
        }
        other
    }
}
