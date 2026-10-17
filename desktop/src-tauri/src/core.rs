//! The app's shared state and what it can do, with no UI in it: the tray, the windows and the toasts live in
//! `app.rs` and listen to the [`Event`]s sent from here.
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::Result;
use crate::cli::Config;
use crate::clipboard::{Clipboard, Clock, LoopGuard};
use crate::connections::{Connections, SendOutcome};
use crate::encoding::{self, unix_ms};
use crate::identity::Identity;
use crate::links::UrlOpener;
use crate::netaddr;
use crate::pairing::{self, PairError, Pairing};
use crate::settings::Settings;
use crate::storage::{Db, Device, NewDevice};
use crate::sync::{HotkeyStatus, Stats, ToastTargets};
use crate::transfers::{Hooks, Transfers};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_PORT: u16 = 47821;
const MAX_NAME_CHARS: usize = 64;

/// What the core tells the UI layer.
#[derive(Debug, Clone)]
pub enum Event {
    /// A phone connected or disconnected, or a device was added or removed.
    StateChanged,
    /// The pairing window has something new to show.
    PairingChanged,
    /// A phone paired. The UI shows the "New device paired" notice.
    DevicePaired(DeviceRow),
    Toast(String),
    /// A toast that has a file name in it. `text` is shown, `log_text` is what goes to the log (the name as a short hash): a file name
    /// is the user's own and stays out of a log line (see desktop/README.md, "Logs").
    ToastRedacted {
        text: String,
        log_text: String,
    },
    /// "Text from <phone>" with a preview. With `has_link` a click on the toast opens the link (`Core::toast_clicked`). The
    /// preview is the user's own text: it goes to the toast and nowhere else (never to a log).
    TextToast {
        id: u32,
        from: String,
        preview: String,
        has_link: bool,
    },
    /// A link from the phone (2B.1). `opened`: it was opened in the browser ("Opened link from phone", button Copy); otherwise it is offered
    /// ("Link from phone", buttons Open and Copy). `body` is the title and the real host. The URL itself is behind the toast's target
    /// (`Core::toast_action`), never in the event and never in a log.
    LinkToast {
        id: u32,
        title: String,
        body: String,
        opened: bool,
    },
    /// "3 files from <phone>". A click opens the file (one file) or the folder (`Core::toast_clicked`).
    FilesToast {
        id: u32,
        title: String,
        body: String,
        /// One file: the toast can offer "Open" and "Show in folder". Several: only "Open folder".
        single: bool,
        /// One file whose type is on the allow-list (`links::may_open_file`): only then is there an "Open" button. Any other file gets
        /// "Show in folder" alone, because a click on a toast must never run a program.
        openable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    NotPaired,
    PhoneConnected,
    PhoneOffline,
}

impl LinkState {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkState::NotPaired => "not_paired",
            LinkState::PhoneConnected => "phone_connected",
            LinkState::PhoneOffline => "phone_offline",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub connected: bool,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub state: LinkState,
    pub devices: Vec<DeviceRow>,
    /// For the tray's check item.
    pub auto_clipboard_sync: bool,
    /// The progress of the transfers that are going ("sending 45%"), for the tray's tooltip.
    pub transfers: Option<String>,
    /// How many sends wait in the queue for the phone.
    pub queued: usize,
    /// While a phone's media is shown on this PC: `playing` or `paused` (never the title: the tray is visible to anyone near the PC).
    pub media: Option<&'static str>,
}

impl Snapshot {
    /// The phone the tray talks about: a connected one first, else the one paired last.
    pub fn headline_device(&self) -> Option<&DeviceRow> {
        self.devices.iter().find(|d| d.connected).or_else(|| self.devices.iter().max_by_key(|d| d.paired_at))
    }
}

/// What the pairing window shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    /// `waiting`, `expired`, `locked` (five wrong guesses killed the token), `paired`, `removed` (the device that just paired
    /// was removed) or `no_network`.
    pub status: &'static str,
    pub svg: Option<String>,
    pub pc_name: String,
    /// `XXXX-XXXX`
    pub short_id: String,
    pub expires_at_ms: u64,
    pub device: Option<DeviceRow>,
}

