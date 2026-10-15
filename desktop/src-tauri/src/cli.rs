//! Command line and environment: where the data lives, the developer settings, and the commands that other launches
//! can send to the running instance (see desktop/README.md).
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use crate::clipboard::Exclusion;
use crate::netaddr;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Settings, key, database, logs. `%APPDATA%\Kin` unless overridden.
    pub data_dir: PathBuf,
    /// Developer setting: bind only to this loopback address. Never anything else.
    pub dev_bind: Option<IpAddr>,
    /// Developer setting: put this address (with the real port) in the QR code instead of the PC's own.
    pub dev_qr_address: Option<Ipv4Addr>,
    pub dev_port: Option<u16>,
    /// Write the pairing URL to `pair-url.txt` in the data directory and to stdout whenever a pairing starts.
    pub dev_print_pair_url: bool,
    /// Don't create windows or show toasts. A toast is logged instead.
    pub dev_no_windows: bool,
    /// Developer setting: received files go here instead of `Downloads\Kin` (and instead of `receivedFolder` of settings.json), so
    /// a test never writes to the real Downloads folder.
    pub dev_received_dir: Option<PathBuf>,
    /// Developer hook: pretend the disk has this many free bytes (the check for `507 disk_full` uses it).
    pub dev_free_bytes: Option<u64>,
}

impl Config {
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            data_dir,
            dev_bind: None,
            dev_qr_address: None,
            dev_port: None,
            dev_print_pair_url: false,
            dev_no_windows: false,
            dev_received_dir: None,
            dev_free_bytes: None,
        }
    }

    /// Whether the app was started with any developer setting. The developer clipboard commands (`--dev-set-clipboard`,
    /// `--dev-get-clipboard`) are honoured only then: in a normal run any other program could launch `Kin.exe` with them.
    pub fn dev_hooks_enabled(&self) -> bool {
        self.dev_bind.is_some()
            || self.dev_qr_address.is_some()
            || self.dev_port.is_some()
            || self.dev_print_pair_url
            || self.dev_no_windows
            || self.dev_received_dir.is_some()
            || self.dev_free_bytes.is_some()
    }

    /// Any developer address setting means loopback only.
    pub fn is_dev_network(&self) -> bool {
        self.dev_bind.is_some() || self.dev_qr_address.is_some()
    }

    /// The address to bind to: loopback in dev mode, every interface otherwise.
    pub fn bind_ip(&self) -> IpAddr {
        match (self.dev_bind, self.dev_qr_address) {
            (Some(ip), _) => ip,
            (None, Some(_)) => Ipv4Addr::LOCALHOST.into(),
            (None, None) => Ipv4Addr::UNSPECIFIED.into(),
        }
    }
}

/// What another launch can ask the running instance to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Send a `ping` to the connected phone and show the round-trip time.
    Ping,
    /// Open the pairing window, like the tray menu does.
    Pair,
    /// Start a pairing without a window. The URL goes to `pair-url.txt`.
    StartPairing,
    /// Send the text on the clipboard to the phone as a `text` message. The same as the hotkey and "Send clipboard to phone"
    /// in the tray, and what the integration agent uses to press the hotkey without a keyboard.
    SendClipboard,
    /// Developer hook: set the PC clipboard as if the user had copied this text. `mark` adds the format a password manager
    /// would add, so the "not sent" rule can be tested.
    DevSetClipboard { text: String, mark: Option<Exclusion> },
    /// Developer hook: write the PC clipboard text to `dev-clipboard.txt` in the data directory.
    DevGetClipboard,
    /// Send these files to the phone: Explorer's "Send to" (`Kin.exe --send <paths>`), the same as dropping them on the drop window.
    Send(Vec<PathBuf>),
    /// Open the drop window, like the tray's left click.
    Drop,
    /// Cancel every transfer that is going, in both directions: what the Cancel button of the drop window does for each row.
    CancelTransfers,
    /// Quit.
    Quit,
}

impl Command {
    /// A word for the log. (The `Debug` form of `DevSetClipboard` would print the text.)
    pub fn name(&self) -> &'static str {
        match self {
            Command::Ping => "ping",
            Command::Pair => "pair",
            Command::StartPairing => "start pairing",
            Command::SendClipboard => "send clipboard",
            Command::DevSetClipboard { .. } => "dev set clipboard",
            Command::DevGetClipboard => "dev get clipboard",
            Command::Send(_) => "send files",
            Command::Drop => "drop window",
            Command::CancelTransfers => "cancel transfers",
            Command::Quit => "quit",
        }
    }
}

