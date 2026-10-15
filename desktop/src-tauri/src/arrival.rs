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

use crate::clipboard::ImageOutcome;
use crate::core::{Core, Event, lock, short_transfer};
use crate::links;
use crate::sync::ToastTarget;
use crate::transfers::Entry;

/// "1 file from Pixel", "3 files from Pixel".
pub fn files_title(count: usize, from: &str) -> String {
    if count == 1 { format!("1 file from {from}") } else { format!("{count} files from {from}") }
}

impl Core {
    pub fn images_to_clipboard(&self) -> bool {
        lock(&self.settings).images_to_clipboard
    }

    /// The choice "images also go to the clipboard" (a settings window comes in 1.4; `imagesToClipboard` in settings.json is it for now).
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
        self.emit(Event::FilesToast { id: toast_id, title: files_title(files.len(), from), body, single: files.len() == 1, openable });

        if !self.images_to_clipboard() {
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
                match tokio::task::spawn_blocking(move || clipboard.write_image_file(&path)).await {
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
