//! `Kin.exe --notify "text"`: "tell me when it's done" for scripts and renders (milestone 2B.5, design 5.3; protocol.md "Do Not Disturb, Focus and alerts").
//!
//! **A pipe client with a result and an exit code, not a hand-off.** `main` calls [`try_run`] **before the app starts** (no Tauri, no single-instance plugin): it
//! connects to the pipe the way `kin-native-host.exe` does ([`HostKind::for_config`]: the installed Kin's pipe, or a dev instance's with the same `--data-dir` and
//! `KIN_DEV_PIPE_SUFFIX`), checks that **the pipe server is `Kin.exe` in the folder of this program and run by this user** (so it never talks to another program's
//! pipe), sends one framed `send` request of `kind: "alert"` and prints one line.
//!
//! **Exit codes:** 0 sent or queued, 2 usage, 3 Kin is not running and could not be started (or `--no-start`), 4 not paired, 5 too long or rate limited (the CLI
//! **never cuts silently**: over 300 characters it fails with the count), 6 the phone doesn't support alerts or any other refusal.
//!
//! **Output.** `Kin.exe` is a GUI-subsystem program (a release build): this mode prints to the console that started it (`AttachConsole`), or to a redirected stdout,
//! and prints nothing if there is none (Task Scheduler). **Known limit of a GUI program:** `cmd.exe` and PowerShell do not wait
//! for a GUI program, so a script that needs the exit code waits for it: `start /wait "" Kin.exe --notify "Done"` in cmd, `Kin.exe --notify "Done" | Out-Null` or
//! `Start-Process -Wait` in PowerShell.
//!
//! **Starting Kin.** "The render finished while Kin wasn't running" is the case this is for: with no pipe, the installed Kin (and only that: a command line with a
//! `--data-dir` or developer setting, `KIN_DEV_PIPE_SUFFIX` set, or a debug build names a developer instance, which is never started from here: `Kin.exe` would open the user's own data) is started with no window and the pipe is waited for up to 10 s. `--no-start` for scripts
//! that must not.
//!
//! **Security.** Any process of the same user can do this, which equals what `--send-clipboard` and the pipe already allow (D19, docs/extension.md "Security notes"). The
//! phone shows the text under the PC's name in its own channel and never as another app. **Nothing here logs a text** (it prints one only on the user's own console).
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::alert::ttl_word;
use crate::alert::wire::{MAX_TEXT_CHARS, MAX_TITLE_CHARS, TTL_DEFAULT_SEC, TTL_MAX_SEC, TTL_MIN_SEC};
use crate::encoding::{unix_ms, uuid_v7};
use crate::nativehost::HostKind;
use crate::nativehost::framing::{self, FrameError, MAX_RESPONSE};

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_RUNNING: i32 = 3;
pub const EXIT_NOT_PAIRED: i32 = 4;
pub const EXIT_TOO_LONG: i32 = 5;
pub const EXIT_REFUSED: i32 = 6;

/// How long a start of Kin is waited for.
const START_WAIT: Duration = Duration::from_secs(10);
/// How much of stdin is read (the form `-`): far more than an alert can hold (300 characters), so more than this is "too long" anyway.
const STDIN_LIMIT: usize = 64 * 1024;
/// How long a busy pipe is waited for.
const BUSY_WAIT: Duration = Duration::from_secs(5);
/// How long Kin may take to answer a `send` (it waits for the phone's ack, 5 s at most, and an offline phone answers at once).
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

// ===== the arguments (pure) ===============================================================================================

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyArgs {
    pub text: String,
    pub title: Option<String>,
    pub ttl_sec: u64,
    /// The name or short id the user typed for `--device`.
    pub device: Option<String>,
    pub no_start: bool,
}

/// A usage error, with the exit code it ends with and a word for the vectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub exit: i32,
    pub reason: &'static str,
    /// For a text or title that is too long: how many characters it has.
    pub count: Option<usize>,
}

impl Usage {
    fn new(reason: &'static str) -> Usage {
        Usage { exit: EXIT_USAGE, reason, count: None }
    }

