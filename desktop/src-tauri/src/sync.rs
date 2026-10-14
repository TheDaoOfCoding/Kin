//! Text, links and the clipboard between the PC and the phone (milestone 1.2): what the PC sends (`clipboard` on every
//! change, `text` on the hotkey), what it does with a `text` from the phone, how it waits for the `ack`, and the timings it
//! keeps for `status.json`.
//!
//! **Privacy.** The text of the clipboard is never logged and never stored: not by automatic sync (D10), and in 1.2 not by
//! explicit sends either, because the history comes in 1.4. A log line has a length and a short hash (see
//! `clipboard::short_hash`) and nothing else. The one place the text is shown is the toast for a `text` from the phone, which
//! is the point of that message.
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::clipboard::{self, DEBOUNCE_MS, Debouncer, Exclusion};
use crate::connections::{ConnCommand, SendOutcome};
use crate::core::{Core, Event, lock, short};
use crate::encoding::{unix_ms, uuid_v7};
use crate::links;
use crate::messages::MAX_FRAME_BYTES;

/// A toast shows at most this much of a received text.
pub const PREVIEW_CHARS: usize = 100;
/// The connection answers within 5 s (or says it has no reply). The extra seconds are for the channel.
const REPLY_WAIT: Duration = Duration::from_secs(7);
/// How many link toasts stay clickable.
const LINK_TOASTS_KEPT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An explicit send (hotkey, tray, `--send-clipboard`): `text`.
    Text,
    /// Automatic sync: `clipboard`.
    Clipboard,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Clipboard => "clipboard",
        }
    }
}

/// What came of a send.
#[derive(Clone, Debug, PartialEq)]
pub enum SendResult {
    Acked {
        rtt: Duration,
        tap_to_ack: Duration,
    },
    /// No `ack` in 5 s. The connection was closed, so the phone counts as offline now.
    NoReply,
    /// A phone is paired but isn't connected. (In 1.2 the send just fails. The queue comes in 1.4.)
    PhoneOffline,
    NoPhonePaired,
    /// The phone answered with an `error`.
    Rejected(String),
    /// The message would be over 1 MiB. A text that long goes as a `.txt` file, which is milestone 1.3.
    TooLarge {
        bytes: usize,
    },
}

impl SendResult {
    /// The word for `status.json`.
    pub fn word(&self) -> String {
        match self {
            SendResult::Acked { .. } => "acked".into(),
            SendResult::NoReply => "no_reply".into(),
            SendResult::PhoneOffline => "phone_offline".into(),
            SendResult::NoPhonePaired => "no_phone_paired".into(),
            SendResult::Rejected(code) => format!("error:{code}"),
            SendResult::TooLarge { .. } => "too_large".into(),
        }
    }
}

/// One send, for `status.json`. No text: an id, a length and times.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentRecord {
    pub id: String,
    /// Unix milliseconds when the send was made.
    pub at: u64,
    pub result: String,
    pub chars: usize,
    /// `hotkey`, `tray`, `command` or `clipboard`
    pub trigger: &'static str,
    /// From the tap (the hotkey press, or the first clipboard change of a burst, so the 300 ms debounce is included) to the
    /// `ack`. The number that targets.md means by "arrival time" (under 1 s).
    pub tap_to_ack_ms: Option<f64>,
    /// From sending the message to its `ack`, on a monotonic clock.
    pub rtt_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedRecord {
    pub id: String,
    pub at: u64,
    pub chars: usize,
    pub is_url: bool,
    /// The link opens when the toast is clicked.
    pub openable: bool,
    /// The text is on the clipboard now.
    pub clipboard_set: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipRecord {
    pub at: u64,
    /// `password_manager:<format>`, `echo_of_phone_text`, `too_large`, `not_text`, `empty`, `no_phone`, or `read_failed`.
    pub reason: String,
}

#[derive(Default)]
pub struct Stats {
    sent_text: Option<SentRecord>,
    sent_clipboard: Option<SentRecord>,
    received_text: Option<ReceivedRecord>,
    last_skip: Option<SkipRecord>,
    /// How many clipboard changes were sent, and how many were left alone on purpose.
    auto_sent: u64,
    auto_skipped: u64,
}

/// Where the global hotkey stands, for `status.json`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    pub accelerator: String,
    pub registered: bool,
    pub error: Option<String>,
}

