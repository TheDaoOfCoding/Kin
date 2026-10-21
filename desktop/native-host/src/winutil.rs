//! The Win32 calls of the host, in one place: the user's SID, who owns a pipe server, cancelling a blocked pipe call, starting a process.
//! Every `unsafe` block says what it relies on.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, FALSE, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{EqualSid, GetTokenInformation, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CreateMutexW, CreateProcessW, DETACHED_PROCESS,
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    ReleaseMutex, STARTUPINFOW, WaitForSingleObject,
};

pub fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// A user's SID, kept in a buffer that holds the TOKEN_USER (the SID pointer inside points into the same buffer).
pub struct UserSid {
    buf: Vec<u64>, // u64 for the alignment TOKEN_USER needs
}

impl UserSid {
    fn from_token(token: HANDLE) -> io::Result<UserSid> {
        let mut needed = 0u32;
        // SAFETY: a size query: a null buffer of length 0 is allowed and fills `needed`; the call is expected to fail with INSUFFICIENT_BUFFER.
        unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buf` is at least `needed` bytes and 8-byte aligned.
        let ok = unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), needed, &mut needed) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(UserSid { buf })
    }

    fn psid(&self) -> PSID {
        // SAFETY: the buffer holds a TOKEN_USER written by GetTokenInformation; the SID it points to lives in the same buffer.
        unsafe { (*self.buf.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    /// The current process's user.
    pub fn current() -> io::Result<UserSid> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess is a pseudo handle; `token` receives a handle that OwnedHandle closes.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken just gave us this handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token as RawHandle) };
        UserSid::from_token(token.as_raw_handle() as HANDLE)
    }

    pub fn equals(&self, other: &UserSid) -> bool {
        // SAFETY: both are valid SIDs inside live buffers.
        unsafe { EqualSid(self.psid(), other.psid()) != FALSE }
    }

    /// The `S-1-5-21-...` text form, as used in the pipe name and in SDDL.
    pub fn to_string_sid(&self) -> io::Result<String> {
        let mut p: *mut u16 = ptr::null_mut();
        // SAFETY: a valid SID; `p` receives a LocalAlloc'd, NUL-terminated string that is freed below.
        let ok = unsafe { ConvertSidToStringSidW(self.psid(), &mut p) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `p` is NUL-terminated (checked by walking to the NUL); freed once.
        let s = unsafe {
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            LocalFree(p.cast());
            s
        };
        Ok(s)
    }
}

/// The current user's SID as text.
pub fn current_user_sid_string() -> io::Result<String> {
    UserSid::current()?.to_string_sid()
}

/// Who runs the process at the other end of a pipe handle (as a client): its user and its executable's full path.
pub struct ServerProcess {
    pub pid: u32,
    pub user: UserSid,
    pub image: PathBuf,
}

pub fn server_process(pipe: &File) -> io::Result<ServerProcess> {
    let mut pid = 0u32;
    // SAFETY: `pipe` is a live pipe handle opened by us as a client.
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: plain call; the handle is closed by OwnedHandle.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if h.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcess just gave us this handle.
    let h = unsafe { OwnedHandle::from_raw_handle(h as RawHandle) };
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: valid process handle; the token handle is closed by OwnedHandle.
    if unsafe { OpenProcessToken(h.as_raw_handle() as HANDLE, TOKEN_QUERY, &mut token) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken just gave us this handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token as RawHandle) };
    let user = UserSid::from_token(token.as_raw_handle() as HANDLE)?;

    let mut buf = vec![0u16; 32 * 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` has `len` u16s; `len` is updated to the number written (without the NUL).
    if unsafe { QueryFullProcessImageNameW(h.as_raw_handle() as HANDLE, 0, buf.as_mut_ptr(), &mut len) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    let image = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    Ok(ServerProcess { pid, user, image })
}

/// Compares two paths the way Windows does: long names, no `\\?\` prefix, case-insensitive. A path that doesn't exist is compared as written.
pub fn same_path(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = c.to_string_lossy().replace('/', "\\");
        s.strip_prefix(r"\\?\").unwrap_or(&s).to_lowercase()
    }
    norm(a) == norm(b)
}

/// Waits until a pipe instance is free (ERROR_PIPE_BUSY on open). `Ok(true)` when one is, `Ok(false)` on timeout; the pipe's absence is an error.
pub fn wait_named_pipe(name: &str, ms: u32) -> io::Result<bool> {
    let w = wide(OsStr::new(name));
    // SAFETY: `w` is NUL-terminated and outlives the call. 0 would mean "the default timeout", so ask for at least 1.
    if unsafe { WaitNamedPipeW(w.as_ptr(), ms.max(1)) } != FALSE {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_SEM_TIMEOUT as i32) { Ok(false) } else { Err(e) }
}

/// Cancels a synchronous call (read or write) that another thread is blocked in on this handle. Safe to call when nothing is pending.
pub fn cancel_io(handle: usize) {
    // SAFETY: the caller keeps the handle open while it calls this (see pipe::exchange); a null OVERLAPPED cancels all of the handle's I/O.
    unsafe { CancelIoEx(handle as HANDLE, ptr::null()) };
}

/// Starts a program detached: no console, no window, not in the browser's job (so the browser closing doesn't kill it), and **none of our
/// handles inherited**. That last point matters: the browser's stdin/stdout pipes are inheritable, and a Kin that held them open would stop
/// the browser from ever seeing the host exit.
pub fn start_detached(exe: &Path, args: &[String]) -> io::Result<()> {
    let mut cmd = String::new();
    cmd.push('"');
    cmd.push_str(&exe.to_string_lossy());
    cmd.push('"');
    for a in args {
        cmd.push(' ');
        cmd.push('"');
        cmd.push_str(&a.replace('"', "\\\""));
        cmd.push('"');
    }
    let exe_w = wide(exe.as_os_str());
    let dir_w = exe.parent().map(|d| wide(d.as_os_str()));
    let base = DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
    let mut last = None;
    for flags in [base | CREATE_BREAKAWAY_FROM_JOB, base] {
        let mut cmd_w = wide(OsStr::new(&cmd)); // CreateProcessW may write into the command line
        // SAFETY: all-zero is a valid STARTUPINFOW once `cb` is set.
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = size_of::<STARTUPINFOW>() as u32;
        // SAFETY: all-zero is a valid PROCESS_INFORMATION (an output).
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: every pointer is NUL-terminated (or null) and outlives the call; bInheritHandles is FALSE.
        let ok = unsafe {
            CreateProcessW(
                exe_w.as_ptr(),
                cmd_w.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                FALSE,
                flags,
                ptr::null(),
                dir_w.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
                &si,
                &mut pi,
            )
        };
        if ok != FALSE {
            // SAFETY: both handles are ours from CreateProcessW; we don't need them.
            drop(unsafe { OwnedHandle::from_raw_handle(pi.hProcess as RawHandle) });
            drop(unsafe { OwnedHandle::from_raw_handle(pi.hThread as RawHandle) });
            return Ok(());
        }
        let e = io::Error::last_os_error();
        // A job that doesn't allow breakaway refuses the flag with "access denied": try again without it.
        let retry = e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
        last = Some(e);
        if !retry {
            break;
        }
    }
    Err(last.unwrap())
}

/// A per-user, per-session lock, so that two hosts started together (a burst of clicks right after sign-in) start Kin once.
pub struct NamedLock(OwnedHandle);

impl NamedLock {
    /// Waits up to `ms` for the lock. `None` when it wasn't got in time (or the mutex couldn't be made: then there is no lock, and the caller goes on).
    pub fn acquire(name: &str, ms: u32) -> Option<NamedLock> {
        let w = wide(OsStr::new(&format!("Local\\{name}")));
        // SAFETY: `w` is NUL-terminated; default security (the current user).
        let h = unsafe { CreateMutexW(ptr::null(), FALSE, w.as_ptr()) };
        if h.is_null() {
            return None;
        }
        // SAFETY: CreateMutexW just gave us this handle.
        let h = unsafe { OwnedHandle::from_raw_handle(h as RawHandle) };
        // SAFETY: a valid mutex handle. WAIT_OBJECT_0 (0) and WAIT_ABANDONED (0x80) both mean it's ours now.
        let r = unsafe { WaitForSingleObject(h.as_raw_handle() as HANDLE, ms) };
        if r == 0 || r == 0x80 { Some(NamedLock(h)) } else { None }
    }
}

impl Drop for NamedLock {
    fn drop(&mut self) {
        // SAFETY: we own the mutex, and release it on the thread that acquired it (the host is single-threaded for this).
        unsafe { ReleaseMutex(self.0.as_raw_handle() as HANDLE) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sid_text_and_equality() {
        let a = UserSid::current().unwrap();
        let b = UserSid::current().unwrap();
        assert!(a.equals(&b));
        let s = a.to_string_sid().unwrap();
        assert!(s.starts_with("S-1-5-"), "{s}");
        assert_eq!(current_user_sid_string().unwrap(), s);
    }

    #[test]
    fn same_path_ignores_case_and_verbatim_prefix() {
        let exe = std::env::current_exe().unwrap();
        let shout = PathBuf::from(exe.to_string_lossy().to_uppercase());
        assert!(same_path(&exe, &shout));
        assert!(same_path(&exe, &std::fs::canonicalize(&exe).unwrap()));
        assert!(!same_path(&exe, Path::new(r"C:\definitely\not\this.exe")));
    }

    #[test]
    fn lock_is_exclusive_across_threads() {
        let name = format!("kin-host-test-lock-{}", std::process::id());
        let first = NamedLock::acquire(&name, 100).expect("first acquire");
        // A mutex is re-entrant for the owning thread, so the second acquire comes from another thread.
        let n2 = name.clone();
        let got = std::thread::spawn(move || NamedLock::acquire(&n2, 200).is_some()).join().unwrap();
        assert!(!got, "a second thread must not get it while the first holds it");
        drop(first);
        let n3 = name.clone();
        assert!(std::thread::spawn(move || NamedLock::acquire(&n3, 1000).is_some()).join().unwrap());
    }
}