/// The values of `--dev-clipboard-mark`.
pub fn parse_mark(value: &str) -> Option<Exclusion> {
    match value {
        "exclude" => Some(Exclusion::ExcludeFromMonitor),
        "no-history" => Some(Exclusion::NotInHistory),
        "no-cloud" => Some(Exclusion::NotToCloud),
        "viewer-ignore" => Some(Exclusion::ViewerIgnore),
        _ => None,
    }
}

#[derive(Debug)]
pub struct Parsed {
    pub config: Config,
    pub commands: Vec<Command>,
    pub unknown: Vec<String>,
}

fn default_data_dir(env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    env("APPDATA").map(|a| PathBuf::from(a).join("Kin")).ok_or_else(|| "%APPDATA% is not set; use --data-dir".to_string())
}

fn truthy(v: Option<String>) -> bool {
    matches!(v.as_deref(), Some("1" | "true"))
}

/// `args` is the command line without the program name. Settings can also come from `KIN_*` environment variables;
/// a flag wins over its variable. Unknown arguments are returned, not rejected: a later milestone adds `--send`.
pub fn parse(args: &[String], env: &dyn Fn(&str) -> Option<String>) -> Result<Parsed, String> {
    let mut data_dir = env("KIN_DATA_DIR").map(PathBuf::from);
    let mut dev_bind = env("KIN_DEV_BIND");
    let mut dev_qr = env("KIN_DEV_QR_ADDRESS");
    let mut dev_port = env("KIN_DEV_PORT");
    let mut print_url = truthy(env("KIN_DEV_PRINT_PAIR_URL"));
    let mut no_windows = truthy(env("KIN_DEV_NO_WINDOWS"));
    let mut received_dir = env("KIN_DEV_RECEIVED_DIR").map(PathBuf::from);
    let mut free_bytes = env("KIN_DEV_FREE_BYTES");
    let (mut commands, mut unknown) = (Vec::new(), Vec::new());
    let mut mark: Option<Exclusion> = None;

    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = |flag: &str| -> Result<String, String> {
            inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("{flag} needs a value"))
        };
        match name {
            "--data-dir" => data_dir = Some(PathBuf::from(value(name)?)),
            "--dev-bind" => dev_bind = Some(value(name)?),
            "--dev-qr-address" => dev_qr = Some(value(name)?),
            "--dev-port" => dev_port = Some(value(name)?),
            "--dev-print-pair-url" => print_url = true,
            "--dev-no-windows" => no_windows = true,
            "--dev-received-dir" => received_dir = Some(PathBuf::from(value(name)?)),
            "--dev-free-bytes" => free_bytes = Some(value(name)?),
            "--drop" => commands.push(Command::Drop),
            "--cancel-transfers" => commands.push(Command::CancelTransfers),
            // Every argument up to the next flag is a path: Explorer's "Send to" appends the selected files to the shortcut.
            "--send" => {
                let mut paths: Vec<PathBuf> = inline.clone().map(PathBuf::from).into_iter().collect();
                while let Some(next) = it.next_if(|a| !a.starts_with("--")) {
                    paths.push(PathBuf::from(next));
                }
                commands.push(Command::Send(paths));
            }
            "--ping" => commands.push(Command::Ping),
            "--pair" => commands.push(Command::Pair),
            "--dev-start-pairing" => commands.push(Command::StartPairing),
            "--send-clipboard" => commands.push(Command::SendClipboard),
            "--dev-set-clipboard" => commands.push(Command::DevSetClipboard { text: value(name)?, mark: None }),
            "--dev-clipboard-mark" => {
                let v = value(name)?;
                mark =
                    Some(parse_mark(&v).ok_or_else(|| {
                        format!("--dev-clipboard-mark: {v:?} is not one of exclude, no-history, no-cloud, viewer-ignore")
                    })?);
            }
            "--dev-get-clipboard" => commands.push(Command::DevGetClipboard),
            "--quit" => commands.push(Command::Quit),
            _ => unknown.push(arg.clone()),
        }
    }

    if let Some(mark) = mark {
        let mut applied = false;
        for c in &mut commands {
            if let Command::DevSetClipboard { mark: m, .. } = c {
                *m = Some(mark);
                applied = true;
            }
        }
        if !applied {
            return Err("--dev-clipboard-mark needs --dev-set-clipboard".to_string());
        }
    }

    let data_dir = match data_dir {
        Some(d) => d,
        None => default_data_dir(env)?,
    };
    let mut config = Config::new(data_dir);
    if let Some(v) = dev_bind {
        let ip: IpAddr = v.parse().map_err(|_| format!("--dev-bind: {v:?} is not an IP address"))?;
        if !ip.is_loopback() {
            return Err(format!("--dev-bind: {ip} is not a loopback address. Developer mode never listens on the network"));
        }
        config.dev_bind = Some(ip);
    }
    if let Some(v) = dev_qr {
        let ip: Ipv4Addr = v.parse().map_err(|_| format!("--dev-qr-address: {v:?} is not an IPv4 address"))?;
        if !netaddr::is_private(ip) && !netaddr::is_shared_space(ip) {
            return Err(format!("--dev-qr-address: {ip} is not an address the phone accepts (private or 100.64.0.0/10)"));
        }
        config.dev_qr_address = Some(ip);
    }
    if let Some(v) = dev_port {
        config.dev_port = Some(v.parse().map_err(|_| format!("--dev-port: {v:?} is not a port"))?);
    }
    config.dev_print_pair_url = print_url;
    config.dev_no_windows = no_windows;
    config.dev_received_dir = received_dir;
    if let Some(v) = free_bytes {
        config.dev_free_bytes = Some(v.parse().map_err(|_| format!("--dev-free-bytes: {v:?} is not a number of bytes"))?);
    }
    Ok(Parsed { config, commands, unknown })
}

