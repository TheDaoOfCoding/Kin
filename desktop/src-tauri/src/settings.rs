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
use tauri_plugin_global_shortcut::{Modifiers, Shortcut};

use crate::features::Overrides;
use crate::presence::pause::PauseState;

/// Why this one: see windows.md, "Hotkey". Three modifiers and a letter that no common Windows or app shortcut uses.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Shift+K";

/// Reads the `hotkey` setting and refuses what would take a key away from every program. The rules (windows.md, "Hotkey"):
///
/// * It needs **Ctrl, Alt or Win** in it, or it is a function key (F1 to F24) alone. A letter on its own, or with Shift only, is typing:
///   `Shift+K` would turn every capital K into a send.
/// * It must not be a **common shortcut** that programs and Windows use: Ctrl+C, V, X, Z, Y, A, S, N, O, P, F, W, T, R; Ctrl+Shift+Z, T, N and
///   Esc; Alt+F4, Tab, Esc, Space, Enter and the arrows; Ctrl+Alt+Delete; and every combination whose only modifier is Win (Windows keeps
///   Win+L, Win+D, Win+E, Win+R, Win+Tab and most of the others for itself).
///
/// The message says why, and what to try.
pub fn parse_hotkey(accelerator: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = accelerator.parse().map_err(|e| format!("{accelerator:?} is not a hotkey ({e})"))?;
    let mods = shortcut.mods;
    let key = format!("{:?}", shortcut.key);
    let is_function_key = key.strip_prefix('F').is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)));
    let strong = mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER);
    if !strong && !(mods.is_empty() && is_function_key) {
        return Err(format!(
            "{accelerator:?} needs Ctrl, Alt or Win in it (or be a function key on its own). Without one the key can't be typed any more in any program. Try {DEFAULT_HOTKEY}."
        ));
    }
    if let Some(what) = common_shortcut(mods, &key) {
        return Err(format!("{accelerator:?} is {what}, which other programs use, so it can't be the hotkey. Try {DEFAULT_HOTKEY}."));
    }
    Ok(shortcut)
}

/// What a common shortcut is for, if `mods` + `key` (the `Debug` name of the key code: `KeyC`, `F4`, `Tab`) is one of them.
fn common_shortcut(mods: Modifiers, key: &str) -> Option<String> {
    let is_function_key = key.strip_prefix('F').is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)));
    let letter = key.strip_prefix("Key").filter(|l| l.len() == 1).and_then(|l| l.chars().next());
    let only = |m: Modifiers| mods == m;
    if only(Modifiers::CONTROL) {
        let what = match letter? {
            'C' => "Copy",
            'V' => "Paste",
            'X' => "Cut",
            'Z' => "Undo",
            'Y' => "Redo",
            'A' => "Select all",
            'S' => "Save",
            'N' => "New",
            'O' => "Open",
            'P' => "Print",
            'F' => "Find",
            'W' => "Close",
            'T' => "New tab",
            'R' => "Reload",
            _ => return None,
        };
        return Some(format!("the shortcut for {what}"));
    }
    let what = if only(Modifiers::CONTROL | Modifiers::SHIFT) {
        match (letter, key) {
            (Some('Z'), _) => "the shortcut for Redo",
            (Some('T'), _) => "the shortcut for reopening a tab",
            (Some('N'), _) => "the shortcut for a new window",
            (_, "Escape") => "the shortcut for the Task Manager",
            _ => return None,
        }
    } else if only(Modifiers::ALT) {
        match key {
            "F4" => "the shortcut that closes a window",
            "Tab" | "Escape" => "the shortcut that switches windows",
            "Space" => "the shortcut for the window menu",
            "Enter" => "the shortcut for Properties",
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => "a shortcut for going back and forward",
            _ => return None,
        }
    } else if only(Modifiers::CONTROL | Modifiers::ALT) && key == "Delete" {
        "the secure attention sequence of Windows"
    } else if only(Modifiers::SUPER) && !is_function_key {
        "a Windows shortcut (Windows keeps Win + a key for itself: Win+L locks the PC, Win+D shows the desktop, ...)"
    } else {
        return None;
    };
    Some(what.to_string())
}

pub const FILE_NAME: &str = "settings.json";

