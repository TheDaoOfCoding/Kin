//! The Windows toast layer behind a trait (design 5.1): the real one ([`WinSurface`], the `windows` crate's WinRT toasts under the phone AUMID) and a
//! fake for tests ([`FakeSurface`]).
//!
//! **Why not the notification plugin.** `tauri-plugin-notification` can't do a text box, a tag and group, an update, a removal from the history or an expiry
//! (design section 2). The Phase 1 toasts keep using it, untouched.
//!
//! **What the spike (docs/results/phase-2b-2-spike-s1.md) found, and this code relies on:** showing a toast with the same tag and group replaces it and raises
//! no `Dismissed` for the old one; `RemoveGroupedTagWithId` raises none either, so Kin's own removals can't look like the user pressing X; with
//! `SuppressPopup` a `TimedOut` comes at once and means nothing; `Show` succeeds for any AUMID, registered or not, so success proves nothing about the
//! registration.
//!
//! **One worker thread does all the WinRT calls**, in the MTA, so the callers (the connection tasks) never wait for the shell and never need COM themselves.
//! The events of a toast (`Dismissed`, `Activated`) come back on a channel as [`SurfaceEvent`]s.
use std::sync::Mutex;
use std::time::Duration;

/// A toast to show.
#[derive(Clone, Debug, PartialEq)]
pub struct ToastSpec {
    pub tag: String,
    pub group: String,
    pub xml: String,
    /// `SuppressPopup`: it goes to the Action Center without a popup.
    pub suppress_popup: bool,
    /// `ExpirationTime` = now + this. (`ExpiresOnReboot` is always on.)
    pub expires_in: Duration,
    /// Counts up with every toast of a `nid`: the events of a toast carry it, so an event of a superseded toast can be told from a current one.
    pub generation: u64,
}

