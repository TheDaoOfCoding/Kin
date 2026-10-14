//! Windows and the commands the web UI calls. A window is created when it is opened and **destroyed** when it is closed
//! (windows.md): an open WebView2 costs tens of MB, and destroying it keeps the idle memory low.
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::app::AppState;
use crate::core::PairingView;

pub const PAIRING_WINDOW: &str = "pairing";

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

/// Tells the pairing window (if open) that there is something new to show.
pub fn pairing_changed(app: &AppHandle) {
    let _ = app.emit_to(PAIRING_WINDOW, "pairing-changed", ());
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
