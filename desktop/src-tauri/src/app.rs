//! The Tauri side: start-up, the single instance, the tray menu, the global hotkey, toasts (and clicks on them), and what the
//! core's events do to the UI.
use std::path::Path;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use crate::autostart::{AutostartStore, FileAutostart, RegistryAutostart};
use crate::cli::{self, Command, Parsed};
use crate::clipboard::{MonotonicClock, WinClipboard};
use crate::core::{Core, Event, Services, computer_name, short};
use crate::identity::Identity;
use crate::links::ShellOpener;
use crate::mdns::Advertiser;
use crate::server::{self, ServerHandle};
use crate::storage::Db;
use crate::sync::{HotkeyStatus, Tap};
use crate::tray::{self, Tray};
use crate::{Result, filelog, ui};

pub struct AppState {
    pub core: Arc<Core>,
    /// "Start with Windows": the Run key in a normal run, a file in the data directory in any developer run (see `autostart.rs`).
    pub autostart: Arc<dyn AutostartStore>,
    tray: Tray,
    server: Mutex<Option<ServerHandle>>,
    advertiser: Mutex<Option<Advertiser>>,
}

pub fn run(parsed: Parsed) {
    // Before any window exists: the process is "Kin" (app.kinrelay.desktop) to Windows, like the Start Menu shortcut of the installer.
    let aumid = crate::integration::set_process_app_user_model_id();
    let built = tauri::Builder::default()
        // Must be the first plugin. A second launch (`kin.exe --ping`, later Explorer's "Send to") runs this callback in the
        // running instance with its arguments, and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let env = |k: &str| std::env::var(k).ok();
            match cli::parse(argv.get(1..).unwrap_or_default(), &env) {
                Ok(mut parsed) => {
                    // `Kin.exe --send a.jpg` from another folder: the path is relative to where that launch was.
                    cli::resolve_paths(&mut parsed.commands, Path::new(&cwd));
                    handle_commands(app, &parsed.commands);
                }
                Err(e) => log::warn!("ignoring the arguments of a second launch: {e}"),
            }
        }))
        .plugin(tauri_plugin_notification::init())
        // One shortcut is ever registered (the "send clipboard" hotkey), so the handler doesn't look at which one it was.
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        send_clipboard(app, "hotkey");
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            ui::pairing_view,
            ui::new_pairing,
            ui::copy_pairing_link,
            ui::remove_device,
            ui::close_window,
            ui::drop_view,
            ui::send_files,
            ui::pick_files,
            ui::cancel_transfer,
            ui::open_window,
            ui::history_view,
            ui::history_open,
            ui::history_cancel,
            ui::history_retry,
            ui::settings_view,
            ui::set_device_name,
            ui::set_auto_clipboard_sync,
            ui::set_images_to_clipboard,
            ui::choose_received_folder,
            ui::reset_received_folder,
            ui::set_hotkey,
            ui::set_start_with_windows,
            ui::debug_view,
            ui::debug_ping,
            ui::export_logs
        ])
        .setup(move |app| setup(app, parsed, aumid).map_err(|e| -> Box<dyn std::error::Error> { e }))
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

