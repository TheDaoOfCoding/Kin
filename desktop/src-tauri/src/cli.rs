//! Command line and environment: where the data lives, the developer settings, and the commands that other launches
//! can send to the running instance (see desktop/README.md).
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

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
}

impl Config {
    pub fn new(data_dir: PathBuf) -> Config {
        Config { data_dir, dev_bind: None, dev_qr_address: None, dev_port: None, dev_print_pair_url: false, dev_no_windows: false }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Send a `ping` to the connected phone and show the round-trip time.
    Ping,
    /// Open the pairing window, like the tray menu does.
    Pair,
    /// Start a pairing without a window. The URL goes to `pair-url.txt`.
    StartPairing,
    /// Quit.
    Quit,
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
    let (mut commands, mut unknown) = (Vec::new(), Vec::new());

    let mut it = args.iter();
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
            "--ping" => commands.push(Command::Ping),
            "--pair" => commands.push(Command::Pair),
            "--dev-start-pairing" => commands.push(Command::StartPairing),
            "--quit" => commands.push(Command::Quit),
            _ => unknown.push(arg.clone()),
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
    Ok(Parsed { config, commands, unknown })
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
    fn unknown_arguments_are_reported_not_fatal() {
        let p = parse(&args("--data-dir x --send C:\\a.txt"), &no_env).unwrap();
        assert_eq!(p.unknown, ["--send", "C:\\a.txt"]);
        assert!(parse(&args("--data-dir"), &no_env).is_err());
    }
}
