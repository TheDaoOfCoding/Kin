//! The settings that live in `settings.json` in the data directory. A settings window comes in 1.4, so for now the file is
//! the way to change them (and the tray has a check item for the clipboard sync).
//!
//! ```json
//! { "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+K", "receivedFolder": "D:\\Received", "imagesToClipboard": true }
//! ```
//!
//! A missing file or a missing key means the default. A key with the wrong type is ignored (with a log line) and keeps its
//! default, so one typo doesn't throw away the other settings. `hotkey` takes the accelerator syntax of the Tauri
//! global-shortcut plugin (`Ctrl+Alt+Shift+K`, `Ctrl+Shift+F9`); an empty string turns the hotkey off. A change of the hotkey
//! takes effect at the next start. `receivedFolder` (absent or empty: `Downloads\Kin`) is where files from the phone go, and
//! `imagesToClipboard` (on by default) puts a received image on the clipboard as well.
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use tauri_plugin_global_shortcut::Shortcut;

/// Why this one: see windows.md, "Hotkey". Three modifiers and a letter that no common Windows or app shortcut uses.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Shift+K";

/// Reads the `hotkey` setting. It must be a key with at least one modifier: a global hotkey on a bare key (a typo such as `"K"`)
/// would take that key away from every program.
pub fn parse_hotkey(accelerator: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = accelerator.parse().map_err(|e| format!("{accelerator:?} is not a hotkey ({e})"))?;
    if shortcut.mods.is_empty() {
        return Err(format!(
            "{accelerator:?} has no modifier (Ctrl, Alt or Shift), and a hotkey on a bare key would take that key from every program"
        ));
    }
    Ok(shortcut)
}

pub const FILE_NAME: &str = "settings.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Automatic clipboard sync, PC to phone. On by default (D11, windows.md).
    pub auto_clipboard_sync: bool,
    /// The accelerator of the global hotkey that sends the clipboard text as a `text` message. Empty means off.
    pub hotkey: String,
    /// Where files from the phone go. `None` is `Downloads\Kin`.
    pub received_folder: Option<String>,
    /// A received image also goes to the clipboard, so Ctrl+V works at once (on by default, protocol of phase-1-mvp.md).
    pub images_to_clipboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { auto_clipboard_sync: true, hotkey: DEFAULT_HOTKEY.to_string(), received_folder: None, images_to_clipboard: true }
    }
}

