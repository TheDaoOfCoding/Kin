use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const TRAY_ID: &str = "kin";
const WINDOW_LABEL: &str = "hello";

// Windows are created on open and destroyed on close, which frees the WebView2 processes
// (see docs/windows.md). The app stays alive because of the ExitRequested handler in `run`.
// Set KIN_HIDE_ON_CLOSE=1 to hide the window instead, to measure what that costs.
fn open_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App("index.html".into()))
        .title("Kin Relay")
        .inner_size(320.0, 160.0)
        .resizable(false)
        .center()
        .build();
}

pub fn run() {
    let hide_on_close = std::env::var_os("KIN_HIDE_ON_CLOSE").is_some();

    tauri::Builder::default()
        // Must be the first plugin. A second launch runs this callback in the first instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_window(app);
        }))
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().expect("bundle icon").clone())
                .tooltip("Kin")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => open_window(app),
                    "quit" => {
                        // Remove the icon first, otherwise Windows can leave a ghost icon until hover.
                        let _ = app.remove_tray_by_id(TRAY_ID);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { .. } = event {
                        open_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(move |window, event| {
            if hide_on_close {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the Kin app")
        .run(|_app, event| {
            // Closing the last window asks to exit with no code. Tray-only apps must refuse that.
            // Quit uses app.exit(0), which carries a code and goes through.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
