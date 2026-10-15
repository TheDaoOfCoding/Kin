//! Milestone 1.5: what the installer leaves to the app.
//!
//! The installer (`windows/hooks.nsh`) runs **elevated** (it is per-machine, for the firewall rule), and an elevated process can be
//! another account than the person at the keyboard (a standard user who typed an administrator's password: `%APPDATA%` and `HKCU` are then
//! the administrator's). So everything that belongs to *the user* is done by the app, which always runs as that user:
//!
//! * **The Explorer "Send to" shortcut** `Kin.lnk` in the user's SendTo folder (`Kin.exe --send`).
//! * **"Start with Windows"**, on by default for a new profile (the Run value of `autostart.rs`).
//! * **The app user model ID** `app.kinrelay.desktop` of the process (see [`APP_USER_MODEL_ID`]).
//!
//! Both of the first two are **repaired at every start** instead of being done once. A reinstall over an older version runs the old
//! uninstaller first, which removes the shortcut and the Run value (it can't tell an upgrade from an uninstall), and a different install
//! folder leaves a shortcut that points at nothing. What the person *wants* is in `integration.json` in the data directory:
//!
//! ```json
//! { "startWithWindows": true, "sendTo": true }
//! ```
//!
//! * `startWithWindows` is written by the Settings window's switch. Missing means "not decided yet": a profile that was just made (a first run)
//!   gets `true` and the Run value; an older profile (a developer build of 1.4 had the switch) keeps whatever the Run value is now.
//! * `sendTo` is `true` unless the file says `false`, which also removes the shortcut. (There is no switch for it in the Settings window.)
//!
//! **Nothing here runs in a developer run** (a debug build, any `--dev-*` flag, a `--data-dir` of its own) **or from the build folder**
//! (`target\release`, `target\debug`): a script or a developer must never get a shortcut in their real SendTo folder or Kin in their real
//! Run key. The tests use temporary folders and the file stand-in of `autostart.rs`, never the real SendTo folder or the Run key.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::autostart::AutostartStore;

/// The app user model ID: the name Windows knows Kin by. The Start Menu shortcut that the installer makes carries it (the Tauri template
/// writes `System.AppUserModel.ID` = the bundle identifier of `tauri.conf.json`), and the notification plugin names it as the origin of
/// every toast when Kin is installed. With a Start Menu shortcut for the same ID, a toast shows "Kin" with the app's icon instead of
/// "Windows PowerShell". The process names it too, so the taskbar groups its windows under that shortcut. A unit test keeps this equal
/// to the identifier in `tauri.conf.json`.
pub const APP_USER_MODEL_ID: &str = "app.kinrelay.desktop";

/// The shortcut in the SendTo folder: Explorer lists it as "Kin".
pub const SENDTO_NAME: &str = "Kin.lnk";
/// What the shortcut passes to `Kin.exe`: Explorer adds the selected files after it.
pub const SENDTO_ARGUMENTS: &str = "--send";
const SENDTO_DESCRIPTION: &str = "Send the selected files to your phone with Kin Relay";

/// The file in the data directory that holds what the person wants (see the module text).
pub const FILE_NAME: &str = "integration.json";

// ----- the app user model ID -------------------------------------------------------------------------------------------------------

/// Gives this process the app user model ID. Call it first thing, before any window exists. A failure only costs the grouping of windows
/// on the taskbar (toasts name the ID on their own), so the caller logs it and carries on.
pub fn set_process_app_user_model_id() -> Result<(), String> {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::HSTRING;
    // SAFETY: the string is valid for the call, and Windows copies it.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_USER_MODEL_ID)).map_err(|e| e.to_string()) }
}

/// The app user model ID this process has now (`None` if it has none), for the tests.
pub fn process_app_user_model_id() -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID;
    // SAFETY: the string that comes back is copied and then freed with `CoTaskMemFree`, as the documentation says.
    unsafe {
        let id = GetCurrentProcessExplicitAppUserModelID().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as *const _));
        text
    }
}

