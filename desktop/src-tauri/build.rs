/// Every command a window may call. Listing them here makes Tauri **refuse any command that no capability names for that window**
/// (`capabilities/<window>.json`): a window gets the commands it needs and no others. Keep this list, `generate_handler!` in `app.rs` and
/// the capability files in step (a test in `tests/review_1_4.rs` reads all three).
const COMMANDS: &[&str] = &[
    "pairing_view",
    "new_pairing",
    "copy_pairing_link",
    "remove_device",
    "close_window",
    "drop_view",
    "send_files",
    "pick_files",
    "cancel_transfer",
    "open_window",
    "history_view",
    "history_open",
    "history_cancel",
    "history_retry",
    "settings_view",
    "set_device_name",
    "set_auto_clipboard_sync",
    "set_images_to_clipboard",
    "set_open_links_automatically",
    "set_mirror_notifications",
    "set_mirror_option",
    "set_media_option",
    "media_command",
    "set_pause",
    "ring_phone",
    "set_presence_option",
    "choose_received_folder",
    "reset_received_folder",
    "set_hotkey",
    "set_start_with_windows",
    "debug_view",
    "debug_ping",
    "export_logs",
];

fn main() {
    let attributes = tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(e) = tauri_build::try_build(attributes) {
        panic!("the Tauri build step failed: {e:#}");
    }
}
