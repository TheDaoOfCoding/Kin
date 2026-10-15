//! "Start with Windows" (milestone 1.4): the settings window's switch, a value in
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! **Only the current user's Run key, never anything machine-wide.** The registry is written by [`RegistryAutostart`] below and by nothing
//! else, and it names `HKEY_CURRENT_USER` and no other root: a process that happens to be elevated still writes only its own user's key
//! (the Tauri autostart plugin this replaces could choose the machine-wide key when it ran elevated). The value is the path of `Kin.exe` **in quotes**
//! (`"C:\Program Files\Kin\Kin.exe"`), because an unquoted path with a space is the classic way for Windows to start the wrong program.
//!
//! **Tests and developer runs never touch the real Run key.** The real switch is used only by a **release build** that runs the way a user
//! runs it: no `--dev-*` flag and the default data directory (`Config::autostart_is_real`). In every other run (a debug build, a script, an
//! integration test, `--data-dir` of its own) the switch is a file, `autostart-mock.json` in the data directory, that records what would have
//! been done, so the settings plumbing is exercised without ever enabling Kin on the real account. `status.json` says which one is in use
//! (`autostart.real`). The unit tests of the registry code use a throwaway key of their own, never the Run key.
//!
//! (For 1.5: the uninstaller has to remove the `Kin` value of the Run key.)
use std::path::{Path, PathBuf};

/// The Run key of the current user.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The name of the value that starts Kin.
pub const RUN_VALUE: &str = "Kin";

/// The command line the Run key holds for `exe`: the path in quotes. A path that already has quotes in it (Windows doesn't allow `"` in a
/// file name, so this is only a safeguard) loses them.
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\"", exe.to_string_lossy().replace('"', ""))
}

/// The registry switch: `HKCU\<subkey>`, value `name`, holding [`run_command`] of `exe`. [`RegistryAutostart::run_key`] is the real one.
pub struct RegistryAutostart {
    subkey: String,
    name: String,
    exe: PathBuf,
}

impl RegistryAutostart {
    /// The real switch: the Run key of the current user, the value `Kin`, this executable.
    pub fn run_key() -> std::io::Result<RegistryAutostart> {
        Ok(RegistryAutostart { subkey: RUN_KEY.to_string(), name: RUN_VALUE.to_string(), exe: std::env::current_exe()? })
    }

    /// A switch over another key of the current user (the unit tests use a throwaway one).
    pub fn with_key(subkey: &str, name: &str, exe: &Path) -> RegistryAutostart {
        RegistryAutostart { subkey: subkey.to_string(), name: name.to_string(), exe: exe.to_path_buf() }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The value, if there is one.
    pub fn value(&self) -> Result<Option<String>, String> {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW};
        use windows::core::PCWSTR;
        let (subkey, name) = (Self::wide(&self.subkey), Self::wide(&self.name));
        // SAFETY: every pointer is to a NUL-terminated wide string or a buffer that lives for the call; the key is closed before returning.
        unsafe {
            let mut key = Default::default();
            let opened = RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), None, KEY_QUERY_VALUE, &mut key);
            if opened == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if opened != ERROR_SUCCESS {
                return Err(format!("opening the registry key failed ({})", opened.0));
            }
            let mut kind = Default::default();
            let mut size = 0u32;
            let probe = RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, Some(&mut kind), None, Some(&mut size));
            let result = if probe == ERROR_FILE_NOT_FOUND {
                Ok(None)
            } else if probe != ERROR_SUCCESS || kind != REG_SZ {
                Err(format!("reading the registry value failed ({})", probe.0))
            } else {
                let mut buffer = vec![0u8; size as usize];
                let read = RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, Some(buffer.as_mut_ptr()), Some(&mut size));
                if read != ERROR_SUCCESS {
                    Err(format!("reading the registry value failed ({})", read.0))
                } else {
                    let units: Vec<u16> = buffer[..size as usize].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                    Ok(Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string()))
                }
            };
            let _ = RegCloseKey(key);
            result
        }
    }

    fn write(&self, on: bool) -> Result<(), String> {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{
            HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteValueW,
            RegSetValueExW,
        };
        use windows::core::PCWSTR;
        let (subkey, name) = (Self::wide(&self.subkey), Self::wide(&self.name));
        let command = Self::wide(&run_command(&self.exe));
        // SAFETY: as in `value`: wide strings and buffers outlive each call, and the key is closed before returning.
        unsafe {
            let mut key = Default::default();
            let created = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            );
            if created != ERROR_SUCCESS {
                return Err(format!("opening the registry key failed ({})", created.0));
            }
            let result = if on {
                let bytes: Vec<u8> = command.iter().flat_map(|u| u.to_le_bytes()).collect();
                let set = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&bytes));
                if set == ERROR_SUCCESS { Ok(()) } else { Err(format!("writing the registry value failed ({})", set.0)) }
            } else {
                let deleted = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
                if deleted == ERROR_SUCCESS || deleted == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("removing the registry value failed ({})", deleted.0))
                }
            };
            let _ = RegCloseKey(key);
            result
        }
    }
}

