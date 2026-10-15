//! Text, links and the clipboard between the PC and the phone (milestone 1.2): what the PC sends (`clipboard` on every
//! change, `text` on the hotkey), what it does with a `text` from the phone, how it waits for the `ack`, and the timings it
//! keeps for `status.json`.
//!
//! **Privacy.** The text of the clipboard is never logged and never stored: not by automatic sync (D10), and in 1.2 not by
//! explicit sends either, because the history comes in 1.4. A log line has a length and a short hash (see
//! `clipboard::short_hash`) and nothing else, and a *sensitive* text (protocol.md, "Sensitive text") not even that. The one place
//! the text is shown is the toast for a `text` from the phone, which is the point of that message, and not for a sensitive one.
//!
//! Milestone 1.3 adds: the `sensitive` flag, the 128 KiB sender threshold (a longer `text` goes as a `.txt` file), LF line ends on
//! the wire, and the toast targets (a click on a toast opens a link, a file or a folder).
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
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

/// A toast shows at most this much of a received text.
pub const PREVIEW_CHARS: usize = 100;
/// The connection answers within 5 s (or says it has no reply). The extra seconds are for the channel.
const REPLY_WAIT: Duration = Duration::from_secs(7);
/// How many toasts stay clickable.
const TOAST_TARGETS_KEPT: usize = 16;
/// Senders send a `text` as a message only up to this many bytes of UTF-8, measured on the wire form (protocol.md, "Size").
pub const SENDER_THRESHOLD_BYTES: usize = 128 * 1024;

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

/// What a sender does with a text of `wire_bytes` bytes (UTF-8, after the line-end conversion): protocol.md, "Size". The vectors of
/// `text-rules.json` (`senderSize[]`) test this function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendPlan {
    /// A `text` message.
    Text,
    /// A `clipboard` message.
    Clipboard,
    /// A `.txt` file through the file transfer.
    File,
    /// An automatic `clipboard` that is too big: not sent, status `too_large`.
    TooLarge,
    /// A sensitive `text` that is too big: not sent (a file can't carry the mark).
    NotSent,
}

