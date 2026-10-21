//! Which Kin this is: the user's own (installed) one, or another one that runs beside it on the same PC (a developer or test copy
//! started with its own `--data-dir`).
//!
//! **The rule:** an instance is identified by its data directory. The data directory the installed Kin uses (`%APPDATA%\Kin`) is *the*
//! instance, with no key. Any other data directory has a key, a short hash of the directory's normalised path. Everything that Windows
//! would otherwise share between two copies of the program is made from the key:
//!
//! - the single-instance mutex, window class and window of `tauri-plugin-single-instance` (they are built from the bundle identifier, so a
//!   keyed instance gets `app.kinrelay.desktop.dev-<key>` and a second launch with the same `--data-dir` reaches this instance and no
//!   other);
//! - the app user model ID of the process, and with it the origin of the toasts (the notification plugin takes it from the identifier);
//! - the data directory of the web views (`ui.rs`), so a test copy never touches the profile of the installed one's windows;
//! - the port (a keyed instance never tries the default port: it asks for the one it was given, or for any free one);
//! - mDNS (a keyed instance never advertises);
//! - "Start with Windows", the "Send to" shortcut (they already needed a normal release run on the user's own profile).
//!
//! **What a second launch may do.** `Kin.exe --send a.jpg` hands its command line to the running instance with the same key. A launch with
//! no `--data-dir` has no key, and its target is the installed Kin. That is right for the installed program (the "Send to" shortcut) and
//! wrong for a developer's copy, so a launch refuses to hand anything to a running Kin that is a *different program file*
//! ([`forwarding_refusal`]).
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The bundle identifier of the installed app (`tauri.conf.json`), also its app user model ID.
pub const BASE_IDENTIFIER: &str = "app.kinrelay.desktop";

/// How many hex characters of the hash a key has. 48 bits: a collision between two throwaway directories on one PC is not a concern, and a
/// name that is short stays inside every limit that a mutex, a window class or an app user model ID has.
const KEY_LENGTH: usize = 12;

/// The path of a data directory in one form that every spelling of it shares: absolute, with `.` and `..` resolved, the part that exists
/// resolved through the file system (so a short `PROGRA~1` name and a long one agree, and so does a path that goes through a link), without
/// the `\\?\` prefix, with one kind of separator and no trailing one, and in lower case (NTFS is case-insensitive). The directory itself does
/// not have to exist: the instance computes its key before it creates the directory, and a second launch computes it after.
pub fn normalize(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut base = None;
    for ancestor in absolute.ancestors() {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            let rest = absolute.strip_prefix(ancestor).map(Path::to_path_buf).unwrap_or_default();
            base = Some(real.join(rest));
            break;
        }
    }
    let text = base.unwrap_or(absolute).to_string_lossy().replace('/', "\\");
    let text = match text.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text),
    };
    let trimmed = text.trim_end_matches('\\');
    // A drive root ("C:\") keeps its separator: "C:" alone means "the current folder of drive C".
    let text = if trimmed.ends_with(':') { format!("{trimmed}\\") } else { trimmed.to_string() };
    text.to_lowercase()
}

/// The key of the instance whose data lives in `data_dir`: `None` for the installed Kin's own directory (`installed_data_dir`, which is
/// `%APPDATA%\Kin`), a short hash of the normalised path for any other.
pub fn key_for(data_dir: &Path, installed_data_dir: Option<&Path>) -> Option<String> {
    let normal = normalize(data_dir);
    if installed_data_dir.is_some_and(|installed| normalize(installed) == normal) {
        return None;
    }
    let digest = Sha256::digest(format!("kin-instance\0{normal}").as_bytes());
    Some(digest.iter().take(KEY_LENGTH / 2).map(|b| format!("{b:02x}")).collect())
}

/// The bundle identifier this instance runs under: the installed one's for the instance with no key, `<identifier>.dev-<key>` for another.
/// The single-instance plugin builds its mutex (`<id>-sim`), window class (`<id>-sic`) and window name (`<id>-siw`) from it.
pub fn identifier(key: Option<&str>) -> String {
    match key {
        None => BASE_IDENTIFIER.to_string(),
        Some(key) => format!("{BASE_IDENTIFIER}.dev-{key}"),
    }
}

/// The app user model ID of the process. It is the identifier, as for the installed app, so a toast of an instance is shown under its own
/// name and never in the group of the installed Kin's.
pub fn app_user_model_id(key: Option<&str>) -> String {
    identifier(key)
}

