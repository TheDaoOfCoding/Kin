//! Kin Relay for Windows, milestone 1.1: identity, the HTTPS server, pairing, mDNS, the WebSocket and its ping, and a tray.
//!
//! The protocol and the app's state have no UI in them (`messages`, `server`, `ws`, `core`, ...), so the integration tests
//! can run them in the test process. The Tauri side is `app`, `tray` and `ui`.
pub mod app;
pub mod cli;
pub mod connections;
pub mod core;
mod dpapi;
pub mod encoding;
pub mod filelog;
pub mod identity;
pub mod json;
pub mod mdns;
pub mod messages;
pub mod netaddr;
pub mod pairing;
pub mod server;
pub mod storage;
pub mod tls;
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