fn setup(app: &mut tauri::App, parsed: Parsed, aumid: std::result::Result<(), String>) -> Result<()> {
    let Parsed { config, mut commands, unknown } = parsed;
    if let Ok(cwd) = std::env::current_dir() {
        cli::resolve_paths(&mut commands, &cwd);
    }
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
    if let Err(e) = aumid {
        // Toasts name the ID themselves (the notification plugin), so this only costs the grouping of windows on the taskbar.
        log::warn!("setting the app user model ID failed: {e}");
    }

    let loaded = Identity::load_or_create(&config.data_dir)?;
    log::info!(
        "identity {}: device {} (short ID {})",
        if loaded.created { "created" } else { "loaded" },
        short(&loaded.identity.device_id),
        loaded.identity.short_id
    );
    if let Some(secs) = config.dev_clock_offset_secs {
        crate::encoding::set_clock_offset_ms(secs.saturating_mul(1000));
        log::warn!("developer clock offset: the app's clock is {secs} s ahead");
    }
    let db = Db::open(&config.data_dir.join("kin.db"))?;
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    // A message-only window listens for clipboard changes (`AddClipboardFormatListener`). It also owns the clipboard when the app
    // writes to it.
    let (win_clipboard, clipboard_changes) = WinClipboard::start()?;
    let services = Services { clipboard: Arc::new(win_clipboard), opener: Arc::new(ShellOpener), clock: Arc::new(MonotonicClock::new()) };
    let core = Arc::new(Core::new(config, loaded.identity, db, computer_name(), events_tx, services));

    let server = tauri::async_runtime::block_on(server::start(core.clone()))?;
    // mDNS opens multicast sockets on every interface, which developer mode (loopback only) must not do.
    let advertiser = if core.config.is_dev_network() {
        log::info!("mDNS: not advertising in developer network mode");
        None
    } else {
        Advertiser::start(&core.identity.short_id, server.local_addr.port()).map_err(|e| log::error!("mDNS advertising failed: {e}")).ok()
    };

    let tray = tray::build(app, &core.snapshot())?;
    // The Run key of the current user is written only by a release build that runs the way a user runs it (`Config::autostart_is_real`);
    // a debug build, a script or a developer setting gets the stand-in file.
    let real_autostart = if core.config.autostart_is_real() {
        RegistryAutostart::run_key()
            .map_err(|e| log::warn!("start with Windows: the path of Kin.exe is not known ({e}); the stand-in file is used"))
            .ok()
    } else {
        None
    };
    let autostart: Arc<dyn AutostartStore> = match real_autostart {
        Some(real) => Arc::new(real),
        None => {
            log::info!("start with Windows: not a normal release run, the registry is never touched (autostart-mock.json stands in)");
            Arc::new(FileAutostart::new(&core.config.data_dir))
        }
    };
    let for_integration = autostart.clone();
    app.manage(AppState { core: core.clone(), autostart, tray, server: Mutex::new(Some(server)), advertiser: Mutex::new(advertiser) });
    integrate_with_windows(&core, for_integration, loaded.created);

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(event_loop(handle.clone(), core.clone(), events_rx));
    {
        let core = core.clone();
        tauri::async_runtime::spawn(async move { core.run_clipboard_sync(clipboard_changes).await });
    }
    // Housekeeping: old offers and `.part` files go (at start, and every hour), and `status.json` keeps up with a running transfer.
    {
        let core = core.clone();
        tauri::async_runtime::spawn(async move {
            let mut hourly = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                hourly.tick().await;
                core.clean_up().await;
                // Queued items that waited too long fail with "expired" (at start and every hour).
                core.expire_queue(crate::encoding::unix_ms() as i64).await;
                // The send log and the old text bodies are trimmed every hour, not only at start (an app that runs for weeks grows otherwise).
                core.prune_storage(crate::encoding::unix_ms() as i64);
            }
        });
    }
    {
        let (core, handle) = (core.clone(), handle.clone());
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                tick.tick().await;
                if core.has_active_transfers() {
                    // The tray's tooltip ("sending 45%"), `status.json` and the drop window all follow the numbers.
                    refresh(&handle, &core);
                    ui::transfers_changed(&handle);
                }
            }
        });
    }
    register_toast_clicks(&handle, &core);
    register_hotkey(&handle, &core);
    refresh(&handle, &core);
    handle_commands(&handle, &commands);
    Ok(())
}

