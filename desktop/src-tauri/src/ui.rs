//! Windows and the commands the web UI calls. A window is created when it is opened and **destroyed** when it is closed
//! (windows.md): an open WebView2 costs tens of MB, and destroying it keeps the idle memory low.
//!
//! There are five windows: the pairing window, (1.3) the drop window, which the tray's left click opens: a target for files
//! dragged from Explorer (Windows doesn't support dropping on a tray icon), a button that opens the file dialog, and the progress of
//! the transfers with a Cancel button, and (1.4) the history, settings and debug windows. They are all the same page (`index.html`), told
//! apart by the `?w=` of their URL.
use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::app::AppState;
use crate::core::PairingView;

pub const PAIRING_WINDOW: &str = "pairing";
pub const DROP_WINDOW: &str = "drop";

/// Defence in depth for the capabilities (`capabilities/<window>.json`, which already give each window only its own commands): a command also
/// checks the label of the window that called it. A page that got into the wrong window (or a capability file that was edited too widely) still
/// can't make this window do what another one is for.
fn only(window: &WebviewWindow, allowed: &[&str], command: &str) -> Result<(), String> {
    if allowed.contains(&window.label()) {
        Ok(())
    } else {
        log::warn!("the {:?} window called {command}, which is not for it: refused", window.label());
        Err("This window may not do that.".to_string())
    }
}

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
            .inner_size(380.0, 620.0)
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
pub fn pairing_view(window: WebviewWindow, state: State<'_, AppState>) -> Result<PairingView, String> {
    only(&window, &[PAIRING_WINDOW], "pairing_view")?;
    Ok(state.core.pairing_view().unwrap_or_else(|| state.core.start_pairing()))
}

/// "Show a new code" after the old one expired.
#[tauri::command]
pub fn new_pairing(window: WebviewWindow, state: State<'_, AppState>) -> Result<PairingView, String> {
    only(&window, &[PAIRING_WINDOW], "new_pairing")?;
    Ok(state.core.start_pairing())
}

/// "Copy pairing link": the link of the QR code on the clipboard, marked sensitive (see `Core::copy_pairing_link`). Pairing window only.
#[tauri::command]
pub async fn copy_pairing_link(window: WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    only(&window, &[PAIRING_WINDOW], "copy_pairing_link")?;
    state.core.copy_pairing_link().await
}

/// The "Remove this device" button of the "New device paired" notice.
#[tauri::command]
pub fn remove_device(window: WebviewWindow, state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    only(&window, &[PAIRING_WINDOW, SETTINGS_WINDOW], "remove_device")?;
    state.core.remove_device(&device_id).map(|_| ()).map_err(|e| e.to_string())
}

/// Closes (destroys) the window that asked.
#[tauri::command]
pub fn close_window(window: WebviewWindow) -> Result<(), String> {
    only(&window, &[PAIRING_WINDOW, DROP_WINDOW, HISTORY_WINDOW, SETTINGS_WINDOW, DEBUG_WINDOW], "close_window")?;
    let _ = window.destroy();
    Ok(())
}

// ----- the drop window --------------------------------------------------------------------------------------------------

/// The state of the drop window: the phone, the transfers that are going and the ones that finished lately.
#[tauri::command]
pub fn drop_view(window: WebviewWindow, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    only(&window, &[DROP_WINDOW], "drop_view")?;
    Ok(state.core.drop_view())
}