// ----- shortcuts -------------------------------------------------------------------------------------------------------------------

/// What is in a `.lnk` file that matters here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub target: PathBuf,
    pub arguments: String,
}

/// Runs `work` with COM initialised on this thread (and undone afterwards).
fn with_com<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    // SAFETY: balanced with `CoUninitialize` below when it worked. A thread that already has COM in another mode answers
    // `RPC_E_CHANGED_MODE`, and the work then runs on the mode it has.
    unsafe {
        let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = work();
        if initialised {
            CoUninitialize();
        }
        result
    }
}

/// Writes (or replaces) the shortcut `path`: it starts `target` with `arguments` in the folder of `target`, with the icon of `target`.
pub fn write_shortcut(path: &Path, target: &Path, arguments: &str, description: &str) -> Result<(), String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};
    with_com(|| {
        // SAFETY: COM is initialised on this thread; the interfaces are used on it only and released when they go out of scope; every string
        // is valid for its call.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
            link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| e.to_string())?;
            link.SetArguments(&HSTRING::from(arguments)).map_err(|e| e.to_string())?;
            link.SetDescription(&HSTRING::from(description)).map_err(|e| e.to_string())?;
            if let Some(dir) = target.parent() {
                link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())).map_err(|e| e.to_string())?;
            }
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0).map_err(|e| e.to_string())?;
            let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
            file.Save(&HSTRING::from(path.as_os_str()), true).map_err(|e| e.to_string())
        }
    })
}

/// Reads the shortcut `path`. `Ok(None)` if there is no such file.
pub fn read_shortcut(path: &Path) -> Result<Option<Shortcut>, String> {
    use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};
    if !path.is_file() {
        return Ok(None);
    }
    with_com(|| {
        // SAFETY: as in `write_shortcut`; the buffers outlive the calls that fill them.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
            let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
            file.Load(&HSTRING::from(path.as_os_str()), STGM_READ).map_err(|e| e.to_string())?;
            // `SLGP_RAWPATH` (4): the path as stored, without expanding variables or looking for the target.
            let mut target = [0u16; 1024];
            link.GetPath(&mut target, std::ptr::null_mut::<WIN32_FIND_DATAW>(), 4).map_err(|e| e.to_string())?;
            let mut arguments = [0u16; 1024];
            link.GetArguments(&mut arguments).map_err(|e| e.to_string())?;
            Ok(Some(Shortcut { target: PathBuf::from(from_wide(&target)), arguments: from_wide(&arguments) }))
        }
    })
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// The user's SendTo folder, as Windows says (it can be moved), or `%APPDATA%\Microsoft\Windows\SendTo` if it can't.
pub fn send_to_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_SendTo, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: the string that comes back is copied and then freed with `CoTaskMemFree`, as the documentation says.
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_SendTo, KF_FLAG_DEFAULT, None) {
            let text = p.to_string();
            CoTaskMemFree(Some(p.0 as *const _));
            if let Ok(text) = text {
                return Some(PathBuf::from(text));
            }
        }
    }
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\SendTo"))
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

/// What `ensure_send_to` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendTo {
    /// There was none, and now there is.
    Created,
    /// There was one that started something else (an older install folder), or with other arguments, and now it starts this `Kin.exe`.
    Repaired,
    /// It was right already.
    Unchanged,
    /// The person turned it off (`"sendTo": false`) and it is gone (it was removed now, or it was not there).
    Off,
    Failed(String),
}