impl Settings {
    /// Reads the settings out of a JSON text. The second value lists what was wrong, for the log.
    pub fn from_json(text: &str) -> (Settings, Vec<String>) {
        let mut settings = Settings::default();
        let mut problems = Vec::new();
        let obj: Map<String, Value> = match serde_json::from_str(text) {
            Ok(Value::Object(o)) => o,
            Ok(_) => return (settings, vec!["the file is not a JSON object".to_string()]),
            Err(e) => return (settings, vec![format!("the file is not valid JSON: {e}")]),
        };
        match obj.get("autoClipboardSync") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => settings.auto_clipboard_sync = *b,
            Some(_) => problems.push("autoClipboardSync must be true or false".to_string()),
        }
        match obj.get("hotkey") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => settings.hotkey = s.trim().to_string(),
            Some(_) => problems.push("hotkey must be a string such as \"Ctrl+Alt+Shift+K\"".to_string()),
        }
        match obj.get("receivedFolder") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => settings.received_folder = Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            Some(_) => problems.push("receivedFolder must be a folder path as a string".to_string()),
        }
        match obj.get("imagesToClipboard") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => settings.images_to_clipboard = *b,
            Some(_) => problems.push("imagesToClipboard must be true or false".to_string()),
        }
        (settings, problems)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "autoClipboardSync": self.auto_clipboard_sync,
            "hotkey": self.hotkey,
            "receivedFolder": self.received_folder,
            "imagesToClipboard": self.images_to_clipboard,
        })
    }

    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Loads `settings.json` from the data directory. A missing file is the defaults, silently.
    pub fn load(data_dir: &Path) -> Settings {
        let path = Settings::path(data_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                // A UTF-8 byte order mark (Notepad and PowerShell 5 write one) is not part of the JSON.
                let (settings, problems) = Settings::from_json(text.trim_start_matches('\u{feff}'));
                for p in problems {
                    log::warn!("{}: {p}; using the default", path.display());
                }
                settings
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                log::warn!("reading {} failed ({e}); using the defaults", path.display());
                Settings::default()
            }
        }
    }

    /// Writes the file (through a temporary file, so a crash never leaves half of it).
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = Settings::path(data_dir);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.to_json())?)?;
        std::fs::rename(&tmp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_and_the_documented_hotkey() {
        let s = Settings::default();
        assert!(s.auto_clipboard_sync);
        assert_eq!(s.hotkey, "Ctrl+Alt+Shift+K");
    }

    #[test]
    fn the_default_hotkey_is_a_valid_accelerator_with_three_modifiers_and_k() {
        use tauri_plugin_global_shortcut::{Code, Modifiers};
        let shortcut = parse_hotkey(DEFAULT_HOTKEY).unwrap();
        assert_eq!(shortcut.mods, Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT);
        assert_eq!(shortcut.key, Code::KeyK);
        // Other spellings people will try.
        assert_eq!(parse_hotkey("ctrl+shift+f9").unwrap().key, Code::F9);
        assert_eq!(parse_hotkey("Control+Alt+KeyP").unwrap().key, Code::KeyP);
        assert!(parse_hotkey("Ctrl+Shift+F9").is_ok());
    }

    #[test]
    fn a_hotkey_that_is_not_one_is_refused() {
        for bad in ["", "Ctrl+", "Ctrl++K", "Ctrl+Alt+Shift+Banana", "Ctrl+K+L", "not a hotkey"] {
            assert!(parse_hotkey(bad).is_err(), "{bad:?}");
        }
        // A bare key would be taken from every program.
        for bare in ["K", "F9", "Space"] {
            let e = parse_hotkey(bare).unwrap_err();
            assert!(e.contains("no modifier"), "{bare}: {e}");
        }
    }

    #[test]
    fn reads_the_keys_it_knows_and_keeps_defaults_for_the_rest() {
        let (s, problems) = Settings::from_json(r#"{ "autoClipboardSync": false, "unknown": 1 }"#);
        assert!(problems.is_empty());
        assert_eq!((s.auto_clipboard_sync, s.hotkey.as_str()), (false, DEFAULT_HOTKEY));
        let (s, _) = Settings::from_json(r#"{ "hotkey": "  Ctrl+Shift+F9 " }"#);
        assert_eq!((s.auto_clipboard_sync, s.hotkey.as_str()), (true, "Ctrl+Shift+F9"));
        let (s, _) = Settings::from_json(r#"{ "hotkey": "" }"#);
        assert_eq!(s.hotkey, "", "an empty hotkey means off");
    }

    #[test]
    fn the_received_folder_and_images_to_clipboard_settings() {
        let (s, problems) = Settings::from_json(r#"{ "receivedFolder": "  D:\\Received  ", "imagesToClipboard": false }"#);
        assert!(problems.is_empty());
        assert_eq!((s.received_folder.as_deref(), s.images_to_clipboard), (Some("D:\\Received"), false));
        let (s, _) = Settings::from_json(r#"{ "receivedFolder": "" }"#);
        assert_eq!(s.received_folder, None, "empty means the default folder");
        assert!(Settings::default().images_to_clipboard, "on by default");
        let (s, problems) = Settings::from_json(r#"{ "receivedFolder": 5, "imagesToClipboard": "yes" }"#);
        assert_eq!(problems.len(), 2);
        assert_eq!((s.received_folder, s.images_to_clipboard), (None, true));
    }

    #[test]
    fn a_wrong_type_is_reported_and_only_that_key_falls_back() {
        let (s, problems) = Settings::from_json(r#"{ "autoClipboardSync": "no", "hotkey": "Ctrl+Shift+F9" }"#);
        assert_eq!(problems.len(), 1);
        assert!(s.auto_clipboard_sync);
        assert_eq!(s.hotkey, "Ctrl+Shift+F9");
        for text in ["not json", "[1]", "42"] {
            let (s, problems) = Settings::from_json(text);
            assert_eq!(s, Settings::default(), "{text}");
            assert_eq!(problems.len(), 1, "{text}");
        }
    }

    #[test]
    fn saves_and_loads_through_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default(), "no file: the defaults");
        let s = Settings {
            auto_clipboard_sync: false,
            hotkey: "Ctrl+Shift+F9".into(),
            received_folder: Some("D:/Received".into()),
            images_to_clipboard: false,
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        // A file written by Notepad starts with a byte order mark.
        std::fs::write(Settings::path(dir.path()), "\u{feff}{ \"autoClipboardSync\": false }").unwrap();
        assert!(!Settings::load(dir.path()).auto_clipboard_sync);
    }
}