    fn too_long(reason: &'static str, count: usize) -> Usage {
        Usage { exit: EXIT_TOO_LONG, reason, count: Some(count) }
    }

    /// The line the user reads.
    pub fn message(&self) -> String {
        match (self.reason, self.count) {
            ("text_too_long", Some(n)) => format!("The text has {n} characters; an alert is at most {MAX_TEXT_CHARS}. Nothing was sent."),
            ("title_too_long", Some(n)) => format!("The title has {n} characters; it is at most {MAX_TITLE_CHARS}. Nothing was sent."),
            ("missing_text", _) => "There is no text to send. Usage: Kin.exe --notify [--title <t>] [--ttl <n>s|m|h|d] [--device <name>] [--no-start] [--] <text>".into(),
            ("unknown_flag", _) => "Unknown option. Put a text that starts with - after --.".into(),
            ("missing_value", _) => "An option is missing its value.".into(),
            ("bad_ttl", _) => format!("--ttl needs a whole number and a unit (s, m, h or d), from {TTL_MIN_SEC} seconds to 7 days, like 90m or 2h."),
            ("stdin_not_utf8", _) => "The text on stdin is not UTF-8.".into(),
            ("ambiguous_device", _) => "More than one phone matches --device. Use more of its name or id.".into(),
            _ => "Bad arguments.".into(),
        }
    }
}

/// The arguments after `--notify`. `stdin` is called once, only for the form `-`. Positionals are joined with one space; a word that starts with `-` (and is not just `-`) before
/// `--` is an unknown flag; `--title X` and `--title=X` are the same; the last of a repeated flag wins; `--ttl` needs a whole number and a unit; the text is trimmed and
/// **never cut**: over 300 characters (code points) it is an error with the count (exit 5), and so is a title over 60.
pub fn parse_args(args: &[String], stdin: impl FnOnce() -> Vec<u8>) -> Result<NotifyArgs, Usage> {
    let mut title = String::new();
    let mut ttl_sec = TTL_DEFAULT_SEC;
    let mut device: Option<String> = None;
    let mut no_start = false;
    let mut positionals: Vec<&str> = Vec::new();
    let mut rest = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if rest || a == "-" || !a.starts_with('-') {
            positionals.push(a);
            i += 1;
            continue;
        }
        if a == "--" {
            rest = true;
            i += 1;
            continue;
        }
        let eq = if a.starts_with("--") { a.find('=') } else { None };
        let name = eq.map_or(a, |at| &a[..at]);
        if name == "--no-start" && eq.is_none() {
            no_start = true;
            i += 1;
            continue;
        }
        if !matches!(name, "--title" | "--ttl" | "--device") {
            return Err(Usage::new("unknown_flag"));
        }
        let value: String = match eq {
            Some(at) => {
                i += 1;
                a[at + 1..].to_string()
            }
            None => {
                let Some(v) = args.get(i + 1) else {
                    return Err(Usage::new("missing_value"));
                };
                i += 2;
                v.clone()
            }
        };
        match name {
            "--title" => title = value.trim().to_string(),
            "--device" => device = Some(value),
            _ => ttl_sec = parse_ttl(&value).ok_or_else(|| Usage::new("bad_ttl"))?,
        }
    }
    let text = if positionals == ["-"] {
        let bytes = stdin();
        let decoded = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            // The read stops at the limit, which can cut a character in two: that is a text that is too long (counted below), not bad UTF-8.
            Err(e) if e.error_len().is_none() && bytes.len() >= STDIN_LIMIT => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
            Err(_) => return Err(Usage::new("stdin_not_utf8")),
        };
        decoded.strip_prefix('\u{feff}').unwrap_or(decoded).trim().to_string()
    } else {
        positionals.join(" ").trim().to_string()
    };
    if text.is_empty() {
        return Err(Usage::new("missing_text"));
    }
    let chars = text.chars().count();
    if chars > MAX_TEXT_CHARS {
        return Err(Usage::too_long("text_too_long", chars));
    }
    let title_chars = title.chars().count();
    if title_chars > MAX_TITLE_CHARS {
        return Err(Usage::too_long("title_too_long", title_chars));
    }
    Ok(NotifyArgs { text, title: (!title.is_empty()).then_some(title), ttl_sec, device, no_start })
}

