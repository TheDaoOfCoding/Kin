//! **Only the installed Kin may read and control the user's real media sessions and write the PC's real master volume** (design 6.1, the pattern of
//! `registry::may_register_release`). Every other instance (a copy beside the installed one, a test, a debug build, a build folder) runs in **test mode**:
//!
//! * it reads and controls **only** the one session whose `SourceAppUserModelId` is the one given with `--dev-media-aumid`, and that has to be a test session of
//!   Kin's own making ([`TEST_AUMID_PREFIX`]); with the option missing it reads no session at all. The AUMID is compared **before** any property of a session is
//!   asked for (see `reader::Guarded`), so a real session's title, thumbnail and timeline are never even requested;
//! * the master volume is a **fake** (a number in memory); the real `Set*` calls are reachable only with `--dev-media-real-volume`, which no automated test or
//!   agent ever passes (only the person's own manual test does);
//! * the flyout never shows `Playing` (a `Playing` Windows session takes the keyboard's media keys), so `--dev-media-quiet` is implied.
//!
//! Everything that decides is a pure function, and the tests below pin it.
use std::path::Path;

use crate::cli::Config;

/// A test session of Kin's own making has an AUMID that starts like this (`examples/media_test_session.rs`: `app.kinrelay.dev.mediatest-<pid>`). A dev
/// instance refuses any other `--dev-media-aumid`, so it can never be pointed at a real app's session.
pub const TEST_AUMID_PREFIX: &str = "app.kinrelay.dev.mediatest-";

/// A dev Kin's own flyout session is `<identifier>.dev-<key>` (`instance::app_user_model_id`); the installed Kin's is `app.kinrelay.desktop` with no suffix.
pub const DEV_KIN_AUMID_PREFIX: &str = "app.kinrelay.desktop.dev-";