/// The tap to start measuring from.
#[derive(Clone, Copy, Debug)]
pub struct Tap(pub Instant);

impl Tap {
    pub fn now() -> Tap {
        Tap(Instant::now())
    }
}

impl Core {
    // ----- settings and hotkey -------------------------------------------------------------------------------------

    pub fn auto_clipboard_sync(&self) -> bool {
        lock(&self.settings).auto_clipboard_sync
    }

    /// The tray check item. The choice is written to `settings.json`.
    pub fn set_auto_clipboard_sync(&self, on: bool) {
        let settings = {
            let mut s = lock(&self.settings);
            if s.auto_clipboard_sync == on {
                return;
            }
            s.auto_clipboard_sync = on;
            s.clone()
        };
        log::info!("automatic clipboard sync turned {}", if on { "on" } else { "off" });
        if let Err(e) = settings.save(&self.config.data_dir) {
            log::warn!("saving settings.json failed: {e}");
        }
        self.emit(Event::StateChanged);
    }

    pub fn hotkey_accelerator(&self) -> String {
        lock(&self.settings).hotkey.clone()
    }

    pub fn set_hotkey_status(&self, status: HotkeyStatus) {
        *lock(&self.hotkey) = status;
        self.emit(Event::StateChanged);
    }

    // ----- sending -------------------------------------------------------------------------------------------------

    /// The hotkey, the tray item and `--send-clipboard`: send the text on the clipboard now, as a `text` message. The password
    /// manager rule is for the automatic sync only: whoever presses the hotkey means it.
    pub async fn send_clipboard_text(&self, trigger: &'static str, tap: Tap) -> SendResult {
        let read = {
            let clipboard = self.clipboard.clone();
            tokio::task::spawn_blocking(move || clipboard.read()).await
        };
        let text = match read {
            Ok(Ok(snapshot)) => snapshot.text.filter(|t| !t.is_empty()),
            Ok(Err(e)) => {
                log::warn!("{trigger}: reading the clipboard failed: {e}");
                self.emit(Event::Toast("Couldn't read the clipboard. Try again.".into()));
                return SendResult::Rejected("clipboard_unavailable".into());
            }
            Err(e) => {
                log::error!("{trigger}: the clipboard read task failed: {e}");
                return SendResult::Rejected("internal".into());
            }
        };
        let Some(text) = text else {
            log::info!("{trigger}: the clipboard has no text to send");
            self.emit(Event::Toast("The clipboard has no text to send.".into()));
            return SendResult::Rejected("no_text".into());
        };
        self.deliver(Kind::Text, text, trigger, tap, true).await
    }

    /// Sends one message and waits for its `ack`. `explicit` means a person asked for it, so a failure gets a toast. An
    /// automatic sync that fails stays quiet (it would be a toast on every copy while the phone sleeps) and only shows up in
    /// the log and in `status.json`.
    pub async fn deliver(&self, kind: Kind, text: String, trigger: &'static str, tap: Tap, explicit: bool) -> SendResult {
        let id = uuid_v7(unix_ms());
        let chars = text.chars().count();
        let hash = clipboard::short_hash(&text);
        let frame = match kind {
            Kind::Text => json!({ "type": "text", "id": id, "text": text, "isUrl": links::is_http_url(&text) }),
            Kind::Clipboard => json!({ "type": "clipboard", "id": id, "text": text }),
        };
        let bytes = frame.to_string().len();
        let result = if bytes > MAX_FRAME_BYTES {
            // protocol.md: "Longer text is sent as a `.txt` file instead." That is milestone 1.3, so for now it is skipped.
            log::warn!(
                "{} {id}: {chars} characters ({bytes} bytes as a message) is over 1 MiB: not sent (as a .txt file in 1.3)",
                kind.as_str()
            );
            SendResult::TooLarge { bytes }
        } else {
            self.send_frame(kind, &id, frame, tap).await
        };
        let phone = self.snapshot().headline_device().map(|d| d.name.clone());
        self.log_and_record(kind, &id, chars, &hash, trigger, &result, phone.as_deref(), explicit);
        result
    }