/// `90s`, `5m`, `2h`, `7d`: digits and one lower-case unit, 60 s to 7 days.
pub fn parse_ttl(value: &str) -> Option<u64> {
    let unit = value.chars().last()?;
    let digits = &value[..value.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let seconds = n.checked_mul(match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => return None,
    })?;
    (TTL_MIN_SEC..=TTL_MAX_SEC).contains(&seconds).then_some(seconds)
}

/// What `--device` found among the phones `status` listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceMatch {
    Match(String),
    Ambiguous,
    None,
}

/// A full device id, else a name (ASCII case-insensitive, exact), else an id prefix of at least 4 characters. One match is that device, more than one is ambiguous.
pub fn match_device(selector: &str, devices: &[(String, String)]) -> DeviceMatch {
    let by_id: Vec<&(String, String)> = devices.iter().filter(|(id, _)| id == selector).collect();
    let by_name: Vec<&(String, String)> = devices.iter().filter(|(_, name)| name.eq_ignore_ascii_case(selector)).collect();
    let by_prefix: Vec<&(String, String)> =
        if selector.chars().count() >= 4 { devices.iter().filter(|(id, _)| id.starts_with(selector)).collect() } else { Vec::new() };
    for found in [by_id, by_name, by_prefix] {
        match found.as_slice() {
            [] => {}
            [one] => return DeviceMatch::Match(one.0.clone()),
            _ => return DeviceMatch::Ambiguous,
        }
    }
    DeviceMatch::None
}

/// What the pipe's answer ends as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    Sent,
    Queued,
    Skipped,
    Refused,
    NotRunning,
}

impl Word {
    pub fn vector_name(self) -> &'static str {
        match self {
            Word::Sent => "sent",
            Word::Queued => "queued",
            Word::Skipped => "skipped",
            Word::Refused => "refused",
            Word::NotRunning => "not_running",
        }
    }
}

/// The exit code and word for the pipe's answer to the send (`None`: there was no answer, Kin is not running).
pub fn exit_for(response: Option<&Value>) -> (i32, Word) {
    let Some(r) = response else {
        return (EXIT_NOT_RUNNING, Word::NotRunning);
    };
    if r["ok"] == json!(true) {
        let word = if r["queued"] == json!(true) && r["code"] == json!("DEVICE_OFFLINE_QUEUED") {
            Word::Queued
        } else if r["code"] == json!("PAUSED_SKIPPED") {
            Word::Skipped
        } else {
            Word::Sent
        };
        return (EXIT_OK, word);
    }
    let exit = match r["code"].as_str() {
        Some("KIN_NOT_RUNNING") => EXIT_NOT_RUNNING,
        Some("NOT_PAIRED") => EXIT_NOT_PAIRED,
        Some("TOO_LARGE") => EXIT_TOO_LONG,
        Some("INTERNAL") if r["rateLimited"] == json!(true) => EXIT_TOO_LONG,
        _ => EXIT_REFUSED,
    };
    (exit, Word::Refused)
}

// ===== the request (pure) =================================================================================================

/// The pipe `send` request for an alert (docs/extension.md "send"): `source` is `cli` and `auto` is false (a script's own alert is sent even while the PC is paused).
pub fn send_request(id: &str, args: &NotifyArgs, device_id: Option<&str>) -> Value {
    let mut v = json!({ "v": 1, "id": id, "type": "send", "kind": "alert", "text": args.text, "ttlSec": args.ttl_sec, "source": "cli", "auto": false });
    if let Some(t) = &args.title {
        v["title"] = json!(t);
    }
    if let Some(d) = device_id {
        v["deviceId"] = json!(d);
    }
    v
}

