//! `kin-native-host`: the native messaging host of the Kin browser extension. A thin relay between the browser (stdin/stdout, native
//! messaging framing) and the running Kin (a named pipe, same framing). The contract is docs/extension.md.

pub mod config;
pub mod envelope;
pub mod framing;
pub mod host;
pub mod pipe;
pub mod winutil;

#[cfg(feature = "mock")]
pub mod mock;

pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A line on stderr (the browser discards it; a test can read it). Never a request body, a URL or a text: only what happened.
pub fn log(msg: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "kin-native-host: {msg}");
}

/// What the browser told the host about its caller, to be passed to Kin as `origin`: Chromium's first argument is the origin
/// (`chrome-extension://<id>/`); Firefox's first is the manifest's path and its second the extension ID. Anything else gives "".
pub fn origin_from_args(args: &[String]) -> String {
    let candidate = match args.first().map(String::as_str) {
        Some(a) if a.starts_with("chrome-extension://") => a,
        Some(a) if a.to_ascii_lowercase().ends_with(".json") => args.get(1).map(String::as_str).unwrap_or(""),
        _ => "",
    };
    let ok = candidate.len() <= 200 && candidate.chars().all(|c| c.is_ascii_graphic());
    if ok && !candidate.starts_with("--") { candidate.to_string() } else { String::new() }
}

#[cfg(test)]
mod tests {
    use super::origin_from_args;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn chromium_and_firefox_arguments() {
        assert_eq!(origin_from_args(&a(&["chrome-extension://abc/", "--parent-window=0"])), "chrome-extension://abc/");
        assert_eq!(
            origin_from_args(&a(&[r"C:\Program Files\Kin\native-messaging\app.kinrelay.desktop.firefox.json", "kin@kinrelay.app"])),
            "kin@kinrelay.app"
        );
        assert_eq!(origin_from_args(&a(&[])), "");
        assert_eq!(origin_from_args(&a(&["--parent-window=5"])), "");
        assert_eq!(origin_from_args(&a(&["chrome-extension://abc\n/"])), "");
        assert_eq!(origin_from_args(&a(&["chrome-extension://".to_string().repeat(30).as_str()])), "");
    }
}
