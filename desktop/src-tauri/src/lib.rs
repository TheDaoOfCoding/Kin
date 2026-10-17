//! Kin Relay for Windows, milestone 1.2: identity, the HTTPS server, pairing, mDNS, the WebSocket and its ping, a tray, and text,
//! links and the clipboard (`clipboard`, `links`, `sync`, `settings`).
//!
//! The protocol and the app's state have no UI in them (`messages`, `server`, `ws`, `core`, ...), so the integration tests
//! can run them in the test process. The Tauri side is `app`, `tray` and `ui`.
pub mod app;
pub mod arrival;
pub mod autostart;
pub mod cli;
pub mod clipboard;
pub mod connections;
pub mod core;
mod dpapi;
pub mod encoding;
pub mod export;
pub mod features;
pub mod filelog;
pub mod filenames;
pub mod fileserver;
pub mod history;
pub mod identity;
pub mod images;
pub mod instance;
pub mod integration;
pub mod json;
pub mod links;
pub mod localtime;
pub mod mdns;
pub mod media;
pub mod messages;
pub mod metrics;
pub mod mirror;
pub mod motw;
pub mod nativehost;
pub mod netaddr;
pub mod netprofile;
pub mod outgoing;
pub mod pairing;
pub mod paths;
pub mod phonelinks;
pub mod powerwatch;
pub mod prefs;
pub mod presence;
pub mod privacy;
pub mod queue;
pub mod queuedb;
pub mod quiet;
pub mod ring;
pub mod server;
pub mod settings;
pub mod storage;
pub mod sync;
pub mod tls;
pub mod transfers;
pub mod tray;
pub mod ui;
pub mod ws;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The entry point of the `Kin` executable.
pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args, &|k| std::env::var(k).ok()) {
        Ok(parsed) => app::run(parsed),
        Err(e) => {
            eprintln!("Kin: {e}");
            std::process::exit(2);
        }
    }
}