/// The line printed for an answer. `name` is the phone's name from the answer.
pub fn success_line(word: Word, name: &str, ttl_sec: u64) -> String {
    match word {
        Word::Queued => format!("Queued for {name} (offline), expires in {}.", ttl_word(ttl_sec)),
        Word::Skipped => "Skipped: Kin is paused.".to_string(),
        _ => format!("Sent to {name}."),
    }
}

// ===== the pipe (Windows) =================================================================================================

#[derive(Debug)]
pub enum ConnectError {
    /// No pipe of that name: Kin is not running.
    NotThere,
    /// Every instance is in use for too long.
    Busy,
    /// There is a pipe, but it is not Kin's (another program, another user).
    Untrusted(String),
    Other(io::Error),
}

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
/// SECURITY_IDENTIFICATION: the server can see who we are but cannot act as us.
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;

/// Opens the pipe once, without waiting. The server is not checked yet.
fn open_once(name: &str) -> Result<File, ConnectError> {
    match std::fs::OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(name) {
        Ok(f) => Ok(f),
        Err(e) => Err(match e.raw_os_error() {
            Some(ERROR_FILE_NOT_FOUND) => ConnectError::NotThere,
            Some(ERROR_PIPE_BUSY) => ConnectError::Busy,
            Some(5) => ConnectError::Untrusted("access denied opening the pipe".into()),
            _ => ConnectError::Other(e),
        }),
    }
}

fn wait_pipe(name: &str, millis: u32) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Pipes::WaitNamedPipeW;
    use windows::core::PCWSTR;
    let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    if unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), millis) }.as_bool() {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    // ERROR_SEM_TIMEOUT: no instance came free in time.
    if e.raw_os_error() == Some(121) { Ok(false) } else { Err(e) }
}

/// Opens the pipe; if every instance is busy waits for one for up to `busy_wait`.
fn open_waiting(name: &str, busy_wait: Duration) -> Result<File, ConnectError> {
    let deadline = Instant::now() + busy_wait;
    loop {
        match open_once(name) {
            Err(ConnectError::Busy) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(ConnectError::Busy);
                }
                match wait_pipe(name, left.as_millis().min(u32::MAX as u128) as u32) {
                    Ok(true) => {}
                    Ok(false) => return Err(ConnectError::Busy),
                    Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => return Err(ConnectError::NotThere),
                    Err(e) => return Err(ConnectError::Other(e)),
                }
            }
            other => return other,
        }
    }
}

/// The program file of the process at the other end of the pipe, and the user it runs as.
fn server_identity(pipe: &File) -> io::Result<(PathBuf, String)> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;
    let mut pid = 0u32;
    // SAFETY: the handle is a live pipe handle owned by `pipe`; `pid` is a valid out pointer.
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.map_err(io::Error::other)?;
    // SAFETY: the process handle is closed on every path; the buffer is as long as the length says.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(io::Error::other)?;
        let result = (|| {
            let mut buffer = vec![0u16; 1024];
            let mut len = buffer.len() as u32;
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len).map_err(io::Error::other)?;
            let image = PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]));
            let user = crate::nativehost::user_sid_of_process(process)?;
            Ok((image, user))
        })();
        let _ = CloseHandle(process);
        result
    }
}

/// The same program file, by its real path, ignoring case (Windows paths are case-insensitive).
pub fn same_program(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().trim_start_matches(r"\\?\").to_lowercase()
    };
    norm(a) == norm(b)
}

/// Checks the server before anything is sent: its process belongs to this user and is `expected_image` (`Kin.exe` in the folder of this program).
fn verify_server(pipe: &File, me: &str, expected_image: &Path) -> Result<(), String> {
    let (image, user) = server_identity(pipe).map_err(|e| format!("can't identify the pipe server: {e}"))?;
    if user != me {
        return Err("the pipe server is not run by this user".into());
    }
    if !same_program(&image, expected_image) {
        return Err("the pipe server is not this Kin".into());
    }
    Ok(())
}

