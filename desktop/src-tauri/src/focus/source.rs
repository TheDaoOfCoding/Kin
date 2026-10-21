//! The real source of the two Windows facts behind "the PC is in Focus" (design 5.1, 5.2): `FocusSessionManager` and `SHQueryUserNotificationState`.
//!
//! **Read-only, always.** The only calls made on the manager are `GetDefault`, `IsFocusActive` and the `IsFocusActiveChanged` event. `TryStartFocusSession` and
//! `DeactivateFocus` (a Limited Access Feature that needs a token from Microsoft) are **never called**, and nothing here writes a Windows setting. A test greps this
//! file for the two names.
//!
//! **One worker thread, in the MTA**, owns the WinRT object (the pattern of `mirror/surface.rs` and `media/reader.rs`), so callers never need COM and the event handler
//! (which only wakes the presence pump: it reads nothing and decides nothing) can come on any thread. `GetDefault` failing means Windows 10 or an older Windows 11: the
//! source says "not supported" and reads quiet time only.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::state::QuietSource;
use crate::quiet::{UserState, UserStateProbe, WindowsUserState};

enum Request {
    Read(Sender<bool>),
    Stop,
}

/// What the worker found out at start, for `supported`.
pub struct WindowsQuiet {
    to_worker: Mutex<Sender<Request>>,
    supported: Arc<AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl WindowsQuiet {
    /// Starts the worker. `on_change` runs (on a WinRT thread) when Windows says the Focus state changed: it must only wake something up.
    pub fn new(on_change: Arc<dyn Fn() + Send + Sync>) -> WindowsQuiet {
        let (tx, rx) = mpsc::channel::<Request>();
        let supported = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let supported_in = supported.clone();
        let thread = std::thread::Builder::new()
            .name("focus-source".into())
            .stack_size(256 * 1024)
            .spawn(move || worker(rx, supported_in, on_change, ready_tx))
            .ok();
        // The start (GetDefault and the event) takes a few milliseconds; a read right after `new` must know whether it is supported.
        let _ = ready_rx.recv_timeout(Duration::from_secs(5));
        WindowsQuiet { to_worker: Mutex::new(tx), supported, thread: Mutex::new(thread) }
    }
}

#[cfg(windows)]
fn worker(rx: mpsc::Receiver<Request>, supported: Arc<AtomicBool>, on_change: Arc<dyn Fn() + Send + Sync>, ready: Sender<()>) {
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Shell::FocusSessionManager;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows_core::IInspectable;

    // SAFETY: balanced by the `CoUninitialize` below when it succeeded.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let manager = FocusSessionManager::GetDefault().ok();
    supported.store(manager.is_some(), Ordering::SeqCst);
    let token = manager.as_ref().and_then(|m| {
        let handler = TypedEventHandler::<FocusSessionManager, IInspectable>::new(move |_, _| {
            on_change();
            Ok(())
        });
        m.IsFocusActiveChanged(&handler).ok()
    });
    let _ = ready.send(());
    while let Ok(request) = rx.recv() {
        match request {
            Request::Read(reply) => {
                // A failing read is "not in a Focus session": a check that fails must not silence anybody's phone.
                let _ = reply.send(manager.as_ref().and_then(|m| m.IsFocusActive().ok()).unwrap_or(false));
            }
            Request::Stop => break,
        }
    }
    if let (Some(m), Some(t)) = (&manager, token) {
        let _ = m.RemoveIsFocusActiveChanged(t);
    }
    // The WinRT object is released **before** COM is torn down (the other way round it was an access violation at exit: found by the probe, spike 2B.5).
    drop(manager);
    if com {
        // SAFETY: balances the `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
}

#[cfg(not(windows))]
fn worker(rx: mpsc::Receiver<Request>, supported: Arc<AtomicBool>, _on_change: Arc<dyn Fn() + Send + Sync>, ready: Sender<()>) {
    supported.store(false, Ordering::SeqCst);
    let _ = ready.send(());
    while let Ok(request) = rx.recv() {
        match request {
            Request::Read(reply) => {
                let _ = reply.send(false);
            }
            Request::Stop => break,
        }
    }
}

impl QuietSource for WindowsQuiet {
    fn focus_session(&self) -> bool {
        if !self.supported.load(Ordering::SeqCst) {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        let sent = self.to_worker.lock().unwrap_or_else(|p| p.into_inner()).send(Request::Read(tx)).is_ok();
        sent && rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
    }

    fn quiet_state(&self) -> UserState {
        WindowsUserState.state()
    }

    fn focus_sessions_supported(&self) -> bool {
        self.supported.load(Ordering::SeqCst)
    }
}

impl Drop for WindowsQuiet {
    fn drop(&mut self) {
        let _ = self.to_worker.lock().unwrap_or_else(|p| p.into_inner()).send(Request::Stop);
        if let Some(t) = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real source reads, never writes: it starts, answers and stops. Whatever the user's Windows is doing, this changes nothing (no Focus session is started or ended).
    #[test]
    fn the_real_source_reads_and_stops_without_changing_anything() {
        let source = WindowsQuiet::new(Arc::new(|| {}));
        let _ = source.focus_session();
        let _ = source.quiet_state();
        let _ = source.focus_sessions_supported();
        drop(source);
    }

    /// Plan rule 7 and design 7: no code path starts or ends a Windows Focus session (a Limited Access Feature, and a change of the user's own setting).
    #[test]
    fn the_source_never_calls_the_two_calls_that_change_focus() {
        let text = include_str!("source.rs");
        // The names are assembled here so this file does not contain them in one piece outside this test.
        let start = ["TryStart", "FocusSession"].concat();
        let end = ["Deactivate", "Focus"].concat();
        let code_before_tests = text.split("#[cfg(test)]").next().unwrap();
        // Only comments mention them: no line of code does.
        for line in code_before_tests.lines().filter(|l| !l.trim_start().starts_with("//")) {
            assert!(!line.contains(&start) && !line.contains(&end), "a call that changes Focus: {line}");
        }
    }
}