pub fn plan_send(kind: Kind, sensitive: bool, wire_bytes: usize) -> SendPlan {
    match (kind, wire_bytes > SENDER_THRESHOLD_BYTES) {
        (Kind::Clipboard, false) => SendPlan::Clipboard,
        (Kind::Clipboard, true) => SendPlan::TooLarge,
        (Kind::Text, false) => SendPlan::Text,
        (Kind::Text, true) if sensitive => SendPlan::NotSent,
        (Kind::Text, true) => SendPlan::File,
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
    /// Not sent: an automatic `clipboard` over 128 KiB, or a `sensitive` text over 128 KiB (a longer ordinary `text` goes as a `.txt`
    /// file instead).
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
    /// `None` for a sensitive text: nothing about it is kept, not even its length.
    pub chars: Option<usize>,
    /// `hotkey`, `tray`, `command` or `clipboard`
    pub trigger: &'static str,
    /// From the tap (the hotkey press, or the first clipboard change of a burst, so the 300 ms debounce is included) to the
    /// `ack`. The number that targets.md means by "arrival time" (under 1 s).
    pub tap_to_ack_ms: Option<f64>,
    /// From sending the message to its `ack`, on a monotonic clock.
    pub rtt_ms: Option<f64>,
    pub sensitive: bool,
    /// The text was over 128 KiB and went as a `.txt` file (`result` is then the result of that offer).
    pub as_file: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedRecord {
    pub id: String,
    pub at: u64,
    /// `None` for a sensitive text.
    pub chars: Option<usize>,
    pub is_url: bool,
    /// The link opens when the toast is clicked.
    pub openable: bool,
    /// The text is on the clipboard now.
    pub clipboard_set: bool,
    /// The sender marked it sensitive: no preview, no link, nothing logged.
    pub sensitive: bool,
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
    /// manager rule is for the automatic sync only: whoever presses the hotkey means it, and content that carried an opt-out format
    /// goes with `sensitive` set (the phone then shows no preview and keeps it out of its history).
    pub async fn send_clipboard_text(self: &Arc<Self>, trigger: &'static str, tap: Tap) -> SendResult {
        let read = {
            let clipboard = self.clipboard.clone();
            tokio::task::spawn_blocking(move || clipboard.read()).await
        };
        let (text, sensitive) = match read {
            Ok(Ok(snapshot)) => (snapshot.text.filter(|t| !t.is_empty()), clipboard::exclusion(&snapshot.formats).is_some()),
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
        self.deliver(Kind::Text, text, sensitive, trigger, tap, true).await
    }

    /// Sends one message and waits for its `ack`. `explicit` means a person asked for it, so a failure gets a toast. An automatic
    /// sync that fails stays quiet (it would be a toast on every copy while the phone sleeps) and only shows up in the log and in
    /// `status.json`.
    ///
    /// The text goes out in its wire form (LF line ends, protocol.md "Line ends"). A `text` over 128 KiB goes as a `.txt` file
    /// instead, an automatic `clipboard` that big is not sent (`too_large`), and a `sensitive` text that big is not sent at all (a
    /// file has no `sensitive` mark). A sensitive text leaves no length and no hash in the log or in `status.json`.
    pub async fn deliver(
        self: &Arc<Self>,
        kind: Kind,
        text: String,
        sensitive: bool,
        trigger: &'static str,
        tap: Tap,
        explicit: bool,
    ) -> SendResult {
        let id = uuid_v7(unix_ms());
        let text = clipboard::to_wire(&text);
        let bytes = text.len();
        let chars = (!sensitive).then(|| text.chars().count());
        let hash = (!sensitive).then(|| clipboard::short_hash(&text));
        let describe = |chars: Option<usize>, hash: &Option<String>| match (chars, hash) {
            (Some(c), Some(h)) => format!("{c} characters, hash {h}"),
            _ => "sensitive text".to_string(),
        };
        let mut as_file = false;
        let plan = plan_send(kind, sensitive, bytes);
        let result = match plan {
            SendPlan::TooLarge => {
                log::warn!("clipboard {id}: {bytes} bytes is over 128 KiB: not sent");
                SendResult::TooLarge { bytes }
            }
            SendPlan::NotSent => {
                log::warn!("text {id}: sensitive text of {bytes} bytes is over 128 KiB: not sent (a file can't carry the mark)");
                SendResult::TooLarge { bytes }
            }
            SendPlan::File => {
                log::info!("text {id}: {} is over 128 KiB: sent as a .txt file", describe(chars, &hash));
                as_file = true;
                self.send_text_file(&text).await
            }
            SendPlan::Text | SendPlan::Clipboard => {
                let frame = match kind {
                    Kind::Text => {
                        let mut frame = json!({ "type": "text", "id": id, "text": text, "isUrl": links::is_http_url(&text) });
                        if sensitive {
                            frame["sensitive"] = json!(true);
                        }
                        frame
                    }
                    Kind::Clipboard => json!({ "type": "clipboard", "id": id, "text": text }),
                };
                self.send_frame(kind, &id, frame, tap).await
            }
        };
        let phone = self.snapshot().headline_device().map(|d| d.name.clone());
        self.log_and_record(kind, &id, chars, hash.as_deref(), sensitive, as_file, trigger, &result, phone.as_deref(), explicit);
        result
    }

    async fn send_frame(&self, kind: Kind, id: &str, frame: Value, tap: Tap) -> SendResult {
        let (result, device) = self.send_to_phone(frame, tap).await;
        log::debug!("{} {id} to device {}: {}", kind.as_str(), device.as_deref().map_or("-", short), result.word());
        result
    }

    /// Sends one message that needs an `ack` to the connected phone and waits for it (5 s). Also says which device it went to.
    pub(crate) async fn send_to_phone(&self, frame: Value, tap: Tap) -> (SendResult, Option<String>) {
        let Some((device_id, tx)) = self.connections.most_recent_open() else {
            let result = if self.snapshot().devices.is_empty() { SendResult::NoPhonePaired } else { SendResult::PhoneOffline };
            return (result, None);
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if tx.send(ConnCommand::Send { frame, reply: reply_tx }).is_err() {
            return (SendResult::PhoneOffline, Some(device_id));
        }
        let outcome = tokio::time::timeout(REPLY_WAIT, reply_rx).await;
        let tap_to_ack = tap.0.elapsed();
        let result = match outcome {
            Ok(Ok(SendOutcome::Ack(rtt))) => SendResult::Acked { rtt, tap_to_ack },
            Ok(Ok(SendOutcome::Failed(code))) => SendResult::Rejected(code),
            Ok(Ok(SendOutcome::NotConnected)) => SendResult::PhoneOffline,
            Ok(Ok(SendOutcome::NoReply)) | Ok(Err(_)) | Err(_) => SendResult::NoReply,
        };
        (result, Some(device_id))
    }

    #[allow(clippy::too_many_arguments)]
    fn log_and_record(
        &self,
        kind: Kind,
        id: &str,
        chars: Option<usize>,
        hash: Option<&str>,
        sensitive: bool,
        as_file: bool,
        trigger: &'static str,
        result: &SendResult,
        phone: Option<&str>,
        explicit: bool,
    ) {
        let (tap_to_ack_ms, rtt_ms) = match result {
            SendResult::Acked { rtt, tap_to_ack } => (Some(ms(*tap_to_ack)), Some(ms(*rtt))),
            _ => (None, None),
        };
        // What the log says about the text: a length and a short hash, and for a sensitive text nothing at all.
        let about = match (chars, hash) {
            (Some(c), Some(h)) => format!("{c} characters, hash {h}"),
            _ => "sensitive text".to_string(),
        };
        let how = if as_file { ", as a file" } else { "" };
        match result {
            SendResult::Acked { .. } => log::info!(
                "{} {id} ({trigger}{how}): {about}: acked, tap to ack {:.1} ms (round trip {:.1} ms)",
                kind.as_str(),
                tap_to_ack_ms.unwrap_or_default(),
                rtt_ms.unwrap_or_default()
            ),
            other => log::warn!("{} {id} ({trigger}{how}): {about}: not delivered ({})", kind.as_str(), other.word()),
        }
        let record = SentRecord {
            id: id.to_string(),
            at: unix_ms(),
            result: result.word(),
            chars,
            trigger,
            tap_to_ack_ms,
            rtt_ms,
            sensitive,
            as_file,
        };
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
        if explicit && !as_file {
            let toast = match result {
                SendResult::Acked { .. } => Some(format!("Sent to {}", phone.unwrap_or("your phone"))),
                SendResult::NoReply => Some("No reply from the phone. The text was not sent.".to_string()),
                SendResult::PhoneOffline => Some("The phone isn't connected. The text was not sent.".to_string()),
                SendResult::NoPhonePaired => Some("No phone is paired yet.".to_string()),
                SendResult::Rejected(code) => Some(format!("The phone couldn't take the text ({code}).")),
                SendResult::TooLarge { .. } if sensitive => {
                    Some("That text is marked sensitive and is over 128 KiB, so it was not sent (a file can't carry the mark).".to_string())
                }
                SendResult::TooLarge { .. } => Some("That text is too big to send.".to_string()),
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
    pub async fn run_clipboard_sync(self: &Arc<Self>, mut changes: UnboundedReceiver<()>) {
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
    pub async fn process_clipboard_change(self: &Arc<Self>, tap: Tap) {
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
        // From here on the text is in its wire form (LF), which is also what the loop guard hashed when it was set.
        let text = clipboard::to_wire(&text);
        if lock(&self.loop_guard).is_echo(&text, self.clock.now_ms()) {
            log::info!(
                "clipboard changed: {} characters, hash {}: this is the text the phone just sent, not sent back",
                text.chars().count(),
                clipboard::short_hash(&text)
            );
            self.skip("echo_of_phone_text");
            return;
        }
        let result = self.deliver(Kind::Clipboard, text, false, "clipboard", tap, false).await;
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
    /// (`isUrl`, and really an openable URL) can be opened by clicking the toast.
    ///
    /// A `sensitive` text goes to the clipboard marked so that Windows keeps it out of its history and the cloud clipboard. The
    /// toast says "Text from <phone>" with no preview and no way to open it, and nothing about the text (not even its length) is
    /// logged or kept in `status.json`.
    pub async fn receive_text(&self, device_id: &str, from: &str, id: &str, text: String, is_url: bool, sensitive: bool) {
        let chars = text.chars().count();
        let hash = clipboard::short_hash(&text);
        // The loop guard remembers the wire form (what the phone sent), and the clipboard gets Windows line ends.
        let wire = clipboard::to_wire(&text);
        let to_set = clipboard::to_windows_newlines(&wire);
        // Remember the text first: the change event of the write can arrive before the write call returns.
        lock(&self.loop_guard).note_set(&wire, self.clock.now_ms());
        let written = {
            let clipboard = self.clipboard.clone();
            tokio::task::spawn_blocking(move || {
                if sensitive { clipboard.write_text_sensitive(&to_set) } else { clipboard.write_text(&to_set, None) }
            })
            .await
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

        // A sensitive text never opens: there is nothing on the toast to click.
        let link = if sensitive { None } else { links::openable_url(&text, is_url).map(str::to_string) };
        if sensitive {
            log::info!(
                "text {id} from device {}: sensitive, clipboard {}",
                short(device_id),
                if clipboard_set { "set" } else { "NOT set" }
            );
        } else {
            log::info!(
                "text {id} from device {}: {chars} characters, hash {hash}, clipboard {}{}",
                short(device_id),
                if clipboard_set { "set" } else { "NOT set" },
                if link.is_some() {
                    ", link (opens when the toast is clicked)"
                } else if is_url {
                    ", isUrl set but not an openable link: it will not be opened"
                } else {
                    ""
                }
            );
        }
        lock(&self.stats).received_text = Some(ReceivedRecord {
            id: id.to_string(),
            at: unix_ms(),
            chars: (!sensitive).then_some(chars),
            is_url: !sensitive && is_url,
            openable: link.is_some(),
            clipboard_set,
            sensitive,
        });

        let toast_id = self.next_toast_id.fetch_add(1, Ordering::Relaxed) + 1;
        let has_link = link.is_some();
        if let Some(url) = link {
            self.register_toast_target(toast_id, ToastTarget::Link(url));
        }
        if clipboard_set {
            self.emit(Event::TextToast {
                id: toast_id,
                from: from.to_string(),
                preview: if sensitive { String::new() } else { clipboard::preview(&text, PREVIEW_CHARS) },
                has_link,
            });
        }
        self.emit(Event::StateChanged);
    }

    /// Remembers what a click on a toast does. The newest few are kept.
    pub(crate) fn register_toast_target(&self, id: u32, target: ToastTarget) {
        let mut targets = lock(&self.toast_targets);
        targets.push_back((id, target));
        while targets.len() > TOAST_TARGETS_KEPT {
            targets.pop_front();
        }
    }

    /// Whether a toast has something to do when clicked (for the tests and the app).
    pub fn toast_has_target(&self, toast_id: u32) -> bool {
        lock(&self.toast_targets).iter().any(|(i, _)| *i == toast_id)
    }

    /// The user clicked a toast. A link opens in the default browser (http and https only: checked when the toast was made, and
    /// the opener checks again). A received file opens with its default program only if its type is on the allow-list
    /// (`links::may_open_file`: pictures, audio, video, PDF, text, macro-free Office files); any other file is shown in its folder,
    /// selected. Several files open their folder. A click on a toast that has nothing to do is ignored: there is no guess
    /// about which toast it was. Returns whether something was opened.
    pub fn toast_clicked(&self, toast_id: u32) -> bool {
        let target = {
            let mut targets = lock(&self.toast_targets);
            targets.iter().position(|(i, _)| *i == toast_id).and_then(|pos| targets.remove(pos)).map(|(_, t)| t)
        };
        let Some(target) = target else {
            return false;
        };
        let (what, result) = match &target {
            // Not the link itself: it can carry a secret.
            ToastTarget::Link(url) => {
                (format!("a link ({} characters) in the default browser", url.chars().count()), self.opener.open(url))
            }
            ToastTarget::OpenFile(path) if !links::may_open_file(path) => {
                ("the folder of a file whose type isn't on the allow-list (not opened)".to_string(), self.opener.show_in_folder(path))
            }
            ToastTarget::OpenFile(path) => ("a received file".to_string(), self.opener.open_path(path)),
            ToastTarget::ShowFolder(folder) => ("the folder of the received files".to_string(), self.opener.open_folder(folder)),
        };
        match result {
            Ok(()) => {
                log::info!("toast {toast_id}: opened {what}");
                true
            }
            Err(e) => {
                log::warn!("toast {toast_id}: opening {what} failed: {e}");
                false
            }
        }
    }

    /// A button of a toast was pressed: `show` shows the file in its folder (or opens the folder of several), anything else is the same
    /// as a click on the toast. Returns whether something was opened.
    pub fn toast_action(&self, toast_id: u32, action: &str) -> bool {
        if action != "show" {
            return self.toast_clicked(toast_id);
        }
        let target = {
            let mut targets = lock(&self.toast_targets);
            targets.iter().position(|(i, _)| *i == toast_id).and_then(|pos| targets.remove(pos)).map(|(_, t)| t)
        };
        let result = match target {
            Some(ToastTarget::OpenFile(path)) => self.opener.show_in_folder(&path),
            Some(ToastTarget::ShowFolder(folder)) => self.opener.open_folder(&folder),
            Some(ToastTarget::Link(_)) | None => return false,
        };
        match result {
            Ok(()) => {
                log::info!("toast {toast_id}: showed the received file in its folder");
                true
            }
            Err(e) => {
                log::warn!("toast {toast_id}: showing the folder failed: {e}");
                false
            }
        }
    }

    /// Developer hook: sets the clipboard as if the user had copied (no loop guard), optionally marked like a password
    /// manager would mark it. The change goes through the normal pipeline. Refused unless the app was started with a developer
    /// setting.
    pub async fn dev_set_clipboard(&self, text: String, mark: Option<Exclusion>) {
        if !self.dev_hook_allowed("--dev-set-clipboard") {
            return;
        }
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
    /// file if there is none). Refused unless the app was started with a developer setting.
    pub async fn dev_get_clipboard(&self) {
        if !self.dev_hook_allowed("--dev-get-clipboard") {
            return;
        }
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

    /// The developer clipboard commands work only when the running instance was started with a developer setting (`--dev-bind`, …).
    /// In a normal run any program could launch `Kin.exe --dev-get-clipboard` and have the clipboard written to a file.
    fn dev_hook_allowed(&self, command: &str) -> bool {
        if self.config.dev_hooks_enabled() {
            return true;
        }
        log::warn!("refused {command}: it works only when the app was started with a developer setting (--dev-bind and the like)");
        false
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

/// What a click on a toast does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastTarget {
    /// Open this link in the default browser.
    Link(String),
    /// Open this received file (shown in its folder instead unless its type is on the allow-list).
    OpenFile(PathBuf),
    /// Open this folder.
    ShowFolder(PathBuf),
}

/// The toasts that can still be clicked, newest last.
pub type ToastTargets = VecDeque<(u32, ToastTarget)>;