/// Opens the pipe and checks the server: a request only ever goes to what this returns.
pub fn connect(name: &str, expected_image: &Path, busy_wait: Duration) -> Result<File, ConnectError> {
    let f = open_waiting(name, busy_wait)?;
    let me = crate::nativehost::current_user_sid().map_err(ConnectError::Other)?;
    verify_server(&f, &me, expected_image).map_err(ConnectError::Untrusted)?;
    Ok(f)
}

/// One request on an open connection: a frame out, a frame in, with a time limit. The I/O runs on a thread (a pipe read can't time out by itself); when the limit
/// passes the process is about to exit, which ends the thread.
pub fn exchange(pipe: File, request: &Value, timeout: Duration) -> Result<Value, String> {
    let body = request.to_string().into_bytes();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("notify-pipe".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let mut pipe = pipe;
            let result = framing::write_frame(&mut pipe, &body).map_err(|e| e.to_string()).and_then(|()| {
                match framing::read_frame(&mut pipe, MAX_RESPONSE) {
                    Ok(Some(answer)) => serde_json::from_slice::<Value>(&answer).map_err(|e| e.to_string()),
                    Ok(None) | Err(FrameError::Truncated) => Err("Kin closed the connection".to_string()),
                    Err(FrameError::TooLarge(_)) => Err("Kin's answer was too big".to_string()),
                    Err(FrameError::Io(e)) => Err(e.to_string()),
                }
            });
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(timeout).unwrap_or_else(|_| Err("Kin didn't answer in time".to_string()))
}

/// Starts the installed Kin with no window. Only ever the program file next to this one.
fn start_kin(kin: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(kin)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

// ===== the console ==============================================================================================================

/// Where a GUI-subsystem program prints: a redirected stdout/stderr handle if there is one, else the console that started it (`AttachConsole`), else nowhere.
struct Console {
    attached: bool,
}

impl Console {
    fn new() -> Console {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // SAFETY: no pointers; fails (and is ignored) when there is no parent console.
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok();
        Console { attached }
    }

    fn print(&self, error: bool, line: &str) {
        use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
        let which = if error { STD_ERROR_HANDLE } else { STD_OUTPUT_HANDLE };
        // SAFETY: `GetStdHandle` only reads the process's standard handle.
        let handle = unsafe { GetStdHandle(which) };
        let valid = handle.as_ref().is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
        let text = format!("{line}\r\n");
        if valid {
            // A redirected or inherited handle (`> file`, a pipe, a console of a console-subsystem parent). Not closed: it is not ours.
            use std::os::windows::io::{FromRawHandle, RawHandle};
            let raw = handle.map(|h| h.0 as RawHandle).unwrap_or(std::ptr::null_mut());
            // SAFETY: the handle is the process's own standard handle; `ManuallyDrop` keeps it open.
            let mut file = std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(raw) });
            let _ = file.write_all(text.as_bytes());
        } else if self.attached
            && let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$")
        {
            let _ = console.write_all(text.as_bytes());
        }
    }

    fn out(&self, line: &str) {
        self.print(false, line);
    }

    fn err(&self, line: &str) {
        self.print(true, line);
    }
}

/// Reads all of stdin (the form `-`), at most a little over the limit: more than that is an error anyway.
fn read_stdin() -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = io::stdin().take(STDIN_LIMIT as u64).read_to_end(&mut bytes);
    bytes
}

// ===== run ====================================================================================================================

/// Splits a command line at `--notify`: what comes before it (`--data-dir x`, developer settings) names the instance, what comes after is the notify arguments.
pub fn split_notify(args: &[String]) -> Option<(&[String], &[String])> {
    let at = args.iter().position(|a| a == "--notify")?;
    Some((&args[..at], &args[at + 1..]))
}

/// `Some(exit code)` when the command line is a `--notify`; `None` for everything else (the app starts).
pub fn try_run(args: &[String]) -> Option<i32> {
    let (globals, rest) = split_notify(args)?;
    Some(run(globals, rest))
}

