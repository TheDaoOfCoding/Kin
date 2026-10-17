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
    /// Developer setting (1.4): windows are shown, but toasts are only logged (for screenshots of the windows without a stream of toasts).
    pub dev_no_toasts: bool,
    /// Developer setting (2B.1): nothing is ever opened (no browser, no Explorer): the app only logs that it would have, with the length of a link and
    /// never the link. For a test run that receives links from the phone and must not open a window.
    pub dev_no_open: bool,
    /// Developer setting (2B.2): every phone notification goes to the Action Center without a popup and without a sound, so a test never puts a popup on the
    /// screen of the person at the keyboard.
    pub dev_toast_quiet: bool,
    /// Developer setting (2B.3): in a test run (every instance but the installed Kin) the PC reads and controls **only** the media session whose
    /// `SourceAppUserModelId` is this (it has to be a test session of Kin's own making, `media::guard::TEST_AUMID_PREFIX`); with it missing no session is read.
    pub dev_media_aumid: Option<String>,
    /// Developer setting (2B.3): a test run writes the PC's real master volume after all. **No agent and no automated test ever passes it.**
    pub dev_media_real_volume: bool,
    /// Developer setting (2B.3): the media flyout never shows `Playing`. A test run does that anyway; the flag only says it out loud.
    pub dev_media_quiet: bool,
    /// Developer setting (2B.4): a fake battery for this PC instead of the real power status: `Some(Some(battery))` (`--dev-battery 9:unplugged`) or `Some(None)`
    /// (`--dev-battery none`: no battery, whatever the machine has). The real call is read-only and safe, this is for a test that needs a laptop.
    pub dev_battery: Option<Option<crate::presence::wire::Battery>>,
    /// Developer setting: received files go here instead of `Downloads\Kin` (and instead of `receivedFolder` of settings.json), so
    /// a test never writes to the real Downloads folder.
    pub dev_received_dir: Option<PathBuf>,
    /// Developer hook: pretend the disk has this many free bytes (the check for `507 disk_full` uses it).
    pub dev_free_bytes: Option<u64>,
    /// Developer hook (1.4): the app's wall clock runs this many seconds ahead, so a text that is 7 days old or an offer that is a day old
    /// can be tested in a running app (`--dev-clock-offset 604801` makes a queued text expire).
    pub dev_clock_offset_secs: Option<i64>,
    /// `--data-dir` (or `KIN_DATA_DIR`) was given: this is not the user's own profile, so "Start with Windows" never touches the registry.
    pub data_dir_overridden: bool,
    /// The data directory of the installed Kin (`%APPDATA%\Kin`), if the environment says where it is. A data directory that is not this
    /// one makes the app a separate instance (see `instance.rs`). `None` (as in `Config::new`, which the tests use) means no directory is
    /// the installed one's.
    pub installed_data_dir: Option<PathBuf>,
}