impl AutostartStore for RegistryAutostart {
    fn is_enabled(&self) -> Result<bool, String> {
        self.value().map(|v| v.is_some())
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        self.write(on)
    }

    fn is_real(&self) -> bool {
        true
    }
}

/// Where the switch lives.
pub trait AutostartStore: Send + Sync {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
    /// `true` for the registry, `false` for the file that stands in for it.
    fn is_real(&self) -> bool;
}

/// The stand-in: a JSON file `{ "enabled": true }` in the data directory.
pub struct FileAutostart {
    path: PathBuf,
}

pub const MOCK_FILE: &str = "autostart-mock.json";

impl FileAutostart {
    pub fn new(data_dir: &Path) -> FileAutostart {
        FileAutostart { path: data_dir.join(MOCK_FILE) }
    }
}

impl AutostartStore for FileAutostart {
    fn is_enabled(&self) -> Result<bool, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                Ok(v["enabled"].as_bool().unwrap_or(false))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let text = serde_json::json!({ "enabled": on, "note": "a stand-in for the Run key: a developer run never changes the registry" });
        std::fs::write(&self.path, text.to_string()).map_err(|e| e.to_string())
    }

    fn is_real(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_has_the_path_in_quotes() {
        assert_eq!(run_command(Path::new(r"C:\Program Files\Kin Relay\Kin.exe")), r#""C:\Program Files\Kin Relay\Kin.exe""#);
        assert_eq!(
            run_command(Path::new(r"C:\Users\Anna Berg\AppData\Local\Kin\Kin.exe")),
            r#""C:\Users\Anna Berg\AppData\Local\Kin\Kin.exe""#
        );
        assert_eq!(run_command(Path::new(r"C:\a\Kin.exe")), r#""C:\a\Kin.exe""#, "quoted even when there is no space");
        assert_eq!(
            run_command(Path::new("C:\\odd\"name\\Kin.exe")),
            r#""C:\oddname\Kin.exe""#,
            "a quote inside can't end the quoting early"
        );
    }

    /// A throwaway key of the current user: **never the Run key**. It is deleted at the end.
    fn throwaway() -> (RegistryAutostart, String) {
        let subkey = format!(r"Software\KinRelayUnitTest\{}", crate::encoding::uuid_v7(crate::encoding::unix_ms()));
        (RegistryAutostart::with_key(&subkey, "Kin", Path::new(r"C:\Program Files\Kin Relay\Kin.exe")), subkey)
    }

    fn delete_key(subkey: &str) {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW};
        use windows::core::HSTRING;
        // SAFETY: a valid wide string for the call.
        unsafe {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(subkey));
        }
    }

    #[test]
    fn the_registry_switch_writes_the_quoted_path_reads_it_back_and_removes_it() {
        let (store, subkey) = throwaway();
        assert_ne!(subkey, RUN_KEY, "a test never uses the real Run key");
        assert_eq!(store.is_enabled(), Ok(false), "no key yet: off");
        store.set_enabled(true).unwrap();
        assert_eq!(store.is_enabled(), Ok(true));
        assert_eq!(store.value().unwrap().as_deref(), Some(r#""C:\Program Files\Kin Relay\Kin.exe""#), "the value is the quoted path");
        store.set_enabled(true).unwrap(); // twice is the same
        assert_eq!(store.is_enabled(), Ok(true));
        store.set_enabled(false).unwrap();
        assert_eq!(store.is_enabled(), Ok(false));
        store.set_enabled(false).unwrap(); // off again is not an error
        delete_key(&subkey);
        delete_key(r"Software\KinRelayUnitTest");
    }

    #[test]
    fn the_registry_code_names_the_current_user_and_no_machine_wide_root() {
        let source = include_str!("autostart.rs");
        // The words are built here so that this test doesn't contain what it looks for.
        for root in
            [["HKEY_LOCAL", "_MACHINE"].concat(), ["HK", "LM"].concat(), ["HKEY_CLASSES", "_ROOT"].concat(), ["HKEY_US", "ERS"].concat()]
        {
            assert_eq!(source.matches(root.as_str()).count(), 0, "{root} must not appear in the autostart code");
        }
        assert!(source.contains("HKEY_CURRENT_USER"));
    }

    #[test]
    fn the_real_run_key_is_the_one_of_the_current_user_and_the_value_is_called_kin() {
        assert_eq!(RUN_KEY, r"Software\Microsoft\Windows\CurrentVersion\Run");
        assert_eq!(RUN_VALUE, "Kin");
    }

    #[test]
    fn the_stand_in_remembers_the_switch_in_a_file_and_never_anywhere_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileAutostart::new(dir.path());
        assert_eq!(store.is_enabled(), Ok(false), "off until asked");
        assert!(!store.is_real());
        store.set_enabled(true).unwrap();
        assert_eq!(store.is_enabled(), Ok(true));
        assert!(dir.path().join(MOCK_FILE).is_file());
        // A new object over the same folder sees it (a restart of the app).
        assert_eq!(FileAutostart::new(dir.path()).is_enabled(), Ok(true));
        store.set_enabled(false).unwrap();
        assert_eq!(store.is_enabled(), Ok(false));
    }
}