fn run(globals: &[String], rest: &[String]) -> i32 {
    let console = Console::new();
    let args = match parse_args(rest, read_stdin) {
        Ok(a) => a,
        Err(u) => {
            console.err(&format!("Kin --notify: {}", u.message()));
            return u.exit;
        }
    };
    let parsed = match crate::cli::parse(globals, &|k| std::env::var(k).ok()) {
        Ok(p) => p,
        Err(e) => {
            console.err(&format!("Kin --notify: {e}"));
            return EXIT_USAGE;
        }
    };
    let kind = HostKind::for_config(&parsed.config);
    let Ok(sid) = crate::nativehost::current_user_sid() else {
        console.err("Kin --notify: can't tell which user this is.");
        return EXIT_REFUSED;
    };
    let suffix = std::env::var("KIN_DEV_PIPE_SUFFIX").ok();
    let pipe = kind.pipe_name(&sid, suffix.as_deref());
    let Some(kin) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("Kin.exe"))) else {
        console.err("Kin --notify: can't find Kin.exe next to this program.");
        return EXIT_REFUSED;
    };
    let may_start = may_start_kin(args.no_start, &parsed.config, globals, suffix.as_deref());
    let status = |c: &Console| -> Result<Option<Value>, i32> {
        let name = &pipe;
        match request(name, &kin, &json!({ "v": 1, "id": uuid_v7(unix_ms()), "type": "status" }), may_start, SEND_TIMEOUT) {
            Ok(v) => Ok(Some(v)),
            Err(Failure::NotRunning) => Ok(None),
            Err(f) => {
                c.err(&format!("Kin --notify: {}", f.message()));
                Err(EXIT_REFUSED)
            }
        }
    };
    // --device: the name or id the user typed, looked up among the phones Kin lists.
    let device_id = match &args.device {
        None => None,
        Some(selector) => {
            let listing = match status(&console) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    console.err(&format!("Kin --notify: {}", not_running_text(may_start)));
                    return EXIT_NOT_RUNNING;
                }
                Err(code) => return code,
            };
            let devices: Vec<(String, String)> = listing["devices"]
                .as_array()
                .map(|a| a.iter().filter_map(|d| Some((d["id"].as_str()?.to_string(), d["name"].as_str()?.to_string()))).collect())
                .unwrap_or_default();
            match match_device(selector, &devices) {
                DeviceMatch::Match(id) => Some(id),
                DeviceMatch::Ambiguous => {
                    console.err(&format!("Kin --notify: {}", Usage::new("ambiguous_device").message()));
                    return EXIT_USAGE;
                }
                DeviceMatch::None => {
                    console.err("Kin --notify: that phone isn't paired.");
                    return EXIT_NOT_PAIRED;
                }
            }
        }
    };
    let id = uuid_v7(unix_ms());
    let answer = match request(&pipe, &kin, &send_request(&id, &args, device_id.as_deref()), may_start, SEND_TIMEOUT) {
        Ok(v) => Some(v),
        Err(Failure::NotRunning) => None,
        Err(f) => {
            console.err(&format!("Kin --notify: {}", f.message()));
            return EXIT_REFUSED;
        }
    };
    let (exit, word) = exit_for(answer.as_ref());
    match (&answer, word) {
        (None, _) => console.err(&format!("Kin --notify: {}", not_running_text(may_start))),
        (Some(a), Word::Refused) => {
            let message = a["message"].as_str().unwrap_or("Kin refused it.");
            console.err(&format!("Kin --notify: {message}"));
        }
        (Some(a), w) => console.out(&success_line(w, a["deviceName"].as_str().unwrap_or("your phone"), args.ttl_sec)),
    }
    exit
}

/// Whether `--notify` may start the `Kin.exe` next to it. Only for the installed Kin: a release build, no `--data-dir` or developer setting of its own
/// (the same test as "Start with Windows" uses for the real Run key, `Config::autostart_is_real`), and no `KIN_DEV_PIPE_SUFFIX`. Any of those names a
/// developer instance, and starting `Kin.exe` with no `--data-dir` would open the user's own data directory instead.
fn may_start_kin(no_start: bool, config: &crate::cli::Config, globals: &[String], pipe_suffix: Option<&str>) -> bool {
    !no_start && globals.is_empty() && config.autostart_is_real() && pipe_suffix.is_none()
}