/// Makes the SendTo shortcut in `dir` what it should be for this `exe`: made if it is missing, rewritten if it points elsewhere, and
/// removed if `wanted` is false.
pub fn ensure_send_to(dir: &Path, exe: &Path, wanted: bool) -> SendTo {
    let path = dir.join(SENDTO_NAME);
    if !wanted {
        return match std::fs::remove_file(&path) {
            Ok(()) => SendTo::Off,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SendTo::Off,
            Err(e) => SendTo::Failed(format!("removing the shortcut failed: {e}")),
        };
    }
    let existing = read_shortcut(&path);
    let (create, repair) = match &existing {
        Ok(None) => (true, false),
        Ok(Some(s)) => (false, !(same_path(&s.target, exe) && s.arguments == SENDTO_ARGUMENTS)),
        // A shortcut that can't be read is replaced.
        Err(_) => (false, true),
    };
    if !create && !repair {
        return SendTo::Unchanged;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        return SendTo::Failed(format!("the SendTo folder can't be made: {e}"));
    }
    match write_shortcut(&path, exe, SENDTO_ARGUMENTS, SENDTO_DESCRIPTION) {
        Ok(()) if create => SendTo::Created,
        Ok(()) => SendTo::Repaired,
        Err(e) => SendTo::Failed(e),
    }
}

// ----- what the person wants -------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    /// `None`: not decided yet (see the module text).
    pub start_with_windows: Option<bool>,
    pub send_to: bool,
}

impl Default for Intent {
    fn default() -> Self {
        Intent { start_with_windows: None, send_to: true }
    }
}

impl Intent {
    /// The file in `data_dir`. A missing or broken file is the default (and a broken one is said in the log).
    pub fn load(data_dir: &Path) -> Intent {
        let path = data_dir.join(FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Intent::default(),
            Err(e) => {
                log::warn!("reading {} failed ({e}); using the defaults", path.display());
                return Intent::default();
            }
        };
        match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
            Ok(Value::Object(o)) => Intent {
                start_with_windows: o.get("startWithWindows").and_then(Value::as_bool),
                send_to: o.get("sendTo").and_then(Value::as_bool).unwrap_or(true),
            },
            _ => {
                log::warn!("{} is not a JSON object; using the defaults", path.display());
                Intent::default()
            }
        }
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join(FILE_NAME);
        let tmp = path.with_extension("json.tmp");
        let text = json!({ "startWithWindows": self.start_with_windows, "sendTo": self.send_to });
        std::fs::write(&tmp, serde_json::to_vec_pretty(&text)?)?;
        std::fs::rename(&tmp, &path)
    }
}

/// The Settings window's switch was changed: remember it, so that the repair at the next start does what the person chose.
pub fn remember_start_with_windows(data_dir: &Path, on: bool) {
    let mut intent = Intent::load(data_dir);
    intent.start_with_windows = Some(on);
    if let Err(e) = intent.save(data_dir) {
        log::warn!("saving {FILE_NAME} failed: {e}");
    }
}

/// What to do about "Start with Windows" at start: the intent to keep, and whether to (re)write the Run value.
///
/// * `Some(true)`: write it again (it is gone after an uninstall that was the first half of an upgrade, or points at an old folder).
/// * `Some(false)`: the person turned it off; leave it.
/// * not decided, and this is a new profile: on, and write it. Not decided on an older profile: keep whatever it is now.
pub fn plan_autostart(wanted: Option<bool>, new_profile: bool, currently_on: bool) -> (Option<bool>, bool) {
    match wanted {
        Some(true) => (Some(true), true),
        Some(false) => (Some(false), false),
        None if new_profile => (Some(true), true),
        None => (Some(currently_on), false),
    }
}

/// What `reconcile` did, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub send_to: SendTo,
    /// `Ok(true)`: the Run value was written; `Ok(false)`: left as it was.
    pub autostart: Result<bool, String>,
}