/// The longest device name that is kept (protocol.md: a name longer than 64 characters is cut for display, so the settings keep 64).
pub const MAX_DEVICE_NAME_CHARS: usize = 64;

/// A device name as the settings keep it: trimmed, without control characters, at most 64 characters. Empty means "use the computer's name".
pub fn clean_device_name(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned = crate::encoding::truncate_chars(cleaned.trim(), MAX_DEVICE_NAME_CHARS).trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Automatic clipboard sync, PC to phone. On by default (D11, windows.md).
    pub auto_clipboard_sync: bool,
    /// The accelerator of the global hotkey that sends the clipboard text as a `text` message. Empty means off.
    pub hotkey: String,
    /// Where files from the phone go. `None` is `Downloads\Kin`.
    pub received_folder: Option<String>,
    /// A received image also goes to the clipboard, so Ctrl+V works at once (on by default).
    pub images_to_clipboard: bool,
    /// The name this PC shows to the phone (1.4, the settings window). `None`: the computer's own name.
    pub device_name: Option<String>,
    /// "Open links from phone automatically" (2B.1, on by default): a `link` from the phone opens in the default browser at once. Off: a toast
    /// with Open and Copy. A fullscreen game or a presentation always gets the toast (phonelinks.rs).
    pub open_links_automatically: bool,
    /// "Show phone notifications" (2B.2, **off by default**): the phone's notifications appear as toasts on this PC. While it is on the PC lists the `notif`
    /// capability in its `hello`.
    pub mirror_notifications: bool,
    /// "Play a sound for phone notifications" (off: the phone has rung already).
    pub mirror_sound: bool,
    /// "Hide notification content" (off): every phone notification is shown as "Private notification".
    pub mirror_hide_content: bool,
    /// "Closing a phone notification here also dismisses it on the phone" (on): pressing X on a toast sends `notif.dismiss`.
    pub mirror_dismiss_to_phone: bool,
    /// The fallback of decision 2 (off): text only on the toasts, no reply box and no buttons, for a Windows where the activator doesn't work.
    pub mirror_text_only: bool,
    /// "Share this PC's media with the phone" (2B.3, **off by default**): what plays on the PC shows in the phone's media controls (and the phone's buttons and
    /// volume keys can control it).
    pub media_share: bool,
    /// "Show the phone's media on this PC" (2B.3, **off by default**): what plays on the phone shows in the Windows media flyout and the tray panel.
    pub media_show: bool,
    /// "Share this PC's battery with the phone" (2B.4, on by default). A PC with no battery shares nothing whatever this says.
    pub presence_share_battery: bool,
    /// "Show my phone's battery here" (2B.4, on by default): the tray tooltip, the status line and the drop window.
    pub presence_show_phone: bool,
    /// "Warn me when the phone's battery is low" (2B.4, on by default): one toast when it drops to 15 % or less and is not charging.
    pub low_battery_nudge: bool,
    /// "Silence my phone while this PC is in Focus" (2B.5, **off by default**): Kin reads whether Windows Focus or Do Not Disturb is on (it never changes them) and tells the
    /// phone, which may turn on its own Do Not Disturb by its own switch. While off nothing is registered, nothing is polled and `focus` is never in a snapshot.
    pub focus_share: bool,
    /// "Also when I'm presenting or playing full screen" (2B.5, **off by default**): a full-screen or presenting state counts as Focus.
    pub focus_also_presenting: bool,
    /// "Hold Kin's own toasts while my phone is in Do Not Disturb" (2B.5, on by default): the low-battery nudge and the informational toasts wait.
    pub hold_toasts_on_phone_dnd: bool,
    /// Pause Kin (2B.4): this PC's own state. Kept in the file, so it survives a restart and a reboot. `None`: not paused. (The gate compares the clock on
    /// each use: an end that has passed is simply no pause.)
    pub pause: Option<PauseState>,
    /// The per-peer overrides of the features (design section 4). Empty, and with no UI in 2B.4.
    pub feature_overrides: Overrides,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_clipboard_sync: true,
            hotkey: DEFAULT_HOTKEY.to_string(),
            received_folder: None,
            images_to_clipboard: true,
            device_name: None,
            open_links_automatically: true,
            mirror_notifications: false,
            mirror_sound: false,
            mirror_hide_content: false,
            mirror_dismiss_to_phone: true,
            mirror_text_only: false,
            media_share: false,
            media_show: false,
            presence_share_battery: true,
            presence_show_phone: true,
            low_battery_nudge: true,
            focus_share: false,
            focus_also_presenting: false,
            hold_toasts_on_phone_dnd: true,
            pause: None,
            feature_overrides: Overrides::new(),
        }
    }
}