/// The part of the installation that is the user's (integration.rs): the "Send to" shortcut and "Start with Windows" are made what the person
/// wants, at every start. Only where the Run key is real, which is a release build that runs the way a user runs it, and not from the build
/// folder: a script or a developer never gets a shortcut in their real SendTo folder.
fn integrate_with_windows(core: &Arc<Core>, autostart: Arc<dyn AutostartStore>, new_profile: bool) {
    use crate::integration;
    if !autostart.is_real() {
        log::info!("integration: not a normal release run, the SendTo shortcut and the Run key are not touched");
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            log::warn!("integration: the path of Kin.exe is not known ({e})");
            return;
        }
    };
    if integration::is_build_folder(&exe) {
        log::info!("integration: running from a build folder, the SendTo shortcut is not made");
        return;
    }
    let Some(send_to) = integration::send_to_dir() else {
        log::warn!("integration: the SendTo folder is not known");
        return;
    };
    let data_dir = core.config.data_dir.clone();
    // COM and the registry, on the blocking pool: start-up doesn't wait for it.
    tauri::async_runtime::spawn_blocking(move || {
        let report = integration::reconcile(&data_dir, &exe, &send_to, autostart.as_ref(), new_profile);
        match &report.send_to {
            integration::SendTo::Failed(e) => log::warn!("integration: the SendTo shortcut failed: {e}"),
            other => log::info!("integration: the SendTo shortcut: {other:?}"),
        }
        match &report.autostart {
            Ok(true) => log::info!("integration: start with Windows is on (the Run value is written)"),
            Ok(false) => log::info!("integration: start with Windows is left as the person set it"),
            Err(e) => log::warn!("integration: start with Windows failed: {e}"),
        }
    });
}

/// What the core's events do to the UI. It also wakes up when the pairing code runs out, so the window and `status.json` stop
/// showing a code that no longer works.
async fn event_loop(app: AppHandle, core: Arc<Core>, mut events: tokio::sync::mpsc::UnboundedReceiver<Event>) {
    loop {
        let deadline = core.pairing_deadline().map(tokio::time::Instant::from_std);
        let wait_until = deadline.unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(3600));
        let event = tokio::select! {
            event = events.recv() => match event {
                Some(event) => event,
                None => break,
            },
            () = tokio::time::sleep_until(wait_until), if deadline.is_some() => {
                core.pairing_expired();
                continue;
            }
        };
        match event {
            Event::StateChanged => refresh(&app, &core),
            Event::PairingChanged => {
                ui::pairing_changed(&app);
                write_status_in_background(&core);
            }
            Event::DevicePaired(device) => {
                // The "New device paired" notice, with a way to remove the device in case someone else scanned the code.
                toast(&app, &core, &format!("New device paired: {}", device.name));
                ui::open_pairing_window(&app, false);
            }
            Event::Toast(text) => toast(&app, &core, &text),
            Event::ToastRedacted { text, log_text } => toast_logged(&app, &core, &text, &log_text),
            Event::TextToast { id, from, preview, has_link } => text_toast(&app, &core, id, &from, &preview, has_link),
            Event::FilesToast { id, title, body, single, openable } => files_toast(&app, &core, id, &title, &body, single, openable),
        }
    }
}

/// Updates the tray, the open windows and `status.json` after a change of state.
fn refresh(app: &AppHandle, core: &Arc<Core>) {
    if let Some(state) = app.try_state::<AppState>() {
        state.tray.update(app, &core.snapshot());
    }
    ui::lists_changed(app);
    write_status_in_background(core);
}

/// One write of `status.json` at a time, and none after the last one (the one that says `stopped`).
static STATUS_WRITE: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// `status.json` is written on the blocking pool: the event loop is an async task and a file write (which can wait for a script that has
/// the old file open) must not hold it.
fn write_status_in_background(core: &Arc<Core>) {
    let core = core.clone();
    tauri::async_runtime::spawn_blocking(move || write_status(&core, false));
}

/// `status.json` in the data directory, for scripts: written on every state change.
fn write_status(core: &Core, stopped: bool) {
    let mut done = STATUS_WRITE.lock().unwrap_or_else(|p| p.into_inner());
    if *done {
        return;
    }
    *done = stopped;
    let mut status = core.status_json();
    if stopped {
        status["state"] = "stopped".into();
    }
    let path = core.config.data_dir.join("status.json");
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(&status).map_err(std::io::Error::other).and_then(|bytes| std::fs::write(&tmp, bytes));
    // `rename` replaces the old file, which is what we want for a status file. A script that has the old one open at that very moment
    // makes Windows say "access denied": try again a few times (the file is rewritten every second during a transfer).
    let result = written.and_then(|()| {
        let mut last = Ok(());
        for attempt in 0..5 {
            last = std::fs::rename(&tmp, &path);
            if last.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10 * (attempt + 1)));
        }
        last
    });
    if let Err(e) = result {
        log::warn!("writing status.json failed: {e}");
    }
}