/// Files that were dropped on the window (the page gets their paths from the drag-and-drop event).
#[tauri::command]
pub async fn send_files(window: WebviewWindow, state: State<'_, AppState>, paths: Vec<String>) -> Result<(), String> {
    only(&window, &[DROP_WINDOW], "send_files")?;
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
pub async fn pick_files(window: WebviewWindow, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    only(&window, &[DROP_WINDOW], "pick_files")?;
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
pub async fn cancel_transfer(window: WebviewWindow, state: State<'_, AppState>, id: String) -> Result<bool, String> {
    only(&window, &[DROP_WINDOW], "cancel_transfer")?;
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

// ----- the history, settings and debug windows (1.4) -----------------------------------------------------------------------

pub const HISTORY_WINDOW: &str = "history";
pub const SETTINGS_WINDOW: &str = "settings";
pub const DEBUG_WINDOW: &str = "debug";

/// Opens a window of the page `index.html?w=<label>`, or brings it to the front. It is destroyed when it is closed, like the others.
fn open_page(app: &AppHandle, label: &'static str, title: &'static str, size: (f64, f64), min: (f64, f64)) {
    let state = app.state::<AppState>();
    if state.core.config.dev_no_windows {
        log::info!("{label} window skipped (--dev-no-windows)");
        return;
    }
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let built = WebviewWindowBuilder::new(&app, label, WebviewUrl::App(format!("index.html?w={label}").into()))
            .title(title)
            .inner_size(size.0, size.1)
            .min_inner_size(min.0, min.1)
            .center()
            .build();
        if let Err(e) = built {
            log::error!("opening the {label} window failed: {e}");
        }
    });
}

pub fn open_history_window(app: &AppHandle) {
    open_page(app, HISTORY_WINDOW, "History", (560.0, 600.0), (420.0, 320.0));
}

pub fn open_settings_window(app: &AppHandle) {
    open_page(app, SETTINGS_WINDOW, "Settings", (480.0, 780.0), (420.0, 420.0));
}

pub fn open_debug_window(app: &AppHandle) {
    open_page(app, DEBUG_WINDOW, "Debug", (620.0, 680.0), (460.0, 380.0));
}

/// Tells the windows that show lists that something changed.
pub fn lists_changed(app: &AppHandle) {
    let _ = app.emit_to(HISTORY_WINDOW, "history-changed", ());
    let _ = app.emit_to(SETTINGS_WINDOW, "settings-changed", ());
}

// ----- the history window

#[tauri::command]
pub fn history_view(window: WebviewWindow, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    only(&window, &[HISTORY_WINDOW], "history_view")?;
    Ok(state.core.history_view())
}

/// A click on an item: a file opens (or is shown in its folder), a text is copied. The answer is a short line for the window.
#[tauri::command]
pub async fn history_open(window: WebviewWindow, state: State<'_, AppState>, id: String) -> Result<String, String> {
    only(&window, &[HISTORY_WINDOW], "history_open")?;
    state.core.history_open(&id).await
}

#[tauri::command]
pub async fn history_cancel(window: WebviewWindow, state: State<'_, AppState>, id: String) -> Result<bool, String> {
    only(&window, &[HISTORY_WINDOW], "history_cancel")?;
    Ok(state.core.history_cancel(&id).await)
}

/// Retry on a failed item that this PC sent: a new item, a new ID.
#[tauri::command]
pub async fn history_retry(window: WebviewWindow, state: State<'_, AppState>, id: String) -> Result<String, String> {
    only(&window, &[HISTORY_WINDOW], "history_retry")?;
    state.core.retry_item(&id).await
}

// ----- the settings window

fn settings_json(state: &AppState) -> serde_json::Value {
    let start = state.autostart.is_enabled().ok();
    let mut view = state.core.settings_view(start);
    view["startWithWindowsIsReal"] = serde_json::json!(state.autostart.is_real());
    view
}

#[tauri::command]
pub fn settings_view(window: WebviewWindow, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    only(&window, &[SETTINGS_WINDOW], "settings_view")?;
    Ok(settings_json(&state))
}

#[tauri::command]
pub fn set_device_name(window: WebviewWindow, state: State<'_, AppState>, name: String) -> Result<String, String> {
    only(&window, &[SETTINGS_WINDOW], "set_device_name")?;
    Ok(state.core.set_device_name(&name))
}

#[tauri::command]
pub fn set_auto_clipboard_sync(window: WebviewWindow, state: State<'_, AppState>, on: bool) -> Result<(), String> {
    only(&window, &[SETTINGS_WINDOW], "set_auto_clipboard_sync")?;
    state.core.set_auto_clipboard_sync(on);
    Ok(())
}

#[tauri::command]
pub fn set_images_to_clipboard(window: WebviewWindow, state: State<'_, AppState>, on: bool) -> Result<(), String> {
    only(&window, &[SETTINGS_WINDOW], "set_images_to_clipboard")?;
    state.core.set_images_to_clipboard(on);
    Ok(())
}

/// "Choose folder…": the Windows folder picker, then the folder is checked and kept. `None` if the user closed the dialog.
#[tauri::command]
pub async fn choose_received_folder(window: WebviewWindow, app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    only(&window, &[SETTINGS_WINDOW], "choose_received_folder")?;
    let owner = app.get_webview_window(SETTINGS_WINDOW).and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
    let picked = tauri::async_runtime::spawn_blocking(move || pick_folder_dialog(owner))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    match picked {
        None => Ok(None),
        Some(folder) => state.core.set_received_folder(Some(&folder.to_string_lossy())).map(|p| Some(p.to_string_lossy().into_owned())),
    }
}

#[tauri::command]
pub fn reset_received_folder(window: WebviewWindow, state: State<'_, AppState>) -> Result<String, String> {
    only(&window, &[SETTINGS_WINDOW], "reset_received_folder")?;
    state.core.set_received_folder(None).map(|p| p.to_string_lossy().into_owned())
}

/// The hotkey: the new one is registered first. If another program has it, or the text isn't a hotkey, the old one stays and the answer
/// says why (the window shows it). An empty text turns the hotkey off.
#[tauri::command]
pub fn set_hotkey(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    accelerator: String,
) -> Result<serde_json::Value, String> {
    only(&window, &[SETTINGS_WINDOW], "set_hotkey")?;
    crate::app::change_hotkey(&app, &state.core, accelerator.trim())
}

#[tauri::command]
pub fn set_start_with_windows(window: WebviewWindow, state: State<'_, AppState>, on: bool) -> Result<bool, String> {
    only(&window, &[SETTINGS_WINDOW], "set_start_with_windows")?;
    state.autostart.set_enabled(on)?;
    // What the person chose, so that the repair at the next start (integration.rs) does not turn it back on.
    crate::integration::remember_start_with_windows(&state.core.config.data_dir, on);
    log::info!(
        "start with Windows turned {} ({})",
        if on { "on" } else { "off" },
        if state.autostart.is_real() { "the Run key" } else { "the stand-in file" }
    );
    Ok(state.autostart.is_enabled().unwrap_or(on))
}

// ----- the debug window

/// What the debug window shows. The network profile is asked of Windows on the blocking pool (it is a COM call).
#[tauri::command]
pub async fn debug_view(window: WebviewWindow, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    only(&window, &[DEBUG_WINDOW], "debug_view")?;
    let core = state.core.clone();
    Ok(debug_json(&core).await)
}

pub async fn debug_json(core: &std::sync::Arc<crate::core::Core>) -> serde_json::Value {
    let network = tauri::async_runtime::spawn_blocking(crate::netprofile::connected_networks).await;
    let network = match network {
        Ok(Ok(list)) => serde_json::json!({ "networks": list }),
        Ok(Err(e)) => serde_json::json!({ "networks": [], "error": e }),
        Err(e) => serde_json::json!({ "networks": [], "error": e.to_string() }),
    };
    core.debug_view(network)
}

#[tauri::command]
pub async fn debug_ping(window: WebviewWindow, state: State<'_, AppState>) -> Result<String, String> {
    only(&window, &[DEBUG_WINDOW], "debug_ping")?;
    Ok(state.core.ping_phone().await.text())
}

/// "Export logs": asks where to save, then writes the zip. `None` if the user closed the dialog.
#[tauri::command]
pub async fn export_logs(window: WebviewWindow, app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    only(&window, &[DEBUG_WINDOW], "export_logs")?;
    let owner = app.get_webview_window(DEBUG_WINDOW).and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
    let name = default_export_name();
    let dest = tauri::async_runtime::spawn_blocking(move || save_file_dialog(owner, &name))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let Some(dest) = dest else {
        return Ok(None);
    };
    let core = state.core.clone();
    let network = match tauri::async_runtime::spawn_blocking(crate::netprofile::connected_networks).await {
        Ok(Ok(list)) => serde_json::json!({ "networks": list }),
        _ => serde_json::json!({ "networks": [] }),
    };
    let report = tauri::async_runtime::spawn_blocking(move || core.export_logs(&dest, network)).await.map_err(|e| e.to_string())??;
    Ok(Some(report.path.to_string_lossy().into_owned()))
}

/// `kin-logs-20261015-143207.zip`, from the local time.
fn default_export_name() -> String {
    match crate::localtime::local_time(crate::encoding::unix_ms() as i64) {
        Some(t) => format!("kin-logs-{:04}{:02}{:02}-{:02}{:02}.zip", t.year, t.month, t.day, t.hour, t.minute),
        None => "kin-logs.zip".to_string(),
    }
}

/// The Windows folder picker. `None` if the user cancelled.
fn pick_folder_dialog(owner: Option<isize>) -> crate::Result<Option<PathBuf>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    use windows::core::w;
    // SAFETY: as in `pick_files_dialog`: COM is initialised for this thread and undone at the end, and the string Windows allocates is
    // freed with `CoTaskMemFree`.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> crate::Result<Option<PathBuf>> {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let options = dialog.GetOptions()?;
            dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
            dialog.SetTitle(w!("Folder for files from your phone"))?;
            if dialog.Show(owner.map(|h| HWND(h as *mut _))).is_err() {
                return Ok(None); // cancelled
            }
            let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(Some(PathBuf::from(path?)))
        })();
        if com {
            CoUninitialize();
        }
        result
    }
}