impl Settings {
    /// The default of a feature (the flat key), for `features::resolve`. A feature this PC has no switch for is off.
    pub fn feature_default(&self, feature: &str) -> bool {
        match feature {
            crate::features::CLIPBOARD_SYNC => self.auto_clipboard_sync,
            crate::features::LINKS_AUTO_OPEN => self.open_links_automatically,
            crate::features::NOTIF_SHOW => self.mirror_notifications,
            crate::features::MEDIA_SHARE => self.media_share,
            crate::features::MEDIA_SHOW => self.media_show,
            crate::features::PRESENCE_SHARE => self.presence_share_battery,
            crate::features::PRESENCE_SHOW => self.presence_show_phone,
            crate::features::PRESENCE_NUDGE => self.low_battery_nudge,
            crate::features::FOCUS_SHARE => self.focus_share,
            crate::features::DND_HOLD_TOASTS => self.hold_toasts_on_phone_dnd,
            _ => false,
        }
    }

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
        match obj.get("openLinksAutomatically") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => settings.open_links_automatically = *b,
            Some(_) => problems.push("openLinksAutomatically must be true or false".to_string()),
        }
        for (key, field) in [
            ("mirrorNotifications", &mut settings.mirror_notifications),
            ("mirrorSound", &mut settings.mirror_sound),
            ("mirrorHideContent", &mut settings.mirror_hide_content),
            ("mirrorDismissToPhone", &mut settings.mirror_dismiss_to_phone),
            ("mirrorTextOnly", &mut settings.mirror_text_only),
            ("mediaShare", &mut settings.media_share),
            ("mediaShow", &mut settings.media_show),
            ("presenceShareBattery", &mut settings.presence_share_battery),
            ("presenceShowPhone", &mut settings.presence_show_phone),
            ("lowBatteryNudge", &mut settings.low_battery_nudge),
            ("focusShare", &mut settings.focus_share),
            ("focusAlsoPresenting", &mut settings.focus_also_presenting),
            ("holdToastsOnPhoneDnd", &mut settings.hold_toasts_on_phone_dnd),
        ] {
            match obj.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::Bool(b)) => *field = *b,
                Some(_) => problems.push(format!("{key} must be true or false")),
            }
        }
        match obj.get("deviceName") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => settings.device_name = clean_device_name(s),
            Some(_) => problems.push("deviceName must be a string".to_string()),
        }
        match obj.get("pause") {
            None | Some(Value::Null) => {}
            Some(v) => match PauseState::from_json(v) {
                Some(p) => settings.pause = Some(p),
                None => problems.push("pause must be {\"until\": <Unix ms>} or {\"untilResumed\": true}".to_string()),
            },
        }
        match obj.get("featureOverrides") {
            None | Some(Value::Null) => {}
            Some(v @ Value::Object(_)) => settings.feature_overrides = crate::features::overrides_from_json(v),
            Some(_) => problems.push("featureOverrides must be an object".to_string()),
        }
        (settings, problems)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "autoClipboardSync": self.auto_clipboard_sync,
            "hotkey": self.hotkey,
            "receivedFolder": self.received_folder,
            "imagesToClipboard": self.images_to_clipboard,
            "deviceName": self.device_name,
            "openLinksAutomatically": self.open_links_automatically,
            "mirrorNotifications": self.mirror_notifications,
            "mirrorSound": self.mirror_sound,
            "mirrorHideContent": self.mirror_hide_content,
            "mirrorDismissToPhone": self.mirror_dismiss_to_phone,
            "mirrorTextOnly": self.mirror_text_only,
            "mediaShare": self.media_share,
            "mediaShow": self.media_show,
            "presenceShareBattery": self.presence_share_battery,
            "presenceShowPhone": self.presence_show_phone,
            "lowBatteryNudge": self.low_battery_nudge,
            "focusShare": self.focus_share,
            "focusAlsoPresenting": self.focus_also_presenting,
            "holdToastsOnPhoneDnd": self.hold_toasts_on_phone_dnd,
            "pause": self.pause.map(|p| p.to_json()),
            "featureOverrides": crate::features::overrides_to_json(&self.feature_overrides),
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
    fn the_media_switches_are_off_by_default_and_a_wrong_type_keeps_the_default() {
        let s = Settings::default();
        assert!(!s.media_share && !s.media_show, "both directions are off until the user turns them on");
        let (s, problems) = Settings::from_json(r#"{ "mediaShare": true, "mediaShow": "yes" }"#);
        assert!(s.media_share && !s.media_show);
        assert_eq!(problems.len(), 1);
        let v = Settings::default().to_json();
        assert_eq!((v["mediaShare"].as_bool(), v["mediaShow"].as_bool()), (Some(false), Some(false)));
    }

    #[test]
    fn the_focus_switches_are_off_and_the_hold_is_on_by_default() {
        let s = Settings::default();
        assert!(
            !s.focus_share && !s.focus_also_presenting,
            "Kin reads Focus only when the user turns it on, and presenting is a second opt-in"
        );
        assert!(s.hold_toasts_on_phone_dnd);
        let v = s.to_json();
        assert_eq!(
            (v["focusShare"].as_bool(), v["focusAlsoPresenting"].as_bool(), v["holdToastsOnPhoneDnd"].as_bool()),
            (Some(false), Some(false), Some(true))
        );
        let (s, problems) = Settings::from_json(r#"{ "focusShare": true, "focusAlsoPresenting": "yes", "holdToastsOnPhoneDnd": false }"#);
        assert!(s.focus_share && !s.focus_also_presenting && !s.hold_toasts_on_phone_dnd);
        assert_eq!(problems.len(), 1);
        assert!(s.feature_default(crate::features::FOCUS_SHARE) && !s.feature_default(crate::features::DND_HOLD_TOASTS));
    }

    #[test]
    fn the_presence_switches_are_on_by_default_and_the_pause_and_overrides_are_empty() {
        let s = Settings::default();
        assert!(
            s.presence_share_battery && s.presence_show_phone && s.low_battery_nudge,
            "battery sharing and the nudge are on (decision Q5)"
        );
        assert_eq!((s.pause, s.feature_overrides.len()), (None, 0));
        let v = s.to_json();
        assert_eq!(v["pause"], Value::Null);
        assert_eq!(v["featureOverrides"], json!({}));
        let (s, problems) = Settings::from_json(
            r#"{ "pause": {"until": 1760000000000}, "lowBatteryNudge": false, "presenceShowPhone": "x", "featureOverrides": {"a": {"media.show": true}} }"#,
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(s.pause, Some(PauseState::Until(1_760_000_000_000)));
        assert!(!s.low_battery_nudge && s.presence_show_phone);
        assert!(!s.feature_default(crate::features::PRESENCE_NUDGE) && !s.feature_default("no.such.feature"));
        assert!(crate::features::resolve(crate::features::MEDIA_SHOW, Some("a"), s.media_show, &s.feature_overrides));
        let (s, problems) = Settings::from_json(r#"{ "pause": {"untilResumed": true} }"#);
        assert!(problems.is_empty() && s.pause == Some(PauseState::UntilResumed));
        let (_, problems) = Settings::from_json(r#"{ "pause": "1h" }"#);
        assert_eq!(problems.len(), 1);
    }

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
        // A bare key, or Shift and a letter, would be taken from every program: typing breaks.
        for typing in ["K", "Space", "Shift+K", "Shift+A", "Shift+Space", "Shift+F9", "Enter"] {
            let e = parse_hotkey(typing).unwrap_err();
            assert!(e.contains("needs Ctrl, Alt or Win") && e.contains(DEFAULT_HOTKEY), "{typing}: {e}");
        }
    }

    #[test]
    fn a_function_key_alone_or_with_ctrl_alt_or_win_is_fine() {
        for ok in [
            "F9",
            "F12",
            "F24",
            "Ctrl+F9",
            "Alt+F9",
            "Super+F9",
            "Ctrl+Shift+F9",
            "Ctrl+Alt+K",
            "Alt+K",
            "Ctrl+Shift+K",
            "Ctrl+Alt+Shift+K",
            "Super+Shift+K",
            "Alt+Shift+Space",
        ] {
            assert!(parse_hotkey(ok).is_ok(), "{ok}: {:?}", parse_hotkey(ok).err());
        }
    }

    #[test]
    fn common_shortcuts_are_refused_with_a_message_that_says_what_they_are() {
        let refused = [
            ("Ctrl+C", "Copy"),
            ("Ctrl+V", "Paste"),
            ("Ctrl+X", "Cut"),
            ("Ctrl+Z", "Undo"),
            ("Ctrl+Y", "Redo"),
            ("Ctrl+A", "Select all"),
            ("Ctrl+S", "Save"),
            ("ctrl+f", "Find"),
            ("Ctrl+Shift+Z", "Redo"),
            ("Ctrl+Shift+Esc", "Task Manager"),
            ("Alt+F4", "closes a window"),
            ("Alt+Tab", "switches windows"),
            ("Alt+Space", "window menu"),
            ("Alt+Enter", "Properties"),
            ("Alt+Left", "back and forward"),
            ("Ctrl+Alt+Delete", "secure attention"),
            ("Super+L", "Windows shortcut"),
            ("Super+D", "Windows shortcut"),
            ("Super+Tab", "Windows shortcut"),
            ("Super+3", "Windows shortcut"),
        ];
        for (hotkey, why) in refused {
            let e = parse_hotkey(hotkey).unwrap_err();
            assert!(e.contains(why) && e.contains("can't be the hotkey") && e.contains(DEFAULT_HOTKEY), "{hotkey}: {e}");
        }
        // The neighbours of the refused ones are fine: other letters, more modifiers.
        for ok in ["Ctrl+Alt+C", "Ctrl+Shift+C", "Ctrl+Alt+S", "Alt+Shift+F4", "Ctrl+B", "Ctrl+Alt+Super+L", "Alt+K"] {
            assert!(parse_hotkey(ok).is_ok(), "{ok}: {:?}", parse_hotkey(ok).err());
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
    fn the_device_name_is_trimmed_cut_to_64_characters_and_empty_means_the_computers_name() {
        assert_eq!(clean_device_name("  Anna's laptop \t"), Some("Anna's laptop".into()));
        assert_eq!(clean_device_name(""), None);
        assert_eq!(clean_device_name("   "), None);
        assert_eq!(clean_device_name("line\nbreak\u{7}"), Some("linebreak".into()), "control characters are dropped");
        let long = clean_device_name(&"\u{e9}".repeat(100)).unwrap();
        assert_eq!(long.chars().count(), 64, "64 characters, not bytes");
        let (s, problems) = Settings::from_json(r#"{ "deviceName": "  Desk  " }"#);
        assert!(problems.is_empty());
        assert_eq!(s.device_name.as_deref(), Some("Desk"));
        let (s, problems) = Settings::from_json(r#"{ "deviceName": 5 }"#);
        assert_eq!((s.device_name, problems.len()), (None, 1));
        assert_eq!(Settings::default().device_name, None);
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
            device_name: Some("Desk".into()),
            open_links_automatically: false,
            mirror_notifications: true,
            mirror_sound: true,
            mirror_hide_content: true,
            mirror_dismiss_to_phone: false,
            mirror_text_only: true,
            media_share: true,
            media_show: true,
            presence_share_battery: false,
            presence_show_phone: false,
            low_battery_nudge: false,
            focus_share: true,
            focus_also_presenting: true,
            hold_toasts_on_phone_dnd: false,
            pause: Some(PauseState::Until(1_760_000_000_000)),
            feature_overrides: crate::features::overrides_from_json(&json!({ "dev-a": { "clipboard.sync": false } })),
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        // A file written by Notepad starts with a byte order mark.
        std::fs::write(Settings::path(dir.path()), "\u{feff}{ \"autoClipboardSync\": false }").unwrap();
        assert!(!Settings::load(dir.path()).auto_clipboard_sync);
    }
}
