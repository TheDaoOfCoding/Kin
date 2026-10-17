//! The client side of the pipe to Kin: open it as the current user without letting the server impersonate us, check who the server is, and run
//! one request (write a frame, read a frame) with a time limit.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::framing::{self, FrameError, MAX_PIPE_MESSAGE};
use crate::winutil::{self, UserSid};

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_PIPE_BUSY: i32 = 231;
/// SECURITY_IDENTIFICATION: the server can see who we are, but can't act as us. (The flag that also marks the QOS fields present is set by std.)
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;

#[derive(Debug)]
pub enum ConnectError {
    /// No pipe of that name: Kin isn't running (or has no extension pipe).
    NotThere,
    /// Every instance is in use.
    Busy,
    /// The pipe is there but isn't one we may use: access denied, or the server isn't our user's Kin.
    Untrusted(String),
    Other(io::Error),
}

/// Opens the pipe once, without waiting. The server isn't checked yet (see `verify_server`).
pub fn open_once(name: &str) -> Result<File, ConnectError> {
    match OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(name) {
        Ok(f) => Ok(f),
        Err(e) => Err(match e.raw_os_error() {
            Some(ERROR_FILE_NOT_FOUND) => ConnectError::NotThere,
            Some(ERROR_PIPE_BUSY) => ConnectError::Busy,
            Some(ERROR_ACCESS_DENIED) => ConnectError::Untrusted("access denied opening the pipe".into()),
            _ => ConnectError::Other(e),
        }),
    }
}

/// Opens the pipe, and if every instance is busy waits for one for up to `busy_wait`. Returns `Busy` when none came free.
pub fn open_waiting(name: &str, busy_wait: Duration) -> Result<File, ConnectError> {
    let deadline = Instant::now() + busy_wait;
    loop {
        match open_once(name) {
            Err(ConnectError::Busy) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(ConnectError::Busy);
                }
                match winutil::wait_named_pipe(name, left.as_millis().min(u32::MAX as u128) as u32) {
                    Ok(true) => {} // free now: try to open it (somebody else may get there first, then we wait again)
                    Ok(false) => return Err(ConnectError::Busy),
                    Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => return Err(ConnectError::NotThere),
                    Err(e) => return Err(ConnectError::Other(e)),
                }
            }
            other => return other,
        }
    }
}

/// Checks the server before anything is sent: its process must belong to the current user and, when `expected_image` is set, be that program.
pub fn verify_server(pipe: &File, me: &UserSid, expected_image: Option<&std::path::Path>) -> Result<(), String> {
    let server = winutil::server_process(pipe).map_err(|e| format!("can't identify the pipe server: {e}"))?;
    if !server.user.equals(me) {
        return Err(format!("the pipe server (pid {}) is not run by this user", server.pid));
    }
    if let Some(expected) = expected_image
        && !winutil::same_path(&server.image, expected) {
            return Err(format!("the pipe server (pid {}) is not Kin (it's {})", server.pid, server.image.display()));
        }
    Ok(())
}

/// Opens the pipe and checks the server: the host only ever sends to what this returns.
pub fn connect(cfg: &Config, name: &str, busy_wait: Duration) -> Result<File, ConnectError> {
    let f = open_waiting(name, busy_wait)?;
    let me = UserSid::current().map_err(ConnectError::Other)?;
    verify_server(&f, &me, cfg.expected_server_image.as_deref()).map_err(ConnectError::Untrusted)?;
    Ok(f)
}

#[derive(Debug)]
pub enum ExchangeError {
    /// Kin closed the connection (or the pipe broke) before a whole answer came.
    Closed,
    /// Kin's answer was over the limit.
    TooLarge,
    Timeout,
    Io(io::Error),
}

fn do_exchange(pipe: &mut File, request: &[u8]) -> Result<Vec<u8>, ExchangeError> {
    framing::write_frame(pipe, request).map_err(|e| match e.kind() {
        io::ErrorKind::BrokenPipe => ExchangeError::Closed,
        _ if e.raw_os_error() == Some(232) /* ERROR_NO_DATA: the server closed */ => ExchangeError::Closed,
        _ => ExchangeError::Io(e),
    })?;
    match framing::read_frame(pipe, MAX_PIPE_MESSAGE, 0) {
        Ok(Some(answer)) => Ok(answer),
        Ok(None) | Err(FrameError::Truncated) => Err(ExchangeError::Closed),
        Err(FrameError::TooLarge(_)) => Err(ExchangeError::TooLarge),
        Err(FrameError::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe => Err(ExchangeError::Closed),
        Err(FrameError::Io(e)) => Err(ExchangeError::Io(e)),
    }
}

/// One request on an open connection: write the request frame, read the answer frame, give up after `timeout`.
/// The I/O runs in a thread; on a timeout the blocked call is cancelled (CancelIoEx), so nothing is left running.
pub fn exchange(pipe: File, request: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, ExchangeError> {
    let raw = pipe.as_raw_handle() as usize;
    let (tx, rx) = mpsc::channel();
    // The thread hands the file back with the result, so the handle stays open (and `raw` stays ours) until this function has seen the end.
    let worker = thread::Builder::new().name("pipe-exchange".into()).stack_size(256 * 1024).spawn(move || {
        let mut pipe = pipe;
        let r = do_exchange(&mut pipe, &request);
        let _ = tx.send((pipe, r));
    });
    if let Err(e) = worker {
        return Err(ExchangeError::Io(e));
    }
    match rx.recv_timeout(timeout) {
        Ok((_pipe, r)) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Cancel until the worker reports back: a cancel that lands between its write and its read would be lost, so repeat it.
            let stop = Instant::now() + Duration::from_secs(2);
            while Instant::now() < stop {
                winutil::cancel_io(raw);
                if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
                    break;
                }
            }
            Err(ExchangeError::Timeout)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ExchangeError::Io(io::Error::other("the pipe thread panicked"))),
    }
}
