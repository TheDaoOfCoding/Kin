//! The host's loop and the handling of one request: docs/extension.md, "Messages" and "Host build notes".

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::envelope::{self, Envelope};
use crate::framing::{self, FrameError, MAX_REQUEST, MAX_RESPONSE};
use crate::pipe::{self, ConnectError, ExchangeError};
use crate::winutil::{self, NamedLock};
use crate::log;

/// Why the loop ended (the browser doesn't look at the exit code; the tests do).
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// stdin was closed at a frame boundary: the normal end.
    InputClosed,
    /// stdin ended inside a frame, or couldn't be read.
    InputBroken,
    /// A request was over 64 MiB: answered with TOO_LARGE, then stopped (the body was not read).
    TooLarge,
    /// stdout couldn't be written (the browser went away).
    OutputBroken,
}

/// Reads requests from `input` and answers each on `output`, until `input` ends. `origin` is what the browser told the host about its caller.
pub fn run<R: Read, W: Write>(cfg: &Config, origin: &str, input: &mut R, output: &mut W) -> Ended {
    // Room for the two fields added to every request, so a 64 MiB body is never copied.
    let spare = origin.len() * 6 + cfg.host_version.len() + 64;
    loop {
        let body = match framing::read_frame(input, MAX_REQUEST, spare) {
            Ok(Some(b)) => b,
            Ok(None) => return Ended::InputClosed,
            Err(FrameError::TooLarge(n)) => {
                log(&format!("request refused: {n} bytes announced, the limit is {MAX_REQUEST}"));
                let _ = framing::write_frame(output, &envelope::error_response(None, "error", envelope::TOO_LARGE));
                return Ended::TooLarge;
            }
            Err(FrameError::Truncated) => return Ended::InputBroken,
            Err(FrameError::Io(e)) => {
                log(&format!("stdin: {e}"));
                return Ended::InputBroken;
            }
        };
        let answer = handle(cfg, origin, body);
        debug_assert!(answer.len() <= MAX_RESPONSE);
        if framing::write_frame(output, &answer).is_err() {
            return Ended::OutputBroken;
        }
    }
}

/// One request in, one response out. Never fails: every problem is an answer.
pub fn handle(cfg: &Config, origin: &str, body: Vec<u8>) -> Vec<u8> {
    let env = match envelope::parse(&body) {
        Ok(e) => e,
        Err(rejection) => return rejection,
    };
    let (budget, may_start) = match env.ty.as_str() {
        "status" => (cfg.status_timeout, false),
        "open" => (cfg.status_timeout, false),
        "start" => (cfg.start_timeout, true),
        "send" => (cfg.send_timeout, true),
        _ => return envelope::error_response(Some(&env.id), &env.ty, envelope::UNSUPPORTED_TYPE),
    };
    let deadline = Instant::now() + budget;
    let err = |what| envelope::error_response(Some(&env.id), &env.ty, what);

    let Some(name) = cfg.pipe_name.as_deref() else {
        log("the user's SID couldn't be read");
        return err(envelope::INTERNAL);
    };

    // 1. Connect (and start Kin first, for the types that may).
    let conn = match connect_for(cfg, &env, name, may_start, deadline) {
        Ok(f) => f,
        // docs/extension.md: no pipe, or a pipe that stays busy for more than a second, is `running: false` for a status (not an error).
        Err(Early::NotRunning | Early::Timeout) if env.ty == "status" => return envelope::not_running_status(&env.id, &cfg.host_version),
        Err(Early::NotRunning | Early::Untrusted) => return err(envelope::KIN_NOT_RUNNING),
        Err(Early::Timeout) => return err(envelope::TIMEOUT),
        Err(Early::Internal) => return err(envelope::INTERNAL),
    };

    // 2. Forward the request with the host's fields added, and wait for the answer.
    let request = envelope::add_host_fields(body, origin, &cfg.host_version);
    let left = deadline.saturating_duration_since(Instant::now());
    match pipe::exchange(conn, request, left) {
        Ok(answer) => match envelope::finish_response(answer, &env.ty, &cfg.host_version, MAX_RESPONSE) {
            Some(a) => a,
            None => {
                log("Kin's answer was not a JSON object under 1 MB");
                err(envelope::INTERNAL)
            }
        },
        Err(ExchangeError::Timeout) => err(envelope::TIMEOUT),
        Err(ExchangeError::Closed) => {
            log("Kin closed the connection without an answer");
            err(envelope::INTERNAL)
        }
        Err(ExchangeError::TooLarge) => {
            log("Kin's answer was over the limit");
            err(envelope::INTERNAL)
        }
        Err(ExchangeError::Io(e)) => {
            log(&format!("pipe: {e}"));
            err(envelope::INTERNAL)
        }
    }
}

