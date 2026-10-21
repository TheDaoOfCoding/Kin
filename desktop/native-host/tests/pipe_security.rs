// The security properties of the pipe, checked against a real server (the mock's `create_instance` is the reference for Kin's own pipe).
mod common;

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::ptr;

use common::unique_pipe;
use kin_native_host::mock::server::{UNLIMITED, create_instance};
use kin_native_host::pipe;
use kin_native_host::winutil::wide;
use windows_sys::Win32::Foundation::{FALSE, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetTokenInformation, PSECURITY_DESCRIPTOR, RevertToSelf, TokenImpersonationLevel,
};
use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, ImpersonateNamedPipeClient};
use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

const TOKEN_QUERY: u32 = 0x0008;

/// The impersonation level the server gets when it impersonates the client that connected: 0 anonymous, 1 identification, 2 impersonation.
fn level_seen_by_server(open_client: impl FnOnce(&str) -> std::fs::File + Send + 'static) -> u32 {
    let name = unique_pipe();
    let server = create_instance(&name, true, UNLIMITED).unwrap();
    let n2 = name.clone();
    let client = std::thread::spawn(move || {
        let mut f = open_client(&n2);
        f.write_all(b"x").unwrap();
        let mut b = [0u8; 1];
        let _ = f.read(&mut b); // wait for the server to be done
    });
    // SAFETY: valid server handle; blocks until the client is there.
    unsafe { ConnectNamedPipe(server.as_raw_handle() as HANDLE, ptr::null_mut()) };
    let mut b = [0u8; 1];
    (&server).read_exact(&mut b).unwrap();
    // SAFETY: the pipe is connected, we are the server's thread; the impersonation is reverted below.
    assert_ne!(unsafe { ImpersonateNamedPipeClient(server.as_raw_handle() as HANDLE) }, FALSE);
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: opens the impersonation token of this thread.
    assert_ne!(unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) }, FALSE);
    let mut level = 99u32;
    let mut n = 0u32;
    // SAFETY: `level` is a u32, which is what TokenImpersonationLevel returns.
    assert_ne!(unsafe { GetTokenInformation(token, TokenImpersonationLevel, (&mut level as *mut u32).cast(), 4, &mut n) }, FALSE);
    // SAFETY: ends the impersonation on this thread.
    unsafe { RevertToSelf() };
    (&server).write_all(b"y").unwrap();
    client.join().unwrap();
    level
}

#[test]
fn the_host_connects_with_identification_only_so_a_server_cannot_act_as_the_user() {
    let host_level = level_seen_by_server(|name| match pipe::open_once(name) {
        Ok(f) => f,
        Err(e) => panic!("{e:?}"),
    });
    assert_eq!(host_level, 1, "SecurityIdentification");
    // The control: a plain open (what the host must not do) gives the server the right to impersonate.
    let plain_level = level_seen_by_server(|name| OpenOptions::new().read(true).write(true).open(name).unwrap());
    assert_eq!(plain_level, 2, "SecurityImpersonation: this is what the QOS flag prevents");
}

#[test]
fn the_reference_server_has_a_dacl_for_the_current_user_only() {
    let name = unique_pipe();
    let server = create_instance(&name, true, UNLIMITED).unwrap();
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: a valid pipe handle; `sd` receives a LocalAlloc'd descriptor, freed below.
    let rc = unsafe {
        GetSecurityInfo(
            server.as_raw_handle() as HANDLE,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut sd,
        )
    };
    assert_eq!(rc, 0);
    let s = dacl_text(sd);
    // SAFETY: allocated by Windows with LocalAlloc.
    unsafe { LocalFree(sd) };
    let sid = kin_native_host::winutil::current_user_sid_string().unwrap();
    // Protected (P), and exactly one entry: the user. No Everyone (WD), Users (BU), Network (NU), Administrators (BA), SYSTEM (SY).
    // Windows writes some well-known SIDs as an alias (the built-in Administrator account is "LA"), so the expected text goes through the same
    // conversion: the two only match if the one entry is for this user's SID.
    assert_eq!(s, canonical_dacl(&format!("D:P(A;;FA;;;{sid})")), "the DACL must be exactly this");
}

/// The DACL of a descriptor in SDDL, written by Windows.
fn dacl_text(sd: PSECURITY_DESCRIPTOR) -> String {
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: a valid descriptor; `text` receives a LocalAlloc'd string, freed below.
    assert_ne!(
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, DACL_SECURITY_INFORMATION, &mut text, ptr::null_mut())
        },
        FALSE
    );
    // SAFETY: `text` is a NUL-terminated string allocated by Windows with LocalAlloc.
    unsafe {
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        s
    }
}

/// An SDDL DACL the way Windows writes it back (SIDs that have an alias become the alias).
fn canonical_dacl(sddl: &str) -> String {
    let w = wide(std::ffi::OsStr::new(sddl));
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `w` is NUL-terminated; `sd` receives a LocalAlloc'd descriptor, freed below.
    assert_ne!(
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), SDDL_REVISION_1, &mut sd, ptr::null_mut()) },
        FALSE
    );
    let s = dacl_text(sd);
    // SAFETY: allocated by Windows with LocalAlloc.
    unsafe { LocalFree(sd) };
    s
}

#[test]
fn a_second_first_instance_of_the_same_name_is_refused() {
    // FILE_FLAG_FIRST_PIPE_INSTANCE: a program that takes the name first can't be joined, and one that comes later can't squat it.
    let name = unique_pipe();
    let _first = create_instance(&name, true, UNLIMITED).unwrap();
    let again = create_instance(&name, true, UNLIMITED);
    assert_eq!(again.unwrap_err().raw_os_error(), Some(5), "ERROR_ACCESS_DENIED: the name is taken");
    assert!(create_instance(&name, false, UNLIMITED).is_ok(), "more instances (not first) are fine");
}

#[test]
fn the_reference_server_rejects_remote_clients() {
    // Can't connect from another machine here; check the flag is in the creation call (the pipe's own PIPE_REJECT_REMOTE_CLIENTS state can't
    // be read back from the client side).
    let src = include_str!("../src/mock/server.rs");
    assert!(src.contains("PIPE_REJECT_REMOTE_CLIENTS"));
    assert!(src.contains("FILE_FLAG_FIRST_PIPE_INSTANCE"));
}
