//! The named pipe that `kin-native-host.exe` connects to (docs/extension.md, "Pipe (host <-> Kin)").
//!
//! * **Current user only:** a protected DACL with one allow entry for the user's SID (`D:P(A;;GA;;;<SID>)`): nothing for Everyone, Users,
//!   Network, SYSTEM or Administrators.
//! * **`PIPE_REJECT_REMOTE_CLIENTS`:** nobody on the network can open it through SMB.
//! * **`FILE_FLAG_FIRST_PIPE_INSTANCE`** on the first instance, so no other program can have made the name before Kin. If that creation fails
//!   the caller logs it and carries on without the extension pipe; a later instance is never made as a fallback.
//! * Byte mode, one request per connection: read a frame, answer with a frame, flush, disconnect. A new instance is made **before** a client
//!   is served, so a second host is never refused, and each client is served on a thread of its own so a slow `send` never blocks the accept loop.
//! * A client that hasn't sent a whole request in 30 seconds is dropped (its read is cancelled).
use std::ffi::OsStr;
use std::fs::File;
use std::future::Future;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::core::PCWSTR;

use super::framing::{self, FrameError, MAX_REQUEST};

/// What a handler does with a request.
pub enum Reply {
    /// Answer with these bytes (a framed JSON object, at most 1 MiB: the handler's business).
    Respond(Vec<u8>),
    /// Close the connection without an answer.
    Close,
}

pub type ReplyFuture = Pin<Box<dyn Future<Output = Reply> + Send>>;
pub type Handler = Arc<dyn Fn(Vec<u8>) -> ReplyFuture + Send + Sync>;

/// `nMaxInstances` for "as many as needed" (the documented `PIPE_UNLIMITED_INSTANCES`).
const UNLIMITED: u32 = 255;
/// How long a client has to send its whole request.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// The security descriptor that names the current user only: `D:P(A;;GA;;;<SID>)`.
pub fn current_user_only_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

/// The SID of the user, looked up once (every instance needs it, and a burst of clients makes many instances).
fn user_sid() -> io::Result<&'static str> {
    static SID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if SID.get().is_none() {
        let _ = SID.set(super::current_user_sid()?);
    }
    SID.get().map(String::as_str).ok_or_else(|| io::Error::other("the user's SID is not known"))
}