/// What Windows still holds for the AUMID (also after a restart of Kin).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub tag: String,
    pub group: String,
    pub xml: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissReason {
    /// The user pressed X (popup or Action Center).
    UserCanceled,
    /// The app hid it.
    ApplicationHidden,
    /// The popup timed out, or (with `SuppressPopup`) it was never shown: **means nothing**.
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceEvent {
    Dismissed {
        tag: String,
        group: String,
        generation: u64,
        reason: DismissReason,
    },
    /// A click on a toast of this process, reported by the toast object itself (the in-process route; the COM class is the other one).
    Activated {
        arguments: String,
        inputs: Vec<(String, String)>,
    },
}

pub trait ToastSurface: Send + Sync {
    /// Shows or replaces (same tag and group) a toast.
    fn show(&self, spec: ToastSpec) -> Result<(), String>;
    /// Removes the toast with this tag and group from the screen and the Action Center.
    fn remove(&self, tag: &str, group: &str);
    /// Removes every toast of a group (one phone).
    fn remove_group(&self, group: &str);
    /// Removes every toast of this AUMID (the feature is turned off).
    fn clear_all(&self);
    /// What Windows holds now. Blocking: a caller on an async task uses `spawn_blocking`.
    fn history(&self) -> Vec<HistoryEntry>;
}

// ----- the fake ---------------------------------------------------------------------------------------------------------

/// A toast layer that keeps what it was told, for tests. Nothing here touches Windows.
#[derive(Default)]
pub struct FakeSurface {
    shown: Mutex<Vec<ToastSpec>>,
    log: Mutex<Vec<ToastSpec>>,
    removed: Mutex<Vec<(String, String)>>,
    cleared: Mutex<usize>,
}

impl FakeSurface {
    /// The toasts on screen now, oldest first.
    pub fn current(&self) -> Vec<ToastSpec> {
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Every `show` call so far, replacements included.
    pub fn shows(&self) -> Vec<ToastSpec> {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn removals(&self) -> Vec<(String, String)> {
        self.removed.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn clears(&self) -> usize {
        *self.cleared.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn find(&self, tag: &str) -> Option<ToastSpec> {
        self.current().into_iter().find(|t| t.tag == tag)
    }

    /// What the user pressing X on the toast would report.
    pub fn user_closes(&self, tag: &str) -> Option<SurfaceEvent> {
        self.find(tag).map(|t| SurfaceEvent::Dismissed {
            tag: t.tag,
            group: t.group,
            generation: t.generation,
            reason: DismissReason::UserCanceled,
        })
    }
}

impl ToastSurface for FakeSurface {
    fn show(&self, spec: ToastSpec) -> Result<(), String> {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).push(spec.clone());
        let mut shown = self.shown.lock().unwrap_or_else(|p| p.into_inner());
        // Same tag and group: replaced in place, as Windows does (and with no event for the old one).
        match shown.iter().position(|t| t.tag == spec.tag && t.group == spec.group) {
            Some(i) => shown[i] = spec,
            None => shown.push(spec),
        }
        Ok(())
    }

    fn remove(&self, tag: &str, group: &str) {
        self.removed.lock().unwrap_or_else(|p| p.into_inner()).push((tag.to_string(), group.to_string()));
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).retain(|t| !(t.tag == tag && t.group == group));
    }

    fn remove_group(&self, group: &str) {
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).retain(|t| t.group != group);
    }

    fn clear_all(&self) {
        *self.cleared.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    fn history(&self) -> Vec<HistoryEntry> {
        self.current().into_iter().map(|t| HistoryEntry { tag: t.tag, group: t.group, xml: t.xml }).collect()
    }
}

// ----- Windows ----------------------------------------------------------------------------------------------------------

pub use win::{Events as SurfaceEvents, WinSurface};

mod win {
    use std::sync::{Arc, mpsc};
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
    use windows::UI::Notifications::{
        NotificationMirroring, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification,
        ToastNotificationManager, ToastNotifier,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::{HSTRING, IInspectable, Interface};

    use super::{DismissReason, HistoryEntry, SurfaceEvent, ToastSpec, ToastSurface};

    enum Cmd {
        Show(ToastSpec, mpsc::Sender<Result<(), String>>),
        Remove(String, String),
        RemoveGroup(String),
        ClearAll,
        History(mpsc::Sender<Vec<HistoryEntry>>),
        Stop,
    }

    /// Where the events of the toasts go.
    pub type Events = Arc<dyn Fn(SurfaceEvent) + Send + Sync>;

    /// The real toast layer: WinRT toasts of one AUMID, driven from a thread of its own.
    pub struct WinSurface {
        tx: Mutex<mpsc::Sender<Cmd>>,
        worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    }

    use std::sync::Mutex;

    impl WinSurface {
        /// Starts the worker. `events` is called with the `Dismissed` and `Activated` events of every toast this surface shows (on pool threads: it must not block).
        pub fn start(aumid: &str, events: Events) -> Result<WinSurface, String> {
            let (tx, rx) = mpsc::channel::<Cmd>();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let aumid = aumid.to_string();
            let worker = std::thread::Builder::new()
                .name("toast-surface".into())
                .spawn(move || {
                    // SAFETY: COM for this thread, in the multithreaded apartment (event handlers then arrive on pool threads).
                    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid.as_str())) {
                        Ok(n) => n,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("the toast notifier for {aumid} can't be made: {e}")));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    for cmd in rx {
                        match cmd {
                            Cmd::Show(spec, reply) => {
                                let _ = reply.send(show(&notifier, &spec, &events));
                            }
                            Cmd::Remove(tag, group) => {
                                if let Err(e) = history().and_then(|h| {
                                    h.RemoveGroupedTagWithId(
                                        &HSTRING::from(tag.as_str()),
                                        &HSTRING::from(group.as_str()),
                                        &HSTRING::from(aumid.as_str()),
                                    )
                                }) {
                                    log::warn!("removing a toast failed: {e}");
                                }
                            }
                            Cmd::RemoveGroup(group) => {
                                if let Err(e) = history()
                                    .and_then(|h| h.RemoveGroupWithId(&HSTRING::from(group.as_str()), &HSTRING::from(aumid.as_str())))
                                {
                                    log::warn!("removing the toasts of a group failed: {e}");
                                }
                            }
                            Cmd::ClearAll => {
                                if let Err(e) = history().and_then(|h| h.ClearWithId(&HSTRING::from(aumid.as_str()))) {
                                    log::warn!("clearing the toasts failed: {e}");
                                }
                            }
                            Cmd::History(reply) => {
                                let _ = reply.send(read_history(&aumid));
                            }
                            Cmd::Stop => break,
                        }
                    }
                })
                .map_err(|e| format!("the toast thread can't start: {e}"))?;
            ready_rx.recv().map_err(|_| "the toast thread ended at once".to_string())??;
            Ok(WinSurface { tx: Mutex::new(tx), worker: Mutex::new(Some(worker)) })
        }

        fn send(&self, cmd: Cmd) -> bool {
            self.tx.lock().unwrap_or_else(|p| p.into_inner()).send(cmd).is_ok()
        }

        /// Stops the worker (after the commands already queued).
        pub fn stop(&self) {
            self.send(Cmd::Stop);
            if let Some(w) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
                let _ = w.join();
            }
        }
    }

    impl Drop for WinSurface {
        fn drop(&mut self) {
            let _ = self.send(Cmd::Stop);
        }
    }

    impl ToastSurface for WinSurface {
        fn show(&self, spec: ToastSpec) -> Result<(), String> {
            let (reply_tx, reply_rx) = mpsc::channel();
            if !self.send(Cmd::Show(spec, reply_tx)) {
                return Err("the toast thread is gone".into());
            }
            reply_rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "showing the toast took too long".to_string())?
        }

        fn remove(&self, tag: &str, group: &str) {
            self.send(Cmd::Remove(tag.to_string(), group.to_string()));
        }

        fn remove_group(&self, group: &str) {
            self.send(Cmd::RemoveGroup(group.to_string()));
        }

        fn clear_all(&self) {
            self.send(Cmd::ClearAll);
        }

        fn history(&self) -> Vec<HistoryEntry> {
            let (reply_tx, reply_rx) = mpsc::channel();
            if !self.send(Cmd::History(reply_tx)) {
                return Vec::new();
            }
            reply_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap_or_default()
        }
    }

    fn history() -> windows::core::Result<windows::UI::Notifications::ToastNotificationHistory> {
        ToastNotificationManager::History()
    }

    fn read_history(aumid: &str) -> Vec<HistoryEntry> {
        let list = match history().and_then(|h| h.GetHistoryWithId(&HSTRING::from(aumid))) {
            Ok(l) => l,
            Err(e) => {
                log::warn!("reading the toast history failed: {e}");
                return Vec::new();
            }
        };
        list.into_iter()
            .map(|t| HistoryEntry {
                tag: t.Tag().map(|s| s.to_string()).unwrap_or_default(),
                group: t.Group().map(|s| s.to_string()).unwrap_or_default(),
                xml: t.Content().and_then(|c| c.GetXml()).map(|s| s.to_string()).unwrap_or_default(),
            })
            .collect()
    }

    /// 100-nanosecond ticks since 1601, as a `DateTime`.
    fn windows_time(unix_secs: u64) -> DateTime {
        DateTime { UniversalTime: ((unix_secs + 11_644_473_600) * 10_000_000) as i64 }
    }

    fn show(notifier: &ToastNotifier, spec: &ToastSpec, events: &Events) -> Result<(), String> {
        let e = |what: &str, err: windows::core::Error| format!("{what}: {err}");
        let doc = XmlDocument::new().map_err(|x| e("xml document", x))?;
        doc.LoadXml(&HSTRING::from(spec.xml.as_str())).map_err(|x| e("the toast XML was refused", x))?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|x| e("toast", x))?;
        toast.SetTag(&HSTRING::from(spec.tag.as_str())).map_err(|x| e("tag", x))?;
        toast.SetGroup(&HSTRING::from(spec.group.as_str())).map_err(|x| e("group", x))?;
        // The properties below came with later versions of Windows 10: a build that doesn't have one still shows the toast, so a failure here is logged and
        // not fatal (the text, the tag and the group are what the feature needs).
        let soft = |what: &str, result: windows::core::Result<()>| {
            if let Err(err) = result {
                log::warn!("a toast property is not available on this Windows ({what}): {err}");
            }
        };
        soft("suppress popup", toast.SetSuppressPopup(spec.suppress_popup));
        soft("expires on reboot", toast.SetExpiresOnReboot(true));
        // The spike's correction to the design: the enum value is `Disabled`. It keeps Windows from mirroring the toast on to other devices.
        soft("notification mirroring", toast.SetNotificationMirroring(NotificationMirroring::Disabled));
        let expires = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) + spec.expires_in.as_secs();
        match PropertyValue::CreateDateTime(windows_time(expires)).and_then(|v| v.cast::<IReference<DateTime>>()) {
            Ok(expiry) => soft("expiration time", toast.SetExpirationTime(&expiry)),
            Err(err) => log::warn!("the expiration time of a toast could not be made: {err}"),
        }

        let (tx, tag, group, generation) = (events.clone(), spec.tag.clone(), spec.group.clone(), spec.generation);
        toast
            .Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, args| {
                let reason = match args.as_ref().map(|a| a.Reason()) {
                    Some(Ok(r)) if r == ToastDismissalReason::UserCanceled => DismissReason::UserCanceled,
                    Some(Ok(r)) if r == ToastDismissalReason::ApplicationHidden => DismissReason::ApplicationHidden,
                    _ => DismissReason::TimedOut,
                };
                tx(SurfaceEvent::Dismissed { tag: tag.clone(), group: group.clone(), generation, reason });
                Ok(())
            }))
            .map_err(|x| e("dismissed handler", x))?;
        let tx = events.clone();
        toast
            .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
                if let Some(a) = args.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()) {
                    let arguments = a.Arguments().map(|s| s.to_string()).unwrap_or_default();
                    let inputs = read_inputs(&a);
                    tx(SurfaceEvent::Activated { arguments, inputs });
                }
                Ok(())
            }))
            .map_err(|x| e("activated handler", x))?;
        notifier.Show(&toast).map_err(|x| e("show", x))
    }

    /// The values of the toast's input boxes (`ToastActivatedEventArgs::UserInput`): the reply text, by the id of its box.
    fn read_inputs(args: &ToastActivatedEventArgs) -> Vec<(String, String)> {
        let Ok(set) = args.UserInput() else {
            return Vec::new();
        };
        set.into_iter()
            .filter_map(|pair| {
                let key = pair.Key().ok()?.to_string();
                let value = pair.Value().ok()?.cast::<windows::Foundation::IReference<HSTRING>>().ok()?.Value().ok()?.to_string();
                Some((key, value))
            })
            .collect()
    }
}