/// The window class and the window name that the single-instance plugin gives the window which receives a second launch's command line.
pub fn single_instance_window(identifier: &str) -> (String, String) {
    (format!("{identifier}-sic"), format!("{identifier}-siw"))
}

/// Whether two paths of program files are the same file (case, `\\?\` and short names ignored). A path that doesn't exist is compared as
/// text.
pub fn same_program(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

/// The reason a launch must not hand its command line to the instance that is running, or `None` if it may.
///
/// A launch hands its arguments to the running instance with the same key. For the installed Kin's own key (no `--data-dir`) that instance
/// is the user's, and the only program that may talk to it is the same program file: the "Send to" shortcut and the Start Menu shortcut start
/// `Kin.exe` of the install folder, which is what is running. A developer's copy (the build folder, a copy in a temp folder) with no
/// `--data-dir` would hand it `--quit` or `--send`, which is never wanted. So: a launch with no key whose running instance is another
/// file is refused. **It fails closed**: if a window answers for the key but Windows won't say which program file owns it
/// ([`Running::Unreadable`]), the launch is refused too, because it can't be shown to be the same file.
pub fn forwarding_refusal(key: Option<&str>, mine: &Path, running: &Running) -> Option<String> {
    if key.is_some() {
        return None;
    }
    let running = match running {
        Running::Nobody => return None,
        Running::Unreadable => {
            return Some(format!(
                "another Kin is running with the user's own data directory, but Windows won't say which program file it is, so this copy ({}) \
                 can't tell whether it is the same program. A second launch would hand its command line to it, so it is refused. Give this \
                 copy its own --data-dir (and --dev-bind 127.0.0.1) to run it beside the installed one.",
                mine.display()
            ));
        }
        Running::Program(path) => path,
    };
    if same_program(mine, running) {
        return None;
    }
    Some(format!(
        "another Kin is running ({}) with the user's own data directory, and this copy ({}) is a different program. A second launch would hand \
         its command line to it, which is never what a developer's copy should do. Give this copy its own --data-dir (and --dev-bind \
         127.0.0.1) to run it beside the installed one.",
        running.display(),
        mine.display()
    ))
}

/// The reason developer settings can't be used with this data directory, or `None`. Developer settings (`--dev-*`) change what the app
/// does to the network, the clipboard and the files of a profile, which is never right for the profile of the person's own Kin.
pub fn dev_settings_refusal(dev_settings: bool, key: Option<&str>) -> Option<String> {
    (dev_settings && key.is_none()).then(|| {
        "developer settings (--dev-*) need a data directory of their own: add --data-dir <throwaway folder>. Without it they would run against \
         the profile of the installed Kin (its identity, its pairing, its history)"
            .to_string()
    })
}

/// Who holds an identifier (the single-instance names), as a second launch finds out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Running {
    /// There is no window under the names: this launch becomes the instance.
    Nobody,
    /// There is one, and this is the program file of the process that owns it.
    Program(PathBuf),
    /// There is one, but its owner or the owner's program file can't be read (the process ended meanwhile, or Windows refuses).
    Unreadable,
}

/// The program file of the instance that is running with this identifier, found the way a second launch finds it: the window of the
/// single-instance plugin, and the process that owns it.
#[cfg(windows)]
pub fn running_program(identifier: &str) -> Running {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
    use windows::core::{HSTRING, PWSTR};
    let (class, name) = single_instance_window(identifier);
    // SAFETY: plain Win32 calls; every string is valid for its call, the buffer is as long as it says, and the handle is closed again.
    unsafe {
        let Ok(hwnd) = FindWindowW(&HSTRING::from(class), &HSTRING::from(name)) else {
            return Running::Nobody;
        };
        // From here on a window answers for the key: whatever can't be read is `Unreadable`, never `Nobody`.
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return Running::Unreadable;
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return Running::Unreadable;
        };
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length);
        let _ = CloseHandle(process);
        if queried.is_err() {
            return Running::Unreadable;
        }
        Running::Program(PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
    }
}

