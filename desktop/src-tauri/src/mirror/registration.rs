//! What Windows needs to know about Kin's phone notifications, all under `HKCU` and written by the app itself (the recipe was
//! worked out by experiment on Windows 10 and 11):
//!
//! ```text
//! HKCU\Software\Classes\CLSID\{CLSID}                      (Default) = Kin phone notifications activator
//! HKCU\Software\Classes\CLSID\{CLSID}\LocalServer32        (Default) = "C:\Program Files\Kin\Kin.exe" --toast-activated
//! HKCU\Software\Classes\AppUserModelId\<AUMID>             DisplayName, IconUri, CustomActivator = {CLSID}
//! ```
//!
//! No Start Menu shortcut, no `ToastActivatorCLSID`. The installed Kin uses [`Identity::release`] (`app.kinrelay.desktop.phone` and a fixed CLSID) and
//! **only after `may_register_release` says so**; any other instance, and every test, uses an identity of its own (`dev-<key>`, a CLSID made from the
//! key) and removes **all** of its keys again, including the two that Windows itself creates for an AUMID that has shown a toast
//! (`Notifications\Settings\<aumid>` and `PushNotifications\Backup\<aumid>`, the second one a while later).
//!
//! The installed Kin's uninstaller removes the CLSID and the AppUserModelId key (`windows/hooks.nsh`); the two that belong to the OS are left to it.
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::instance::BASE_IDENTIFIER;

/// The AUMID of the installed Kin's phone notifications: its own entry in Windows Settings > Notifications, apart from the Phase 1 toasts (`app.kinrelay.desktop`).
pub const RELEASE_AUMID: &str = "app.kinrelay.desktop.phone";
/// The CLSID of the installed Kin's activator: a random GUID, fixed forever (it is in the installer's uninstall hook too).
pub const RELEASE_CLSID: &str = "{6B0A1D3E-2F54-4E8A-9C71-3D5B8E2A4F90}";
/// What Windows Settings calls these notifications.
pub const DISPLAY_NAME: &str = "Kin: phone notifications";
/// The switch the shell's COM start-up adds to the registered command line, which the arguments parser accepts and ignores.
pub const ACTIVATED_SWITCH: &str = "--toast-activated";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Release,
    /// A copy beside the installed Kin, or a test: its own AUMID and CLSID, and everything it wrote is removed again.
    Dev,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub kind: Kind,
    pub aumid: String,
    /// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`, upper case.
    pub clsid: String,
}

impl Identity {
    pub fn release() -> Identity {
        Identity { kind: Kind::Release, aumid: RELEASE_AUMID.to_string(), clsid: RELEASE_CLSID.to_string() }
    }