    async fn send_frame(&self, kind: Kind, id: &str, frame: Value, tap: Tap) -> SendResult {
        let Some((device_id, tx)) = self.connections.most_recent_open() else {
            return if self.snapshot().devices.is_empty() { SendResult::NoPhonePaired } else { SendResult::PhoneOffline };
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if tx.send(ConnCommand::Send { frame, reply: reply_tx }).is_err() {
            return SendResult::PhoneOffline;
        }
        let outcome = tokio::time::timeout(REPLY_WAIT, reply_rx).await;
        let tap_to_ack = tap.0.elapsed();
        log::debug!("{} {id} to device {}: {outcome:?}", kind.as_str(), short(&device_id));
        match outcome {
            Ok(Ok(SendOutcome::Ack(rtt))) => SendResult::Acked { rtt, tap_to_ack },
            Ok(Ok(SendOutcome::Failed(code))) => SendResult::Rejected(code),
            Ok(Ok(SendOutcome::NotConnected)) => SendResult::PhoneOffline,
            Ok(Ok(SendOutcome::NoReply)) | Ok(Err(_)) | Err(_) => SendResult::NoReply,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn log_and_record(
        &self,
        kind: Kind,
        id: &str,
        chars: usize,
        hash: &str,
        trigger: &'static str,
        result: &SendResult,
        phone: Option<&str>,
        explicit: bool,
    ) {
        let (tap_to_ack_ms, rtt_ms) = match result {
            SendResult::Acked { rtt, tap_to_ack } => (Some(ms(*tap_to_ack)), Some(ms(*rtt))),
            _ => (None, None),
        };
        match result {
            SendResult::Acked { .. } => log::info!(
                "{} {id} ({trigger}): {chars} characters, hash {hash}: acked, tap to ack {:.1} ms (round trip {:.1} ms)",
                kind.as_str(),
                tap_to_ack_ms.unwrap_or_default(),
                rtt_ms.unwrap_or_default()
            ),
            other => log::warn!("{} {id} ({trigger}): {chars} characters, hash {hash}: not delivered ({})", kind.as_str(), other.word()),
        }
        let record = SentRecord { id: id.to_string(), at: unix_ms(), result: result.word(), chars, trigger, tap_to_ack_ms, rtt_ms };
        {
            let mut stats = lock(&self.stats);
            match kind {
                Kind::Text => stats.sent_text = Some(record),
                Kind::Clipboard => {
                    stats.sent_clipboard = Some(record);
                    if matches!(result, SendResult::Acked { .. }) {
                        stats.auto_sent += 1;
                    }
                }
            }
        }
        if explicit {
            let toast = match result {
                SendResult::Acked { .. } => Some(format!("Sent to {}", phone.unwrap_or("your phone"))),
                SendResult::NoReply => Some("No reply from the phone. The text was not sent.".to_string()),
                SendResult::PhoneOffline => Some("The phone isn't connected. The text was not sent.".to_string()),
                SendResult::NoPhonePaired => Some("No phone is paired yet.".to_string()),
                SendResult::Rejected(code) => Some(format!("The phone couldn't take the text ({code}).")),
                SendResult::TooLarge { .. } => Some("That text is over 1 MiB. Sending it as a file comes in a later version.".to_string()),
            };
            if let Some(t) = toast {
                self.emit(Event::Toast(t));
            }
        }
        self.emit(Event::StateChanged);
    }

    // ----- automatic clipboard sync ----------------------------------------------------------------------------------

    /// Runs for as long as the app does. Every clipboard change event restarts a 300 ms timer. When it runs out (no change for
    /// 300 ms) the clipboard is looked at once. `changes` is the receiver that the clipboard hands out.
    pub async fn run_clipboard_sync(&self, mut changes: UnboundedReceiver<()>) {
        let mut debounce = Debouncer::new(DEBOUNCE_MS);
        loop {
            let now = self.clock.now_ms();
            let wait = debounce.deadline().map(|d| Duration::from_millis(d.saturating_sub(now)));
            tokio::select! {
                event = changes.recv() => match event {
                    Some(()) => debounce.event(self.clock.now_ms()),
                    None => break,
                },
                () = tokio::time::sleep(wait.unwrap_or(Duration::from_secs(3600))), if wait.is_some() => {
                    let now = self.clock.now_ms();
                    if let Some(first) = debounce.poll(now) {
                        let tap = Instant::now().checked_sub(Duration::from_millis(now.saturating_sub(first))).unwrap_or_else(Instant::now);
                        self.process_clipboard_change(Tap(tap)).await;
                    }
                }
            }
        }
        log::info!("clipboard sync stopped");
    }

    /// One debounced clipboard change: should it go to the phone, and if so send it as a `clipboard` message.
    pub async fn process_clipboard_change(&self, tap: Tap) {
        if !self.auto_clipboard_sync() {
            log::debug!("clipboard changed: automatic sync is off");
            return;
        }
        let read = {
            let clipboard = self.clipboard.clone();
            tokio::task::spawn_blocking(move || clipboard.read()).await
        };
        let snapshot = match read {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                log::warn!("clipboard changed, but reading it failed: {e}");
                self.skip("read_failed");
                return;
            }
            Err(e) => {
                log::error!("the clipboard read task failed: {e}");
                return;
            }
        };
        // Password managers and other apps that opt out: nothing is sent, and the text is not even looked at.
        if let Some(why) = clipboard::exclusion(&snapshot.formats) {
            log::info!("clipboard changed: not sent, the source app marked it {}", why.format_name());
            self.skip(&format!("password_manager:{}", why.format_name()));
            return;
        }
        let Some(text) = snapshot.text else {
            log::debug!("clipboard changed: no text (an image, files or nothing)");
            self.skip("not_text");
            return;
        };
        if text.is_empty() {
            self.skip("empty");
            return;
        }
        if lock(&self.loop_guard).is_echo(&text, self.clock.now_ms()) {
            log::info!(
                "clipboard changed: {} characters, hash {}: this is the text the phone just sent, not sent back",
                text.chars().count(),
                clipboard::short_hash(&text)
            );
            self.skip("echo_of_phone_text");
            return;
        }
        let result = self.deliver(Kind::Clipboard, text, "clipboard", tap, false).await;
        if let SendResult::TooLarge { .. } = result {
            self.skip("too_large");
        }
    }

    fn skip(&self, reason: &str) {
        {
            let mut stats = lock(&self.stats);
            stats.auto_skipped += 1;
            stats.last_skip = Some(SkipRecord { at: unix_ms(), reason: reason.to_string() });
        }
        self.emit(Event::StateChanged);
    }

    // ----- receiving -------------------------------------------------------------------------------------------------

    /// A `text` from the phone (already acked): put it on the clipboard, without sending it back, and show a toast. A link
    /// (`isUrl`, and really one http(s) URL) can be opened by clicking the toast.
    pub async fn receive_text(&self, device_id: &str, from: &str, id: &str, text: String, is_url: bool) {
        let chars = text.chars().count();
        let hash = clipboard::short_hash(&text);
        let to_set = clipboard::to_windows_newlines(&text);
        // Remember the text first: the change event of the write can arrive before the write call returns.
        lock(&self.loop_guard).note_set(&to_set, self.clock.now_ms());
        let written = {
            let clipboard = self.clipboard.clone();
            tokio::task::spawn_blocking(move || clipboard.write_text(&to_set, None)).await
        };
        let clipboard_set = match written {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                log::error!("text {id} from device {}: putting it on the clipboard failed: {e}", short(device_id));
                false
            }
            Err(e) => {
                log::error!("the clipboard write task failed: {e}");
                false
            }
        };
        if !clipboard_set {
            lock(&self.loop_guard).forget();
            self.emit(Event::Toast(format!("Text from {from} arrived, but the clipboard was busy. Ask for it again.")));
        }

        let link = links::openable_url(&text, is_url).map(str::to_string);
        log::info!(
            "text {id} from device {}: {chars} characters, hash {hash}, clipboard {}{}",
            short(device_id),
            if clipboard_set { "set" } else { "NOT set" },
            if link.is_some() {
                ", link (opens when the toast is clicked)"
            } else if is_url {
                ", isUrl set but not an http(s) link: it will not be opened"
            } else {
                ""
            }
        );
        lock(&self.stats).received_text =
            Some(ReceivedRecord { id: id.to_string(), at: unix_ms(), chars, is_url, openable: link.is_some(), clipboard_set });

        let toast_id = self.next_toast_id.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(url) = link {
            let mut links = lock(&self.link_toasts);
            links.push_back((toast_id, url));
            while links.len() > LINK_TOASTS_KEPT {
                links.pop_front();
            }
        }
        if clipboard_set {
            self.emit(Event::TextToast {
                id: toast_id,
                from: from.to_string(),
                preview: clipboard::preview(&text, PREVIEW_CHARS),
                has_link: lock(&self.link_toasts).iter().any(|(i, _)| *i == toast_id),
            });
        }
        self.emit(Event::StateChanged);
    }

    /// The user clicked a toast. If it was a link toast, the link opens in the default browser (http and https only: the
    /// check was made when the toast was made, and the opener makes it again). Returns whether something was opened.
    pub fn toast_clicked(&self, toast_id: u32) -> bool {
        let url = {
            let mut links = lock(&self.link_toasts);
            links.iter().position(|(i, _)| *i == toast_id).and_then(|pos| links.remove(pos)).map(|(_, url)| url)
        };
        let Some(url) = url else {
            return false;
        };
        match self.opener.open(&url) {
            Ok(()) => {
                // Not the link itself: it can carry a secret.
                log::info!("toast {toast_id}: opened a link ({} characters) in the default browser", url.chars().count());
                true
            }
            Err(e) => {
                log::warn!("toast {toast_id}: opening the link failed: {e}");
                false
            }
        }
    }

    /// The link toast to open when a click can't say which toast it was: the newest one.
    pub fn newest_link_toast(&self) -> Option<u32> {
        lock(&self.link_toasts).back().map(|(id, _)| *id)
    }

    /// Developer hook: sets the clipboard as if the user had copied (no loop guard), optionally marked like a password
    /// manager would mark it. The change goes through the normal pipeline.
    pub async fn dev_set_clipboard(&self, text: String, mark: Option<Exclusion>) {
        let chars = text.chars().count();
        let hash = clipboard::short_hash(&text);
        let clipboard = self.clipboard.clone();
        let result = tokio::task::spawn_blocking(move || clipboard.write_text(&text, mark)).await;
        match result {
            Ok(Ok(())) => log::info!(
                "developer hook: clipboard set ({chars} characters, hash {hash}{})",
                mark.map_or(String::new(), |m| format!(", marked {}", m.format_name()))
            ),
            Ok(Err(e)) => log::warn!("developer hook: setting the clipboard failed: {e}"),
            Err(e) => log::error!("developer hook: {e}"),
        }
    }

    /// Developer hook: writes the clipboard text to `dev-clipboard.txt` in the data directory (UTF-8, exactly the text, and no
    /// file if there is none).
    pub async fn dev_get_clipboard(&self) {
        let clipboard = self.clipboard.clone();
        let result = tokio::task::spawn_blocking(move || clipboard.read()).await;
        let path = self.config.data_dir.join("dev-clipboard.txt");
        match result {
            Ok(Ok(snapshot)) => match snapshot.text {
                Some(text) => {
                    log::info!(
                        "developer hook: clipboard read ({} characters, hash {}) into dev-clipboard.txt",
                        text.chars().count(),
                        clipboard::short_hash(&text)
                    );
                    if let Err(e) = std::fs::write(&path, text) {
                        log::warn!("developer hook: writing dev-clipboard.txt failed: {e}");
                    }
                }
                None => {
                    log::info!("developer hook: the clipboard has no text");
                    let _ = std::fs::remove_file(&path);
                }
            },
            Ok(Err(e)) => log::warn!("developer hook: reading the clipboard failed: {e}"),
            Err(e) => log::error!("developer hook: {e}"),
        }
    }

    // ----- status.json -----------------------------------------------------------------------------------------------

    /// The 1.2 part of `status.json`: settings, the hotkey, the last sends and receives with their timings.
    pub fn sync_status(&self) -> Value {
        let settings = lock(&self.settings).clone();
        let stats = lock(&self.stats);
        json!({
            "settings": settings.to_json(),
            "hotkey": *lock(&self.hotkey),
            "lastSent": { "text": stats.sent_text, "clipboard": stats.sent_clipboard },
            "lastReceived": { "text": stats.received_text },
            "clipboardSync": {
                "enabled": settings.auto_clipboard_sync,
                "sent": stats.auto_sent,
                "skipped": stats.auto_skipped,
                "lastSkip": stats.last_skip,
            },
        })
    }
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1000.0 * 10.0).round() / 10.0
}

/// The pending link toasts, for the tests.
pub type LinkToasts = VecDeque<(u32, String)>;