fn not_running_text(may_start: bool) -> &'static str {
    if may_start { "Kin isn't running and could not be started." } else { "Kin isn't running." }
}

enum Failure {
    NotRunning,
    Other(String),
}

impl Failure {
    fn message(&self) -> String {
        match self {
            Failure::NotRunning => "Kin isn't running.".into(),
            Failure::Other(m) => m.clone(),
        }
    }
}

/// One request: connect (starting the installed Kin if there is no pipe and that is allowed), exchange, return the answer.
fn request(pipe: &str, kin: &Path, request: &Value, may_start: bool, timeout: Duration) -> Result<Value, Failure> {
    let file = match connect(pipe, kin, BUSY_WAIT) {
        Ok(f) => f,
        Err(ConnectError::NotThere) if may_start => {
            start_kin(kin).map_err(|_| Failure::NotRunning)?;
            let deadline = Instant::now() + START_WAIT;
            loop {
                match connect(pipe, kin, BUSY_WAIT) {
                    Ok(f) => break f,
                    Err(ConnectError::NotThere) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(150)),
                    Err(ConnectError::NotThere) => return Err(Failure::NotRunning),
                    Err(e) => return Err(Failure::Other(connect_text(&e))),
                }
            }
        }
        Err(ConnectError::NotThere) => return Err(Failure::NotRunning),
        Err(e) => return Err(Failure::Other(connect_text(&e))),
    };
    exchange(file, request, timeout).map_err(Failure::Other)
}