    /// The identity of an instance with a key (`instance.rs`): `app.kinrelay.desktop.dev-<key>.phone` and a CLSID that is a hash of the key, so
    /// the same instance always finds its own keys and no two share one.
    pub fn dev(key: &str) -> Identity {
        let digest = Sha256::digest(format!("kin-phone-activator\0{key}").as_bytes());
        let mut b: [u8; 16] = digest[..16].try_into().expect("a SHA-256 has 32 bytes");
        b[6] = 0x40 | (b[6] & 0x0f);
        b[8] = 0x80 | (b[8] & 0x3f);
        let h: String = b.iter().map(|x| format!("{x:02X}")).collect();
        Identity {
            kind: Kind::Dev,
            aumid: format!("{BASE_IDENTIFIER}.dev-{key}.phone"),
            clsid: format!("{{{}-{}-{}-{}-{}}}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]),
        }
    }

    /// The identity of the instance a configuration describes: the installed Kin's, or the key's.
    pub fn for_config(config: &crate::cli::Config) -> Identity {
        match config.instance_key() {
            None => Identity::release(),
            Some(key) => Identity::dev(&key),
        }
    }

    /// The CLSID as a `u128` for the COM class table.
    pub fn clsid_u128(&self) -> u128 {
        let hex: String = self.clsid.chars().filter(char::is_ascii_hexdigit).collect();
        u128::from_str_radix(&hex, 16).unwrap_or(0)
    }

    fn classes(&self) -> (String, String, String) {
        (
            format!(r"Software\Classes\CLSID\{}", self.clsid),
            format!(r"Software\Classes\CLSID\{}\LocalServer32", self.clsid),
            format!(r"Software\Classes\AppUserModelId\{}", self.aumid),
        )
    }

    /// The keys Windows creates for an AUMID that has shown a toast. Only a [`Kind::Dev`] identity removes them: for the installed Kin they hold the
    /// user's own choices (muted, banners off) and stay.
    fn os_owned(&self) -> [String; 2] {
        [
            format!(r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{}", self.aumid),
            format!(r"Software\Microsoft\Windows\CurrentVersion\PushNotifications\Backup\{}", self.aumid),
        ]
    }
}

/// Writes the keys. `exe` is the program Windows starts when someone clicks a toast and Kin is not running, `icon` a PNG of the app icon.
pub fn register(identity: &Identity, exe: &Path, icon: &Path) -> io::Result<()> {
    let (clsid, local_server, aumid) = identity.classes();
    set_string(&clsid, None, "Kin phone notifications activator")?;
    // Quoted: Program Files has a space. COM adds `-Embedding` after it.
    set_string(&local_server, None, &format!("\"{}\" {ACTIVATED_SWITCH}", exe.display()))?;
    set_string(&aumid, Some("DisplayName"), DISPLAY_NAME)?;
    set_string(&aumid, Some("IconUri"), &icon.to_string_lossy())?;
    set_string(&aumid, Some("CustomActivator"), &identity.clsid)?;
    Ok(())
}

/// Removes what [`register`] wrote, and for a [`Kind::Dev`] identity the keys Windows made for the AUMID as well. A key that is not there is fine.
/// Returns how many keys were removed.
pub fn unregister(identity: &Identity) -> io::Result<usize> {
    let (clsid, _, aumid) = identity.classes();
    let mut keys = vec![clsid, aumid];
    if identity.kind == Kind::Dev {
        keys.extend(identity.os_owned());
    }
    let mut removed = 0;
    for key in keys {
        removed += delete_tree(&key)? as usize;
    }
    Ok(removed)
}

/// What is registered now (for tests and the log): the activator's command line and the activator named by the AUMID.
pub fn registered(identity: &Identity) -> (Option<String>, Option<String>) {
    let (_, local_server, aumid) = identity.classes();
    (get_string(&local_server, None), get_string(&aumid, Some("CustomActivator")))
}

/// Whether any of the keys of this identity, including the OS-owned ones, exists. A test that says it cleaned up checks this.
pub fn leftovers(identity: &Identity) -> Vec<String> {
    let (clsid, _, aumid) = identity.classes();
    [clsid, aumid].into_iter().chain(identity.os_owned()).filter(|k| key_exists(k)).collect()
}

// ----- the registry, HKCU only ----------------------------------------------------------------------------------------

fn set_string(subkey: &str, name: Option<&str>, value: &str) -> io::Result<()> {
    let subkey = HSTRING::from(subkey);
    let name = name.map(HSTRING::from);
    let bytes: Vec<u8> = value.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    let mut key = HKEY::default();
    // SAFETY: valid wide strings and buffers for each call; the key is closed on every path.
    unsafe {
        let created =
            RegCreateKeyExW(HKEY_CURRENT_USER, &subkey, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None);
        if created != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(created.0 as i32));
        }
        let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        let set = RegSetValueExW(key, name_ptr, None, REG_SZ, Some(&bytes));
        let _ = RegCloseKey(key);
        if set != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(set.0 as i32));
        }
    }
    Ok(())
}

fn get_string(subkey: &str, name: Option<&str>) -> Option<String> {
    let subkey = HSTRING::from(subkey);
    let name = name.map(HSTRING::from);
    let mut key = HKEY::default();
    // SAFETY: valid wide strings and buffers for each call; the key is closed on every path.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, KEY_QUERY_VALUE, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        let mut size = 0u32;
        let mut kind = REG_SZ;
        let probe = RegQueryValueExW(key, name_ptr, None, Some(&mut kind), None, Some(&mut size));
        let value = (probe == ERROR_SUCCESS && kind == REG_SZ && size > 0).then(|| {
            let mut buffer = vec![0u8; size as usize];
            let read = RegQueryValueExW(key, name_ptr, None, None, Some(buffer.as_mut_ptr()), Some(&mut size));
            (read == ERROR_SUCCESS).then(|| {
                let units: Vec<u16> = buffer.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                String::from_utf16_lossy(&units).trim_end_matches('\0').to_string()
            })
        });
        let _ = RegCloseKey(key);
        value.flatten()
    }
}

fn key_exists(subkey: &str) -> bool {
    let subkey = HSTRING::from(subkey);
    let mut key = HKEY::default();
    // SAFETY: a valid wide string; the key is closed again.
    unsafe {
        let ok = RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, KEY_QUERY_VALUE, &mut key) == ERROR_SUCCESS;
        if ok {
            let _ = RegCloseKey(key);
        }
        ok
    }
}