#[cfg(not(windows))]
pub fn running_program(_identifier: &str) -> Running {
    Running::Nobody
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn every_spelling_of_a_directory_has_one_key() {
        let tmp = dir();
        let data = tmp.path().join("Kin Dev 1");
        let key = key_for(&data, None).expect("a directory of its own has a key");
        assert_eq!(key.len(), KEY_LENGTH);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        let spellings = [
            tmp.path().join("kin dev 1"),
            tmp.path().join("KIN DEV 1"),
            PathBuf::from(format!("{}\\", data.display())),
            PathBuf::from(format!("{}/", data.display().to_string().replace('\\', "/"))),
            tmp.path().join(".").join("Kin Dev 1"),
            tmp.path().join("other").join("..").join("Kin Dev 1"),
        ];
        for s in spellings {
            assert_eq!(key_for(&s, None).as_deref(), Some(key.as_str()), "{}", s.display());
        }
    }

    #[test]
    fn the_key_is_the_same_before_and_after_the_directory_exists() {
        // The instance computes it at start (nothing there yet), a second launch computes it when the instance has made the directory.
        let tmp = dir();
        let data = tmp.path().join("later").join("deeper");
        let before = key_for(&data, None);
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(key_for(&data, None), before);
        assert!(before.is_some());
    }

    #[test]
    fn a_relative_directory_is_taken_from_the_folder_of_the_launch() {
        let here = std::env::current_dir().unwrap();
        assert_eq!(key_for(Path::new("kin-test-data"), None), key_for(&here.join("kin-test-data"), None));
    }

    #[test]
    fn different_directories_have_different_keys() {
        let tmp = dir();
        let a = key_for(&tmp.path().join("a"), None);
        let b = key_for(&tmp.path().join("b"), None);
        assert!(a.is_some() && b.is_some());
        assert_ne!(a, b);
        assert_ne!(key_for(&tmp.path().join("a"), None), key_for(&tmp.path().join("a").join("a"), None), "a child is another directory");
    }

    #[test]
    fn the_installed_kins_directory_has_no_key_in_any_spelling() {
        let tmp = dir();
        let installed = tmp.path().join("Roaming").join("Kin");
        assert_eq!(key_for(&installed, Some(&installed)), None);
        assert_eq!(key_for(&tmp.path().join("ROAMING").join("kin"), Some(&installed)), None, "case");
        assert_eq!(key_for(&tmp.path().join("Roaming").join("x").join("..").join("Kin"), Some(&installed)), None, "`..`");
        assert_eq!(key_for(&PathBuf::from(format!("{}\\", installed.display())), Some(&installed)), None, "a trailing separator");
        assert!(key_for(&tmp.path().join("Roaming").join("Kin2"), Some(&installed)).is_some(), "a name that starts the same is another");
        assert!(key_for(&installed, None).is_some(), "with no installed directory known nothing is the installed instance");
    }

    #[test]
    fn a_junction_or_short_name_does_not_make_a_second_key() {
        // The temp folder is where scripts make their throwaway profiles, and Windows can name it by its short form (`PROGRA~1`-style), which
        // `canonicalize` turns into the long one.
        let tmp = dir();
        let data = tmp.path().join("kin-data");
        std::fs::create_dir_all(&data).unwrap();
        let canonical = std::fs::canonicalize(&data).unwrap();
        assert_eq!(key_for(&canonical, None), key_for(&data, None), "the `\\\\?\\` form and the plain one agree");
        assert!(!normalize(&canonical).starts_with(r"\\?\"));
    }

    #[test]
    fn a_drive_root_keeps_its_separator() {
        assert_eq!(normalize(Path::new(r"C:\")), r"c:\");
        assert_eq!(normalize(Path::new(r"C:\Dev\")), r"c:\dev");
    }

    #[test]
    fn identifiers_are_the_installed_one_or_a_keyed_one() {
        assert_eq!(identifier(None), "app.kinrelay.desktop");
        assert_eq!(identifier(Some("0123456789ab")), "app.kinrelay.desktop.dev-0123456789ab");
        assert_ne!(identifier(Some("0123456789ab")), identifier(Some("0123456789ac")));
        assert_eq!(app_user_model_id(None), BASE_IDENTIFIER);
        assert_ne!(app_user_model_id(Some("0123456789ab")), BASE_IDENTIFIER);
        // The single-instance plugin's names come from it and so are apart too.
        let (class, window) = single_instance_window(&identifier(Some("0123456789ab")));
        let (installed_class, installed_window) = single_instance_window(&identifier(None));
        assert_ne!(class, installed_class);
        assert_ne!(window, installed_window);
        assert_eq!(installed_class, "app.kinrelay.desktop-sic");
        assert_eq!(installed_window, "app.kinrelay.desktop-siw");
        // A window class is at most 256 characters, a mutex name 260, an app user model ID 128.
        assert!(identifier(Some("0123456789ab")).len() + 4 < 128);
    }

    #[test]
    fn the_bundle_identifier_is_the_one_in_tauri_conf() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], BASE_IDENTIFIER);
    }

    #[test]
    fn a_second_launch_with_the_same_key_may_reach_its_instance_and_a_different_program_may_not_reach_the_installed_one() {
        let installed = Path::new(r"C:\Program Files\Kin\Kin.exe");
        let build = Path::new(r"C:\Dev\x\src-tauri\target\release\Kin.exe");
        // The installed program talking to the installed instance (the "Send to" shortcut), or to nothing: fine.
        let running = Running::Program(installed.to_path_buf());
        assert_eq!(forwarding_refusal(None, installed, &running), None);
        assert_eq!(forwarding_refusal(None, Path::new(r"c:\program files\kin\KIN.EXE"), &running), None, "case");
        assert_eq!(forwarding_refusal(None, build, &Running::Nobody), None, "nothing is running: this launch becomes the instance");
        // A developer's copy with no data directory of its own: refused, and the message says what to do.
        let refusal = forwarding_refusal(None, build, &running).expect("refused");
        assert!(refusal.contains("--data-dir") && refusal.contains("different program"), "{refusal}");
        // A launch that has a key (its own data directory) talks to the instance with that key, whichever file runs it.
        assert_eq!(forwarding_refusal(Some("0123456789ab"), build, &running), None);
        assert_eq!(forwarding_refusal(Some("0123456789ab"), build, &Running::Program(build.to_path_buf())), None);
    }

    #[test]
    fn a_running_instance_whose_program_cannot_be_read_is_not_forwarded_to() {
        // Fail closed: a window answers for the user's own key, Windows won't say whose it is. Even the installed program's own launch can't be
        // shown to be the same file, so nothing is handed over.
        let installed = Path::new(r"C:\Program Files\Kin\Kin.exe");
        let refusal = forwarding_refusal(None, installed, &Running::Unreadable).expect("refused");
        assert!(refusal.contains("won't say which program file") && refusal.contains("--data-dir"), "{refusal}");
        // A launch with its own data directory never forwards to the installed Kin, so it has nothing to be refused for.
        assert_eq!(forwarding_refusal(Some("0123456789ab"), installed, &Running::Unreadable), None);
    }

    #[test]
    fn developer_settings_need_a_data_directory_of_their_own() {
        assert!(dev_settings_refusal(true, None).is_some());
        assert!(dev_settings_refusal(true, Some("0123456789ab")).is_none());
        assert!(dev_settings_refusal(false, None).is_none(), "an ordinary run of the installed Kin");
        assert!(dev_settings_refusal(false, Some("0123456789ab")).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn no_program_answers_for_an_identifier_nobody_runs() {
        assert_eq!(running_program("app.kinrelay.desktop.dev-nobody-runs-this"), Running::Nobody);
    }

    /// The window the single-instance plugin makes (class `<id>-sic`, window name `<id>-siw`, top level, never shown) is found by its identifier,
    /// and the program file that comes back is the one that owns it. This is the lookup a second launch makes, run against a window of this test
    /// process under an identifier of its own: the installed Kin's window is never looked at.
    #[cfg(windows)]
    #[test]
    fn the_program_that_owns_the_single_instance_window_is_found_by_its_identifier() {
        use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
        };
        use windows::core::{HSTRING, PCWSTR};
        unsafe extern "system" fn proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            // SAFETY: passes the message on to the default handler, as every window procedure may.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        let id = format!("app.kinrelay.desktop.dev-test-{}", std::process::id());
        let (class, name) = single_instance_window(&id);
        let (class, name) = (HSTRING::from(class), HSTRING::from(name));
        // SAFETY: plain Win32 calls with strings that outlive them; the window is destroyed at the end.
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap();
            let registered = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            });
            assert_ne!(registered, 0);
            assert_eq!(running_program(&id), Running::Nobody, "no window yet");
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(name.as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            let running = running_program(&id);
            let Running::Program(found) = &running else { panic!("the window is found with its program: {running:?}") };
            assert!(same_program(found, &std::env::current_exe().unwrap()), "{}", found.display());
            // A launch of another program file with no data directory is refused, the same file is not.
            assert!(forwarding_refusal(None, Path::new(r"C:\Dev\x\target\release\Kin.exe"), &running).is_some());
            assert!(forwarding_refusal(None, &std::env::current_exe().unwrap(), &running).is_none());
            // Another identifier doesn't find it.
            assert_eq!(running_program(&format!("{id}-other")), Running::Nobody);
            let _ = DestroyWindow(hwnd);
        }
    }
}