enum Early {
    NotRunning,
    /// The pipe is there but its server isn't the user's Kin: nothing is sent, and it's an error even for a `status`.
    Untrusted,
    Timeout,
    Internal,
}

/// Gets a verified connection to Kin, or the reason there is none.
fn connect_for(cfg: &Config, env: &Envelope, name: &str, may_start: bool, deadline: Instant) -> Result<std::fs::File, Early> {
    let is_status = env.ty == "status";
    let busy_wait = if is_status { cfg.status_busy_wait } else { cfg.busy_wait };
    let first = pipe::connect(cfg, name, busy_wait);
    let first = match first {
        Err(ConnectError::NotThere) if may_start => return start_and_connect(cfg, name, deadline),
        other => other,
    };
    classify(first)
}

fn classify(r: Result<std::fs::File, ConnectError>) -> Result<std::fs::File, Early> {
    match r {
        Ok(f) => Ok(f),
        Err(ConnectError::NotThere) => Err(Early::NotRunning),
        // Every instance in use for the whole wait: Kin is there but very busy. A `status` says "not running" (docs/extension.md); the others time out.
        Err(ConnectError::Busy) => Err(Early::Timeout),
        Err(ConnectError::Untrusted(why)) => {
            log(&format!("pipe refused: {why}"));
            Err(Early::Untrusted)
        }
        Err(ConnectError::Other(e)) => {
            log(&format!("pipe: {e}"));
            Err(Early::Internal)
        }
    }
}

/// The pipe isn't there and the request may start Kin: start it (once, even if several hosts try together) and wait for the pipe.
fn start_and_connect(cfg: &Config, name: &str, deadline: Instant) -> Result<std::fs::File, Early> {
    let wait_until = (Instant::now() + cfg.start_wait).min(deadline);
    let left = |t: Instant| t.saturating_duration_since(Instant::now());

    // Only one host at a time starts Kin. Whoever gets the lock second finds the pipe (or the failure) on its own first look.
    let _lock = NamedLock::acquire(&cfg.lock_name, left(wait_until).as_millis().min(u32::MAX as u128) as u32);
    match pipe::connect(cfg, name, cfg.busy_wait) {
        Err(ConnectError::NotThere) => {}
        other => return classify(other),
    }
    let Some(exe) = cfg.kin_exe.as_deref() else {
        log("no Kin.exe to start");
        return Err(Early::NotRunning);
    };
    if !exe.is_file() {
        log(&format!("Kin.exe not found at {}", exe.display()));
        return Err(Early::NotRunning);
    }
    if let Err(e) = winutil::start_detached(exe, &cfg.kin_args) {
        log(&format!("starting Kin failed: {e}"));
        return Err(Early::NotRunning);
    }
    loop {
        std::thread::sleep(Duration::from_millis(100).min(left(wait_until)));
        match pipe::connect(cfg, name, cfg.busy_wait) {
            Err(ConnectError::NotThere) => {
                if Instant::now() >= wait_until {
                    log("Kin was started but its pipe didn't appear in time");
                    return Err(Early::NotRunning);
                }
            }
            other => return classify(other),
        }
    }
}