/// Deletes a key and everything under it. Only ever called with the keys above, which are built from an identity.
fn delete_tree(subkey: &str) -> io::Result<bool> {
    // SAFETY: a valid wide string for the call.
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(subkey)) };
    if result == ERROR_SUCCESS {
        Ok(true)
    } else if result == ERROR_FILE_NOT_FOUND {
        Ok(false)
    } else {
        Err(io::Error::from_raw_os_error(result.0 as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_identity_is_the_fixed_one_and_a_dev_identity_is_never_the_release_one() {
        let release = Identity::release();
        assert_eq!(
            (release.aumid.as_str(), release.clsid.as_str()),
            ("app.kinrelay.desktop.phone", "{6B0A1D3E-2F54-4E8A-9C71-3D5B8E2A4F90}")
        );
        assert_eq!(release.clsid_u128(), 0x6B0A1D3E_2F54_4E8A_9C71_3D5B8E2A4F90);
        let dev = Identity::dev("0123456789ab");
        assert_eq!(dev.kind, Kind::Dev);
        assert_eq!(dev.aumid, "app.kinrelay.desktop.dev-0123456789ab.phone");
        assert_ne!(dev.clsid, release.clsid);
        assert_ne!(dev.aumid, release.aumid);
        assert_eq!(Identity::dev("0123456789ab"), dev, "the same key, the same identity");
        assert_ne!(Identity::dev("0123456789ac").clsid, dev.clsid);
        assert!(dev.clsid.starts_with('{') && dev.clsid.ends_with('}') && dev.clsid.len() == 38);
        assert_eq!(dev.clsid_u128().to_be_bytes()[6] >> 4, 4, "a version 4 GUID");
    }

    #[test]
    fn the_configuration_decides_which_identity_runs() {
        let installed =
            crate::cli::Config { installed_data_dir: Some("C:\\x\\Kin".into()), ..crate::cli::Config::new("C:\\x\\Kin".into()) };
        assert_eq!(Identity::for_config(&installed), Identity::release());
        let other =
            crate::cli::Config { installed_data_dir: Some("C:\\x\\Kin".into()), ..crate::cli::Config::new("C:\\tmp\\other".into()) };
        let id = Identity::for_config(&other);
        assert_eq!(id.kind, Kind::Dev);
        assert!(id.aumid.contains(".dev-"));
    }

    #[test]
    fn registering_writes_exactly_the_recipe_and_unregistering_removes_every_key_including_the_ones_windows_made() {
        // A throwaway identity (its own AUMID and CLSID), never the installed Kin's. The finally removes everything even if an assertion fails.
        let id = Identity::dev(&format!("unittest{}", std::process::id()));
        assert_eq!(id.kind, Kind::Dev);
        assert_ne!(id.aumid, RELEASE_AUMID);
        let result = std::panic::catch_unwind(|| {
            let exe = Path::new(r"C:\Program Files\Kin\Kin.exe");
            register(&id, exe, Path::new(r"C:\Users\x\AppData\Roaming\Kin\notif-app-icon.png")).unwrap();
            let (command, activator) = registered(&id);
            assert_eq!(command.as_deref(), Some(r#""C:\Program Files\Kin\Kin.exe" --toast-activated"#), "quoted: a space in the path");
            assert_eq!(activator.as_deref(), Some(id.clsid.as_str()));
            assert_eq!(
                get_string(&format!(r"Software\Classes\AppUserModelId\{}", id.aumid), Some("DisplayName")).as_deref(),
                Some(DISPLAY_NAME)
            );
            // What Windows adds by itself the first time a toast is shown under the AUMID.
            for os in id.os_owned() {
                set_string(&os, Some("Enabled"), "1").unwrap();
            }
            assert_eq!(leftovers(&id).len(), 4);
            assert_eq!(unregister(&id).unwrap(), 4);
            assert!(leftovers(&id).is_empty(), "{:?}", leftovers(&id));
            assert_eq!(unregister(&id).unwrap(), 0, "a second time is fine");
        });
        let _ = unregister(&id);
        assert!(leftovers(&id).is_empty());
        result.unwrap();
    }

    #[test]
    fn the_uninstaller_removes_exactly_the_installed_kins_two_keys() {
        let hooks = include_str!("../../windows/hooks.nsh");
        for key in [format!(r"Software\Classes\CLSID\{RELEASE_CLSID}"), format!(r"Software\Classes\AppUserModelId\{RELEASE_AUMID}")] {
            assert!(hooks.contains(&format!("DeleteRegKey HKCU \"{key}\"")), "{key} is removed in the installer's own context");
            assert!(hooks.contains(&format!("reg delete HKCU\\{key} /f")), "{key} is removed in the signed-in user's context");
        }
        // Nothing of the OS's own (the user's notification choices) and nothing of a dev identity is deleted (comments may name them).
        let deletes: Vec<&str> = hooks.lines().filter(|l| l.trim_start().starts_with("DeleteRegKey") || l.contains("reg delete")).collect();
        assert!(
            deletes.iter().all(|l| !l.contains("Notifications\\Settings") && !l.contains("PushNotifications") && !l.contains(".dev-")),
            "{deletes:?}"
        );
    }

    #[test]
    fn the_installed_kins_os_owned_keys_are_never_removed() {
        // Only the keys of a dev identity are deleted by `unregister`; the list is built in one place and the release one has two keys of its own.
        let release = Identity::release();
        let (clsid, _, aumid) = release.classes();
        assert_eq!(clsid, format!(r"Software\Classes\CLSID\{RELEASE_CLSID}"));
        assert_eq!(aumid, r"Software\Classes\AppUserModelId\app.kinrelay.desktop.phone");
        // (Not called here: that would write to the user's real registry.)
        assert_eq!(release.kind, Kind::Release);
    }
}
