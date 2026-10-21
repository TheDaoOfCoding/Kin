//! The native messaging host manifests and the `HKCU` keys that tell the browsers where they are (docs/extension.md, "Host manifests" and
//! "Registry keys"; docs/windows.md, "Install").
//!
//! **Who writes them.** The installer is per machine and runs elevated, so its `HKCU` can be another account's. Kin therefore writes the keys
//! itself, as the signed-in user, **at every start** (like the "Send to" shortcut and the Run value in `integration.rs`), pointing at manifests it
//! writes into its own data folder (`%APPDATA%\Kin\native-messaging`), with the path of `kin-native-host.exe` next to the running `Kin.exe`. A moved
//! install repairs itself at the next start, and a new extension ID is one edit of [`STORE_ORIGINS`]. The uninstaller removes the keys in both
//! contexts (`windows/hooks.nsh`).
//!
//! **Only the release host name, only a real run.** [`may_register_release`] says when: a release build, running the way a user runs it, not from a
//! build folder, with the host exe next to it. Every other Kin (a debug build, a developer setting, a data directory of its own, any test) never
//! writes these keys. The dev host name `app.kinrelay.dev` has no keys of Kin's: a test that needs one writes it itself and removes it in a `finally`.
//! The functions take the registry base (`Software` for the real thing) so that the unit tests write below a throwaway key of their own, never
//! under a browser's.
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

use super::{HostKind, RELEASE_HOST_NAME};

/// The extension ID that the manifest `key` of the extension gives it (a stable dev ID: docs/extension.md, "Names"). Side-loaded builds use it.
pub const DEV_EXTENSION_ID: &str = "cbioefpddjpaionhljhffgbedncfchmn";
/// The Firefox extension ID (`browser_specific_settings.gecko.id`).
pub const FIREFOX_EXTENSION_ID: &str = "kin@kinrelay.app";

/// TODO(store): the extension IDs of the Chrome Web Store and of Edge Add-ons go here when the extension is published (they differ from the dev
/// ID and need the maintainers' store developer accounts). **Edge reads only the first manifest it finds**, so this one
/// file must list every ID, which is why they are all in one list. Each entry is the bare ID (32 characters from `a` to `p`).
pub const STORE_ORIGINS: &[&str] = &[];

/// The Chromium-family browsers that have a key of their own, as `Software\<parent>`. Opera shares the Chrome key (reported, **not verified**: the
/// user checks it on real hardware at v0.2.0), so it has none here.
pub const CHROMIUM_PARENTS: &[&str] = &["Google\\Chrome", "Chromium", "Microsoft\\Edge", "BraveSoftware\\Brave-Browser", "Vivaldi"];
/// Firefox's key.
pub const FIREFOX_PARENT: &str = "Mozilla";

/// The `allowed_origins` of the Chromium manifest: the dev ID and every store ID, each as `chrome-extension://<id>/` (no wildcards: Chromium
/// refuses them; the trailing slash is required).
pub fn allowed_origins() -> Vec<String> {
    std::iter::once(DEV_EXTENSION_ID).chain(STORE_ORIGINS.iter().copied()).map(|id| format!("chrome-extension://{id}/")).collect()
}

const DESCRIPTION: &str = "Kin Relay: send things from the browser to your phone";

/// The Chromium manifest (`app.kinrelay.desktop.chromium.json`). `path` is absolute: the file is in the data folder, the host is in the install
/// folder.
pub fn chromium_manifest(host_name: &str, host_exe: &Path, origins: &[String]) -> String {
    serde_json::to_string_pretty(&json!({
        "name": host_name,
        "description": DESCRIPTION,
        "path": host_exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": origins,
    }))
    .expect("a manifest is JSON")
}

/// The Firefox manifest (`app.kinrelay.desktop.firefox.json`): `allowed_extensions` holds IDs, not origins.
pub fn firefox_manifest(host_name: &str, host_exe: &Path) -> String {
    serde_json::to_string_pretty(&json!({
        "name": host_name,
        "description": DESCRIPTION,
        "path": host_exe.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID],
    }))
    .expect("a manifest is JSON")
}

/// Where Kin keeps the manifests.
pub fn manifest_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("native-messaging")
}

pub fn chromium_manifest_name(host_name: &str) -> String {
    format!("{host_name}.chromium.json")
}

pub fn firefox_manifest_name(host_name: &str) -> String {
    format!("{host_name}.firefox.json")
}

/// Whether this run may write the **release** keys: a release build that runs the way a user runs it (`Config::autostart_is_real`), not from
/// a build folder, with the host exe next to it. (The installed Kin is the one that has `kin-native-host.exe` beside it.)
pub fn may_register_release(config: &crate::cli::Config, kin_exe: &Path) -> bool {
    HostKind::for_config(config) == HostKind::Release
        && !crate::integration::is_build_folder(kin_exe)
        && host_exe_next_to(kin_exe).is_file()
}