struct Session {
    expires_ms: u64,
    /// Holds the token: kept in memory only, written to a file only when a developer flag asks for it.
    svg: Option<String>,
    /// The `kinrelay://pair?...` link the QR code encodes, for the "Copy pairing link" button. It holds the token: memory only, never logged.
    url: Option<String>,
    /// "Copy pairing link" was pressed for this code, so the link may be on the clipboard: once the phone has paired, it is taken off again
    /// (if the clipboard still holds exactly it). Without the button the clipboard is never looked at.
    link_copied: bool,
    paired: Option<DeviceRow>,
}

pub struct PairRequest {
    pub token: String,
    pub name: String,
    pub platform: String,
    pub model: Option<String>,
    pub app_version: Option<String>,
}

pub enum PairFailure {
    Rejected(PairError),
    /// The device couldn't be saved. The token is still valid.
    Internal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PingReport {
    Rtt(Duration),
    NoReply,
    NotConnected,
    Failed(String),
}

impl PingReport {
    pub fn text(&self) -> String {
        match self {
            PingReport::Rtt(d) => format!("Ping: {} ms", (d.as_secs_f64() * 1000.0).round().max(1.0)),
            PingReport::NoReply => "Ping: no reply".to_string(),
            PingReport::NotConnected => "No phone connected".to_string(),
            PingReport::Failed(code) => format!("Ping failed ({code})"),
        }
    }
}

pub struct Core {
    pub config: Config,
    pub identity: Identity,
    /// The name sent in `hello` and shown to the phone: the `deviceName` setting, else the computer's name (`Core::pc_name()`).
    pc_name: Mutex<String>,
    /// The computer's name, for when the setting is empty.
    default_pc_name: String,
    pub db: Db,
    pub connections: Connections,
    pairing: Mutex<Pairing>,
    session: Mutex<Option<Session>>,
    last_ping: Mutex<Option<(u64, PingReport)>>,
    port: AtomicU16,
    events: UnboundedSender<Event>,
    // Text, links and clipboard (sync.rs). The fields are used there.
    pub(crate) settings: Mutex<Settings>,
    pub(crate) hotkey: Mutex<HotkeyStatus>,
    pub(crate) stats: Mutex<Stats>,
    pub(crate) loop_guard: Mutex<LoopGuard>,
    pub(crate) toast_targets: Mutex<ToastTargets>,
    /// File transfers (1.3): both directions, by transfer ID.
    pub transfers: Transfers,
    pub hooks: Hooks,
    /// The queue of 1.4 (queue.rs).
    pub(crate) queue: crate::queue::QueueState,
    /// Phone notifications on the PC (2B.2, mirror/). In memory only, and nothing of a notification is ever stored.
    pub(crate) mirror: crate::mirror::Mirror,
    /// Media controls in both directions (2B.3, media/). In memory only, and nothing about what plays is ever stored or logged.
    pub(crate) media: crate::media::Media,
    pub(crate) next_toast_id: AtomicU32,
    pub(crate) clipboard: Arc<dyn Clipboard>,
    pub(crate) opener: Arc<dyn UrlOpener>,
    pub(crate) clock: Arc<dyn Clock>,
    /// Whether the user is in a fullscreen program (quiet.rs). The real one unless a test says otherwise (`set_user_state_probe`).
    user_state: Mutex<Arc<dyn crate::quiet::UserStateProbe>>,
}

/// What the core uses from the outside world. The app passes the real ones, the tests pass fakes.
pub struct Services {
    pub clipboard: Arc<dyn Clipboard>,
    pub opener: Arc<dyn UrlOpener>,
    pub clock: Arc<dyn Clock>,
}

/// The developer settings that change how transfers behave (`--dev-free-bytes`).
fn hooks_of(config: &Config) -> Hooks {
    let hooks = Hooks::default();
    *lock(&hooks.free_bytes) = config.dev_free_bytes;
    hooks
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The first 8 characters of a device ID: enough to tell devices apart in a log.
pub fn short(device_id: &str) -> &str {
    device_id.get(..8).unwrap_or(device_id)
}

/// The last 8 characters of a transfer ID, for logs. A transfer ID is usually a UUIDv7, whose first characters are a timestamp: two
/// transfers started in the same few minutes share them, the last characters are the random part.
pub fn short_transfer(id: &str) -> &str {
    id.len().checked_sub(8).and_then(|from| id.get(from..)).unwrap_or(id)
}

/// A file name as it may appear in a log: a short hash of it, `<file 1a2b3c4d>`. A file name is the user's own, so a normal log line never has
/// one. The name itself is logged only at debug level, which is off.
pub fn name_tag(name: &str) -> String {
    format!("<file {}>", crate::clipboard::short_hash(name))
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "Windows PC".to_string())
}

impl Core {
    pub fn new(config: Config, identity: Identity, db: Db, pc_name: String, events: UnboundedSender<Event>, services: Services) -> Core {
        // The key of the tags that the log uses for texts and file names (`privacy.rs`).
        crate::privacy::init(&config.data_dir);
        let settings = Settings::load(&config.data_dir);
        let hotkey = HotkeyStatus { accelerator: settings.hotkey.clone(), registered: false, error: None };
        let config_for_hooks = config.clone();
        let shown_name = settings.device_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| pc_name.clone());
        let core = Core {
            config,
            identity,
            pc_name: Mutex::new(shown_name),
            default_pc_name: pc_name,
            db,
            connections: Connections::default(),
            pairing: Mutex::new(Pairing::default()),
            session: Mutex::new(None),
            last_ping: Mutex::new(None),
            port: AtomicU16::new(0),
            events,
            settings: Mutex::new(settings),
            hotkey: Mutex::new(hotkey),
            stats: Mutex::new(Stats::default()),
            loop_guard: Mutex::new(LoopGuard::default()),
            toast_targets: Mutex::new(ToastTargets::default()),
            transfers: Transfers::default(),
            hooks: hooks_of(&config_for_hooks),
            queue: crate::queue::QueueState::default(),
            mirror: crate::mirror::Mirror::new(&config_for_hooks.data_dir),
            media: crate::media::Media::new(),
            next_toast_id: AtomicU32::new(0),
            clipboard: services.clipboard,
            opener: services.opener,
            clock: services.clock,
            user_state: Mutex::new(Arc::new(crate::quiet::WindowsUserState)),
        };
        core.recover_transfers();
        core.prune_storage(unix_ms() as i64);
        core
    }