/// The Windows "Save as" dialog for the log export, with `.zip` as the default extension. `None` if the user cancelled.
fn save_file_dialog(owner: Option<isize>, default_name: &str) -> crate::Result<Option<PathBuf>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FileSaveDialog, IFileSaveDialog, SIGDN_FILESYSPATH};
    use windows::core::{HSTRING, w};
    // SAFETY: as above.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> crate::Result<Option<PathBuf>> {
            let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
            let options = dialog.GetOptions()?;
            dialog.SetOptions(options | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;
            dialog.SetTitle(w!("Export logs"))?;
            dialog.SetDefaultExtension(w!("zip"))?;
            dialog.SetFileName(&HSTRING::from(default_name))?;
            if dialog.Show(owner.map(|h| HWND(h as *mut _))).is_err() {
                return Ok(None); // cancelled
            }
            let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(Some(PathBuf::from(path?)))
        })();
        if com {
            CoUninitialize();
        }
        result
    }
}

/// The Settings window's "Debug…" button (and a way for any window to open another).
#[tauri::command]
pub fn open_window(window: WebviewWindow, app: AppHandle, name: String) -> Result<(), String> {
    only(&window, &[SETTINGS_WINDOW], "open_window")?;
    match name.as_str() {
        "history" => open_history_window(&app),
        "settings" => open_settings_window(&app),
        "debug" => open_debug_window(&app),
        other => log::warn!("no window called {other:?}"),
    }
    Ok(())
}