/// "Text from <phone>" with a preview. With a link, a click opens it (`register_toast_clicks`). The preview is the user's own
/// text and is shown only here: the log gets the title and a length.
fn text_toast(app: &AppHandle, core: &Core, id: u32, from: &str, preview: &str, has_link: bool) {
    let title = format!("Text from {from}");
    log::info!("toast: {title} ({} characters shown{})", preview.chars().count(), if has_link { ", click opens the link" } else { "" });
    if core.config.toasts_are_logged_only() {
        return;
    }
    if let Err(e) = app.notification().builder().id(id as i32).title(title).body(preview).show() {
        log::warn!("showing a toast failed: {e}");
    }
}

/// "<n> file(s) from <phone>": a click opens the file (one file) or the folder (several). The title and the names are the PC's
/// own (the names were cleaned when the files were saved).
fn files_toast(app: &AppHandle, core: &Core, id: u32, title: &str, body: &str, single: bool, openable: bool) {
    log::info!("toast: {title} ({} characters of names shown)", body.chars().count());
    if core.config.toasts_are_logged_only() {
        return;
    }
    let kind = match (single, openable) {
        (true, true) => ACTIONS_ONE_FILE,
        (true, false) => ACTIONS_ONE_FILE_SHOW_ONLY,
        (false, _) => ACTIONS_SEVERAL_FILES,
    };
    if let Err(e) = app.notification().builder().id(id as i32).title(title).body(body).action_type_id(kind).show() {
        log::warn!("showing a toast failed: {e}");
    }
}

/// The buttons of a toast for one received file ("Open", "Show in folder"), for one whose type may not be opened ("Show in folder"
/// alone: see `links::may_open_file`) and for several ("Open folder"). A click on the toast itself does what the first button does.
const ACTIONS_ONE_FILE: &str = "received_file";
const ACTIONS_ONE_FILE_SHOW_ONLY: &str = "received_file_show_only";
const ACTIONS_SEVERAL_FILES: &str = "received_files";

fn register_toast_buttons(app: &AppHandle) {
    use tauri_plugin_notification::{Action, ActionType};
    let one = ActionType::builder(ACTIONS_ONE_FILE)
        .actions(vec![Action::builder("open", "Open").build(), Action::builder("show", "Show in folder").build()])
        .build();
    let show_only =
        ActionType::builder(ACTIONS_ONE_FILE_SHOW_ONLY).actions(vec![Action::builder("show", "Show in folder").build()]).build();
    let several = ActionType::builder(ACTIONS_SEVERAL_FILES).actions(vec![Action::builder("open", "Open folder").build()]).build();
    if let Err(e) = app.notification().register_action_types(vec![one, show_only, several]) {
        log::warn!("toast buttons are not available: {e}");
    }
}

/// A click on a toast ("tap") does what the toast was made for: opens its link, its file or its folder. The plugin reports a click
/// only while the toast is on screen, not once it has moved to the Action Center. A click that doesn't say which toast it was
/// opens nothing: there is no guess about the newest one.
fn register_toast_clicks(app: &AppHandle, core: &Arc<Core>) {
    let core = core.clone();
    let registered = app.notification().on_action(move |performed| {
        // "tap" is a click on the toast, "open" and "show" are its buttons. Anything else (a dismissal) does nothing.
        let action = performed.action_id();
        if !matches!(action, "tap" | "open" | "show") {
            return;
        }
        if let Some(id) = performed.notification().map(|n| n.id() as u32) {
            core.toast_action(id, action);
        }
    });
    register_toast_buttons(app);
    if let Err(e) = registered {
        log::warn!("toast clicks are not available: {e}");
    }
}

