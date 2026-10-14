//! The Tauri side: start-up, the single instance, the tray menu, toasts, and what the core's events do to the UI.
use std::path::Path;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use crate::cli::{self, Command, Parsed};
use crate::core::{Core, Event, computer_name, short};
use crate::identity::Identity;
use crate::mdns::Advertiser;
use crate::server::{self, ServerHandle};
use crate::storage::Db;
use crate::tray::{self, Tray};
use crate::{Result, filelog, ui};

pub struct AppState {
    pub core: Arc<Core>,
    tray: Tray,
    server: Mutex<Option<ServerHandle>>,
    advertiser: Mutex<Option<Advertiser>>,
}

pub fn run(parsed: Parsed) {
    let built = tauri::Builder::default()
        // Must be the first plugin. A second launch (`kin.exe --ping`, later Explorer's "Send to") runs this callback in the
        // running instance with its arguments, and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let env = |k: &str| std::env::var(k).ok();
            match cli::parse(argv.get(1..).unwrap_or_default(), &env) {
                Ok(parsed) => handle_commands(app, &parsed.commands),
                Err(e) => log::warn!("ignoring the arguments of a second launch: {e}"),
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![ui::pairing_view, ui::new_pairing, ui::remove_device, ui::close_window])
        .setup(move |app| setup(app, parsed).map_err(|e| -> Box<dyn std::error::Error> { e }))
        .on_window_event(|window, event| {
            // The pairing window is destroyed when it closes. An unused token stops working with it.
            if let WindowEvent::Destroyed = event
                && window.label() == ui::PAIRING_WINDOW
                && let Some(state) = window.app_handle().try_state::<AppState>()
            {
                state.core.end_pairing();
            }
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| fatal(&e));

    built.run(|app, event| match event {
        // Closing the last window asks to exit with no code. A tray-only app refuses that. Quit uses `app.exit(0)`.
        RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(),
        RunEvent::Exit => {
            if let Some(state) = app.try_state::<AppState>() {
                write_status(&state.core, true);
            }
            log::info!("Kin stopped");
        }
        _ => {}
    });
}

/// The app can't start (no data directory, a broken database, ...). A release build has no console, so the reason goes to a
/// file in the temp folder as well.
fn fatal(error: &dyn std::fmt::Display) -> ! {
    let text = format!("Kin could not start: {error}\n");
    eprint!("{text}");
    let _ = std::fs::write(std::env::temp_dir().join("kin-startup-error.txt"), &text);
    std::process::exit(1);
}

fn setup(app: &mut tauri::App, parsed: Parsed) -> Result<()> {
    let Parsed { config, commands, unknown } = parsed;
    std::fs::create_dir_all(&config.data_dir)?;
    filelog::init(&config.data_dir.join("logs"))?;
    filelog::install_panic_hook();
    log::info!(
        "Kin {} started (pid {}), data dir {}{}",
        crate::core::APP_VERSION,
        std::process::id(),
        config.data_dir.display(),
        if config.is_dev_network() { ", developer network settings on: loopback only" } else { "" }
    );
    if !unknown.is_empty() {
        log::warn!("ignoring unknown arguments: {unknown:?}");
    }

    let loaded = Identity::load_or_create(&config.data_dir)?;
    log::info!(
        "identity {}: device {} (short ID {})",
        if loaded.created { "created" } else { "loaded" },
        short(&loaded.identity.device_id),
        loaded.identity.short_id
    );
    let db = Db::open(&config.data_dir.join("kin.db"))?;
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let core = Arc::new(Core::new(config, loaded.identity, db, computer_name(), events_tx));

    let server = tauri::async_runtime::block_on(server::start(core.clone()))?;
    // mDNS opens multicast sockets on every interface, which developer mode (loopback only) must not do.
    let advertiser = if core.config.is_dev_network() {
        log::info!("mDNS: not advertising in developer network mode");
        None
    } else {
        Advertiser::start(&core.identity.short_id, server.local_addr.port()).map_err(|e| log::error!("mDNS advertising failed: {e}")).ok()
    };

    let tray = tray::build(app, &core.snapshot())?;
    app.manage(AppState { core: core.clone(), tray, server: Mutex::new(Some(server)), advertiser: Mutex::new(advertiser) });

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(event_loop(handle.clone(), core.clone(), events_rx));
    refresh(&handle, &core);
    handle_commands(&handle, &commands);
    Ok(())
}

/// What the core's events do to the UI.
async fn event_loop(app: AppHandle, core: Arc<Core>, mut events: tokio::sync::mpsc::UnboundedReceiver<Event>) {
    while let Some(event) = events.recv().await {
        match event {
            Event::StateChanged => refresh(&app, &core),
            Event::PairingChanged => {
                ui::pairing_changed(&app);
                write_status(&core, false);
            }
            Event::DevicePaired(device) => {
                // The "New device paired" notice, with a way to remove the device in case someone else scanned the code.
                toast(&app, &core, &format!("New device paired: {}", device.name));
                ui::open_pairing_window(&app, false);
            }
            Event::Toast(text) => toast(&app, &core, &text),
        }
    }
}

/// Updates the tray and `status.json` after a change of state.
fn refresh(app: &AppHandle, core: &Core) {
    if let Some(state) = app.try_state::<AppState>() {
        state.tray.update(app, &core.snapshot());
    }
    write_status(core, false);
}

/// `status.json` in the data directory, for scripts: written on every state change.
fn write_status(core: &Core, stopped: bool) {
    let mut status = core.status_json();
    if stopped {
        status["state"] = "stopped".into();
    }
    let path = core.config.data_dir.join("status.json");
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(&status).map_err(std::io::Error::other).and_then(|bytes| std::fs::write(&tmp, bytes));
    // `rename` replaces the old file, which is what we want for a status file.
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, &path)) {
        log::warn!("writing status.json failed: {e}");
    }
}

