//! The browser extension's way into Kin (milestone 2B.1; docs/extension.md is the contract): the extension talks to `kin-native-host.exe`, which
//! the browser starts, and the host relays each request over a named pipe to **this** program.
//!
//! * `pipe.rs`: the pipe server (current user only, no remote clients, the first instance can't have been squatted).
//! * `api.rs`: what a request means: `status`, `start`, `open` and `send` of a text, a link, a page or an image, with every limit of the contract
//!   checked again here, because any program of the user can write to the pipe.
//! * `imagefetch.rs`: Kin downloads an image URL itself (no cookies, 64 MiB, 30 s).
//! * `registry.rs`: the host manifests and the `HKCU` keys that tell the browsers where they are.
//!
//! **Which host name this Kin uses.** The installed Kin (a release build that runs the way a user runs it: [`HostKind::for_config`]) is
//! `app.kinrelay.desktop`. **Every other Kin** (a debug build, any developer setting, a `--data-dir` of its own, a test) is `app.kinrelay.dev`: its
//! pipe is the dev pipe, so a test can never talk to the user's installed Kin, and it never registers or overwrites the release keys.
use std::io;

pub mod api;
pub mod framing;
pub mod imagefetch;
pub mod pipe;
pub mod registry;

use crate::cli::Config;

pub const RELEASE_HOST_NAME: &str = "app.kinrelay.desktop";
pub const DEV_HOST_NAME: &str = "app.kinrelay.dev";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    /// The installed Kin: `app.kinrelay.desktop`.
    Release,
    /// Everything else: `app.kinrelay.dev`.
    Dev,
}

impl HostKind {
    /// Release only for a Kin that autostarts for real (`Config::autostart_is_real`: a release build with no developer setting, no data directory
    /// of its own and no instance key). Anything else is a dev Kin.
    pub fn for_config(config: &Config) -> HostKind {
        if config.autostart_is_real() { HostKind::Release } else { HostKind::Dev }
    }

    pub fn host_name(self) -> &'static str {
        match self {
            HostKind::Release => RELEASE_HOST_NAME,
            HostKind::Dev => DEV_HOST_NAME,
        }
    }

    /// `\\.\pipe\<host name>.<user SID>` (docs/extension.md, "Names"). A dev Kin may add `.<suffix>` of the environment variable
    /// `KIN_DEV_PIPE_SUFFIX` so that two dev instances can listen at once (the dev host build reads the whole name from `KIN_HOST_DEV_PIPE`);
    /// the release pipe has no such thing.
    pub fn pipe_name(self, sid: &str, dev_suffix: Option<&str>) -> String {
        let suffix = match (self, dev_suffix) {
            (HostKind::Dev, Some(s)) if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => {
                format!(".{s}")
            }
            _ => String::new(),
        };
        format!(r"\\.\pipe\{}.{sid}{suffix}", self.host_name())
    }
}

/// The SID of the user this program runs as, as `S-1-5-21-...`.
pub fn current_user_sid() -> io::Result<String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: the token handle is closed on every path; the buffer is as big as `GetTokenInformation` asked for and holds a `TOKEN_USER` whose
    // SID pointer points into the same buffer; the string that `ConvertSidToStringSidW` allocates is freed with `LocalFree`.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(io::Error::other)?;
        let result = (|| {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut buffer = vec![0u8; needed as usize];
            GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr().cast()), needed, &mut needed).map_err(io::Error::other)?;
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            let mut text = windows::core::PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(io::Error::other)?;
            let sid = text.to_string().map_err(io::Error::other);
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sid
        })();
        let _ = CloseHandle(token);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_and_the_dev_pipe_have_their_own_names() {
        let sid = "S-1-5-21-1-2-3-1001";
        assert_eq!(HostKind::Release.pipe_name(sid, None), r"\\.\pipe\app.kinrelay.desktop.S-1-5-21-1-2-3-1001");
        assert_eq!(HostKind::Dev.pipe_name(sid, None), r"\\.\pipe\app.kinrelay.dev.S-1-5-21-1-2-3-1001");
        assert_eq!(HostKind::Dev.pipe_name(sid, Some("e2e-1")), r"\\.\pipe\app.kinrelay.dev.S-1-5-21-1-2-3-1001.e2e-1");
        // The release pipe takes no suffix, and a suffix with odd characters is ignored.
        assert_eq!(HostKind::Release.pipe_name(sid, Some("x")), HostKind::Release.pipe_name(sid, None));
        assert_eq!(HostKind::Dev.pipe_name(sid, Some("a\\..\\b")), HostKind::Dev.pipe_name(sid, None));
        assert!(!HostKind::Dev.pipe_name(sid, Some("x")).contains("desktop"));
    }

    #[test]
    fn only_a_kin_that_autostarts_for_real_is_the_release_host() {
        let config = Config::new(std::path::PathBuf::from(r"C:\Users\someone\AppData\Roaming\Kin"));
        // `cargo test` is a debug build: never the release host, whatever else is true. A release build with this plain config would be.
        assert_eq!(HostKind::for_config(&config), if cfg!(debug_assertions) { HostKind::Dev } else { HostKind::Release });
        let mut dev = config.clone();
        dev.dev_bind = Some(std::net::Ipv4Addr::LOCALHOST.into());
        assert_eq!(HostKind::for_config(&dev), HostKind::Dev);
        let mut own_dir = config.clone();
        own_dir.data_dir_overridden = true;
        assert_eq!(HostKind::for_config(&own_dir), HostKind::Dev);
    }

    #[test]
    fn the_user_sid_looks_like_a_sid() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
    }
}