/// Registers the global hotkey from the settings. If it can't be registered (another program has it, or the text isn't an
/// accelerator) the app carries on without it, says so once, and `status.json` shows why.
fn register_hotkey(app: &AppHandle, core: &Core) {
    let accelerator = core.hotkey_accelerator();
    let mut status = HotkeyStatus { accelerator: accelerator.clone(), registered: false, error: None };
    if accelerator.is_empty() {
        log::info!("hotkey: turned off in settings.json");
    } else {
        let outcome = crate::settings::parse_hotkey(&accelerator).and_then(|shortcut| {
            app.global_shortcut()
                .register(shortcut)
                .map_err(|e| format!("{accelerator} could not be registered ({e}). Another program may use it"))
        });
        match outcome {
            Ok(()) => {
                log::info!("hotkey {accelerator} registered: sends the clipboard text to the phone");
                status.registered = true;
            }
            Err(e) => {
                log::warn!("hotkey: {e}");
                toast(app, core, &format!("The hotkey doesn't work: {e}. Change \"hotkey\" in settings.json."));
                status.error = Some(e);
            }
        }
    }
    core.set_hotkey_status(status);
}

/// The hotkey, the tray item and `--send-clipboard` all end up here. The tap is now: the time from here to the phone's `ack` is
/// what targets.md calls arrival time.
fn send_clipboard(app: &AppHandle, trigger: &'static str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let core = state.core.clone();
    let tap = Tap::now();
    log::info!("send clipboard ({trigger})");
    tauri::async_runtime::spawn(async move {
        core.send_clipboard_text(trigger, tap).await;
    });
}

fn toast(app: &AppHandle, core: &Core, text: &str) {
    toast_logged(app, core, text, text);
}

/// A toast whose text has something in it that stays out of the log (a file name): `log_text` is what the log gets.
fn toast_logged(app: &AppHandle, core: &Core, text: &str, log_text: &str) {
    log::info!("toast: {log_text}");
    if core.config.toasts_are_logged_only() {
        return;
    }
    if let Err(e) = app.notification().builder().title("Kin").body(text).show() {
        log::warn!("showing a toast failed: {e}");
    }
}

pub fn on_menu(app: &AppHandle, id: &str) {
    match id {
        tray::MENU_SEND_CLIPBOARD => send_clipboard(app, "tray"),
        tray::MENU_SEND_FILES => {
            if let Some(state) = app.try_state::<AppState>() {
                ui::pick_and_send(app, state.core.clone());
            }
        }
        tray::MENU_OPEN_FOLDER => open_received_folder(app),
        tray::MENU_PAIR => handle_commands(app, &[Command::Pair]),
        tray::MENU_HISTORY => handle_commands(app, &[Command::History]),
        tray::MENU_SETTINGS => handle_commands(app, &[Command::Settings]),
        tray::MENU_QUIT => handle_commands(app, &[Command::Quit]),
        _ => {}
    }
}