/// `kin-native-host.exe` in the folder of `Kin.exe`.
pub fn host_exe_next_to(kin_exe: &Path) -> PathBuf {
    kin_exe.parent().map_or_else(|| PathBuf::from("kin-native-host.exe"), |d| d.join("kin-native-host.exe"))
}

/// What a registration did, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub manifests_written: usize,
    pub keys_written: usize,
    pub keys_already_right: usize,
}

fn write_if_different(path: &Path, text: &str) -> io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    std::fs::write(path, text)?;
    Ok(true)
}

/// Writes both manifests for `host_name` into `dir`, pointing at `host_exe`. Returns their paths and how many files changed.
pub fn write_manifests(dir: &Path, host_name: &str, host_exe: &Path) -> io::Result<(PathBuf, PathBuf, usize)> {
    std::fs::create_dir_all(dir)?;
    let chromium = dir.join(chromium_manifest_name(host_name));
    let firefox = dir.join(firefox_manifest_name(host_name));
    let mut changed = 0;
    changed += write_if_different(&chromium, &chromium_manifest(host_name, host_exe, &allowed_origins()))? as usize;
    changed += write_if_different(&firefox, &firefox_manifest(host_name, host_exe))? as usize;
    Ok((chromium, firefox, changed))
}

/// The keys that belong to `host_name` under `base` (`Software` for the real thing): five Chromium-family keys pointing at the Chromium manifest
/// and Firefox's pointing at the Firefox manifest. Nothing else is ever written or removed.
pub fn host_keys(base: &str, host_name: &str, chromium_manifest: &Path, firefox_manifest: &Path) -> Vec<(String, PathBuf)> {
    let mut keys: Vec<(String, PathBuf)> = CHROMIUM_PARENTS
        .iter()
        .map(|p| (format!(r"{base}\{p}\NativeMessagingHosts\{host_name}"), chromium_manifest.to_path_buf()))
        .collect();
    keys.push((format!(r"{base}\{FIREFOX_PARENT}\NativeMessagingHosts\{host_name}"), firefox_manifest.to_path_buf()));
    keys
}

fn read_default(subkey: &str) -> Option<String> {
    let subkey = HSTRING::from(subkey);
    let mut key = HKEY::default();
    // SAFETY: valid wide strings and buffers for each call; the key is closed on every path.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, KEY_QUERY_VALUE, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let mut size = 0u32;
        let mut kind = REG_SZ;
        let probe = RegQueryValueExW(key, PCWSTR::null(), None, Some(&mut kind), None, Some(&mut size));
        let value = (probe == ERROR_SUCCESS && kind == REG_SZ && size > 0).then(|| {
            let mut buffer = vec![0u8; size as usize];
            let read = RegQueryValueExW(key, PCWSTR::null(), None, None, Some(buffer.as_mut_ptr()), Some(&mut size));
            (read == ERROR_SUCCESS).then(|| {
                let units: Vec<u16> = buffer.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                String::from_utf16_lossy(&units).trim_end_matches('\0').to_string()
            })
        });
        let _ = RegCloseKey(key);
        value.flatten()
    }
}

fn write_default(subkey: &str, value: &str) -> io::Result<()> {
    let subkey_w = HSTRING::from(subkey);
    let bytes: Vec<u8> = value.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    let mut key = HKEY::default();
    // SAFETY: valid wide strings and buffers; the key is closed on every path.
    unsafe {
        let created = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey_w,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if created != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(created.0 as i32));
        }
        let set = RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(&bytes));
        let _ = RegCloseKey(key);
        if set != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(set.0 as i32));
        }
    }
    Ok(())
}

/// Deletes exactly one key (and nothing under it: these keys have no subkeys). A key that isn't there is fine.
fn delete_key(subkey: &str) -> io::Result<bool> {
    // SAFETY: a valid wide string for the call.
    let result = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(subkey)) };
    if result == ERROR_SUCCESS {
        Ok(true)
    } else if result == ERROR_FILE_NOT_FOUND {
        Ok(false)
    } else {
        Err(io::Error::from_raw_os_error(result.0 as i32))
    }
}

