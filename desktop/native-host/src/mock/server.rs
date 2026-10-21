//! A named pipe server that follows the requirements for Kin's pipe (docs/extension.md "Pipe"):
//!  - a protected DACL that names only the current user (no Everyone, Users, Network or SYSTEM entry),
//!  - PIPE_REJECT_REMOTE_CLIENTS (no SMB access),
//!  - FILE_FLAG_FIRST_PIPE_INSTANCE on the first instance (nobody can have made the name before),
//!  - several instances, and a new one made as soon as a client connects, so a second host is never refused,
//!  - byte mode, one request per connection: read a frame, answer with a frame, flush, disconnect.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, FALSE, HANDLE, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::framing::{self, FrameError, MAX_PIPE_MESSAGE};
use crate::winutil::{self, wide};

/// What a handler does with a request.
pub enum Reply {
    /// Answer with these bytes.
    Respond(Vec<u8>),
    /// Close the connection without an answer (Kin crashed).
    Close,
}

pub type Handler = Arc<dyn Fn(Vec<u8>) -> Reply + Send + Sync>;

pub const UNLIMITED: u32 = 255;

/// Creates one instance of the pipe. This is what Kin does, once with `first` (and keeps going without the extension pipe if that fails: someone
/// else has the name), then once more for every client that connects.
pub fn create_instance(name: &str, first: bool, max_instances: u32) -> io::Result<File> {
    let sid = winutil::current_user_sid_string()?;
    // D:P = protected DACL (no inheritance); one allow entry: Generic All for the current user's SID. Nobody else, not even Administrators.
    let sddl = wide(OsStr::new(&format!("D:P(A;;GA;;;{sid})")));
    let mut sd = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; `sd` receives a LocalAlloc'd descriptor that is freed below.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, ptr::null_mut()) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: FALSE };
    let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    let name_w = wide(OsStr::new(name));
    // SAFETY: `name_w` is NUL-terminated; `sa` and the descriptor it points to are alive for the call.
    let h = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            max_instances,
            64 * 1024,
            64 * 1024,
            0,
            &sa,
        )
    };
    let err = io::Error::last_os_error();
    // SAFETY: allocated by ConvertStringSecurityDescriptor...; the pipe keeps its own copy.
    unsafe { LocalFree(sd) };
    if h == INVALID_HANDLE_VALUE {
        return Err(err);
    }
    // SAFETY: a valid pipe handle that we own.
    Ok(unsafe { File::from_raw_handle(h as RawHandle) })
}

fn connect_client(pipe: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    // SAFETY: a valid server-side pipe handle, no OVERLAPPED: blocks until a client connects.
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle() as HANDLE, ptr::null_mut()) } != FALSE {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    // The client connected between CreateNamedPipe and ConnectNamedPipe: that's a connection.
    if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) { Ok(()) } else { Err(e) }
}

fn serve_connection(mut pipe: File, handler: &Handler) {
    use std::os::windows::io::AsRawHandle;
    let h = pipe.as_raw_handle() as HANDLE;
    match framing::read_frame(&mut pipe, MAX_PIPE_MESSAGE, 0) {
        Ok(Some(request)) => {
            if let Reply::Respond(answer) = handler(request)
                && framing::write_frame(&mut pipe, &answer).is_ok()
            {
                // Without this the answer can be thrown away when the handle closes, before the client has read it.
                // SAFETY: valid handle; blocks until the client has read everything or has gone.
                unsafe { FlushFileBuffers(h) };
            }
        }
        Ok(None) | Err(FrameError::Truncated | FrameError::TooLarge(_) | FrameError::Io(_)) => {}
    }
    // SAFETY: valid handle.
    unsafe { DisconnectNamedPipe(h) };
}

pub struct Server {
    pub name: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts serving on `name` (the first instance is made before this returns, so the pipe exists). Fails if the name is already taken.
    pub fn start(name: &str, handler: Handler) -> io::Result<Server> {
        Server::start_with(name, handler, UNLIMITED)
    }

    pub fn start_with(name: &str, handler: Handler, max_instances: u32) -> io::Result<Server> {
        let first = create_instance(name, true, max_instances)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, name2) = (stop.clone(), name.to_string());
        let thread = thread::Builder::new().name("mock-pipe-accept".into()).spawn(move || {
            let mut instance = first;
            loop {
                if connect_client(&instance).is_err() {
                    return;
                }
                if stop2.load(Ordering::SeqCst) {
                    return;
                }
                // The next instance first, then the work: there is always a pipe to connect to. (With a limit of instances that is already
                // used up this isn't possible: the work starts first, and the next instance is made as soon as one is free.)
                let mut next = create_instance(&name2, false, max_instances).ok();
                let (conn, handler) = (instance, handler.clone());
                thread::spawn(move || serve_connection(conn, &handler));
                instance = loop {
                    if let Some(f) = next.take() {
                        break f;
                    }
                    match create_instance(&name2, false, max_instances) {
                        Ok(f) => break f,
                        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => thread::sleep(Duration::from_millis(10)),
                        Err(_) => return,
                    }
                };
            }
        })?;
        Ok(Server { name: name.to_string(), stop, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.stop.store(true, Ordering::SeqCst);
            // Wake the accept loop with a connection of our own (again, until it has gone: it may be waiting for a free instance).
            for _ in 0..40 {
                if t.is_finished() {
                    let _ = t.join();
                    return;
                }
                let _ = std::fs::OpenOptions::new().read(true).write(true).open(&self.name);
                thread::sleep(Duration::from_millis(50));
            }
            // Still stuck: leave the thread behind rather than hang a test.
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
