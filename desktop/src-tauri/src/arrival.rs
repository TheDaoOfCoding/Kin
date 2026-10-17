//! What happens when files from the phone have all arrived (protocol.md and windows.md, "Receiving from the phone"): a toast
//! "<n> file(s) from <phone>" whose click opens the file (one file of a type on the allow-list: `links::may_open_file`; any other
//! file is shown in its folder) or the folder (several), and an image goes to the clipboard as well (on by default), so Ctrl+V works
//! at once.
//!
//! **Images and the clipboard.** Exactly one image in a transfer goes to the clipboard, with all the formats that make a paste work in
//! common apps (see `images.rs`). Several images leave the clipboard alone: one clipboard can't hold them as pictures, and picking
//! one at random would be a surprise. A very large image is skipped too (its pixels would take more than 64 MiB of memory), and the
//! log says why. What is an image is decided by the decoder, from the content of the file. All of this happens after the `PUT` has
//! been answered, in a task of its own.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::clipboard::ImageOutcome;
use crate::core::{Core, Event, lock, short_transfer};
use crate::links;
use crate::sync::ToastTarget;
use crate::transfers::Entry;

/// "1 file from Pixel", "3 files from Pixel".
pub fn files_title(count: usize, from: &str) -> String {
    if count == 1 { format!("1 file from {from}") } else { format!("{count} files from {from}") }
}

/// Where an upload came from when the phone sent it by itself (2B.4, protocol.md "Auto-sent screenshots: `origin`"): `POST /v1/transfers` takes an optional
/// `origin`. The PC **never trusts it for anything but wording and the clipboard rule**: a hostile phone can already put an image on the clipboard with an
/// ordinary upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Screenshot,
    Camera,
}

impl Origin {
    /// `"screenshot"` or `"camera"` exactly; any other value, or a value of another type, is "absent" (a manual send).
    pub fn parse(v: &Value) -> Option<Origin> {
        match v.as_str()? {
            "screenshot" => Some(Origin::Screenshot),
            "camera" => Some(Origin::Camera),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Screenshot => "screenshot",
            Origin::Camera => "camera",
        }
    }

    pub fn from_db(text: &str) -> Option<Origin> {
        Origin::parse(&Value::String(text.to_string()))
    }
}

/// What the toast of a finished upload says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// "Screenshot from <phone>"
    Screenshot,
    /// "Photo from <phone>"
    Photo,
    /// "1 file from <phone>", as before 2B.4.
    Files,
    /// A paused PC saves the file and shows nothing.
    None,
}

impl ToastKind {
    pub fn word(self) -> &'static str {
        match self {
            ToastKind::Screenshot => "screenshot",
            ToastKind::Photo => "photo",
            ToastKind::Files => "files",
            ToastKind::None => "none",
        }
    }
}

/// What a 0.2.0 PC does with the image of a finished transfer (`protocol/presence.json` `origin[]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginPlan {
    pub toast: ToastKind,
    /// The image goes on the clipboard (if the setting "images also go to the clipboard" is on).
    pub clipboard: bool,
    /// With the formats that keep it out of Win+V and the cloud clipboard.
    pub sensitive_formats: bool,
}

/// The one rule, pure: a **screenshot** gets its own toast and goes on the clipboard (if the setting allows) with the sensitive formats; a **camera photo** gets
/// "Photo from" and **never** the clipboard; no origin is as before. A paused PC saves the file quietly whatever the origin: no toast, no clipboard.
pub fn plan(origin: Option<Origin>, images_to_clipboard: bool, paused: bool) -> OriginPlan {
    if paused {
        return OriginPlan { toast: ToastKind::None, clipboard: false, sensitive_formats: false };
    }
    match origin {
        Some(Origin::Screenshot) => {
            OriginPlan { toast: ToastKind::Screenshot, clipboard: images_to_clipboard, sensitive_formats: images_to_clipboard }
        }
        Some(Origin::Camera) => OriginPlan { toast: ToastKind::Photo, clipboard: false, sensitive_formats: false },
        None => OriginPlan { toast: ToastKind::Files, clipboard: images_to_clipboard, sensitive_formats: false },
    }
}

/// The toast title of a finished upload: by origin ("Screenshot from Pixel", "Photo from Pixel"), else as always.
pub fn origin_title(kind: ToastKind, count: usize, from: &str) -> String {
    match kind {
        ToastKind::Screenshot => format!("Screenshot from {from}"),
        ToastKind::Photo => format!("Photo from {from}"),
        _ => files_title(count, from),
    }
}

