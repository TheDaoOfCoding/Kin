//! Windows and the commands the web UI calls. A window is created when it is opened and **destroyed** when it is closed
//! (windows.md): an open WebView2 costs tens of MB, and destroying it keeps the idle memory low.
//!
//! There are two windows: the pairing window and (1.3) the drop window, which the tray's left click opens: a target for files
//! dragged from Explorer (Windows doesn't support dropping on a tray icon), a button that opens the file dialog, and the progress of
//! the transfers with a Cancel button. Both are the same page (`index.html`), told apart by the `?w=` of their URL.
use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::app::AppState;
use crate::core::PairingView;

pub const PAIRING_WINDOW: &str = "pairing";
pub const DROP_WINDOW: &str = "drop";

/// Opens the pairing window, or brings it to the front. With `fresh` a new pairing (a new token) starts first. Without it
/// the window shows what is there, such as the "New device paired" notice.
pub fn open_pairing_window(app: &AppHandle, fresh: bool) {
    let state = app.state::<AppState>();
    if state.core.config.dev_no_windows {
        log::info!("pairing window skipped (--dev-no-windows)");
        if fresh {
            state.core.start_pairing();
        }
        return;
    }
    if let Some(window) = app.get_webview_window(PAIRING_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    if fresh {
        state.core.start_pairing();
    }
    // Building a window blocks until the main thread has made it, so don't do it on the main thread.
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let built = WebviewWindowBuilder::new(&app, PAIRING_WINDOW, WebviewUrl::App("index.html".into()))
            .title("Pair new phone")
            .inner_size(380.0, 560.0)
            .resizable(false)
            .center()
            .build();
        if let Err(e) = built {
            log::error!("opening the pairing window failed: {e}");
        }
    });
}

/// Opens the drop window, or brings it to the front.
pub fn open_drop_window(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.core.config.dev_no_windows {
        log::info!("drop window skipped (--dev-no-windows)");
        return;
    }
    if let Some(window) = app.get_webview_window(DROP_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let built = WebviewWindowBuilder::new(&app, DROP_WINDOW, WebviewUrl::App("index.html?w=drop".into()))
            .title("Send to phone")
            .inner_size(400.0, 520.0)
            .min_inner_size(360.0, 360.0)
            .center()
            .build();
        if let Err(e) = built {
            log::error!("opening the drop window failed: {e}");
        }
    });
}

/// Tells the pairing window (if open) that there is something new to show.
pub fn pairing_changed(app: &AppHandle) {
    let _ = app.emit_to(PAIRING_WINDOW, "pairing-changed", ());
}

/// Tells the drop window (if open) that a transfer moved on.
pub fn transfers_changed(app: &AppHandle) {
    let _ = app.emit_to(DROP_WINDOW, "transfers-changed", ());
}

/// What the window shows. If nobody started a pairing yet (the window was opened some other way), this starts one.
#[tauri::command]
pub fn pairing_view(state: State<'_, AppState>) -> PairingView {
    state.core.pairing_view().unwrap_or_else(|| state.core.start_pairing())
}

/// "Show a new code" after the old one expired.
#[tauri::command]
pub fn new_pairing(state: State<'_, AppState>) -> PairingView {
    state.core.start_pairing()
}

/// The "Remove this device" button of the "New device paired" notice.
#[tauri::command]
pub fn remove_device(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    state.core.remove_device(&device_id).map(|_| ()).map_err(|e| e.to_string())
}

/// Closes (destroys) the window that asked.
#[tauri::command]
pub fn close_window(window: WebviewWindow) {
    let _ = window.destroy();
}

// ----- the drop window --------------------------------------------------------------------------------------------------

/// The state of the drop window: the phone, the transfers that are going and the ones that finished lately.
#[tauri::command]
pub fn drop_view(state: State<'_, AppState>) -> serde_json::Value {
    state.core.drop_view()
}

/// Files that were dropped on the window (the page gets their paths from the drag-and-drop event).
#[tauri::command]
pub async fn send_files(state: State<'_, AppState>, paths: Vec<String>) -> Result<(), String> {
    log::info!("the drop window got {} dropped path(s)", paths.len());
    let core = state.core.clone();
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn(async move {
        core.send_paths(paths, "drop window").await;
    });
    Ok(())
}

/// "Choose files…": the file dialog of Windows, then the same as a drop.
#[tauri::command]
pub async fn pick_files(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    pick_and_send(&app, state.core.clone());
    Ok(())
}

/// The tray's "Send files…" and the drop window's button: ask which files, and send them.
pub fn pick_and_send(app: &AppHandle, core: std::sync::Arc<crate::core::Core>) {
    let owner = app.get_webview_window(DROP_WINDOW).and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
    // The dialog blocks its thread until it is closed, so it gets a thread of its own.
    tauri::async_runtime::spawn_blocking(move || match pick_files_dialog(owner) {
        Ok(paths) if !paths.is_empty() => {
            tauri::async_runtime::spawn(async move {
                core.send_paths(paths, "file dialog").await;
            });
        }
        Ok(_) => {}
        Err(e) => log::warn!("the file dialog failed: {e}"),
    });
}

#[tauri::command]
pub async fn cancel_transfer(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state.core.cancel_transfer(&id).await)
}

/// The Windows "Open" dialog with several files allowed. `owner` is a window handle (as a number) to keep it on top of.
fn pick_files_dialog(owner: Option<isize>) -> crate::Result<Vec<PathBuf>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    use windows::core::w;
    // SAFETY: COM is initialised for this thread and undone at the end if it worked; the dialog and the items are used on this
    // thread only, and every string that Windows allocates is freed with `CoTaskMemFree`.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> crate::Result<Vec<PathBuf>> {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let options = dialog.GetOptions()?;
            dialog.SetOptions(options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)?;
            dialog.SetTitle(w!("Send files to your phone"))?;
            let parent = owner.map(|h| HWND(h as *mut _));
            if dialog.Show(parent).is_err() {
                return Ok(Vec::new()); // cancelled
            }
            let items = dialog.GetResults()?;
            let mut paths = Vec::new();
            for i in 0..items.GetCount()? {
                let name = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
                let path = name.to_string();
                CoTaskMemFree(Some(name.0 as *const _));
                paths.push(PathBuf::from(path?));
            }
            Ok(paths)
        })();
        if com {
            CoUninitialize();
        }
        result
    }
}