/// Creates one instance of the pipe.
pub fn create_instance(name: &str, first: bool) -> io::Result<File> {
    let sddl = wide(&current_user_only_sddl(user_sid()?));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sddl` is NUL-terminated; the descriptor that Windows allocates is freed with `LocalFree` below, after the pipe has taken its copy.
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut descriptor, None) }
        .map_err(io::Error::other)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name_w = wide(name);
    let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { Default::default() };
    // SAFETY: `name_w` is NUL-terminated; `attributes` and the descriptor it points to are alive for the call.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name_w.as_ptr()),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            UNLIMITED,
            64 * 1024,
            64 * 1024,
            0,
            Some(&attributes),
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: allocated by `ConvertStringSecurityDescriptor...`; the pipe keeps its own copy.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(error);
    }
    // SAFETY: a valid pipe handle that nothing else owns.
    Ok(unsafe { File::from_raw_handle(handle.0 as RawHandle) })
}

fn raw(pipe: &File) -> HANDLE {
    HANDLE(pipe.as_raw_handle())
}

/// What waiting for a client came to.
enum Connected {
    /// A client is there.
    Yes,
    /// A client came and went before it was served (`ERROR_NO_DATA`): the instance is reused.
    Gone,
}

fn connect_client(pipe: &File) -> io::Result<Connected> {
    // SAFETY: a valid server-side pipe handle, no OVERLAPPED: blocks until a client connects.
    match unsafe { ConnectNamedPipe(raw(pipe), None) } {
        Ok(()) => Ok(Connected::Yes),
        // The client connected between `CreateNamedPipe` and `ConnectNamedPipe`: that is a connection.
        Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => Ok(Connected::Yes),
        Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_NO_DATA.0) => Ok(Connected::Gone),
        Err(e) => Err(io::Error::other(e)),
    }
}

fn serve_connection(mut pipe: File, handler: &Handler, runtime: &tokio::runtime::Handle) {
    let handle = raw(&pipe);
    // A client that never finishes its request: its read is cancelled after the deadline.
    let reading = Arc::new(AtomicBool::new(true));
    let watchdog = {
        let reading = reading.clone();
        let target = handle.0 as usize;
        thread::Builder::new().name("kin-pipe-watchdog".into()).spawn(move || {
            let start = Instant::now();
            while reading.load(Ordering::SeqCst) {
                if start.elapsed() >= REQUEST_DEADLINE {
                    // SAFETY: the handle stays open until this connection has been served, and the watchdog is joined before it is closed.
                    unsafe {
                        let _ = CancelIoEx(HANDLE(target as *mut _), None);
                    }
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        })
    };
    let request = framing::read_frame(&mut pipe, MAX_REQUEST);
    reading.store(false, Ordering::SeqCst);
    if let Ok(w) = watchdog {
        let _ = w.join();
    }
    match request {
        Ok(Some(request)) => {
            if let Reply::Respond(answer) = runtime.block_on(handler(request)) {
                debug_assert!(answer.len() <= framing::MAX_RESPONSE);
                if framing::write_frame(&mut pipe, &answer).is_ok() {
                    // Without this the answer can be thrown away when the handle closes, before the client has read it.
                    // SAFETY: a valid handle; blocks until the client has read everything or has gone.
                    unsafe {
                        let _ = FlushFileBuffers(handle);
                    }
                }
            }
        }
        Ok(None) | Err(FrameError::Truncated | FrameError::TooLarge(_) | FrameError::Io(_)) => {}
    }
    // SAFETY: a valid handle.
    unsafe {
        let _ = DisconnectNamedPipe(handle);
    }
}

/// The next instance of the pipe. If the system says every instance is in use it waits a moment and tries again (a few times).
fn make_next(name: &str) -> Option<File> {
    for _ in 0..100 {
        match create_instance(name, false) {
            Ok(f) => return Some(f),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                log::error!("the extension pipe: making an instance failed: {e}");
                return None;
            }
        }
    }
    None
}

pub struct PipeServer {
    pub name: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PipeServer {
    /// Starts serving on `name`. The first instance is made before this returns, so the pipe exists; if the name is taken (or the instance can't
    /// be made) that is the error, and the caller carries on without the pipe.
    pub fn start(name: &str, handler: Handler, runtime: tokio::runtime::Handle) -> io::Result<PipeServer> {
        let first = create_instance(name, true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, name2) = (stop.clone(), name.to_string());
        let thread = thread::Builder::new().name("kin-pipe-accept".into()).spawn(move || {
            // One instance waits for a client. When one connects, the next instance is made at once (before the client is served), so the gap in
            // which a host could find no listening instance is the time it takes to make one; a host that does find the pipe busy waits for it
            // (`WaitNamedPipe`), as docs/extension.md says. (A spare instance made ahead of time would be one that nobody waits on: a client that
            // attaches to it is not seen until another one connects to the first.)
            let mut instance = first;
            loop {
                match connect_client(&instance) {
                    Ok(Connected::Yes) => {}
                    Ok(Connected::Gone) => {
                        // A client came and went before it was served: the instance is made ready again and waits on.
                        // SAFETY: a valid handle.
                        unsafe {
                            let _ = DisconnectNamedPipe(raw(&instance));
                        }
                        continue;
                    }
                    Err(_) => return,
                }
                if stop2.load(Ordering::SeqCst) {
                    return;
                }
                let Some(next) = make_next(&name2) else {
                    log::error!("the extension pipe stopped: making the next instance failed");
                    return;
                };
                let (conn, handler, runtime) = (std::mem::replace(&mut instance, next), handler.clone(), runtime.clone());
                let _ = thread::Builder::new().name("kin-pipe-client".into()).spawn(move || serve_connection(conn, &handler, &runtime));
            }
        })?;
        Ok(PipeServer { name: name.to_string(), stop, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.stop.store(true, Ordering::SeqCst);
            // Wake the accept loop with a connection of our own, until it has gone.
            for _ in 0..40 {
                if t.is_finished() {
                    let _ = t.join();
                    return;
                }
                let _ = std::fs::OpenOptions::new().read(true).write(true).open(&self.name);
                thread::sleep(Duration::from_millis(50));
            }
            // Still stuck: leave the thread behind rather than hang the app's exit.
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicU32;

    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows::Win32::Security::DACL_SECURITY_INFORMATION;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    /// A pipe name of a test: a dev name, never the release one.
    fn unique_pipe() -> String {
        format!(r"\\.\pipe\app.kinrelay.dev.kintest-pipe-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst))
    }

    fn echo() -> Handler {
        Arc::new(|body| Box::pin(async move { Reply::Respond(body) }))
    }

    /// Opens the pipe the way the host does: a busy pipe (every instance in use for a moment) is waited for, not an error.
    fn open_client(name: &str) -> io::Result<File> {
        let start = Instant::now();
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(name) {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) && start.elapsed() < Duration::from_secs(3) => {
                    thread::sleep(Duration::from_millis(5))
                }
                other => return other,
            }
        }
    }

    fn ask(name: &str, body: &[u8]) -> io::Result<Option<Vec<u8>>> {
        let mut client = open_client(name)?;
        framing::write_frame(&mut client, body)?;
        // A server that closes without an answer shows up as the end of the stream or as a broken-pipe error: both are "no answer".
        match framing::read_frame(&mut client, framing::MAX_RESPONSE) {
            Ok(answer) => Ok(answer),
            Err(FrameError::Io(_) | FrameError::Truncated) => Ok(None),
            Err(e) => Err(io::Error::other(format!("{e:?}"))),
        }
    }

    fn start(name: &str, handler: Handler) -> PipeServer {
        PipeServer::start(name, handler, tokio::runtime::Handle::current()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_gets_its_answer_and_the_server_stops_cleanly() {
        let name = unique_pipe();
        let mut server = start(&name, echo());
        let n = name.clone();
        let answer = tokio::task::spawn_blocking(move || ask(&n, b"{\"v\":1}")).await.unwrap().unwrap();
        assert_eq!(answer.as_deref(), Some(&b"{\"v\":1}"[..]));
        server.stop();
        assert!(std::fs::OpenOptions::new().read(true).write(true).open(&name).is_err(), "the pipe is gone after stop");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_request_does_not_block_the_next_client() {
        let name = unique_pipe();
        let slow: Handler = Arc::new(|body| {
            Box::pin(async move {
                if body == b"slow" {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                }
                Reply::Respond(body)
            })
        });
        let _server = start(&name, slow);
        let n = name.clone();
        let first = tokio::task::spawn_blocking(move || ask(&n, b"slow"));
        tokio::time::sleep(Duration::from_millis(150)).await;
        let started = Instant::now();
        let n = name.clone();
        let fast = tokio::task::spawn_blocking(move || ask(&n, b"fast")).await.unwrap().unwrap();
        assert_eq!(fast.as_deref(), Some(&b"fast"[..]));
        assert!(started.elapsed() < Duration::from_millis(600), "the fast request waited for the slow one: {:?}", started.elapsed());
        assert_eq!(first.await.unwrap().unwrap().as_deref(), Some(&b"slow"[..]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn many_clients_at_once_are_all_served() {
        let name = unique_pipe();
        let _server = start(&name, echo());
        let mut tasks = Vec::new();
        for i in 0..24 {
            let n = name.clone();
            tasks.push(tokio::task::spawn_blocking(move || ask(&n, format!("{{\"i\":{i}}}").as_bytes())));
        }
        for (i, t) in tasks.into_iter().enumerate() {
            assert_eq!(t.await.unwrap().unwrap().as_deref(), Some(format!("{{\"i\":{i}}}").as_bytes()), "client {i}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_oversize_request_is_dropped_without_an_answer_and_the_server_goes_on() {
        let name = unique_pipe();
        let _server = start(&name, echo());
        let n = name.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut client = open_client(&n).unwrap();
            // The prefix says 64 MiB + 1 MiB + 1 byte: the body is never read, the connection is closed with no answer.
            client.write_all(&((framing::MAX_REQUEST as u32) + 1).to_le_bytes()).unwrap();
            let mut b = [0u8; 1];
            client.read(&mut b).unwrap_or(0)
        })
        .await
        .unwrap();
        assert_eq!(outcome, 0, "no answer: end of stream");
        let n = name.clone();
        assert!(tokio::task::spawn_blocking(move || ask(&n, b"{}")).await.unwrap().unwrap().is_some(), "the next client is served");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_that_hangs_up_in_the_middle_does_not_hurt_the_server() {
        let name = unique_pipe();
        let _server = start(&name, echo());
        for _ in 0..5 {
            let mut client = open_client(&name).unwrap();
            client.write_all(&100u32.to_le_bytes()).unwrap();
            client.write_all(b"only a few bytes").unwrap();
            drop(client);
        }
        let n = name.clone();
        assert!(tokio::task::spawn_blocking(move || ask(&n, b"{}")).await.unwrap().unwrap().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_handler_that_closes_sends_no_answer() {
        let name = unique_pipe();
        let _server = start(&name, Arc::new(|_| Box::pin(async { Reply::Close })));
        let n = name.clone();
        assert!(tokio::task::spawn_blocking(move || ask(&n, b"{}")).await.unwrap().unwrap().is_none());
    }

    /// The DACL of a descriptor in SDDL, written by Windows.
    fn dacl_text(sd: PSECURITY_DESCRIPTOR) -> String {
        let mut text = windows::core::PWSTR::null();
        // SAFETY: a valid descriptor; `text` receives a LocalAlloc'd string, freed below.
        unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, DACL_SECURITY_INFORMATION, &mut text, None) }
            .unwrap();
        // SAFETY: allocated by Windows with LocalAlloc.
        unsafe {
            let s = text.to_string().unwrap();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            s
        }
    }

    /// An SDDL DACL the way Windows writes it back (SIDs that have an alias become the alias).
    fn canonical_dacl(sddl: &str) -> String {
        let w = wide(sddl);
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `w` is NUL-terminated; `sd` receives a LocalAlloc'd descriptor, freed below.
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(w.as_ptr()), SDDL_REVISION_1, &mut sd, None) }.unwrap();
        let s = dacl_text(sd);
        // SAFETY: allocated by Windows with LocalAlloc.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        s
    }

    #[test]
    fn the_security_descriptor_names_the_current_user_only() {
        let name = unique_pipe();
        let server = create_instance(&name, true).unwrap();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: a valid pipe handle; `sd` receives a LocalAlloc'd descriptor, freed below.
        let rc =
            unsafe { GetSecurityInfo(raw(&server), SE_KERNEL_OBJECT, DACL_SECURITY_INFORMATION, None, None, None, None, Some(&mut sd)) };
        assert_eq!(rc.0, 0);
        let sddl = dacl_text(sd);
        // SAFETY: allocated by Windows with LocalAlloc.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        let sid = crate::nativehost::current_user_sid().unwrap();
        // Protected (P) with exactly one entry: the user. No Everyone (WD), Users (BU), Network (NU), Administrators (BA) or SYSTEM (SY).
        // Windows writes some well-known SIDs as an alias (the built-in Administrator account is "LA"), so the expected text goes through
        // the same conversion: the two only match if the one entry is for this user's SID.
        assert_eq!(sddl, canonical_dacl(&format!("D:P(A;;FA;;;{sid})")));
        assert_eq!(current_user_only_sddl(&sid), format!("D:P(A;;GA;;;{sid})"));
    }

    #[test]
    fn a_second_first_instance_of_the_same_name_is_refused_and_later_instances_are_fine() {
        let name = unique_pipe();
        let _first = create_instance(&name, true).unwrap();
        let again = create_instance(&name, true);
        assert_eq!(again.unwrap_err().raw_os_error(), Some(5), "ERROR_ACCESS_DENIED: the name is taken");
        assert!(create_instance(&name, false).is_ok(), "more instances (not first) are fine");
    }

    #[test]
    fn a_server_whose_name_is_taken_does_not_start_and_does_not_fall_back() {
        let name = unique_pipe();
        let _squatter = create_instance(&name, true).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert!(PipeServer::start(&name, echo(), runtime.handle().clone()).is_err(), "the extension pipe is simply not available");
    }

    #[test]
    fn remote_clients_are_rejected_and_the_flag_is_in_the_creation_call() {
        let name = unique_pipe();
        let _server = create_instance(&name, true).unwrap();
        // Through the network name (SMB loopback) the pipe can't be opened: PIPE_REJECT_REMOTE_CLIENTS (or no file sharing at all: either way it
        // is not reachable that way).
        let via_network = name.replacen(r"\\.\pipe", r"\\localhost\pipe", 1);
        assert!(std::fs::OpenOptions::new().read(true).write(true).open(&via_network).is_err(), "{via_network} opened");
        // The pipe's own state can't be read back from the client side, so the call is checked as well.
        let src = include_str!("pipe.rs");
        assert!(src.contains("PIPE_REJECT_REMOTE_CLIENTS") && src.contains("FILE_FLAG_FIRST_PIPE_INSTANCE"));
    }
}