    /// The name this PC shows to the phone (`hello`, the pairing code and the pairing window).
    pub fn pc_name(&self) -> String {
        lock(&self.pc_name).clone()
    }

    /// The computer's own name, which is the device name when the setting is empty.
    pub fn default_pc_name(&self) -> &str {
        &self.default_pc_name
    }

    pub(crate) fn set_shown_pc_name(&self, name: String) {
        *lock(&self.pc_name) = name;
    }

    /// What the user is doing right now (a fullscreen game, a presentation, quiet time, or nothing special).
    pub fn user_state(&self) -> Arc<dyn crate::quiet::UserStateProbe> {
        lock(&self.user_state).clone()
    }

    /// For tests: pretend a fullscreen game (or anything else) is running.
    #[doc(hidden)]
    pub fn set_user_state_probe(&self, probe: Arc<dyn crate::quiet::UserStateProbe>) {
        *lock(&self.user_state) = probe;
    }

    pub fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub fn set_port(&self, port: u16) {
        self.port.store(port, Ordering::Relaxed);
    }

    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    // ----- state --------------------------------------------------------------------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        let open = self.connections.open_device_ids();
        let devices: Vec<DeviceRow> = self
            .db
            .list_devices()
            .unwrap_or_else(|e| {
                log::error!("reading the device list failed: {e}");
                Vec::new()
            })
            .into_iter()
            .map(|d| row(d, &open))
            .collect();
        let state = if devices.is_empty() {
            LinkState::NotPaired
        } else if devices.iter().any(|d| d.connected) {
            LinkState::PhoneConnected
        } else {
            LinkState::PhoneOffline
        };
        Snapshot {
            state,
            devices,
            auto_clipboard_sync: self.auto_clipboard_sync(),
            transfers: self.transfer_summary(),
            queued: self.queue_len(),
            media: self.media_tray_state(),
        }
    }

    /// The contents of `status.json`, for scripts that want to know what the app is doing.
    pub fn status_json(&self) -> Value {
        let snap = self.snapshot();
        let now = Instant::now();
        let pairing_expires_ms =
            lock(&self.pairing).active_expiry(now).map(|e| unix_ms() + e.saturating_duration_since(now).as_millis() as u64);
        let sync = self.sync_status();
        let last_ping = lock(&self.last_ping).as_ref().map(|(at, report)| {
            let mut v = json!({ "at": at, "result": report.text() });
            if let PingReport::Rtt(d) = report {
                v["rttMs"] = json!(d.as_secs_f64() * 1000.0);
            }
            v
        });
        json!({
            "state": snap.state.as_str(),
            "version": APP_VERSION,
            "pid": std::process::id(),
            "port": self.port(),
            "bind": self.config.bind_ip().to_string(),
            "deviceId": self.identity.device_id,
            "shortId": self.identity.short_id,
            "pcName": self.pc_name(),
            "queue": self.queue_status(),
            "tray": { "status": crate::tray::status_text(&snap), "tooltip": crate::tray::tooltip(&snap) },
            "pairing": pairing_expires_ms.map(|e| json!({ "active": true, "expiresAtMs": e })),
            "devices": snap.devices,
            "lastPing": last_ping,
            "settings": sync["settings"],
            "hotkey": sync["hotkey"],
            "lastSent": sync["lastSent"],
            "lastReceived": sync["lastReceived"],
            "clipboardSync": sync["clipboardSync"],
            "transfers": self.transfers_status(),
            "mirror": { "view": self.mirror_view(), "counts": self.mirror.counters_json() },
            "media": { "view": self.media_view(), "counts": self.media.counters_json() },
            "updatedAtMs": unix_ms(),
        })
    }

    // ----- pairing ------------------------------------------------------------------------------------------------

    /// The addresses for the QR code: the developer address, or the PC's own private ones.
    fn qr_addresses(&self) -> Vec<SocketAddrV4> {
        let port = self.port();
        if let Some(ip) = self.config.dev_qr_address {
            return vec![SocketAddrV4::new(ip, port)];
        }
        match netaddr::list_adapters() {
            Ok((adapters, default_route)) => netaddr::select_addresses(&adapters, default_route, port),
            Err(e) => {
                log::error!("listing the network adapters failed: {e}");
                Vec::new()
            }
        }
    }

    /// Starts a pairing (a new token, valid for 5 minutes) and returns what the window shows.
    pub fn start_pairing(&self) -> PairingView {
        let addresses = self.qr_addresses();
        let now = Instant::now();
        let session = if addresses.is_empty() {
            lock(&self.pairing).end(now);
            log::warn!("pairing: no private network address found, no QR code");
            Session { expires_ms: 0, svg: None, url: None, link_copied: false, paired: None }
        } else {
            let token = lock(&self.pairing).begin(now);
            let url = pairing::pair_url(&addresses, &self.identity.device_id, &token, Some(&self.pc_name()));
            self.dev_publish_pair_url(&url);
            log::info!("pairing started: {} address(es), valid for {} s", addresses.len(), pairing::TOKEN_TTL.as_secs());
            Session {
                expires_ms: unix_ms() + pairing::TOKEN_TTL.as_millis() as u64,
                svg: Some(pairing::qr_svg(&url)),
                url: Some(url),
                link_copied: false,
                paired: None,
            }
        };
        *lock(&self.session) = Some(session);
        self.emit(Event::PairingChanged);
        self.emit(Event::StateChanged);
        self.pairing_view().expect("a session was just stored")
    }

    /// Developer hook: the URL goes to `pair-url.txt` and to stdout. It holds the token, so it never goes to the log.
    fn dev_publish_pair_url(&self, url: &str) {
        if !self.config.dev_print_pair_url {
            return;
        }
        println!("{url}");
        if let Err(e) = std::fs::write(self.config.data_dir.join("pair-url.txt"), format!("{url}\n")) {
            log::error!("writing pair-url.txt failed: {e}");
        }
    }

    /// When the token on screen runs out, if one can still be used. The app wakes up then and calls
    /// [`Core::pairing_expired`], so the window and `status.json` stop showing a code that no longer works.
    pub fn pairing_deadline(&self) -> Option<Instant> {
        lock(&self.pairing).active_expiry(Instant::now())
    }

    /// The token ran out (or was shut down): tell the window and rewrite `status.json`.
    pub fn pairing_expired(&self) {
        if lock(&self.session).is_some() {
            log::info!("pairing: the code expired");
        }
        self.emit(Event::PairingChanged);
        self.emit(Event::StateChanged);
    }

    /// Ends the pairing (the window closed). An unused token stops working.
    pub fn end_pairing(&self) {
        lock(&self.pairing).end(Instant::now());
        if lock(&self.session).take().is_some() {
            log::info!("pairing ended");
            self.emit(Event::StateChanged);
        }
    }

    pub fn pairing_view(&self) -> Option<PairingView> {
        // Whether the token can still be used is `Pairing`'s to say: it runs out after 5 minutes and it is shut down by 5 wrong
        // guesses. Taken before the session lock, and never while holding it.
        let (usable, locked) = {
            let pairing = lock(&self.pairing);
            (pairing.active_expiry(Instant::now()).is_some(), pairing.is_invalidated())
        };
        let guard = lock(&self.session);
        let s = guard.as_ref()?;
        let status = match (&s.paired, &s.svg) {
            (Some(d), _) if self.db.is_paired(&d.device_id).unwrap_or(true) => "paired",
            (Some(_), _) => "removed",
            (None, None) => "no_network",
            (None, Some(_)) if locked => "locked",
            (None, Some(_)) if !usable => "expired",
            (None, Some(_)) => "waiting",
        };
        Some(PairingView {
            status,
            svg: if status == "waiting" { s.svg.clone() } else { None },
            pc_name: self.pc_name(),
            short_id: encoding::format_short_id(&self.identity.short_id),
            expires_at_ms: s.expires_ms,
            device: s.paired.clone(),
        })
    }

    /// "Copy pairing link": puts the link the QR code encodes on the clipboard, for a phone whose camera can't scan it. Only while the code
    /// on screen can still be used (not locked, expired or already used). The link holds the one-time token, so it is written as sensitive
    /// (`write_text_sensitive`): out of the Windows clipboard history and cloud clipboard and out of Kin's own clipboard sync. Neither
    /// the link nor the token goes into the log.
    pub async fn copy_pairing_link(&self) -> std::result::Result<(), String> {
        let usable = self.pairing_view().is_some_and(|v| v.status == "waiting");
        let url = lock(&self.session).as_ref().and_then(|s| s.url.clone()).filter(|_| usable);
        let Some(url) = url else {
            return Err("There is no pairing code to copy. Show a new code first.".to_string());
        };
        let clipboard = self.clipboard.clone();
        match tokio::task::spawn_blocking(move || clipboard.write_text_sensitive(&url)).await {
            Ok(Ok(())) => {
                if let Some(s) = lock(&self.session).as_mut() {
                    s.link_copied = true;
                }
                log::info!("pairing link copied to the clipboard (kept out of the history and the sync)");
                Ok(())
            }
            Ok(Err(e)) => {
                log::warn!("copying the pairing link failed: {e}");
                Err("Could not use the clipboard. Try again.".to_string())
            }
            Err(e) => {
                log::error!("copying the pairing link: {e}");
                Err("Could not use the clipboard. Try again.".to_string())
            }
        }
    }

    /// `POST /v1/pair` after the request itself has been validated: check the token and save the device, as one step.
    pub fn pair(&self, device_id: &str, req: &PairRequest, remote: SocketAddr) -> std::result::Result<(), PairFailure> {
        let now = Instant::now();
        let name = encoding::truncate_chars(&req.name, MAX_NAME_CHARS);
        // The lock is held from the check to the "used" mark, so two requests can't both use the token.
        let mut pairing = lock(&self.pairing);
        let was_usable = pairing.active_expiry(now).is_some();
        let result = pairing.check(now, &req.token).map_err(PairFailure::Rejected).and_then(|()| {
            self.db
                .upsert_device(
                    &NewDevice {
                        device_id,
                        name,
                        platform: &req.platform,
                        model: req.model.as_deref(),
                        app_version: req.app_version.as_deref(),
                    },
                    unix_ms() as i64,
                )
                .map_err(|e| {
                    log::error!("saving the paired device failed: {e}");
                    PairFailure::Internal
                })
        });
        if result.is_ok() {
            pairing.consume(now);
        }
        // The failure that shut the token down (the fifth wrong guess): the window stops showing the QR code and `pairing.active`
        // leaves status.json, the same as when the token runs out.
        let shut_down = result.is_err() && was_usable && pairing.active_expiry(now).is_none();
        drop(pairing);
        if shut_down {
            log::warn!("pairing: too many wrong codes, this code no longer works");
            self.emit(Event::PairingChanged);
            self.emit(Event::StateChanged);
        }

        let outcome = match &result {
            Ok(()) => "ok",
            Err(PairFailure::Rejected(e)) => e.code(),
            Err(PairFailure::Internal) => "internal",
        };
        log::info!("pairing attempt from {remote}, device {}: {outcome}", short(device_id));
        if result.is_ok() {
            self.device_paired(device_id);
            self.clear_pairing_link_from_clipboard();
        }
        result
    }

    /// The phone has paired: the link that "Copy pairing link" put on the clipboard is dead now, and it held a token, so it is taken off
    /// the clipboard again, but only if the clipboard still holds exactly that link (the check and the clearing are one step: a copy
    /// the user made in the meantime is never touched). Nothing happens if the button was not used for this code.
    fn clear_pairing_link_from_clipboard(&self) {
        let url = {
            let mut session = lock(&self.session);
            match session.as_mut() {
                Some(s) if s.link_copied => {
                    s.link_copied = false;
                    s.url.clone()
                }
                _ => None,
            }
        };
        let Some(url) = url else {
            return;
        };
        let clipboard = self.clipboard.clone();
        let work = move || match clipboard.clear_if_text(&url) {
            Ok(true) => log::info!("the pairing link was taken off the clipboard (the phone is paired)"),
            Ok(false) => log::info!("the pairing link is not on the clipboard any more: the clipboard was left alone"),
            Err(e) => log::warn!("taking the pairing link off the clipboard failed: {e}"),
        };
        // The clipboard can be busy for a moment, so it is not touched on the thread that answers the phone.
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn_blocking(work);
            }
            Err(_) => work(),
        }
    }

    fn device_paired(&self, device_id: &str) {
        let Ok(Some(device)) = self.db.get_device(device_id) else {
            return;
        };
        let device_row = row(device, &self.connections.open_device_ids());
        if let Some(s) = lock(&self.session).as_mut() {
            s.paired = Some(device_row.clone());
        }
        self.emit(Event::PairingChanged);
        self.emit(Event::StateChanged);
        self.emit(Event::DevicePaired(device_row));
    }

    // ----- devices ------------------------------------------------------------------------------------------------

    /// Removes a paired device. If it's connected it gets an `unpair` message (or a 4003 close if its `hello` isn't
    /// accepted yet), as protocol.md describes.
    pub fn remove_device(&self, device_id: &str) -> Result<bool> {
        let removed = self.db.remove_device(device_id)?;
        let _ = self.db.delete_pending_cancels_of(device_id);
        self.mirror_forget_device(device_id);
        log::info!("device {} removed: {removed}", short(device_id));
        self.connections.revoke(device_id);
        self.emit(Event::PairingChanged);
        self.emit(Event::StateChanged);
        Ok(removed)
    }

    // ----- ping ---------------------------------------------------------------------------------------------------

    /// Sends a `ping` to the connected phone and waits for the `ack`. The result is also shown as a toast.
    pub async fn ping_phone(&self) -> PingReport {
        let report = match self.connections.most_recent_open() {
            None => PingReport::NotConnected,
            Some((id, tx)) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if tx.send(crate::connections::ConnCommand::Ping(reply_tx)).is_err() {
                    PingReport::NotConnected
                } else {
                    // The connection answers within 5 s (or says it has no reply). The extra second is for the channel.
                    let outcome = tokio::time::timeout(Duration::from_secs(7), reply_rx).await;
                    log::info!("ping to {}: {:?}", short(&id), outcome);
                    match outcome {
                        Ok(Ok(SendOutcome::Ack(rtt))) => PingReport::Rtt(rtt),
                        Ok(Ok(SendOutcome::Failed(code))) => PingReport::Failed(code),
                        Ok(Ok(SendOutcome::NotConnected)) => PingReport::NotConnected,
                        Ok(Ok(SendOutcome::NoReply | SendOutcome::Lost)) | Ok(Err(_)) | Err(_) => PingReport::NoReply,
                    }
                }
            }
        };
        *lock(&self.last_ping) = Some((unix_ms(), report.clone()));
        // The round trip and the errors of pings are in the debug window (a ping is not a send: it is left out of the success rate).
        let outcome = match &report {
            PingReport::Rtt(d) => Some(("ok", None, Some(d.as_secs_f64() * 1000.0))),
            PingReport::NoReply => Some(("failed", Some("no_reply".to_string()), None)),
            PingReport::Failed(code) => Some(("failed", Some(code.clone()), None)),
            PingReport::NotConnected => None,
        };
        if let Some((result, error, rtt)) = outcome {
            let id = encoding::uuid_v7(unix_ms());
            self.record_send_start(&id, "ping", 1, false);
            self.record_send_end(&id, "ping", unix_ms() as i64, 1, result, error.as_deref(), rtt);
        }
        self.emit(Event::Toast(report.text()));
        self.emit(Event::StateChanged);
        report
    }

    // ----- shutdown -----------------------------------------------------------------------------------------------

    /// Closes every WebSocket with 1000 ("app quit") and gives them a moment to send the close frame.
    pub async fn shutdown(&self) {
        self.connections.close_all(crate::messages::CLOSE_NORMAL, "app quit");
        for _ in 0..20 {
            if self.connections.open_device_ids().is_empty() && self.connections.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn row(d: Device, open: &[String]) -> DeviceRow {
    DeviceRow {
        connected: open.contains(&d.device_id),
        device_id: d.device_id,
        name: d.name,
        platform: d.platform,
        paired_at: d.paired_at,
        last_seen: d.last_seen,
    }
}

impl Core {
    /// For tests: puts the pairing token in a given state.
    #[doc(hidden)]
    pub fn seed_pairing_token(&self, value: &str, state: pairing::SeedState) {
        lock(&self.pairing).seed(value, state, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_text_rounds_to_whole_milliseconds() {
        assert_eq!(PingReport::Rtt(Duration::from_micros(42_400)).text(), "Ping: 42 ms");
        assert_eq!(PingReport::Rtt(Duration::from_micros(1_554)).text(), "Ping: 2 ms");
        assert_eq!(PingReport::Rtt(Duration::from_micros(200)).text(), "Ping: 1 ms");
        assert_eq!(PingReport::NoReply.text(), "Ping: no reply");
        assert_eq!(PingReport::NotConnected.text(), "No phone connected");
    }
}
