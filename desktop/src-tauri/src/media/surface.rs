//! The phone's media in the Windows media flyout (design 5.3), behind a trait: [`WinSurface`] (SystemMediaTransportControls on a hidden top-level window of its
//! own) and a [`FakeSurface`] for tests.
//!
//! **What was verified (design section 2):** `ISystemMediaTransportControlsInterop::GetForWindow` works for an unpackaged exe, with no audio and no `MediaPlayer`,
//! on a **hidden top-level window** (it fails with `E_INVALIDARG` for a message-only window, so Kin's clipboard window can't be used). Its session shows in the flyout
//! with title, artist, album, a thumbnail from an in-memory stream, the timeline and the buttons, and a press comes back as `ButtonPressed` and a drag of the bar as
//! `PlaybackPositionChangeRequested`.
//!
//! **One thread does every SMTC call** (it owns the window and its message loop); the core talks to it through a queue and a posted message. That thread never waits
//! for GSMTC: reading its own session's thumbnail from it would deadlock (the system needs its message pump), which is also why the reader lives on another thread
//! and skips Kin's own session.
//!
//! **While the Windows session is locked** the session is disabled and its content cleared (a phone's now-playing title must not show on a lock screen).
use std::sync::Arc;
use std::sync::Mutex;

use super::map::{SmtcStatus, SmtcView};

/// Where the bar is, when the source gave a duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    pub duration_ms: u64,
    pub position_ms: u64,
    /// The source lists `seek`: the bar can be dragged.
    pub seek: bool,
}

/// Everything the flyout is told.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceView {
    pub buttons: SmtcView,
    pub title: String,
    pub artist: String,
    /// The album line: `<phone> · <app>` (the flyout may not show it; the tray panel always does).
    pub album_line: String,
    /// A PNG or JPEG, already checked.
    pub thumbnail: Option<Arc<Vec<u8>>>,
    pub timeline: Option<Timeline>,
    /// 1.0 is normal speed.
    pub rate: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Play,
    Pause,
    Next,
    Prev,
}

/// What the user did in the flyout, or Windows told us.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceEvent {
    Button(Button),
    Seek(u64),
    /// The Windows session was locked (`true`) or unlocked.
    Locked(bool),
}

pub type Events = Arc<dyn Fn(SurfaceEvent) + Send + Sync>;

pub trait MediaSurface: Send + Sync {
    /// Shows or updates the session (and enables the flyout's session).
    fn show(&self, view: SurfaceView) -> Result<(), String>;
    /// Only the status and the timeline moved (the position was extrapolated).
    fn update_timeline(&self, status: SmtcStatus, timeline: Option<Timeline>) -> Result<(), String>;
    /// Removes the session from the flyout.
    fn clear(&self);
}

// ----- the fake ---------------------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeSurface {
    shown: Mutex<Option<SurfaceView>>,
    log: Mutex<Vec<SurfaceView>>,
    timelines: Mutex<Vec<(SmtcStatus, Option<Timeline>)>>,
    clears: Mutex<usize>,
}