fn connect_text(e: &ConnectError) -> String {
    match e {
        ConnectError::NotThere => "Kin isn't running.".into(),
        ConnectError::Busy => "Kin is busy. Try again.".into(),
        ConnectError::Untrusted(why) => format!("Not talking to that pipe ({why})."),
        ConnectError::Other(e) => format!("Couldn't reach Kin: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn the_command_line_is_split_at_notify() {
        let args = s(&["--data-dir", "x", "--notify", "--title", "T", "Done"]);
        let (globals, rest) = split_notify(&args).unwrap();
        assert_eq!(globals, &s(&["--data-dir", "x"])[..]);
        assert_eq!(rest, &s(&["--title", "T", "Done"])[..]);
        assert!(split_notify(&s(&["--ring"])).is_none());
        assert_eq!(try_run(&s(&["--ring"])), None, "anything but --notify starts the app");
    }

    #[test]
    fn only_a_release_build_with_no_instance_settings_may_start_kin() {
        let plain = crate::cli::Config::new(PathBuf::from("data"));
        // The installed Kin is started only by a release build; this test binary may be either.
        assert_eq!(may_start_kin(false, &plain, &[], None), !cfg!(debug_assertions));
        // Whatever the build: --no-start, a global argument, a pipe suffix, a data directory of its own or a developer setting say no.
        assert!(!may_start_kin(true, &plain, &[], None));
        assert!(!may_start_kin(false, &plain, &s(&["--data-dir", "x"]), None));
        assert!(!may_start_kin(false, &plain, &[], Some("a")), "KIN_DEV_PIPE_SUFFIX alone names a dev instance");
        let mut own_dir = crate::cli::Config::new(PathBuf::from("data"));
        own_dir.data_dir_overridden = true;
        assert!(!may_start_kin(false, &own_dir, &[], None));
        let mut dev = crate::cli::Config::new(PathBuf::from("data"));
        dev.dev_no_windows = true;
        assert!(!may_start_kin(false, &dev, &[], None));
    }

    #[test]
    fn the_request_is_a_cli_alert_that_is_not_auto() {
        let a = parse_args(&s(&["--title", "T", "--ttl", "2h", "Done"]), Vec::new).unwrap();
        let r = send_request("i", &a, Some("dev1"));
        assert_eq!(
            r,
            json!({"v":1,"id":"i","type":"send","kind":"alert","text":"Done","title":"T","ttlSec":7200,"source":"cli","auto":false,"deviceId":"dev1"})
        );
        let plain = send_request("i", &parse_args(&s(&["Done"]), Vec::new).unwrap(), None);
        assert!(plain.get("deviceId").is_none() && plain.get("title").is_none());
    }

    /// Review round 3: stdin is read up to the limit, which can cut a character in two. That is a text that is too long (exit 5), never "not UTF-8".
    #[test]
    fn stdin_cut_in_the_middle_of_a_character_is_too_long_not_bad_utf8() {
        // '€' is three bytes and the limit is not a multiple of 3, so the read stops inside one.
        let mut bytes = "€".repeat(STDIN_LIMIT / 3 + 10).into_bytes();
        bytes.truncate(STDIN_LIMIT);
        assert!(std::str::from_utf8(&bytes).is_err(), "the cut is inside a character");
        let err = parse_args(&s(&["-"]), || bytes).unwrap_err();
        assert_eq!((err.reason, err.exit), ("text_too_long", EXIT_TOO_LONG));
        // The same cut at a character edge, and plainly bad bytes under the limit, behave as before.
        let whole = "é".repeat(STDIN_LIMIT / 2).into_bytes();
        assert_eq!(parse_args(&s(&["-"]), || whole).unwrap_err().reason, "text_too_long");
        let err = parse_args(&s(&["-"]), || vec![b'a', 0xff, b'b']).unwrap_err();
        assert_eq!(err.reason, "stdin_not_utf8");
        let short_cut = vec![b'a', 0xc3];
        assert_eq!(
            parse_args(&s(&["-"]), || short_cut).unwrap_err().reason,
            "stdin_not_utf8",
            "a short input that ends inside a character is bad UTF-8"
        );
    }

    #[test]
    fn the_lines_say_what_happened() {
        assert_eq!(success_line(Word::Sent, "Pixel 8", 86_400), "Sent to Pixel 8.");
        assert_eq!(success_line(Word::Queued, "Pixel 8", 86_400), "Queued for Pixel 8 (offline), expires in 24h.");
        assert_eq!(success_line(Word::Queued, "Pixel 8", 259_200), "Queued for Pixel 8 (offline), expires in 3d.");
        assert_eq!(success_line(Word::Queued, "Pixel 8", 7_200), "Queued for Pixel 8 (offline), expires in 2h.");
    }

    /// The pipe server must be this program's `Kin.exe` and this user: a pipe of any other program is refused before a byte of the request is written.
    #[test]
    fn a_pipe_whose_server_is_not_the_expected_program_is_refused() {
        let name = format!(r"\\.\pipe\kin-notify-test-{}-{}", std::process::id(), unix_ms());
        // Two instances: a client takes one, and the second is free for the next client.
        let server = crate::nativehost::pipe::create_instance(&name, true).expect("the test pipe");
        let second = crate::nativehost::pipe::create_instance(&name, false).expect("a second instance of the test pipe");
        let me = std::env::current_exe().unwrap();
        // The server of this pipe is this test program: accepted when it is the expected image...
        let accepted = connect(&name, &me, Duration::from_secs(2));
        assert!(accepted.is_ok(), "{accepted:?}");
        drop(accepted);
        // ...and refused, before anything is written, for any other expected program.
        let other = std::env::temp_dir().join("Kin.exe");
        match connect(&name, &other, Duration::from_secs(2)) {
            Err(ConnectError::Untrusted(why)) => assert!(why.contains("not this Kin"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        drop((server, second));
    }

    #[test]
    fn same_program_ignores_case_and_a_verbatim_prefix() {
        let me = std::env::current_exe().unwrap();
        let upper = PathBuf::from(me.to_string_lossy().to_uppercase());
        assert!(same_program(&me, &upper));
        assert!(!same_program(&me, &std::env::temp_dir().join("Kin.exe")));
    }
}