impl Config {
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            installed_data_dir: None,
            data_dir,
            dev_bind: None,
            dev_qr_address: None,
            dev_port: None,
            dev_print_pair_url: false,
            dev_no_windows: false,
            dev_no_toasts: false,
            dev_no_open: false,
            dev_toast_quiet: false,
            dev_media_aumid: None,
            dev_media_real_volume: false,
            dev_media_quiet: false,
            dev_battery: None,
            dev_received_dir: None,
            dev_free_bytes: None,
            dev_clock_offset_secs: None,
            data_dir_overridden: false,
        }
    }

    /// Whether "Start with Windows" may use the real Run key: **only a release build** that runs the way a user runs it. A debug build (what
    /// `cargo run` and `cargo test` make) never writes the registry, even with no flag at all: it used to, and a developer who opened the
    /// settings window and clicked the switch got Kin in their real Run key. Any developer setting, and a data directory of its own (a
    /// script's throwaway profile), get the file stand-in of `autostart.rs` instead. The same rule decides whether the app makes its "Send to"
    /// shortcut (`integration.rs`): only where the Run key is real.
    pub fn autostart_is_real(&self) -> bool {
        !cfg!(debug_assertions) && !self.dev_hooks_enabled() && !self.data_dir_overridden && self.instance_key().is_none()
    }

    /// Whether the app was started with any developer setting. The developer clipboard commands (`--dev-set-clipboard`,
    /// `--dev-get-clipboard`) are honoured only then: in a normal run any other program could launch `Kin.exe` with them.
    pub fn dev_hooks_enabled(&self) -> bool {
        self.dev_bind.is_some()
            || self.dev_qr_address.is_some()
            || self.dev_port.is_some()
            || self.dev_print_pair_url
            || self.dev_no_windows
            || self.dev_no_toasts
            || self.dev_no_open
            || self.dev_toast_quiet
            || self.dev_media_aumid.is_some()
            || self.dev_media_real_volume
            || self.dev_media_quiet
            || self.dev_battery.is_some()
            || self.dev_received_dir.is_some()
            || self.dev_free_bytes.is_some()
            || self.dev_clock_offset_secs.is_some()
    }

    /// The key of this instance (`instance.rs`): `None` for the installed Kin's own data directory, a short hash of the directory for any other.
    /// A keyed instance runs beside the installed one: its own single-instance lock, toast identity, port and web view data, and no mDNS.
    pub fn instance_key(&self) -> Option<String> {
        crate::instance::key_for(&self.data_dir, self.installed_data_dir.as_deref())
    }

    /// The port to ask for: the one that was given, else the default for the installed Kin. **Another instance never asks for the default
    /// port** (the installed Kin has it, and on Windows a bind to 127.0.0.1 can succeed beside a bind to every interface and take the
    /// loopback traffic): it takes any free one, and `status.json` and the pairing URL say which.
    pub fn preferred_port(&self) -> u16 {
        self.dev_port.unwrap_or(if self.instance_key().is_some() { 0 } else { crate::core::DEFAULT_PORT })
    }

    /// Whether the app advertises itself on the local network (mDNS): only the installed Kin's own instance, and not in developer network
    /// mode. A copy with a data directory of its own never does, so the user's phone can't find it and never pairs with a test run.
    pub fn advertises_on_lan(&self) -> bool {
        !self.is_dev_network() && self.instance_key().is_none()
    }

    /// Whether toasts are only written to the log (`--dev-no-windows` or `--dev-no-toasts`).
    pub fn toasts_are_logged_only(&self) -> bool {
        self.dev_no_windows || self.dev_no_toasts
    }

    /// Any developer address setting means loopback only.
    pub fn is_dev_network(&self) -> bool {
        self.dev_bind.is_some() || self.dev_qr_address.is_some()
    }

    /// The address to bind to: `--dev-bind` if given, else loopback in dev mode, **and loopback for any instance with a key** (a copy with a
    /// data directory of its own: binding every interface would raise a Windows Firewall prompt for a program file that has no rule; it never
    /// advertises, so nothing on the network could find it anyway). Every interface only for the installed Kin's own instance.
    pub fn bind_ip(&self) -> IpAddr {
        match (self.dev_bind, self.dev_qr_address) {
            (Some(ip), _) => ip,
            (None, Some(_)) => Ipv4Addr::LOCALHOST.into(),
            (None, None) if self.instance_key().is_some() => Ipv4Addr::LOCALHOST.into(),
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
    /// Open the history window (1.4), like the tray's "History".
    History,
    /// Open the settings window (1.4), like the tray's "Settings".
    Settings,
    /// Open the debug window (1.4), like Shift-click on the tray icon.
    DebugWindow,
    /// Developer hook (1.4): write what a window would show (`history`, `settings`, `debug` or `queue`) to `view-<name>.json` in the data
    /// directory, so a script can look at it without a window. Refused unless the app was started with a developer setting.
    DevDumpView(String),
    /// Developer hook (1.4): what a click in the history window does, without a window: `cancel`, `retry` or `open` on an item (its ID is in
    /// `view-history.json`). The outcome goes to `dev-history-result.json`. Refused unless the app was started with a developer setting.
    DevHistory { action: String, id: String },
    /// Developer hook (1.4): change a setting the way the settings window does (`deviceName`, `autoClipboardSync`, `imagesToClipboard`,
    /// `receivedFolder`, `hotkey`, `startWithWindows`) and write the outcome to `dev-set-result.json`. Refused unless the app was started with
    /// a developer setting.
    DevSet { key: String, value: String },
    /// Developer hook (1.4): the "Export logs" button without a dialog: write the zip to this path. Refused unless the app was started with a
    /// developer setting.
    DevExportLogs(PathBuf),
    /// Developer hook (2B.2): what a click, a reply or the X does to a phone notification that is on screen, without Windows: `reply <nid> <text>`, `copy <nid>`,
    /// `button <nid> <index>`, `dismiss <nid>`. Refused unless the app was started with a developer setting.
    DevToast { action: String, nid: String, arg: Option<String> },
    /// Developer hook (2B.3): what a button of the Windows media flyout does, without Windows: `play`, `pause`, `toggle`, `next`, `prev`, `seek <ms>`, `volume
    /// <level>`, `volume-delta <n>`, `mute`, `unmute`. It sends the `media.cmd` to the phone the phone's media is shown from. Refused unless the app was started with a
    /// developer setting.
    DevMedia { action: String, arg: Option<String> },
    /// Pause Kin (2B.4): `--pause 1h`, `--pause tomorrow`, `--pause resumed` (until the user resumes) or `--pause resume` (end the pause). The same as the tray's
    /// "Pause Kin": it gives a script a way to act without any input injection. The result is in `status.json` (`pause`).
    Pause(Option<crate::presence::pause::PauseChoice>),
    /// Ring my phone (2B.4): `Kin.exe --ring`, the same as the tray item. The PC makes no sound.
    Ring,
    /// Stop ringing the phone: `Kin.exe --ring-stop`.
    RingStop,
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
            Command::History => "history window",
            Command::Settings => "settings window",
            Command::DebugWindow => "debug window",
            Command::DevDumpView(_) => "dev dump view",
            Command::DevExportLogs(_) => "dev export logs",
            Command::DevSet { .. } => "dev set",
            Command::DevHistory { .. } => "dev history",
            Command::DevToast { .. } => "dev toast",
            Command::DevMedia { .. } => "dev media",
            Command::Pause(Some(_)) => "pause",
            Command::Pause(None) => "resume",
            Command::Ring => "ring",
            Command::RingStop => "ring stop",
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
    let mut no_toasts = truthy(env("KIN_DEV_NO_TOASTS"));
    let mut no_open = truthy(env("KIN_DEV_NO_OPEN"));
    let mut toast_quiet = truthy(env("KIN_DEV_TOAST_QUIET"));
    let mut media_aumid = env("KIN_DEV_MEDIA_AUMID");
    let mut media_real_volume = truthy(env("KIN_DEV_MEDIA_REAL_VOLUME"));
    let mut media_quiet = truthy(env("KIN_DEV_MEDIA_QUIET"));
    let mut dev_battery = env("KIN_DEV_BATTERY");
    let mut received_dir = env("KIN_DEV_RECEIVED_DIR").map(PathBuf::from);
    let mut free_bytes = env("KIN_DEV_FREE_BYTES");
    let mut clock_offset = env("KIN_DEV_CLOCK_OFFSET");
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
            "--dev-no-toasts" => no_toasts = true,
            "--dev-no-open" => no_open = true,
            "--dev-toast-quiet" => toast_quiet = true,
            "--dev-media-aumid" => media_aumid = Some(value(name)?),
            "--dev-media-real-volume" => media_real_volume = true,
            "--dev-media-quiet" => media_quiet = true,
            // `--dev-battery 9:unplugged`, `--dev-battery 62:charging` or `--dev-battery none` (the two-word form `9 unplugged` is read too).
            "--dev-battery" => {
                let mut v = value(name)?;
                if let Some(state) = it.next_if(|a| matches!(a.as_str(), "charging" | "unplugged")) {
                    v = format!("{v}:{state}");
                }
                dev_battery = Some(v);
            }
            "--pause" => {
                let v = value(name)?;
                let choice = if v == "resume" {
                    None
                } else {
                    Some(
                        crate::presence::pause::PauseChoice::parse(&v)
                            .ok_or_else(|| format!("--pause: {v:?} is not one of 1h, tomorrow, resumed, resume"))?,
                    )
                };
                commands.push(Command::Pause(choice));
            }
            "--ring" => commands.push(Command::Ring),
            "--ring-stop" => commands.push(Command::RingStop),
            "--dev-media" => {
                let action = value(name)?;
                let arg = it.next_if(|a| !a.starts_with("--")).cloned();
                commands.push(Command::DevMedia { action, arg });
            }
            // What COM adds to the command line when a click on a toast starts Kin (`mirror/registration.rs`): Kin starts as usual and the click reaches the
            // class it registers.
            "--toast-activated" | "-Embedding" => {}
            "--dev-toast" => {
                let action = value(name)?;
                let nid = it.next().cloned().ok_or_else(|| "--dev-toast needs an action and a nid".to_string())?;
                let arg = it.next_if(|a| !a.starts_with("--")).cloned();
                commands.push(Command::DevToast { action, nid, arg });
            }
            "--dev-received-dir" => received_dir = Some(PathBuf::from(value(name)?)),
            "--dev-free-bytes" => free_bytes = Some(value(name)?),
            "--dev-clock-offset" => clock_offset = Some(value(name)?),
            "--history" => commands.push(Command::History),
            "--settings" => commands.push(Command::Settings),
            "--debug-window" => commands.push(Command::DebugWindow),
            "--dev-dump-view" => commands.push(Command::DevDumpView(value(name)?)),
            "--dev-history" => {
                let action = value(name)?;
                let id = it.next().cloned().ok_or_else(|| "--dev-history needs an action and an item ID".to_string())?;
                commands.push(Command::DevHistory { action, id });
            }
            "--dev-set" => {
                let key = value(name)?;
                let val = it.next().cloned().ok_or_else(|| "--dev-set needs a setting and a value".to_string())?;
                commands.push(Command::DevSet { key, value: val });
            }
            "--dev-export-logs" => commands.push(Command::DevExportLogs(PathBuf::from(value(name)?))),
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

    let data_dir_overridden = data_dir.is_some();
    let installed_data_dir = default_data_dir(env);
    let data_dir = match data_dir {
        Some(d) => d,
        None => installed_data_dir.clone()?,
    };
    let mut config = Config::new(data_dir);
    config.data_dir_overridden = data_dir_overridden;
    config.installed_data_dir = installed_data_dir.ok();
    if let Some(v) = clock_offset {
        config.dev_clock_offset_secs = Some(v.parse().map_err(|_| format!("--dev-clock-offset: {v:?} is not a number of seconds"))?);
    }
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
    config.dev_no_toasts = no_toasts;
    config.dev_no_open = no_open;
    config.dev_toast_quiet = toast_quiet;
    config.dev_media_aumid = media_aumid.filter(|a| !a.is_empty());
    config.dev_media_real_volume = media_real_volume;
    config.dev_media_quiet = media_quiet;
    if let Some(v) = dev_battery.filter(|v| !v.is_empty()) {
        config.dev_battery = Some(
            crate::presence::battery::parse_dev_battery(&v)
                .ok_or_else(|| format!("--dev-battery: {v:?} is not <level>:charging, <level>:unplugged or none"))?,
        );
    }
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
    fn an_instance_with_its_own_data_directory_binds_loopback_unless_told_another_address() {
        // Only the installed Kin's own directory listens on every interface (see defaults_to_appdata_kin); a copy beside it would otherwise
        // raise a firewall prompt for its program file.
        let own = parse(&args("--data-dir C:\\tmp\\kin"), &appdata).unwrap();
        assert!(own.config.instance_key().is_some());
        assert_eq!(own.config.bind_ip(), IpAddr::from(Ipv4Addr::LOCALHOST));
        assert!(!own.config.is_dev_network(), "no developer network setting: only the default changed");
        let explicit = parse(&args("--data-dir C:\\tmp\\kin --dev-bind ::1"), &appdata).unwrap();
        assert_eq!(explicit.config.bind_ip(), "::1".parse::<IpAddr>().unwrap(), "an explicit address is kept");
        let installed = parse(&args(&format!("--data-dir {}", r"C:\Users\x\AppData\Roaming\Kin")), &appdata).unwrap();
        assert_eq!(installed.config.instance_key(), None);
        assert_eq!(installed.config.bind_ip(), IpAddr::from(Ipv4Addr::UNSPECIFIED), "the installed Kin's directory keeps every interface");
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
    fn the_1_4_commands_and_the_clock_offset() {
        let p = parse(&args("--data-dir x --history --settings --debug-window --dev-dump-view queue --dev-clock-offset 604801"), &no_env)
            .unwrap();
        assert_eq!(p.commands, [Command::History, Command::Settings, Command::DebugWindow, Command::DevDumpView("queue".into())]);
        assert_eq!(p.config.dev_clock_offset_secs, Some(604_801));
        assert!(p.config.dev_hooks_enabled(), "the offset is a developer setting");
        assert!(parse(&args("--data-dir x --dev-clock-offset soon"), &no_env).is_err());
        let p = parse(&args("--data-dir x --dev-export-logs C:\\out\\logs.zip"), &no_env).unwrap();
        assert_eq!(p.commands, [Command::DevExportLogs("C:\\out\\logs.zip".into())]);
    }

    #[test]
    fn autostart_touches_the_registry_only_in_a_normal_run() {
        // The user's own profile and no developer flag: the real Run key, **in a release build only**. This test runs in a debug build, where a run
        // with no flags at all must never write the registry.
        assert_eq!(parse(&[], &appdata).unwrap().config.autostart_is_real(), !cfg!(debug_assertions));
        #[cfg(debug_assertions)]
        assert!(!parse(&[], &appdata).unwrap().config.autostart_is_real(), "a debug build with no flag never writes the Run key");
        // A throwaway profile, or any developer flag: the stand-in file, never the registry.
        assert!(!parse(&args("--data-dir x"), &no_env).unwrap().config.autostart_is_real());
        assert!(!parse(&args("--dev-no-windows"), &appdata).unwrap().config.autostart_is_real());
        let env = |k: &str| (k == "KIN_DATA_DIR").then(|| "C:\\tmp\\kin".to_string());
        assert!(!parse(&[], &env).unwrap().config.autostart_is_real(), "KIN_DATA_DIR counts too");
    }

    // ----- instances beside the installed Kin (instance.rs) --------------------------------------------------------------

    #[test]
    fn the_default_data_directory_is_the_installed_instance_and_has_no_key() {
        let p = parse(&[], &appdata).unwrap();
        assert_eq!(p.config.instance_key(), None);
        assert_eq!(p.config.preferred_port(), crate::core::DEFAULT_PORT);
        assert!(p.config.advertises_on_lan());
        // Saying the same directory by hand is the same instance (it is the same data).
        let same = parse(&args(r"--data-dir C:\Users\x\AppData\Roaming\KIN"), &appdata).unwrap();
        assert_eq!(same.config.instance_key(), None, "same directory, other spelling");
    }

    #[test]
    fn a_data_directory_of_its_own_is_a_separate_instance_that_never_takes_the_default_port_or_the_lan() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("dev-kin");
        let arg = vec!["--data-dir".to_string(), dir.display().to_string(), "--dev-bind".to_string(), "127.0.0.1".to_string()];
        let p = parse(&arg, &appdata).unwrap();
        let key = p.config.instance_key().expect("a key");
        assert_eq!(key.len(), 12);
        assert_eq!(p.config.preferred_port(), 0, "no --dev-port: any free port, never the installed Kin's");
        assert!(!p.config.advertises_on_lan());
        // A port that was asked for is the port.
        let mut with_port = arg.clone();
        with_port.extend(args("--dev-port 50123"));
        assert_eq!(parse(&with_port, &appdata).unwrap().config.preferred_port(), 50123);
        // The key follows the directory, not the flags and not the spelling of the environment variable.
        let via_env = |k: &str| match k {
            "KIN_DATA_DIR" => Some(dir.display().to_string().to_uppercase()),
            "APPDATA" => Some(r"C:\Users\x\AppData\Roaming".to_string()),
            _ => None,
        };
        assert_eq!(parse(&[], &via_env).unwrap().config.instance_key(), Some(key.clone()));
        let other = tmp.path().join("dev-kin-2");
        let other_args = vec!["--data-dir".to_string(), other.display().to_string()];
        assert_ne!(parse(&other_args, &appdata).unwrap().config.instance_key(), Some(key));
    }

    #[test]
    fn a_separate_instance_advertises_nowhere_even_without_developer_network_flags() {
        let p = parse(&args(r"--data-dir C:\tmp\kin-test-x"), &appdata).unwrap();
        assert!(p.config.instance_key().is_some());
        assert!(!p.config.is_dev_network());
        assert!(!p.config.advertises_on_lan(), "a Kin with its own data directory is never found by the user's phone");
    }

    /// 2B.4: `--pause`, `--ring`, `--ring-stop` are commands for the running Kin (no developer setting), and `--dev-battery` is a developer setting.
    #[test]
    fn the_pause_and_ring_commands_and_the_dev_battery_parse() {
        use crate::presence::pause::PauseChoice;
        use crate::presence::wire::Battery;
        let args = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        let p =
            parse(&args("--data-dir x --pause 1h --pause tomorrow --pause resumed --pause resume --ring --ring-stop"), &no_env).unwrap();
        assert_eq!(
            p.commands,
            [
                Command::Pause(Some(PauseChoice::OneHour)),
                Command::Pause(Some(PauseChoice::Tomorrow)),
                Command::Pause(Some(PauseChoice::UntilResumed)),
                Command::Pause(None),
                Command::Ring,
                Command::RingStop,
            ]
        );
        assert!(!p.config.dev_hooks_enabled(), "a user's own command is not a developer setting");
        assert!(parse(&args("--pause 2h"), &no_env).unwrap_err().contains("--pause"), "a choice that does not exist is refused");
        assert!(parse(&args("--pause"), &no_env).is_err());
        let names: Vec<&str> = p.commands.iter().map(Command::name).collect();
        assert_eq!(names, ["pause", "pause", "pause", "resume", "ring", "ring stop"]);
        // The fake battery: one word, or the two-word form.
        let b = |level, charging| Some(Some(Battery { level, charging, saver: None }));
        let p = parse(&args("--data-dir x --dev-battery 9:unplugged"), &no_env).unwrap();
        assert_eq!(p.config.dev_battery, b(9, false));
        assert!(p.config.dev_hooks_enabled());
        assert_eq!(parse(&args("--data-dir x --dev-battery 62 charging"), &no_env).unwrap().config.dev_battery, b(62, true));
        assert_eq!(parse(&args("--data-dir x --dev-battery none"), &no_env).unwrap().config.dev_battery, Some(None));
        assert_eq!(parse(&args("--data-dir x"), &no_env).unwrap().config.dev_battery, None, "the real power status unless asked");
        assert!(parse(&args("--dev-battery 101:charging"), &no_env).is_err());
        let env = |k: &str| (k == "KIN_DEV_BATTERY").then(|| "30:charging".to_string());
        assert_eq!(parse(&args("--data-dir x"), &env).unwrap().config.dev_battery, b(30, true));
        let appdata = |k: &str| (k == "APPDATA").then(|| "C:\\Users\\x\\AppData\\Roaming".to_string());
        let installed = parse(&args("--dev-battery none"), &appdata).unwrap();
        assert!(
            crate::instance::dev_settings_refusal(installed.config.dev_hooks_enabled(), installed.config.instance_key().as_deref())
                .is_some(),
            "like every developer setting it needs a data directory of its own"
        );
    }

    #[test]
    fn dev_no_open_is_a_developer_setting_that_needs_a_directory_of_its_own() {
        let p = parse(&args("--dev-no-open"), &appdata).unwrap();
        assert!(p.config.dev_no_open && p.config.dev_hooks_enabled());
        assert!(
            crate::instance::dev_settings_refusal(p.config.dev_hooks_enabled(), p.config.instance_key().as_deref()).is_some(),
            "never against the installed Kin's data"
        );
        assert!(!parse(&[], &appdata).unwrap().config.dev_no_open, "off unless asked for");
    }

    #[test]
    fn developer_settings_with_the_installed_kins_directory_are_refused() {
        let p = parse(&args("--dev-no-toasts"), &appdata).unwrap();
        assert!(p.config.dev_hooks_enabled());
        assert!(crate::instance::dev_settings_refusal(p.config.dev_hooks_enabled(), p.config.instance_key().as_deref()).is_some());
        let p = parse(&args(r"--data-dir C:\tmp\kin-test-x --dev-no-toasts"), &appdata).unwrap();
        assert!(crate::instance::dev_settings_refusal(p.config.dev_hooks_enabled(), p.config.instance_key().as_deref()).is_none());
    }

    #[test]
    fn the_media_settings_are_developer_settings_and_a_dev_media_command_takes_an_optional_value() {
        let p = parse(
            &args("--data-dir x --dev-media-aumid app.kinrelay.dev.mediatest-9 --dev-media-quiet --dev-media seek 5000 --dev-media toggle"),
            &no_env,
        )
        .unwrap();
        assert_eq!(p.config.dev_media_aumid.as_deref(), Some("app.kinrelay.dev.mediatest-9"));
        assert!(p.config.dev_media_quiet && !p.config.dev_media_real_volume && p.config.dev_hooks_enabled());
        assert_eq!(
            p.commands,
            [
                Command::DevMedia { action: "seek".into(), arg: Some("5000".into()) },
                Command::DevMedia { action: "toggle".into(), arg: None }
            ]
        );
        // Every one of them counts as a developer setting by itself, so none can be used against the installed Kin's data.
        for flag in ["--dev-media-aumid a", "--dev-media-real-volume", "--dev-media-quiet"] {
            let p = parse(&args(flag), &appdata).unwrap();
            assert!(p.config.dev_hooks_enabled(), "{flag}");
            assert!(
                crate::instance::dev_settings_refusal(p.config.dev_hooks_enabled(), p.config.instance_key().as_deref()).is_some(),
                "{flag}"
            );
            assert!(!p.config.autostart_is_real(), "{flag}");
        }
        assert!(!parse(&args("--data-dir x"), &no_env).unwrap().config.dev_media_real_volume, "off unless asked for");
        assert_eq!(Command::DevMedia { action: "play".into(), arg: None }.name(), "dev media");
    }

    #[test]
    fn the_new_developer_settings() {
        let p = parse(&args("--data-dir x --dev-received-dir D:\\recv --dev-free-bytes 1048576"), &no_env).unwrap();
        assert_eq!(p.config.dev_received_dir, Some(PathBuf::from("D:\\recv")));
        assert_eq!(p.config.dev_free_bytes, Some(1_048_576));
        assert!(parse(&args("--data-dir x --dev-free-bytes lots"), &no_env).is_err());
    }
}
