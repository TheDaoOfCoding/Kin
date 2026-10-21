// A stand-in for Kin on the extension pipe (dev only, feature `mock`). Used by smoke/run-real-host.ts and the tests: it is also what the
// host starts when the tests point KIN_HOST_DEV_KIN_EXE at it.
//
//   mock-kin --pipe <\\.\pipe\name> --state <dir> [--manual]
//
// The pipe and the state folder can come from KIN_MOCK_PIPE and KIN_MOCK_STATE instead (the host starts it without arguments).
// Started by a host (no --manual) it first checks <dir>/scenario.json: "startable": false makes it exit at once, like a Kin that can't start;
// "startDelayMs" waits before the pipe appears. It refuses any pipe name that isn't a dev or test one: it can never take the place of the
// user's real Kin on the release pipe.
#![windows_subsystem = "windows"]

use std::time::Duration;

use kin_native_host::config::DEV_HOST_NAME;
use kin_native_host::mock::kin::KinMock;
use kin_native_host::mock::server::Server;

fn arg(args: &[String], name: &str, env: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()).or_else(|| std::env::var(env).ok())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(pipe) = arg(&args, "--pipe", "KIN_MOCK_PIPE") else { std::process::exit(2) };
    let Some(state) = arg(&args, "--state", "KIN_MOCK_STATE") else { std::process::exit(2) };
    if !(pipe.starts_with(&format!(r"\\.\pipe\{DEV_HOST_NAME}")) || pipe.starts_with(r"\\.\pipe\kin-host-test-")) {
        std::process::exit(2);
    }
    let state = std::path::PathBuf::from(state);
    // One line per start, so a test can count how many times the host started it.
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(state.join("starts.log")) {
        use std::io::Write;
        let _ = writeln!(f, "{}", std::process::id());
    }
    let kin = KinMock::new(&state);
    if !args.iter().any(|a| a == "--manual") {
        let s = kin.scenario();
        if s["startable"] == serde_json::json!(false) {
            std::process::exit(1);
        }
        if let Some(ms) = s["startDelayMs"].as_u64() {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
    let Ok(_server) = Server::start(&pipe, kin.handler()) else { std::process::exit(3) };
    let _ = std::fs::write(state.join("kin.pid"), std::process::id().to_string());
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