/// Writes the manifests and the keys of `kind` under `base`. The keys are only written when their value is not already right. **The real base
/// (`Software`) is for [`HostKind::Release`] after [`may_register_release`] said yes**; the dev name has no keys of Kin's own, so a caller that
/// passes `Dev` and `Software` is refused.
pub fn register(kind: HostKind, base: &str, data_dir: &Path, host_exe: &Path) -> io::Result<Report> {
    if kind == HostKind::Dev && base == "Software" {
        return Err(io::Error::other("Kin never writes browser keys for the dev host name"));
    }
    let name = kind.host_name();
    let (chromium, firefox, changed) = write_manifests(&manifest_dir(data_dir), name, host_exe)?;
    let mut report = Report { manifests_written: changed, ..Report::default() };
    for (key, manifest) in host_keys(base, name, &chromium, &firefox) {
        let wanted = manifest.to_string_lossy().into_owned();
        if read_default(&key).as_deref() == Some(wanted.as_str()) {
            report.keys_already_right += 1;
        } else {
            write_default(&key, &wanted)?;
            report.keys_written += 1;
        }
    }
    Ok(report)
}

/// Removes the keys of `kind` under `base` (and nothing else under the browsers' keys: the empty parents stay, as the browsers own them).
/// Returns how many keys were removed.
pub fn unregister(kind: HostKind, base: &str) -> io::Result<usize> {
    if kind == HostKind::Dev && base == "Software" {
        return Err(io::Error::other("Kin never touches browser keys for the dev host name"));
    }
    let mut removed = 0;
    for (key, _) in host_keys(base, kind.host_name(), Path::new(""), Path::new("")) {
        removed += delete_key(&key)? as usize;
    }
    Ok(removed)
}

/// The value of one key, for the tests and the log.
pub fn key_value(subkey: &str) -> Option<String> {
    read_default(subkey)
}

/// Deletes a whole throwaway tree under `HKCU` (a test's base). Refuses anything that isn't under `Software\KinRelayUnitTest`.
pub fn delete_test_tree(base: &str) -> io::Result<()> {
    if !base.starts_with(r"Software\KinRelayUnitTest\") {
        return Err(io::Error::other("only a throwaway test tree can be deleted"));
    }
    use windows::Win32::System::Registry::RegDeleteTreeW;
    // SAFETY: a valid wide string for the call.
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(base)) };
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(io::Error::from_raw_os_error(result.0 as i32)) }
}