/// The one rule of the developer tool `examples/media_test_session.rs` (`read`, `control`): it may look at a test session of Kin's own making
/// (`app.kinrelay.dev.mediatest-<n>`) or a dev Kin's flyout session (`app.kinrelay.desktop.dev-<key>`), each with a non-empty suffix. **Never** the installed
/// Kin's `app.kinrelay.desktop` (it shows the user's real phone, and a command would skip their track), and nothing else.
pub fn tool_may_look_at(aumid: &str) -> bool {
    [TEST_AUMID_PREFIX, DEV_KIN_AUMID_PREFIX].iter().any(|p| aumid.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The installed Kin, run the way a user runs it.
    Real,
    /// Everything else.
    Test {
        /// The one session it may read and control (already checked against [`TEST_AUMID_PREFIX`]).
        aumid: Option<String>,
        /// `--dev-media-real-volume`: the real endpoint is used after all.
        real_volume: bool,
    },
}

/// Whether a run is the installed Kin's: a release build, no developer setting, no data directory of its own, not from a build folder. (The same rule as
/// `Config::autostart_is_real` plus the build folder, and nothing about media flags can make it true.)
pub fn is_installed_run(autostart_is_real: bool, in_build_folder: bool) -> bool {
    autostart_is_real && !in_build_folder
}

impl Mode {
    pub fn for_config(config: &Config, exe: &Path) -> Mode {
        if is_installed_run(config.autostart_is_real(), crate::integration::is_build_folder(exe)) {
            return Mode::Real;
        }
        Mode::Test {
            aumid: config.dev_media_aumid.clone().filter(|a| a.starts_with(TEST_AUMID_PREFIX) && a.len() > TEST_AUMID_PREFIX.len()),
            real_volume: config.dev_media_real_volume,
        }
    }

    pub fn is_test(&self) -> bool {
        matches!(self, Mode::Test { .. })
    }

    /// Whether a session with this `SourceAppUserModelId` may be read, listened to or controlled at all. `own` is this process's own AUMID: Kin's own session
    /// (the phone's media shown on the PC) is never read back, in any mode.
    pub fn may_use_session(&self, session_aumid: &str, own: Option<&str>) -> bool {
        if own == Some(session_aumid) {
            return false;
        }
        match self {
            Mode::Real => true,
            Mode::Test { aumid, .. } => aumid.as_deref() == Some(session_aumid),
        }
    }

    /// Whether the real master volume may be written.
    pub fn real_volume(&self) -> bool {
        match self {
            Mode::Real => true,
            Mode::Test { real_volume, .. } => *real_volume,
        }
    }

    /// Whether the flyout must never show `Playing`.
    pub fn never_playing(&self) -> bool {
        self.is_test()
    }

    /// One word for the log and `status.json`.
    pub fn name(&self) -> &'static str {
        if self.is_test() { "test" } else { "installed" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const OWN: &str = "app.kinrelay.desktop.dev-abc123";

    fn test_mode(aumid: Option<&str>, real_volume: bool) -> Mode {
        Mode::Test { aumid: aumid.map(str::to_string), real_volume }
    }

    #[test]
    fn the_developer_tool_may_look_only_at_a_test_session_or_a_dev_kin_never_the_installed_one() {
        for ok in ["app.kinrelay.dev.mediatest-4242", "app.kinrelay.desktop.dev-0123abcd", "app.kinrelay.desktop.dev-x"] {
            assert!(tool_may_look_at(ok), "{ok}");
        }
        for refused in [
            "app.kinrelay.desktop",
            "app.kinrelay.desktop.",
            "app.kinrelay.desktop.phone",
            "app.kinrelay.desktop.dev-",
            "app.kinrelay.dev.mediatest-",
            "app.kinrelay.dev",
            "app.kinrelay.",
            "app.kinrelay.other",
            "app.kinrelay.desktopx.dev-1",
            "App.KinRelay.desktop.dev-1",
            "MSEdge",
            "Spotify.exe",
            "",
        ] {
            assert!(!tool_may_look_at(refused), "{refused:?}");
        }
    }

    #[test]
    fn only_a_release_run_outside_a_build_folder_with_no_developer_setting_is_the_installed_kin() {
        assert!(is_installed_run(true, false));
        assert!(!is_installed_run(true, true), "a build folder");
        assert!(!is_installed_run(false, false), "a debug build, a developer setting or a data directory of its own");
        assert!(!is_installed_run(false, true));
    }

    #[test]
    fn every_config_of_a_test_or_a_dev_instance_is_test_mode() {
        let exe = PathBuf::from(r"C:\Program Files\Kin\Kin.exe");
        // `cargo test` is a debug build: `autostart_is_real` is false whatever the config says.
        let plain = Config::new(PathBuf::from(r"C:\x"));
        assert_eq!(Mode::for_config(&plain, &exe), test_mode(None, false));
        let mut dev = Config::new(PathBuf::from(r"C:\x"));
        dev.data_dir_overridden = true;
        dev.dev_media_aumid = Some(format!("{TEST_AUMID_PREFIX}42"));
        assert_eq!(Mode::for_config(&dev, &exe), test_mode(Some(&format!("{TEST_AUMID_PREFIX}42")), false));
        dev.dev_media_real_volume = true;
        assert!(Mode::for_config(&dev, &exe).real_volume(), "only the flag opens the real volume");
        assert!(!Mode::for_config(&plain, &exe).real_volume());
    }

    #[test]
    fn a_dev_instance_can_only_be_pointed_at_a_test_session_of_kins_own_making() {
        let exe = PathBuf::from(r"C:\Program Files\Kin\Kin.exe");
        for bad in
            ["MSEdge", "Spotify.exe", "chrome.exe", "app.kinrelay.desktop", "app.kinrelay.dev.mediatest-", "", "app.kinrelay.dev.other-1"]
        {
            let mut c = Config::new(PathBuf::from(r"C:\x"));
            c.dev_media_aumid = Some(bad.to_string());
            assert_eq!(Mode::for_config(&c, &exe), test_mode(None, false), "{bad:?} is refused");
        }
    }

    #[test]
    fn a_test_run_may_use_only_its_own_test_session_and_never_a_real_one() {
        let mode = test_mode(Some("app.kinrelay.dev.mediatest-7"), false);
        assert!(mode.may_use_session("app.kinrelay.dev.mediatest-7", Some(OWN)));
        for real in [
            "MSEdge",
            "Spotify.exe",
            "chrome.exe",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            "app.kinrelay.desktop",
            "app.kinrelay.dev.mediatest-8",
        ] {
            assert!(!mode.may_use_session(real, Some(OWN)), "{real}");
        }
        // With no test session named it uses nothing at all.
        let none = test_mode(None, false);
        for any in ["MSEdge", "app.kinrelay.dev.mediatest-7", ""] {
            assert!(!none.may_use_session(any, None), "{any:?}");
        }
    }

    #[test]
    fn kins_own_session_is_never_used_in_any_mode() {
        assert!(!Mode::Real.may_use_session(OWN, Some(OWN)));
        assert!(Mode::Real.may_use_session("MSEdge", Some(OWN)));
        assert!(!test_mode(Some(OWN), false).may_use_session(OWN, Some(OWN)), "even a test run that names its own AUMID");
    }

    #[test]
    fn the_real_volume_and_a_playing_flyout_belong_to_the_installed_kin() {
        assert!(Mode::Real.real_volume() && !Mode::Real.never_playing());
        assert!(!test_mode(None, false).real_volume() && test_mode(None, false).never_playing());
        assert!(!test_mode(Some("app.kinrelay.dev.mediatest-7"), false).real_volume());
        assert!(test_mode(None, true).real_volume(), "the flag no agent passes");
        assert!(test_mode(None, true).never_playing(), "and even then the flyout never shows Playing");
    }
}