fn toast(app: &AppHandle, core: &Core, text: &str) {
    log::info!("toast: {text}");
    if core.config.dev_no_windows {
        return;
    }
    if let Err(e) = app.notification().builder().title("Kin").body(text).show() {
        log::warn!("showing a toast failed: {e}");
    }
}

pub fn on_menu(app: &AppHandle, id: &str) {
    match id {
        tray::MENU_PAIR => handle_commands(app, &[Command::Pair]),
        tray::MENU_PING => handle_commands(app, &[Command::Ping]),
        tray::MENU_LOGS => open_log_folder(app),
        tray::MENU_QUIT => handle_commands(app, &[Command::Quit]),
        _ => {}
    }
}

fn open_log_folder(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let dir = state.core.config.data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    open_in_explorer(&dir);
}

fn open_in_explorer(path: &Path) {
    if let Err(e) = std::process::Command::new("explorer.exe").arg(path).spawn() {
        log::warn!("opening {} failed: {e}", path.display());
    }
}

/// The commands of the tray menu and of other launches (`kin.exe --ping`).
pub fn handle_commands(app: &AppHandle, commands: &[Command]) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    for command in commands {
        log::info!("command: {command:?}");
        let core = state.core.clone();
        match command {
            Command::Ping => {
                tauri::async_runtime::spawn(async move {
                    core.ping_phone().await;
                });
            }
            Command::Pair => ui::open_pairing_window(app, true),
            Command::StartPairing => {
                core.start_pairing();
            }
            Command::Quit => quit(app),
        }
    }
}

/// Closes the WebSockets (1000, "app quit"), stops listening, removes the tray icon and exits.
fn quit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        log::info!("quitting");
        state.core.shutdown().await;
        let server = state.server.lock().ok().and_then(|mut s| s.take());
        if let Some(server) = server {
            server.stop().await;
        }
        if let Some(advertiser) = state.advertiser.lock().ok().and_then(|mut a| a.take()) {
            advertiser.stop();
        }
        // Remove the icon first, otherwise Windows can leave a ghost icon until it is hovered.
        let _ = app.remove_tray_by_id(tray::TRAY_ID);
        app.exit(0);
    });
}