/// The release host name, for the installer's sake: the keys it removes are exactly [`host_keys`] of this name.
pub const fn release_host_name() -> &'static str {
    RELEASE_HOST_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    fn throwaway() -> String {
        format!(r"Software\KinRelayUnitTest\{}", crate::encoding::uuid_v7(crate::encoding::unix_ms()))
    }

    #[test]
    fn the_chromium_manifest_lists_the_dev_id_and_every_store_id_in_one_file() {
        let m = chromium_manifest("app.kinrelay.desktop", Path::new(r"C:\Program Files\Kin\kin-native-host.exe"), &allowed_origins());
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["name"], "app.kinrelay.desktop");
        assert_eq!(v["type"], "stdio");
        assert_eq!(v["path"], r"C:\Program Files\Kin\kin-native-host.exe");
        assert_eq!(v["allowed_origins"][0], "chrome-extension://cbioefpddjpaionhljhffgbedncfchmn/", "trailing slash, no wildcard");
        assert_eq!(v["allowed_origins"].as_array().unwrap().len(), 1 + STORE_ORIGINS.len());
        assert!(!m.contains('*'), "Chromium refuses wildcards");
        assert!(v.get("allowed_extensions").is_none());
    }

    #[test]
    fn the_firefox_manifest_has_allowed_extensions_not_origins() {
        let m = firefox_manifest("app.kinrelay.desktop", Path::new(r"C:\Kin\kin-native-host.exe"));
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["allowed_extensions"][0], "kin@kinrelay.app");
        assert!(v.get("allowed_origins").is_none());
    }

    #[test]
    fn the_keys_are_five_chromium_ones_and_firefox_and_nothing_else() {
        let keys = host_keys("Software", "app.kinrelay.desktop", Path::new("c.json"), Path::new("f.json"));
        let names: Vec<&str> = keys.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                r"Software\Google\Chrome\NativeMessagingHosts\app.kinrelay.desktop",
                r"Software\Chromium\NativeMessagingHosts\app.kinrelay.desktop",
                r"Software\Microsoft\Edge\NativeMessagingHosts\app.kinrelay.desktop",
                r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\app.kinrelay.desktop",
                r"Software\Vivaldi\NativeMessagingHosts\app.kinrelay.desktop",
                r"Software\Mozilla\NativeMessagingHosts\app.kinrelay.desktop",
            ]
        );
        assert!(keys[..5].iter().all(|(_, m)| m == Path::new("c.json")) && keys[5].1 == Path::new("f.json"));
    }

    #[test]
    fn the_dev_host_name_never_writes_or_removes_real_browser_keys() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("kin-native-host.exe");
        assert!(register(HostKind::Dev, "Software", dir.path(), &exe).is_err());
        assert!(unregister(HostKind::Dev, "Software").is_err());
        assert!(!manifest_dir(dir.path()).exists(), "nothing was written");
        // And only a throwaway tree can be deleted by the test helper.
        assert!(delete_test_tree("Software\\Google\\Chrome").is_err());
        assert!(delete_test_tree("Software").is_err());
    }

    #[test]
    fn registering_writes_manifests_and_keys_once_and_unregistering_removes_exactly_them() {
        let base = throwaway();
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("kin-native-host.exe");
        // The test runs the release flavor under its own base; the finally removes the whole tree.
        let result = std::panic::catch_unwind(|| {
            let first = register(HostKind::Release, &base, dir.path(), &exe).unwrap();
            assert_eq!(first, Report { manifests_written: 2, keys_written: 6, keys_already_right: 0 });
            let chromium = manifest_dir(dir.path()).join("app.kinrelay.desktop.chromium.json");
            let firefox = manifest_dir(dir.path()).join("app.kinrelay.desktop.firefox.json");
            assert!(chromium.is_file() && firefox.is_file());
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&chromium).unwrap()).unwrap();
            assert_eq!(v["path"], exe.to_string_lossy().as_ref());
            for (key, manifest) in host_keys(&base, "app.kinrelay.desktop", &chromium, &firefox) {
                assert_eq!(key_value(&key).as_deref(), Some(manifest.to_string_lossy().as_ref()), "{key}");
            }
            // A second start changes nothing.
            let again = register(HostKind::Release, &base, dir.path(), &exe).unwrap();
            assert_eq!(again, Report { manifests_written: 0, keys_written: 0, keys_already_right: 6 });
            // A moved install repairs itself: the host path changes, the keys stay right (they point at the manifests), the manifests change.
            let moved = dir.path().join("moved").join("kin-native-host.exe");
            let repaired = register(HostKind::Release, &base, dir.path(), &moved).unwrap();
            assert_eq!((repaired.manifests_written, repaired.keys_written), (2, 0));
            // A key that was changed by hand is put right.
            let (some_key, _) = host_keys(&base, "app.kinrelay.desktop", &chromium, &firefox).remove(2);
            write_default(&some_key, r"C:\elsewhere.json").unwrap();
            assert_eq!(register(HostKind::Release, &base, dir.path(), &moved).unwrap().keys_written, 1);
            // Something else under the same parents survives the removal.
            let neighbour = format!(r"{base}\Google\Chrome\NativeMessagingHosts\com.someone.else");
            write_default(&neighbour, "x").unwrap();
            assert_eq!(unregister(HostKind::Release, &base).unwrap(), 6);
            assert_eq!(unregister(HostKind::Release, &base).unwrap(), 0, "a second removal finds nothing");
            for (key, _) in host_keys(&base, "app.kinrelay.desktop", &chromium, &firefox) {
                assert_eq!(key_value(&key), None, "{key}");
            }
            assert_eq!(key_value(&neighbour).as_deref(), Some("x"), "another program's host is not touched");
        });
        delete_test_tree(&base).unwrap();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
        assert_eq!(key_value(&format!(r"{base}\Mozilla\NativeMessagingHosts\app.kinrelay.desktop")), None, "the test tree is gone");
    }

    #[test]
    fn the_dev_name_can_be_registered_under_a_throwaway_base_for_the_helpers_sake() {
        // The helper API works for the dev name too, but only below a base that is not `Software` (the smoke tests write the real dev key
        // themselves, in the one place a browser reads, and remove it in a `finally`).
        let base = throwaway();
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("kin-native-host.exe");
        let result = std::panic::catch_unwind(|| {
            let report = register(HostKind::Dev, &base, dir.path(), &exe).unwrap();
            assert_eq!(report.keys_written, 6);
            let key = format!(r"{base}\Chromium\NativeMessagingHosts\app.kinrelay.dev");
            assert!(key_value(&key).unwrap().ends_with("app.kinrelay.dev.chromium.json"));
            assert!(!key.contains("desktop"));
        });
        delete_test_tree(&base).unwrap();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn a_test_or_debug_kin_may_not_register_the_release_keys() {
        let config = crate::cli::Config::new(PathBuf::from(r"C:\x"));
        let exe = Path::new(r"C:\Program Files\Kin\Kin.exe");
        assert!(!may_register_release(&config, exe), "debug build / no host exe beside it");
        let mut dev = config.clone();
        dev.dev_no_windows = true;
        assert!(!may_register_release(&dev, exe));
    }
}