fn open_received_folder(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let dir = state.core.received_dir();
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
        // The name only: a developer command can carry clipboard text.
        log::info!("command: {}", command.name());
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
            Command::SendClipboard => send_clipboard(app, "command"),
            Command::Send(paths) => {
                let paths = paths.clone();
                tauri::async_runtime::spawn(async move {
                    core.send_paths(paths, "command").await;
                });
            }
            Command::Drop => ui::open_drop_window(app),
            Command::History => ui::open_history_window(app),
            Command::Settings => ui::open_settings_window(app),
            Command::DebugWindow => ui::open_debug_window(app),
            Command::DevDumpView(name) => {
                let (app, name) = (app.clone(), name.clone());
                tauri::async_runtime::spawn(async move { dev_dump_view(&app, &core, &name).await });
            }
            Command::DevExportLogs(path) => {
                let path = path.clone();
                tauri::async_runtime::spawn(async move { dev_export_logs(&core, &path).await });
            }
            Command::DevHistory { action, id } => {
                let (action, id) = (action.clone(), id.clone());
                tauri::async_runtime::spawn(async move { dev_history(&core, &action, &id).await });
            }
            Command::DevSet { key, value } => {
                let (app, key, value) = (app.clone(), key.clone(), value.clone());
                tauri::async_runtime::spawn_blocking(move || dev_set(&app, &core, &key, &value));
            }
            Command::CancelTransfers => {
                tauri::async_runtime::spawn(async move { core.cancel_all_transfers().await });
            }
            Command::DevSetClipboard { text, mark } => {
                let (text, mark) = (text.clone(), *mark);
                tauri::async_runtime::spawn(async move { core.dev_set_clipboard(text, mark).await });
            }
            Command::DevGetClipboard => {
                tauri::async_runtime::spawn(async move { core.dev_get_clipboard().await });
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

/// The hotkey of the settings window. The new one is registered first; if that fails (another program has it, or the text isn't a hotkey)
/// the old one is put back and the error says why. An empty text turns the hotkey off. Returns the new state of the hotkey.
pub fn change_hotkey(app: &AppHandle, core: &Core, accelerator: &str) -> std::result::Result<serde_json::Value, String> {
    let old = core.hotkey_accelerator();
    let was_registered = lock_hotkey(core).registered;
    let old_shortcut = if was_registered { crate::settings::parse_hotkey(&old).ok() } else { None };
    let shortcuts = app.global_shortcut();
    if accelerator.is_empty() {
        if let Some(o) = old_shortcut {
            let _ = shortcuts.unregister(o);
        }
        core.store_hotkey("");
        core.set_hotkey_status(HotkeyStatus { accelerator: String::new(), registered: false, error: None });
        log::info!("hotkey: turned off in the settings window");
        return Ok(serde_json::json!({ "accelerator": "", "registered": false }));
    }
    let shortcut = crate::settings::parse_hotkey(accelerator)?;
    // The old one goes first, so that choosing the same key again (or a key that only differs in spelling) is not a conflict with itself.
    if let Some(o) = old_shortcut {
        let _ = shortcuts.unregister(o);
    }
    match shortcuts.register(shortcut) {
        Ok(()) => {
            core.store_hotkey(accelerator);
            core.set_hotkey_status(HotkeyStatus { accelerator: accelerator.to_string(), registered: true, error: None });
            log::info!("hotkey {accelerator} registered from the settings window");
            Ok(serde_json::json!({ "accelerator": accelerator, "registered": true }))
        }
        Err(e) => {
            if let Some(o) = old_shortcut {
                let _ = shortcuts.register(o);
            }
            log::warn!("hotkey {accelerator} could not be registered from the settings window: {e}");
            Err(format!(
                "{accelerator} can't be used: another program already has it ({e}). The hotkey stays {}.",
                if old.is_empty() { "off".to_string() } else { old }
            ))
        }
    }
}

fn lock_hotkey(core: &Core) -> HotkeyStatus {
    core.hotkey_status()
}

/// What a window would show, written to `view-<name>.json` in the data directory (the developer hook `--dev-dump-view`).
async fn dev_dump_view(app: &AppHandle, core: &Arc<Core>, name: &str) {
    if !core.config.dev_hooks_enabled() {
        log::warn!("refused --dev-dump-view: it works only when the app was started with a developer setting");
        return;
    }
    let state = app.state::<AppState>();
    let value = match name {
        "history" => core.history_view(),
        "queue" => core.queue_status(),
        "settings" => {
            let mut v = core.settings_view(state.autostart.is_enabled().ok());
            v["startWithWindowsIsReal"] = serde_json::json!(state.autostart.is_real());
            v
        }
        "debug" => ui::debug_json(core).await,
        other => {
            log::warn!("--dev-dump-view: no view called {other:?} (history, queue, settings, debug)");
            return;
        }
    };
    let path = core.config.data_dir.join(format!("view-{name}.json"));
    match std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap_or_default()) {
        Ok(()) => log::info!("view {name} written for a script"),
        Err(e) => log::warn!("writing view-{name}.json failed: {e}"),
    }
}

/// The developer hook `--dev-export-logs <path>`: "Export logs" without the dialog.
async fn dev_export_logs(core: &Arc<Core>, path: &Path) {
    if !core.config.dev_hooks_enabled() {
        log::warn!("refused --dev-export-logs: it works only when the app was started with a developer setting");
        return;
    }
    let network = match tauri::async_runtime::spawn_blocking(crate::netprofile::connected_networks).await {
        Ok(Ok(list)) => serde_json::json!({ "networks": list }),
        _ => serde_json::json!({ "networks": [] }),
    };
    let (core, path) = (core.clone(), path.to_path_buf());
    let result = tauri::async_runtime::spawn_blocking(move || core.export_logs(&path, network)).await;
    match result {
        Ok(Ok(report)) => log::info!("developer export: {} file(s), {} bytes", report.files, report.bytes),
        Ok(Err(e)) => log::warn!("developer export failed: {e}"),
        Err(e) => log::warn!("developer export failed: {e}"),
    }
}

/// The developer hook `--dev-set <setting> <value>`: what the settings window does, without a window. The outcome goes to
/// `dev-set-result.json` in the data directory (and the log).
fn dev_set(app: &AppHandle, core: &Arc<Core>, key: &str, value: &str) {
    let write = |ok: bool, message: String| {
        log::info!("developer setting {key}: {} ({message})", if ok { "ok" } else { "refused" });
        let text = serde_json::json!({ "key": key, "ok": ok, "message": message });
        let _ = std::fs::write(core.config.data_dir.join("dev-set-result.json"), text.to_string());
    };
    if !core.config.dev_hooks_enabled() {
        log::warn!("refused --dev-set: it works only when the app was started with a developer setting");
        return;
    }
    let on = matches!(value, "true" | "1" | "on");
    let state = app.state::<AppState>();
    match key {
        "deviceName" => write(true, core.set_device_name(value)),
        "autoClipboardSync" => {
            core.set_auto_clipboard_sync(on);
            write(true, on.to_string());
        }
        "imagesToClipboard" => {
            core.set_images_to_clipboard(on);
            write(true, on.to_string());
        }
        "receivedFolder" => match core.set_received_folder(Some(value)) {
            Ok(path) => write(true, path.to_string_lossy().into_owned()),
            Err(e) => write(false, e),
        },
        "hotkey" => match change_hotkey(app, core, value.trim()) {
            Ok(v) => write(true, v.to_string()),
            Err(e) => write(false, e),
        },
        "startWithWindows" => match state.autostart.set_enabled(on) {
            Ok(()) => {
                crate::integration::remember_start_with_windows(&core.config.data_dir, on);
                write(true, format!("{} (real: {})", on, state.autostart.is_real()))
            }
            Err(e) => write(false, e),
        },
        other => write(false, format!("no setting called {other:?}")),
    }
}

/// The developer hook `--dev-history <cancel|retry|open> <id>`: what the history window's buttons and clicks do, without a window.
async fn dev_history(core: &Arc<Core>, action: &str, id: &str) {
    if !core.config.dev_hooks_enabled() {
        log::warn!("refused --dev-history: it works only when the app was started with a developer setting");
        return;
    }
    let (ok, message) = match action {
        "cancel" => (core.history_cancel(id).await, "canceled".to_string()),
        "retry" => match core.retry_item(id).await {
            Ok(new_id) => (true, new_id),
            Err(e) => (false, e),
        },
        "open" => match core.history_open(id).await {
            Ok(m) => (true, m),
            Err(e) => (false, e),
        },
        other => (false, format!("no action called {other:?} (cancel, retry, open)")),
    };
    log::info!("developer history action {action}: {}", if ok { "ok" } else { "refused" });
    let text = serde_json::json!({ "action": action, "id": id, "ok": ok, "message": message });
    let _ = std::fs::write(core.config.data_dir.join("dev-history-result.json"), text.to_string());
}