/// A second launch passes its working directory along. A relative path in `--send` is relative to that.
pub fn resolve_paths(commands: &mut [Command], cwd: &std::path::Path) {
    for c in commands {
        if let Command::Send(paths) = c {
            for p in paths.iter_mut() {
                if p.is_relative() {
                    *p = cwd.join(&p);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn appdata(k: &str) -> Option<String> {
        (k == "APPDATA").then(|| r"C:\Users\x\AppData\Roaming".to_string())
    }

    #[test]
    fn defaults_to_appdata_kin() {
        let p = parse(&[], &appdata).unwrap();
        assert_eq!(p.config.data_dir, PathBuf::from(r"C:\Users\x\AppData\Roaming").join("Kin"));
        assert!(!p.config.is_dev_network());
        assert_eq!(p.config.bind_ip(), IpAddr::from(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn dev_flags_and_commands() {
        let p = parse(&args("--data-dir C:\\tmp\\kin --dev-bind 127.0.0.1 --dev-qr-address=10.0.2.2 --dev-port 50000 --dev-print-pair-url --dev-no-windows --ping --quit"), &no_env).unwrap();
        assert_eq!(p.config.data_dir, PathBuf::from("C:\\tmp\\kin"));
        assert_eq!(p.config.bind_ip(), IpAddr::from(Ipv4Addr::LOCALHOST));
        assert_eq!(p.config.dev_qr_address, Some("10.0.2.2".parse().unwrap()));
        assert_eq!(p.config.dev_port, Some(50000));
        assert!(p.config.dev_print_pair_url && p.config.dev_no_windows);
        assert_eq!(p.commands, [Command::Ping, Command::Quit]);
    }

    #[test]
    fn environment_variables_and_flags_win_over_them() {
        let env = |k: &str| match k {
            "KIN_DATA_DIR" => Some("C:\\from-env".to_string()),
            "KIN_DEV_BIND" => Some("127.0.0.1".to_string()),
            _ => None,
        };
        assert_eq!(parse(&[], &env).unwrap().config.data_dir, PathBuf::from("C:\\from-env"));
        assert_eq!(parse(&args("--data-dir C:\\flag"), &env).unwrap().config.data_dir, PathBuf::from("C:\\flag"));
    }

    #[test]
    fn a_qr_address_alone_still_means_loopback_only() {
        let p = parse(&args("--data-dir x --dev-qr-address 10.0.2.2"), &no_env).unwrap();
        assert!(p.config.is_dev_network());
        assert_eq!(p.config.bind_ip(), IpAddr::from(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn dev_bind_refuses_anything_but_loopback() {
        for bad in ["0.0.0.0", "192.168.1.5", "::", "not-an-ip"] {
            assert!(parse(&args(&format!("--data-dir x --dev-bind {bad}")), &no_env).is_err(), "{bad}");
        }
        assert!(parse(&args("--data-dir x --dev-bind ::1"), &no_env).is_ok());
        assert!(parse(&args("--data-dir x --dev-qr-address 8.8.8.8"), &no_env).is_err());
    }

    #[test]
    fn the_clipboard_commands() {
        let p = parse(&args("--data-dir x --send-clipboard --dev-get-clipboard"), &no_env).unwrap();
        assert_eq!(p.commands, [Command::SendClipboard, Command::DevGetClipboard]);
        let argv = vec!["--dev-set-clipboard".to_string(), "two words".to_string(), "--dev-clipboard-mark=no-history".to_string()];
        let p = parse(&[vec!["--data-dir".to_string(), "x".to_string()], argv].concat(), &no_env).unwrap();
        assert_eq!(p.commands, [Command::DevSetClipboard { text: "two words".into(), mark: Some(Exclusion::NotInHistory) }]);
        let p = parse(&args("--data-dir x --dev-set-clipboard plain"), &no_env).unwrap();
        assert_eq!(p.commands, [Command::DevSetClipboard { text: "plain".into(), mark: None }]);
        assert!(parse(&args("--data-dir x --dev-clipboard-mark exclude"), &no_env).is_err(), "a mark with nothing to mark");
        assert!(parse(&args("--data-dir x --dev-set-clipboard a --dev-clipboard-mark nope"), &no_env).is_err());
        assert!(parse(&args("--data-dir x --dev-set-clipboard"), &no_env).is_err(), "no text");
    }

    #[test]
    fn a_command_is_logged_by_name_and_never_with_its_text() {
        let c = Command::DevSetClipboard { text: "secret".into(), mark: None };
        assert_eq!(c.name(), "dev set clipboard");
        assert!(!c.name().contains("secret"));
    }

    #[test]
    fn unknown_arguments_are_reported_not_fatal() {
        let p = parse(&args("--data-dir x --whatever C:\\a.txt"), &no_env).unwrap();
        assert_eq!(p.unknown, ["--whatever", "C:\\a.txt"]);
        assert!(parse(&args("--data-dir"), &no_env).is_err());
    }

    #[test]
    fn send_takes_every_path_up_to_the_next_flag() {
        let p = parse(&args("--data-dir x --send C:\\a.txt D:\\b.jpg --ping"), &no_env).unwrap();
        assert_eq!(p.commands, [Command::Send(vec!["C:\\a.txt".into(), "D:\\b.jpg".into()]), Command::Ping]);
        let p = parse(&args("--data-dir x --send"), &no_env).unwrap();
        assert_eq!(p.commands, [Command::Send(vec![])]);
        let argv = vec!["--data-dir".to_string(), "x".to_string(), "--send".to_string(), "C:\\with space\\a b.txt".to_string()];
        assert_eq!(parse(&argv, &no_env).unwrap().commands, [Command::Send(vec!["C:\\with space\\a b.txt".into()])]);
    }

    #[test]
    fn cancel_transfers_is_a_command() {
        let p = parse(&args("--data-dir x --cancel-transfers"), &no_env).unwrap();
        assert_eq!(p.commands, [Command::CancelTransfers]);
        assert_eq!(Command::CancelTransfers.name(), "cancel transfers");
    }

    #[test]
    fn relative_paths_of_a_second_launch_are_relative_to_its_directory() {
        let mut commands = vec![Command::Send(vec!["a.txt".into(), "C:\\abs.txt".into()])];
        resolve_paths(&mut commands, std::path::Path::new("D:\\work"));
        assert_eq!(commands, [Command::Send(vec!["D:\\work\\a.txt".into(), "C:\\abs.txt".into()])]);
    }

    #[test]
    fn the_developer_clipboard_hooks_need_a_developer_setting() {
        let normal = parse(&args("--data-dir x"), &no_env).unwrap().config;
        assert!(!normal.dev_hooks_enabled());
        for flag in [
            "--dev-bind 127.0.0.1",
            "--dev-no-windows",
            "--dev-print-pair-url",
            "--dev-port 5",
            "--dev-received-dir C:\\r",
            "--dev-free-bytes 5",
            "--dev-qr-address 10.0.2.2",
        ] {
            assert!(parse(&args(&format!("--data-dir x {flag}")), &no_env).unwrap().config.dev_hooks_enabled(), "{flag}");
        }
    }

    #[test]
    fn the_new_developer_settings() {
        let p = parse(&args("--data-dir x --dev-received-dir D:\\recv --dev-free-bytes 1048576"), &no_env).unwrap();
        assert_eq!(p.config.dev_received_dir, Some(PathBuf::from("D:\\recv")));
        assert_eq!(p.config.dev_free_bytes, Some(1_048_576));
        assert!(parse(&args("--data-dir x --dev-free-bytes lots"), &no_env).is_err());
    }
}
