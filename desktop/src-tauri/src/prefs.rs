//! What the settings window changes (milestone 1.4): the device name (sent in `hello`), automatic clipboard sync, images to the clipboard,
//! the received folder, the hotkey (the window registers it, see `ui.rs`) and unpairing. The choice is written to `settings.json` at once,
//! and applies at once: the received folder to the next transfer, the name to the next `hello` (and to the open connection, which gets
//! a new one), the images switch to the next image that arrives.
//!
//! **Start with Windows** is not a setting of this file: its state is the registry value that the autostart plugin writes. In a developer
//! run (any `--dev-*` flag, or a `--data-dir` of its own) it is a file in the data directory instead and **the registry is never touched**
//! (see `autostart.rs`).
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::core::{Core, Event, lock};
use crate::settings::{Settings, clean_device_name};

/// A folder that files can be received into: it exists (or can be made) and a file can be written there. Returns the reason if not.
pub fn check_received_folder(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Choose a full folder path, such as D:\\Received.".into());
    }
    std::fs::create_dir_all(path).map_err(|e| format!("That folder can't be used: {e}"))?;
    if !path.is_dir() {
        return Err("That is not a folder.".into());
    }
    // A file can really be written there (a read-only folder, or one that another user owns, fails here and not at the first transfer).
    let probe = path.join(format!(".kin-write-test-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|e| format!("Files can't be saved in that folder: {e}"))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

impl Core {
    fn save_settings(&self, what: &str) {
        let settings = lock(&self.settings).clone();
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json ({what}) failed: {e}");
        }
    }

    /// Sets the name this PC shows to the phone. An empty name means the computer's own. Returns the name now in use. The phone that is
    /// connected gets a new `hello` with it.
    pub fn set_device_name(&self, name: &str) -> String {
        let custom = clean_device_name(name);
        let shown = custom.clone().unwrap_or_else(|| self.default_pc_name().to_string());
        let changed = {
            let mut s = lock(&self.settings);
            let changed = s.device_name != custom;
            s.device_name = custom;
            changed
        };
        self.set_shown_pc_name(shown.clone());
        if changed {
            log::info!("the device name was changed ({} characters)", shown.chars().count());
            self.save_settings("device name");
            self.connections.send_hello_to_all();
            self.emit(Event::StateChanged);
        }
        shown
    }

    /// Sets (or, with `None`, clears) the folder for received files. The folder is checked first, and the old one stays if it is no good.
    pub fn set_received_folder(&self, folder: Option<&str>) -> Result<PathBuf, String> {
        let chosen = folder.map(str::trim).filter(|f| !f.is_empty());
        if let Some(f) = chosen {
            check_received_folder(Path::new(f))?;
        }
        lock(&self.settings).received_folder = chosen.map(str::to_string);
        log::info!("the received folder was {}", if chosen.is_some() { "changed" } else { "reset to the default" });
        self.save_settings("received folder");
        self.emit(Event::StateChanged);
        Ok(self.received_dir())
    }

    /// Remembers the hotkey after the window has registered it (the registration itself needs the app, see `ui::set_hotkey`).
    pub fn store_hotkey(&self, accelerator: &str) {
        lock(&self.settings).hotkey = accelerator.to_string();
        self.save_settings("hotkey");
    }

    /// What the settings window shows.
    pub fn settings_view(&self, start_with_windows: Option<bool>) -> Value {
        let s: Settings = lock(&self.settings).clone();
        let hotkey = lock(&self.hotkey).clone();
        let snap = self.snapshot();
        json!({
            "deviceName": self.pc_name(),
            "customDeviceName": s.device_name,
            "defaultDeviceName": self.default_pc_name(),
            "autoClipboardSync": s.auto_clipboard_sync,
            "imagesToClipboard": s.images_to_clipboard,
            "receivedFolder": self.received_dir().to_string_lossy(),
            "receivedFolderIsDefault": s.received_folder.is_none() && self.config.dev_received_dir.is_none(),
            "hotkey": { "accelerator": hotkey.accelerator, "registered": hotkey.registered, "error": hotkey.error },
            "defaultHotkey": crate::settings::DEFAULT_HOTKEY,
            "startWithWindows": start_with_windows,
            "devices": snap.devices,
            "version": crate::core::APP_VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_can_be_written_to_is_accepted_and_a_relative_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_received_folder(&dir.path().join("new").join("deeper")).is_ok(), "a folder that doesn't exist yet is made");
        assert!(dir.path().join("new").join("deeper").is_dir());
        assert!(check_received_folder(Path::new("relative\\folder")).unwrap_err().contains("full folder path"));
        // A file in the way is not a folder.
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        assert!(check_received_folder(&file).is_err());
        // The probe file is gone again.
        let left: Vec<_> = std::fs::read_dir(dir.path().join("new").join("deeper")).unwrap().collect();
        assert!(left.is_empty(), "the write test leaves nothing behind");
    }
}