impl FakeSurface {
    /// What the flyout shows now (`None` when cleared).
    pub fn current(&self) -> Option<SurfaceView> {
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Every `show` so far.
    pub fn shows(&self) -> Vec<SurfaceView> {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn timelines(&self) -> Vec<(SmtcStatus, Option<Timeline>)> {
        self.timelines.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn clears(&self) -> usize {
        *self.clears.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl MediaSurface for FakeSurface {
    fn show(&self, view: SurfaceView) -> Result<(), String> {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).push(view.clone());
        *self.shown.lock().unwrap_or_else(|p| p.into_inner()) = Some(view);
        Ok(())
    }

    fn update_timeline(&self, status: SmtcStatus, timeline: Option<Timeline>) -> Result<(), String> {
        self.timelines.lock().unwrap_or_else(|p| p.into_inner()).push((status, timeline));
        if let Some(v) = self.shown.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            v.buttons.status = status;
            v.timeline = timeline;
        }
        Ok(())
    }

    fn clear(&self) {
        *self.clears.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        *self.shown.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

// ----- Windows ----------------------------------------------------------------------------------------------------------

pub use win::WinSurface;

mod win {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex, mpsc};

    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
    };
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG, PostMessageW, RegisterClassW, SendMessageW,
        TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_WTSSESSION_CHANGE, WNDCLASSW,
    };
    use windows::core::{HSTRING, w};

    use super::super::map::SmtcStatus;
    use super::{Button, Events, MediaSurface, SurfaceEvent, SurfaceView, Timeline};

    const WM_WAKE: u32 = WM_APP + 1;
    /// Posted by the window procedure when the Windows session locks (`wParam` 1) or unlocks (0).
    const WM_LOCK: u32 = WM_APP + 2;
    const WTS_SESSION_LOCK: usize = 0x7;
    const WTS_SESSION_UNLOCK: usize = 0x8;

    enum Cmd {
        Show(SurfaceView, mpsc::Sender<Result<(), String>>),
        Timeline(SmtcStatus, Option<Timeline>, mpsc::Sender<Result<(), String>>),
        Clear,
        Stop,
    }

    /// The real flyout session: a thread that owns a hidden window and the `SystemMediaTransportControls` of it.
    pub struct WinSurface {
        queue: Arc<Mutex<Vec<Cmd>>>,
        /// `true` while the thread's window exists. The thread flips it (and empties the queue, so a waiting `call` is released) **before** it destroys the window,
        /// under the same lock `post` holds while it posts: nothing is ever posted to a destroyed window (whose handle Windows may give to another).
        alive: Arc<Mutex<bool>>,
        hwnd: isize,
        thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    }

    /// How long `stop` waits for the thread to leave. It is only ever stuck inside a Windows call; quitting must not wait for that.
    const STOP_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

    /// The lock state of the Windows session as `WTSSessionInfoEx` reports it: `WTS_SESSIONSTATE_LOCK` is 0 and `WTS_SESSIONSTATE_UNLOCK` 1 (unknown: -1, not locked).
    fn locked_from_session_flags(flags: i32) -> bool {
        flags == 0
    }

    /// Whether the Windows session is locked right now (asked once, when the surface starts: the notices only tell about changes after that).
    fn session_is_locked() -> bool {
        use windows::Win32::System::RemoteDesktop::{
            WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSFreeMemory, WTSINFOEXW, WTSQuerySessionInformationW, WTSSessionInfoEx,
        };
        use windows::core::PWSTR;
        let mut buffer = PWSTR::null();
        let mut length = 0u32;
        // SAFETY: the buffer that comes back is read as the `WTSINFOEXW` it is for this information class, and freed with `WTSFreeMemory`.
        unsafe {
            if WTSQuerySessionInformationW(Some(WTS_CURRENT_SERVER_HANDLE), WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buffer, &mut length)
                .is_err()
            {
                return false;
            }
            let info = buffer.0 as *const WTSINFOEXW;
            let locked = !info.is_null()
                && (length as usize) >= std::mem::size_of::<WTSINFOEXW>()
                && (*info).Level == 1
                && locked_from_session_flags((*info).Data.WTSInfoExLevel1.SessionFlags);
            WTSFreeMemory(buffer.0 as *mut _);
            locked
        }
    }

    thread_local! {
        /// The sink of the window procedure's events (the lock notices come as window messages).
        static EVENTS: RefCell<Option<Events>> = const { RefCell::new(None) };
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_WTSSESSION_CHANGE {
            let locked = match wparam.0 {
                WTS_SESSION_LOCK => Some(true),
                WTS_SESSION_UNLOCK => Some(false),
                _ => None,
            };
            if let Some(l) = locked {
                EVENTS.with(|e| {
                    if let Some(f) = e.borrow().as_ref() {
                        f(SurfaceEvent::Locked(l));
                    }
                });
                // The session notice is **sent** to the window (it never passes through the message loop), so the thread's own loop is told with a posted message.
                // SAFETY: posting to this very window.
                let _ = unsafe { PostMessageW(Some(hwnd), WM_LOCK, WPARAM(usize::from(l)), LPARAM(0)) };
            }
            return LRESULT(0);
        }
        // SAFETY: the default procedure of a window of this thread, with the arguments it was called with.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn ticks(ms: u64) -> TimeSpan {
        TimeSpan { Duration: (ms as i64).saturating_mul(10_000) }
    }

    fn status(s: SmtcStatus) -> MediaPlaybackStatus {
        match s {
            SmtcStatus::Changing => MediaPlaybackStatus::Changing,
            SmtcStatus::Stopped => MediaPlaybackStatus::Stopped,
            SmtcStatus::Playing => MediaPlaybackStatus::Playing,
            SmtcStatus::Paused => MediaPlaybackStatus::Paused,
        }
    }

    /// The thread's state: the SMTC object and what was last shown (to show again after an unlock).
    struct Session {
        smtc: SystemMediaTransportControls,
        locked: bool,
        last: Option<SurfaceView>,
    }

    impl Session {
        fn apply(&self, v: &SurfaceView) -> windows::core::Result<()> {
            let s = &self.smtc;
            s.SetIsEnabled(true)?;
            s.SetIsPlayEnabled(v.buttons.play)?;
            s.SetIsPauseEnabled(v.buttons.pause)?;
            s.SetIsNextEnabled(v.buttons.next)?;
            s.SetIsPreviousEnabled(v.buttons.prev)?;
            s.SetPlaybackStatus(status(v.buttons.status))?;
            s.SetPlaybackRate(v.rate)?;
            let du = s.DisplayUpdater()?;
            du.SetType(MediaPlaybackType::Music)?;
            let music = du.MusicProperties()?;
            music.SetTitle(&HSTRING::from(v.title.as_str()))?;
            music.SetArtist(&HSTRING::from(v.artist.as_str()))?;
            music.SetAlbumTitle(&HSTRING::from(v.album_line.as_str()))?;
            match &v.thumbnail {
                Some(bytes) => {
                    let stream = InMemoryRandomAccessStream::new()?;
                    let writer = DataWriter::CreateDataWriter(&stream)?;
                    writer.WriteBytes(bytes)?;
                    writer.StoreAsync()?.join()?;
                    writer.DetachStream()?;
                    stream.Seek(0)?;
                    du.SetThumbnail(&RandomAccessStreamReference::CreateFromStream(&stream)?)?;
                }
                None => du.SetThumbnail(None)?,
            }
            du.Update()?;
            self.timeline(v.timeline)
        }

        fn timeline(&self, t: Option<Timeline>) -> windows::core::Result<()> {
            let props = SystemMediaTransportControlsTimelineProperties::new()?;
            props.SetStartTime(ticks(0))?;
            match t {
                Some(t) => {
                    props.SetEndTime(ticks(t.duration_ms))?;
                    props.SetPosition(ticks(t.position_ms.min(t.duration_ms)))?;
                    props.SetMinSeekTime(ticks(0))?;
                    props.SetMaxSeekTime(ticks(if t.seek { t.duration_ms } else { 0 }))?;
                }
                None => {
                    props.SetEndTime(ticks(0))?;
                    props.SetPosition(ticks(0))?;
                    props.SetMinSeekTime(ticks(0))?;
                    props.SetMaxSeekTime(ticks(0))?;
                }
            }
            self.smtc.UpdateTimelineProperties(&props)
        }

        fn clear(&self) {
            if let Ok(du) = self.smtc.DisplayUpdater() {
                let _ = du.ClearAll();
                let _ = du.Update();
            }
            let _ = self.smtc.SetIsEnabled(false);
        }
    }

    impl WinSurface {
        /// Starts the thread and registers the session. `events` gets the presses of the flyout's buttons, a drag of its bar and the lock notices (on threads of
        /// Windows' own: it must not block).
        pub fn start(events: Events) -> Result<WinSurface, String> {
            let queue: Arc<Mutex<Vec<Cmd>>> = Arc::new(Mutex::new(Vec::new()));
            let (ready_tx, ready_rx) = mpsc::channel::<Result<isize, String>>();
            let thread_queue = queue.clone();
            let alive = Arc::new(Mutex::new(true));
            let thread_alive = alive.clone();
            let thread = std::thread::Builder::new()
                .name("media-surface".into())
                .spawn(move || {
                    // SAFETY: COM for this thread, in the multithreaded apartment: the SMTC events then arrive on pool threads.
                    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                    EVENTS.with(|e| *e.borrow_mut() = Some(events.clone()));
                    // SAFETY: a window class and a window of this thread, never shown; the handle is only used on this thread and by `PostMessageW`.
                    let hwnd = unsafe {
                        let hinstance = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
                        let class = WNDCLASSW {
                            lpfnWndProc: Some(wnd_proc),
                            hInstance: hinstance,
                            lpszClassName: w!("KinMediaControls"),
                            ..Default::default()
                        };
                        RegisterClassW(&class);
                        // No parent, style 0, never shown: a top-level window (a message-only window is refused by GetForWindow).
                        CreateWindowExW(
                            WINDOW_EX_STYLE(0),
                            w!("KinMediaControls"),
                            w!("Kin media controls"),
                            WINDOW_STYLE(0),
                            0,
                            0,
                            0,
                            0,
                            None,
                            None,
                            Some(hinstance),
                            None,
                        )
                    };
                    let hwnd = match hwnd {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("the media window can't be made: {e}")));
                            return;
                        }
                    };
                    let smtc = match register(hwnd, &events) {
                        Ok(s) => s,
                        Err(e) => {
                            // SAFETY: the window made above, on its own thread.
                            let _ = unsafe { DestroyWindow(hwnd) };
                            let _ = ready_tx.send(Err(format!("Windows can't show media here: {e}")));
                            return;
                        }
                    };
                    // SAFETY: the window belongs to this thread; the notice is undone before it is destroyed.
                    let registered = match unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } {
                        Ok(()) => true,
                        Err(e) => {
                            log::warn!("media: lock notices are not available ({e}); the flyout will not hide while the PC is locked");
                            false
                        }
                    };
                    // Asked after the registration, so a lock between the two is not missed: the notices only tell about changes from here on.
                    let mut session = Session { smtc, locked: session_is_locked(), last: None };
                    let _ = ready_tx.send(Ok(hwnd.0 as isize));
                    let mut msg = MSG::default();
                    // SAFETY: the standard message loop of the thread that owns the window.
                    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                        if msg.message == WM_WAKE {
                            let cmds: Vec<Cmd> = std::mem::take(&mut *thread_queue.lock().unwrap_or_else(|p| p.into_inner()));
                            let mut stop = false;
                            for cmd in cmds {
                                match cmd {
                                    Cmd::Show(view, reply) => {
                                        session.last = Some(view.clone());
                                        let r = if session.locked { Ok(()) } else { session.apply(&view).map_err(|e| e.to_string()) };
                                        let _ = reply.send(r);
                                    }
                                    Cmd::Timeline(st, t, reply) => {
                                        if let Some(v) = session.last.as_mut() {
                                            v.buttons.status = st;
                                            v.timeline = t;
                                        }
                                        let r = if session.locked || session.last.is_none() {
                                            Ok(())
                                        } else {
                                            session
                                                .smtc
                                                .SetPlaybackStatus(status(st))
                                                .and_then(|()| session.timeline(t))
                                                .map_err(|e| e.to_string())
                                        };
                                        let _ = reply.send(r);
                                    }
                                    Cmd::Clear => {
                                        session.last = None;
                                        session.clear();
                                    }
                                    Cmd::Stop => stop = true,
                                }
                            }
                            if stop {
                                break;
                            }
                            continue;
                        }
                        // The Windows session locked or unlocked (posted by `wnd_proc`): a locked PC shows nothing of what plays on the phone.
                        if msg.message == WM_LOCK {
                            if msg.wParam.0 == 1 {
                                session.locked = true;
                                session.clear();
                            } else {
                                session.locked = false;
                                if let Some(v) = session.last.clone() {
                                    let _ = session.apply(&v);
                                }
                            }
                            continue;
                        }
                        // SAFETY: the standard dispatch of a message of this thread.
                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                    session.clear();
                    {
                        // From here nothing is posted to this window, and a `call` that was waiting for an answer is released (its sender is dropped with the queue).
                        let mut alive = thread_alive.lock().unwrap_or_else(|p| p.into_inner());
                        *alive = false;
                        thread_queue.lock().unwrap_or_else(|p| p.into_inner()).clear();
                    }
                    // SAFETY: undoing the registration and destroying the window made above, on the thread that owns them.
                    unsafe {
                        if registered {
                            let _ = WTSUnRegisterSessionNotification(hwnd);
                        }
                        let _ = DestroyWindow(hwnd);
                    }
                })
                .map_err(|e| format!("the media thread can't start: {e}"))?;
            let hwnd = ready_rx.recv().map_err(|_| "the media thread ended at once".to_string())??;
            Ok(WinSurface { queue, alive, hwnd, thread: Mutex::new(Some(thread)) })
        }

        /// Queues a command and wakes the thread; `false` if the thread's window is gone (nothing is queued or posted then).
        fn post(&self, cmd: Cmd) -> bool {
            // Held while posting: the thread takes the same lock to say the window is going, so the window exists for the whole call.
            let alive = self.alive.lock().unwrap_or_else(|p| p.into_inner());
            if !*alive {
                return false;
            }
            self.queue.lock().unwrap_or_else(|p| p.into_inner()).push(cmd);
            // SAFETY: the window exists (`alive`, held); posting a message to a window of another thread is what `PostMessageW` is for.
            unsafe { PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0)) }.is_ok()
        }

        fn call(&self, make: impl FnOnce(mpsc::Sender<Result<(), String>>) -> Cmd) -> Result<(), String> {
            let (tx, rx) = mpsc::channel();
            if !self.post(make(tx)) {
                return Err("the media thread is gone".to_string());
            }
            rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "the media thread did not answer".to_string())?
        }

        /// For tests: what Windows does when the session locks or unlocks, to this window and no other (it **sends** `WM_WTSSESSION_CHANGE` to it).
        #[doc(hidden)]
        pub fn simulate_lock_for_tests(&self, locked: bool) {
            let what = if locked { WTS_SESSION_LOCK } else { WTS_SESSION_UNLOCK };
            // SAFETY: a message to a window of this process, which its own thread handles.
            unsafe { SendMessageW(HWND(self.hwnd as *mut _), WM_WTSSESSION_CHANGE, Some(WPARAM(what)), Some(LPARAM(0))) };
        }

        /// Asks the thread to leave and waits for it, but at most [`STOP_WAIT`]: a thread stuck in a Windows call is left behind (it ends with the process) rather
        /// than holding up a quit.
        pub fn stop(&self) {
            self.post(Cmd::Stop);
            let Some(thread) = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
            let deadline = std::time::Instant::now() + STOP_WAIT;
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                log::warn!("media: the flyout thread did not stop in time; it is left behind");
            }
        }
    }

    impl Drop for WinSurface {
        fn drop(&mut self) {
            let _ = self.post(Cmd::Stop);
        }
    }

    impl MediaSurface for WinSurface {
        fn show(&self, view: SurfaceView) -> Result<(), String> {
            self.call(|tx| Cmd::Show(view, tx))
        }

        fn update_timeline(&self, status: SmtcStatus, timeline: Option<Timeline>) -> Result<(), String> {
            self.call(|tx| Cmd::Timeline(status, timeline, tx))
        }

        fn clear(&self) {
            self.post(Cmd::Clear);
        }
    }

    /// `GetForWindow` and the two events. The session is created disabled: nothing shows until the first `show`.
    fn register(hwnd: HWND, events: &Events) -> windows::core::Result<SystemMediaTransportControls> {
        let interop = windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        // SAFETY: `hwnd` is a top-level window of this process and this thread.
        let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
        smtc.SetIsEnabled(false)?;
        let tx = events.clone();
        smtc.ButtonPressed(&TypedEventHandler::<SystemMediaTransportControls, SystemMediaTransportControlsButtonPressedEventArgs>::new(
            move |_, args| {
                if let Some(a) = args.as_ref() {
                    let button = a.Button()?;
                    let mapped = match button {
                        SystemMediaTransportControlsButton::Play => Some(Button::Play),
                        SystemMediaTransportControlsButton::Pause => Some(Button::Pause),
                        SystemMediaTransportControlsButton::Next => Some(Button::Next),
                        SystemMediaTransportControlsButton::Previous => Some(Button::Prev),
                        _ => None,
                    };
                    if let Some(b) = mapped {
                        tx(SurfaceEvent::Button(b));
                    }
                }
                Ok(())
            },
        ))?;
        let tx = events.clone();
        smtc.PlaybackPositionChangeRequested(
            &TypedEventHandler::<SystemMediaTransportControls, PlaybackPositionChangeRequestedEventArgs>::new(move |_, args| {
                if let Some(a) = args.as_ref() {
                    let ticks = a.RequestedPlaybackPosition()?.Duration;
                    tx(SurfaceEvent::Seek((ticks.max(0) / 10_000) as u64));
                }
                Ok(())
            }),
        )?;
        Ok(smtc)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_session_flags_say_locked_only_for_the_lock_value() {
            assert!(locked_from_session_flags(0), "WTS_SESSIONSTATE_LOCK");
            assert!(!locked_from_session_flags(1), "WTS_SESSIONSTATE_UNLOCK");
            assert!(!locked_from_session_flags(-1), "WTS_SESSIONSTATE_UNKNOWN");
        }

        #[test]
        fn a_stopped_surface_posts_nothing_and_does_not_make_a_call_wait() {
            let surface = WinSurface::start(Arc::new(|_| {})).expect("a hidden window and its session");
            surface.stop();
            let started = std::time::Instant::now();
            assert!(!surface.post(Cmd::Clear), "the window is gone: nothing is posted to it");
            let shown = surface.show(SurfaceView {
                buttons: crate::media::map::SmtcView { status: SmtcStatus::Paused, play: false, pause: false, next: false, prev: false },
                title: String::new(),
                artist: String::new(),
                album_line: String::new(),
                thumbnail: None,
                timeline: None,
                rate: 1.0,
            });
            assert!(shown.is_err(), "a call to a stopped surface fails at once");
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            assert!(surface.queue.lock().unwrap().is_empty(), "and queues nothing");
        }
    }
}
