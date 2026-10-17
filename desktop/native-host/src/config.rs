//! Where the host looks for Kin, and how long it waits. `Config::release()` is what the shipped binary uses: nothing in it can be changed
//! from the outside. A build with the `dev` feature reads a few KIN_HOST_DEV_* variables instead; the tests build a Config by hand.

use std::path::PathBuf;
use std::time::Duration;

use crate::winutil;

pub const RELEASE_HOST_NAME: &str = "app.kinrelay.desktop";
pub const DEV_HOST_NAME: &str = "app.kinrelay.dev";

#[derive(Clone, Debug)]
pub struct Config {
    /// `\\.\pipe\<host name>.<user SID>`. `None` when the user's SID couldn't be read (every request then gets INTERNAL).
    pub pipe_name: Option<String>,
    /// The name of the per-user start lock (made unique per pipe).
    pub lock_name: String,
    /// Kin.exe: started when a `send` or `start` finds no pipe, and the program the pipe server must be.
    pub kin_exe: Option<PathBuf>,
    /// The arguments Kin is started with (none in a release; tests give their stand-in some).
    pub kin_args: Vec<String>,
    /// The program the pipe server's process must be. `None` skips that check (tests, and the dev override); the server's USER is always checked.
    pub expected_server_image: Option<PathBuf>,
    /// How long `start` and `send` wait for the pipe after starting Kin.
    pub start_wait: Duration,
    /// How long a `status` waits for a busy pipe (all instances in use) before it says "not running".
    pub status_busy_wait: Duration,
    /// How long a `send` or `start` waits for a busy pipe.
    pub busy_wait: Duration,
    /// How long the whole request may take, per type. Each is a little under what the extension waits (8 s, 15 s, 90 s).
    pub status_timeout: Duration,
    pub start_timeout: Duration,
    pub send_timeout: Duration,
    pub host_version: String,
}

/// Never the release pipe, whatever KIN_HOST_DEV_PIPE says: a test must not be able to reach the user's installed Kin.
pub fn dev_pipe_allowed(name: &str) -> bool {
    let dev = format!(r"\\.\pipe\{DEV_HOST_NAME}");
    // `app.kinrelay.dev` and `app.kinrelay.dev.<anything>` only: not `app.kinrelay.developer`, and never `app.kinrelay.desktop`.
    name == dev || name.starts_with(&format!("{dev}.")) || name.starts_with(r"\\.\pipe\kin-host-test-")
}

impl Config {
    fn base(host_name: &str) -> Config {
        let pipe_name = winutil::current_user_sid_string().ok().map(|sid| format!(r"\\.\pipe\{host_name}.{sid}"));
        Config {
            lock_name: format!("{host_name}.host-start"),
            pipe_name,
            kin_exe: None,
            kin_args: Vec::new(),
            expected_server_image: None,
            start_wait: Duration::from_secs(10),
            status_busy_wait: Duration::from_secs(1),
            busy_wait: Duration::from_secs(5),
            status_timeout: Duration::from_secs(7),
            start_timeout: Duration::from_secs(14),
            send_timeout: Duration::from_secs(85),
            host_version: crate::HOST_VERSION.to_string(),
        }
    }

    /// The shipped host: the release pipe, `Kin.exe` next to the host's own exe (the only Kin it starts and the only one it talks to).
    pub fn release() -> Config {
        let mut c = Config::base(RELEASE_HOST_NAME);
        let kin = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("Kin.exe")));
        c.expected_server_image = kin.clone();
        c.kin_exe = kin;
        c
    }

    /// A developer host: the dev pipe, and these variables (all optional):
    /// `KIN_HOST_DEV_PIPE` the whole pipe name (it must start with `\\.\pipe\app.kinrelay.dev` or `\\.\pipe\kin-host-test-`),
    /// `KIN_HOST_DEV_KIN_EXE` the program to start for Kin, which is also the program the pipe server must be,
    /// `KIN_HOST_DEV_ANY_SERVER=1` skips the check of the server's program.
    #[cfg(feature = "dev")]
    pub fn dev() -> Config {
        let mut c = Config::base(DEV_HOST_NAME);
        if let Ok(p) = std::env::var("KIN_HOST_DEV_PIPE") {
            if dev_pipe_allowed(&p) {
                c.lock_name = format!("{}.host-start", p.rsplit('\\').next().unwrap_or("dev"));
                c.pipe_name = Some(p);
            }
        }
        if let Ok(p) = std::env::var("KIN_HOST_DEV_KIN_EXE") {
            let p = PathBuf::from(p);
            c.expected_server_image = Some(p.clone());
            c.kin_exe = Some(p);
        }
        if std::env::var("KIN_HOST_DEV_ANY_SERVER").as_deref() == Ok("1") {
            c.expected_server_image = None;
        }
        c
    }

    /// The configuration of this build.
    pub fn from_build() -> Config {
        #[cfg(feature = "dev")]
        {
            Config::dev()
        }
        #[cfg(not(feature = "dev"))]
        {
            Config::release()
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_override_can_never_name_the_release_pipe() {
        assert!(dev_pipe_allowed(r"\\.\pipe\app.kinrelay.dev.S-1-5-21-1"));
        assert!(dev_pipe_allowed(r"\\.\pipe\app.kinrelay.dev"));
        assert!(dev_pipe_allowed(r"\\.\pipe\kin-host-test-1-2"));
        assert!(!dev_pipe_allowed(r"\\.\pipe\app.kinrelay.desktop.S-1-5-21-1"));
        assert!(!dev_pipe_allowed(r"\\.\pipe\app.kinrelay.developer"));
        assert!(!dev_pipe_allowed(r"\\.\pipe\something-else"));
        assert!(!dev_pipe_allowed(r"\\server\pipe\app.kinrelay.dev"));
    }

    #[test]
    fn release_config_is_fixed() {
        let c = Config::release();
        let sid = winutil::current_user_sid_string().unwrap();
        assert_eq!(c.pipe_name, Some(format!(r"\\.\pipe\app.kinrelay.desktop.{sid}")));
        let kin = c.kin_exe.clone().unwrap();
        assert!(kin.ends_with("Kin.exe"));
        assert_eq!(c.expected_server_image, Some(kin));
        assert!(c.kin_args.is_empty());
    }
}