impl Core {
    pub fn images_to_clipboard(&self) -> bool {
        lock(&self.settings).images_to_clipboard
    }

    /// The choice "images also go to the clipboard" (the settings window, or `imagesToClipboard` in settings.json).
    pub fn set_images_to_clipboard(&self, on: bool) {
        let settings = {
            let mut s = lock(&self.settings);
            if s.images_to_clipboard == on {
                return;
            }
            s.images_to_clipboard = on;
            s.clone()
        };
        log::info!("images to the clipboard turned {}", if on { "on" } else { "off" });
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json failed: {e}");
        }
        self.emit(crate::core::Event::StateChanged);
    }

    /// The last file of a transfer has its final name: toast, and the image to the clipboard. **This runs in a task of its own**, never
    /// inside the `PUT` that finished the last file: the picture has to be decoded and the clipboard may be busy, and the phone is
    /// waiting for its `200`.
    pub(crate) async fn files_received(self: &Arc<Self>, entry: &Arc<Entry>, from: &str) {
        let files: Vec<(PathBuf, String)> = entry
            .files
            .iter()
            .map(|f| {
                let d = f.data();
                (d.path, d.name)
            })
            .collect();
        let Some(first) = files.first() else {
            return;
        };
        // 2B.4: what the phone says about where the file came from, and the pause, decide the toast and the clipboard (`plan`).
        let origin = *lock(&entry.origin);
        let plan = plan(origin, self.images_to_clipboard(), self.paused());
        if plan.toast == ToastKind::None {
            // A paused PC: the file is saved (it is, by now) and the history has it. Nothing visible happens by itself.
            log::info!("upload {}: arrived while paused, handled quietly (saved, no toast, no clipboard)", short_transfer(&entry.id));
            self.presence_note_quiet_arrival();
            return;
        }
        let target =
            if files.len() == 1 { ToastTarget::OpenFile(first.0.clone()) } else { ToastTarget::ShowFolder(entry.dest_dir.clone()) };
        let body = if files.len() <= 3 {
            files.iter().map(|f| f.1.as_str()).collect::<Vec<_>>().join(", ")
        } else {
            format!("{}, … and {} more", files[..2].iter().map(|f| f.1.as_str()).collect::<Vec<_>>().join(", "), files.len() - 2)
        };
        let toast_id = self.next_toast_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.register_toast_target(toast_id, target);
        // One file whose type is on the allow-list can be opened from the toast; any other is shown in its folder.
        let openable = files.len() == 1 && links::may_open_file(&first.0);
        self.emit(Event::FilesToast {
            id: toast_id,
            title: origin_title(plan.toast, files.len(), from),
            body,
            single: files.len() == 1,
            openable,
        });

        if !plan.clipboard {
            if origin == Some(Origin::Camera) {
                log::info!("upload {}: a camera photo is saved and shown, never put on the clipboard", short_transfer(&entry.id));
            }
            return;
        }
        // Which files are pictures is decided by their content (the decoder), never by the name or the type the phone gave.
        let paths: Vec<PathBuf> = files.into_iter().map(|f| f.0).collect();
        let images = tokio::task::spawn_blocking(move || crate::images::find_images(&paths, 2)).await.unwrap_or_default();
        let id = short_transfer(&entry.id);
        match images.len() {
            0 => {}
            1 => {
                let path = images[0].clone();
                let clipboard = self.clipboard.clone();
                let sensitive = plan.sensitive_formats;
                let write =
                    move || if sensitive { clipboard.write_image_file_with(&path, true) } else { clipboard.write_image_file(&path) };
                match tokio::task::spawn_blocking(write).await {
                    Ok(Ok(ImageOutcome::Set { width, height })) => {
                        log::info!("upload {id}: the image ({width} x {height}) is on the clipboard");
                    }
                    Ok(Ok(ImageOutcome::Skipped(why))) => {
                        log::info!("upload {id}: the image was not put on the clipboard: {why}");
                    }
                    Ok(Err(e)) => log::warn!("upload {id}: putting the image on the clipboard failed: {e}"),
                    Err(e) => log::error!("the image task failed: {e}"),
                }
            }
            _ => log::info!("upload {id}: several images arrived, the clipboard is left alone"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(files_title(1, "Pixel"), "1 file from Pixel");
        assert_eq!(files_title(3, "Pixel"), "3 files from Pixel");
    }
}