/// The start-up work: the SendTo shortcut and "Start with Windows" are made what the person wants. `new_profile` is true when this run
/// made the identity (the first run of this user).
pub fn reconcile(data_dir: &Path, exe: &Path, send_to_dir: &Path, autostart: &dyn AutostartStore, new_profile: bool) -> Report {
    let mut intent = Intent::load(data_dir);
    let send_to = ensure_send_to(send_to_dir, exe, intent.send_to);
    let currently_on = autostart.is_enabled().unwrap_or(false);
    let (wanted, write) = plan_autostart(intent.start_with_windows, new_profile, currently_on);
    let autostart_result = if write { autostart.set_enabled(true).map(|()| true) } else { Ok(false) };
    if wanted != intent.start_with_windows {
        intent.start_with_windows = wanted;
        if let Err(e) = intent.save(data_dir) {
            log::warn!("saving {FILE_NAME} failed: {e}");
        }
    }
    Report { send_to, autostart: autostart_result }
}

/// `…\target\release` and `…\target\debug`: the build folder of a developer, where Kin must not set itself up.
pub fn is_build_folder(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else {
        return false;
    };
    let name = dir.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let parent = dir.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    parent == "target" && (name == "release" || name == "debug")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autostart::FileAutostart;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Program Files\Kin\Kin.exe")
    }

    #[test]
    fn the_app_user_model_id_is_the_identifier_of_the_app() {
        // The Start Menu shortcut gets the bundle identifier as its ID (the Tauri template), and the notification plugin names the identifier as
        // the origin of every toast: the three have to be the same string.
        let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], APP_USER_MODEL_ID);
    }

    #[test]
    fn the_process_gets_the_app_user_model_id() {
        // Every test shares one process, and the ID is the same string for all of them.
        set_process_app_user_model_id().unwrap();
        assert_eq!(process_app_user_model_id().as_deref(), Some(APP_USER_MODEL_ID));
        set_process_app_user_model_id().unwrap(); // twice is fine
    }

    #[test]
    fn a_shortcut_is_written_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Kin.lnk");
        assert_eq!(read_shortcut(&path), Ok(None), "no file yet");
        let target = dir.path().join("Kin.exe");
        write_shortcut(&path, &target, "--send", "a test").unwrap();
        assert!(path.is_file());
        let got = read_shortcut(&path).unwrap().unwrap();
        assert_eq!(got, Shortcut { target: target.clone(), arguments: "--send".into() });
        // A path with a space and non-ASCII letters survives.
        let odd = dir.path().join("Anna Berg \u{e4}\u{f6}").join("Kin.exe");
        write_shortcut(&path, &odd, "--send", "a test").unwrap();
        assert_eq!(read_shortcut(&path).unwrap().unwrap().target, odd);
    }

    #[test]
    fn a_file_that_is_not_a_shortcut_is_an_error_and_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Kin.lnk");
        std::fs::write(&path, b"not a shortcut").unwrap();
        assert!(read_shortcut(&path).is_err());
        // `ensure_send_to` replaces it.
        let target = dir.path().join("Kin.exe");
        assert_eq!(ensure_send_to(dir.path(), &target, true), SendTo::Repaired);
        assert_eq!(read_shortcut(&path).unwrap().unwrap().target, target);
    }

    #[test]
    fn the_send_to_shortcut_is_made_repaired_left_alone_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let send_to = dir.path().join("SendTo"); // does not exist yet: it is made
        let (old, new) = (dir.path().join("old").join("Kin.exe"), dir.path().join("new").join("Kin.exe"));
        assert_eq!(ensure_send_to(&send_to, &old, true), SendTo::Created);
        assert_eq!(ensure_send_to(&send_to, &old, true), SendTo::Unchanged, "twice is the same");
        let got = read_shortcut(&send_to.join(SENDTO_NAME)).unwrap().unwrap();
        assert_eq!((got.target.as_path(), got.arguments.as_str()), (old.as_path(), "--send"));
        // The program moved to another folder (a new install path): the shortcut follows.
        assert_eq!(ensure_send_to(&send_to, &new, true), SendTo::Repaired);
        assert_eq!(read_shortcut(&send_to.join(SENDTO_NAME)).unwrap().unwrap().target, new);
        // The case of the path is not a difference.
        let shouting = PathBuf::from(new.to_string_lossy().to_ascii_uppercase());
        assert_eq!(ensure_send_to(&send_to, &shouting, true), SendTo::Unchanged);
        // Off removes it, and off again has nothing to do.
        assert_eq!(ensure_send_to(&send_to, &new, false), SendTo::Off);
        assert!(!send_to.join(SENDTO_NAME).exists());
        assert_eq!(ensure_send_to(&send_to, &new, false), SendTo::Off);
        // Other shortcuts in the folder are never touched.
        std::fs::write(send_to.join("Other.lnk"), b"x").unwrap();
        assert_eq!(ensure_send_to(&send_to, &new, true), SendTo::Created);
        assert_eq!(ensure_send_to(&send_to, &new, false), SendTo::Off);
        assert!(send_to.join("Other.lnk").is_file());
    }

    #[test]
    fn the_intent_is_read_and_written_and_a_broken_file_is_the_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Intent::load(dir.path()), Intent::default());
        assert_eq!(Intent::default(), Intent { start_with_windows: None, send_to: true });
        let saved = Intent { start_with_windows: Some(false), send_to: false };
        saved.save(dir.path()).unwrap();
        assert_eq!(Intent::load(dir.path()), saved);
        std::fs::write(dir.path().join(FILE_NAME), "\u{feff}{ \"sendTo\": false }").unwrap();
        assert_eq!(Intent::load(dir.path()), Intent { start_with_windows: None, send_to: false }, "a byte order mark is fine");
        for broken in ["not json", "[1]", "{ \"sendTo\": \"no\", \"startWithWindows\": 3 }"] {
            std::fs::write(dir.path().join(FILE_NAME), broken).unwrap();
            assert_eq!(Intent::load(dir.path()), Intent::default(), "{broken}");
        }
        remember_start_with_windows(dir.path(), true);
        assert_eq!(Intent::load(dir.path()).start_with_windows, Some(true));
    }

    #[test]
    fn autostart_defaults_on_for_a_new_profile_and_never_flips_what_the_person_chose() {
        // (what the person wants, a new profile, the Run value is there now) -> (the wish to keep, write the Run value)
        assert_eq!(plan_autostart(None, true, false), (Some(true), true), "a first run turns it on");
        assert_eq!(plan_autostart(None, false, false), (Some(false), false), "an older profile with it off stays off");
        assert_eq!(plan_autostart(None, false, true), (Some(true), false), "an older profile with it on stays on");
        assert_eq!(plan_autostart(Some(true), false, false), (Some(true), true), "gone after an upgrade: written again");
        assert_eq!(plan_autostart(Some(true), false, true), (Some(true), true), "written again also fixes an old folder");
        assert_eq!(plan_autostart(Some(false), true, false), (Some(false), false), "turned off stays off, also on a new profile");
        assert_eq!(plan_autostart(Some(false), false, true), (Some(false), false));
    }

    #[test]
    fn a_first_run_makes_the_shortcut_and_turns_autostart_on_and_a_second_run_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (data, send_to) = (dir.path().join("data"), dir.path().join("SendTo"));
        std::fs::create_dir_all(&data).unwrap();
        let store = FileAutostart::new(&data);
        let report = reconcile(&data, &exe(), &send_to, &store, true);
        assert_eq!(report, Report { send_to: SendTo::Created, autostart: Ok(true) });
        assert_eq!(store.is_enabled(), Ok(true));
        assert_eq!(Intent::load(&data).start_with_windows, Some(true), "remembered");
        // The same again.
        let report = reconcile(&data, &exe(), &send_to, &store, false);
        assert_eq!(report.send_to, SendTo::Unchanged);
        assert_eq!(store.is_enabled(), Ok(true));
    }

    #[test]
    fn what_an_upgrade_removed_is_put_back_and_what_the_person_turned_off_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let (data, send_to) = (dir.path().join("data"), dir.path().join("SendTo"));
        std::fs::create_dir_all(&data).unwrap();
        let store = FileAutostart::new(&data);
        reconcile(&data, &exe(), &send_to, &store, true);
        // The old uninstaller (the first half of an upgrade) took the shortcut and the Run value away; the data folder stays.
        std::fs::remove_file(send_to.join(SENDTO_NAME)).unwrap();
        store.set_enabled(false).unwrap();
        let report = reconcile(&data, &exe(), &send_to, &store, false);
        assert_eq!(report, Report { send_to: SendTo::Created, autostart: Ok(true) });
        assert_eq!(store.is_enabled(), Ok(true));
        // The person turns "Start with Windows" off in the Settings window: the switch and the intent change together.
        store.set_enabled(false).unwrap();
        remember_start_with_windows(&data, false);
        let report = reconcile(&data, &exe(), &send_to, &store, false);
        assert_eq!(report.autostart, Ok(false), "left alone");
        assert_eq!(store.is_enabled(), Ok(false));
        // And a person who set "sendTo": false in integration.json has no shortcut.
        let mut intent = Intent::load(&data);
        intent.send_to = false;
        intent.save(&data).unwrap();
        assert_eq!(reconcile(&data, &exe(), &send_to, &store, false).send_to, SendTo::Off);
        assert!(!send_to.join(SENDTO_NAME).exists());
    }

    #[test]
    fn an_older_profile_keeps_the_choice_it_has() {
        let dir = tempfile::tempdir().unwrap();
        let (data, send_to) = (dir.path().to_path_buf(), dir.path().join("SendTo"));
        let store = FileAutostart::new(&data);
        // A 1.4 profile where the switch was off: the first start of 1.5 must not turn it on.
        let report = reconcile(&data, &exe(), &send_to, &store, false);
        assert_eq!(report.autostart, Ok(false));
        assert_eq!(store.is_enabled(), Ok(false));
        assert_eq!(Intent::load(&data).start_with_windows, Some(false), "and the choice is written down");
        // The SendTo shortcut is new in 1.5 and is made either way.
        assert_eq!(report.send_to, SendTo::Created);
    }

    #[test]
    fn the_build_folder_of_a_developer_is_recognised() {
        assert!(is_build_folder(Path::new(r"C:\Dev\ecosystem-app\desktop\src-tauri\target\release\Kin.exe")));
        assert!(is_build_folder(Path::new(r"c:\dev\x\TARGET\DEBUG\kin.exe")));
        assert!(!is_build_folder(Path::new(r"C:\Program Files\Kin\Kin.exe")));
        assert!(!is_build_folder(Path::new(r"C:\Users\Anna Berg\release\Kin.exe")), "a folder called release alone is not it");
        assert!(!is_build_folder(Path::new("Kin.exe")));
    }

    #[test]
    fn the_installer_hooks_say_what_the_docs_say() {
        let hooks = include_str!("../windows/hooks.nsh");
        // The names the verify scripts and the docs use.
        for needle in [
            "name=\"Kin Relay\"",
            "profile=private,public",
            "protocol=TCP",
            "program=\"$INSTDIR\\${MAINBINARYNAME}.exe\"",
            "delete rule name=\"Kin Relay\"",
            "$DeleteAppDataCheckboxState = 1",
            "$APPDATA\\Kin\"",
            "Kin.lnk",
        ] {
            assert!(hooks.contains(needle), "hooks.nsh lost {needle}");
        }
        assert!(hooks.contains(SENDTO_NAME) && SENDTO_NAME == "Kin.lnk");
        // The installer never writes a Run value of its own: the app does (HKCU of the right user).
        assert!(!hooks.contains("WriteRegStr HKCU"), "the Run value is the app's");
    }
}
