//! Reading and controlling the PC's media sessions (Windows' Global System Media Transport Controls, design 5.2) behind a trait: [`WinBackend`] (GSMTC on a
//! thread of its own) and [`FakeBackend`] for tests, with [`Guarded`] in front of both.
//!
//! **Verified on this kind of PC (design section 2):** GSMTC works from an unpackaged exe with no capability; nothing ticks (no events while a video plays), so the
//! reader is event-driven and the position is extrapolated (`position::windows_source_position`); a session of Kin's own (the phone's media shown in the flyout)
//! appears in the list under this process's AUMID and is **skipped**.
//!
//! **[`Guarded`] is the one way in.** Every call (`list`, `read`, `thumbnail`, `control`) checks the session's `SourceAppUserModelId` against
//! `guard::Mode::may_use_session` **before** the backend is asked for anything but the id, so in a test run a real session's metadata, thumbnail and timeline are
//! never even requested, and a command never reaches a session that isn't the test one.
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs, SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

use super::guard::Mode;
use super::map::Controls;

/// A thumbnail bigger than this is not read.
pub const MAX_THUMBNAIL_BYTES: u64 = 2 * 1024 * 1024;

/// What one session says right now. Strings are raw (the wire cleans them); `position_ms` is already extrapolated to the moment of the read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub aumid: String,
    /// GSMTC's `PlaybackStatus` as an integer (Closed 0 ... Paused 5).
    pub status: i32,
    pub controls: Controls,
    pub duration_ms: Option<u64>,
    pub position_ms: u64,
    pub rate_milli: Option<u64>,
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// A command for a session (`media.cmd` after the checks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    SeekTo(u64),
}

/// The calls of a media session backend. `list` gives only the `SourceAppUserModelId`s, in the platform's priority order (the system's current session first).
pub trait SessionBackend: Send + Sync {
    fn list(&self) -> Vec<String>;
    fn read(&self, aumid: &str) -> Option<SessionInfo>;
    fn thumbnail(&self, aumid: &str) -> Option<Vec<u8>>;
    /// `Ok(false)`: the session refused (the app says no).
    fn control(&self, aumid: &str, control: Control) -> Result<bool, String>;
}

/// The only way the rest of the app reaches the sessions of the PC.
pub struct Guarded {
    backend: Arc<dyn SessionBackend>,
    mode: Mode,
    /// This process's own AUMID: its own session is never read.
    own: Option<String>,
}

impl Guarded {
    pub fn new(backend: Arc<dyn SessionBackend>, mode: Mode, own: Option<String>) -> Guarded {
        Guarded { backend, mode, own }
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// Whether this session may be used at all (the check every call makes first).
    pub fn allows(&self, aumid: &str) -> bool {
        self.mode.may_use_session(aumid, self.own.as_deref())
    }

    /// The ids of the sessions that may be read, in priority order.
    pub fn list(&self) -> Vec<String> {
        self.backend.list().into_iter().filter(|a| self.allows(a)).collect()
    }

    /// Every allowed session, read.
    pub fn snapshot(&self) -> Vec<SessionInfo> {
        self.list().iter().filter_map(|a| self.read(a)).collect()
    }

    pub fn read(&self, aumid: &str) -> Option<SessionInfo> {
        if !self.allows(aumid) {
            return None;
        }
        self.backend.read(aumid)
    }

    pub fn thumbnail(&self, aumid: &str) -> Option<Vec<u8>> {
        if !self.allows(aumid) {
            return None;
        }
        self.backend.thumbnail(aumid)
    }

    pub fn control(&self, aumid: &str, control: Control) -> Result<bool, String> {
        if !self.allows(aumid) {
            return Err("that session is not one this run may control".to_string());
        }
        self.backend.control(aumid, control)
    }
}

// ----- the display name of an app ----------------------------------------------------------------------------------------

/// The name a user knows an app by, from its `SourceAppUserModelId`: a small table of the common players, else the id without `.exe`. A packaged app's name
/// (`Package_hash!App`) comes from Windows (`packaged_name`), and this is only the fallback.
pub fn display_name(aumid: &str) -> String {
    let lower = aumid.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    let known = match stem {
        "spotify" => "Spotify",
        "chrome" => "Chrome",
        "msedge" | "microsoftedge" => "Edge",
        "firefox" => "Firefox",
        "vlc" => "VLC",
        "foobar2000" => "foobar2000",
        "brave" => "Brave",
        "opera" | "operagx" => "Opera",
        "musicbee" => "MusicBee",
        "itunes" | "applemusic" => "Apple Music",
        "youtubemusic" | "youtube music" => "YouTube Music",
        "zunemusic" | "microsoft.zunemusic" => "Media Player",
        "wmplayer" => "Windows Media Player",
        "mpc-hc64" | "mpc-hc" => "MPC-HC",
        "potplayermini64" | "potplayermini" => "PotPlayer",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    if aumid.contains('!') {
        // A packaged app: the part before the underscore is usually the name (`SpotifyAB.SpotifyMusic`).
        let package = aumid.split('!').next().unwrap_or(aumid);
        let name = package.split('_').next().unwrap_or(package);
        return name.rsplit('.').next().unwrap_or(name).to_string();
    }
    let trimmed = aumid.strip_suffix(".exe").or_else(|| aumid.strip_suffix(".EXE")).unwrap_or(aumid);
    trimmed.to_string()
}

// ----- the fake -----------------------------------------------------------------------------------------------------------

/// A backend with sessions written by the test. It keeps every call it gets (for tests that prove what was **not** asked).
#[derive(Default)]
pub struct FakeBackend {
    sessions: Mutex<Vec<(SessionInfo, Option<Vec<u8>>)>>,
    pub calls: Mutex<Vec<String>>,
    pub controls: Mutex<Vec<(String, Control)>>,
    pub refuse: Mutex<bool>,
}

impl FakeBackend {
    pub fn set(&self, sessions: Vec<(SessionInfo, Option<Vec<u8>>)>) {
        *self.sessions.lock().unwrap_or_else(|p| p.into_inner()) = sessions;
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn controls(&self) -> Vec<(String, Control)> {
        self.controls.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn note(&self, what: &str, aumid: &str) {
        self.calls.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{what} {aumid}"));
    }
}

impl SessionBackend for FakeBackend {
    fn list(&self) -> Vec<String> {
        self.note("list", "");
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).iter().map(|(s, _)| s.aumid.clone()).collect()
    }

    fn read(&self, aumid: &str) -> Option<SessionInfo> {
        self.note("read", aumid);
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|(s, _)| s.aumid == aumid).map(|(s, _)| s.clone())
    }

    fn thumbnail(&self, aumid: &str) -> Option<Vec<u8>> {
        self.note("thumbnail", aumid);
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|(s, _)| s.aumid == aumid).and_then(|(_, t)| t.clone())
    }

    fn control(&self, aumid: &str, control: Control) -> Result<bool, String> {
        self.note("control", aumid);
        self.controls.lock().unwrap_or_else(|p| p.into_inner()).push((aumid.to_string(), control));
        Ok(!*self.refuse.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

// ----- GSMTC --------------------------------------------------------------------------------------------------------------

/// 100-nanosecond ticks since 1601, for now.
pub fn windows_now_ticks() -> i64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    ((d.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(d.subsec_nanos() / 100)) as i64
}

type Reply<T> = mpsc::Sender<T>;

enum Req {
    List(Reply<Vec<String>>),
    Read(String, Reply<Option<SessionInfo>>),
    Thumbnail(String, Reply<Option<Vec<u8>>>),
    Control(String, Control, Reply<Result<bool, String>>),
    /// The list of sessions changed: listen to the allowed ones.
    Resubscribe,
    Stop,
}

/// Called (on a thread of Windows' own) when anything about a session may have changed. It must not block.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

/// GSMTC on a thread of its own (the multithreaded apartment). Every call is a message to that thread and a wait for its answer, so a caller never needs COM.
pub struct WinBackend {
    tx: Mutex<mpsc::Sender<Req>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `stop` waits for the reader thread to leave.
const STOP_WAIT: Duration = Duration::from_millis(1500);

impl WinBackend {
    /// Starts the thread. `allowed` says which sessions to listen to (the guard's rule): the others are never subscribed to. `on_change` is called for any event
    /// of an allowed session and for a change of the list of sessions.
    pub fn start(allowed: Arc<dyn Fn(&str) -> bool + Send + Sync>, on_change: OnChange) -> Result<WinBackend, String> {
        let (tx, rx) = mpsc::channel::<Req>();
        let own_tx = tx.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("media-reader".into())
            .spawn(move || {
                // SAFETY: COM for this thread, in the multithreaded apartment (event handlers then arrive on pool threads, and a blocking `join` is fine).
                let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if hr.is_err() && hr != RPC_E_CHANGED_MODE {
                    let _ = ready_tx.send(Err(format!("COM could not be started for the media reader: {hr:?}")));
                    return;
                }
                let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.join()) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("Windows' media sessions are not available: {e}")));
                        return;
                    }
                };
                let mut reader = Reader { manager, allowed, on_change, own_tx, subscriptions: HashMap::new(), manager_tokens: None };
                reader.listen_to_the_manager();
                reader.resubscribe();
                let _ = ready_tx.send(Ok(()));
                for req in rx {
                    match req {
                        Req::List(reply) => {
                            let _ = reply.send(reader.list());
                        }
                        Req::Read(aumid, reply) => {
                            let _ = reply.send(reader.read(&aumid));
                        }
                        Req::Thumbnail(aumid, reply) => {
                            let _ = reply.send(reader.thumbnail(&aumid));
                        }
                        Req::Control(aumid, control, reply) => {
                            let _ = reply.send(reader.control(&aumid, control));
                        }
                        Req::Resubscribe => reader.resubscribe(),
                        Req::Stop => break,
                    }
                }
                reader.unsubscribe_all();
            })
            .map_err(|e| format!("the media reader thread can't start: {e}"))?;
        ready_rx.recv().map_err(|_| "the media reader ended at once".to_string())??;
        Ok(WinBackend { tx: Mutex::new(tx), thread: Mutex::new(Some(thread)) })
    }

    fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Req) -> Option<T> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.lock().unwrap_or_else(|p| p.into_inner()).send(make(reply_tx)).ok()?;
        reply_rx.recv_timeout(CALL_TIMEOUT).ok()
    }

    /// Asks the thread to leave and waits for it, but at most [`STOP_WAIT`]: the thread may be inside a slow Windows call (up to [`CALL_TIMEOUT`]), and a quit must
    /// not wait for that. A thread left behind ends with the process.
    pub fn stop(&self) {
        let _ = self.tx.lock().unwrap_or_else(|p| p.into_inner()).send(Req::Stop);
        let Some(thread) = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
        let deadline = std::time::Instant::now() + STOP_WAIT;
        while !thread.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            log::warn!("media: the reader thread did not stop in time; it is left behind");
        }
    }
}

impl Drop for WinBackend {
    fn drop(&mut self) {
        let _ = self.tx.lock().unwrap_or_else(|p| p.into_inner()).send(Req::Stop);
    }
}

impl SessionBackend for WinBackend {
    fn list(&self) -> Vec<String> {
        self.ask(Req::List).unwrap_or_default()
    }

    fn read(&self, aumid: &str) -> Option<SessionInfo> {
        self.ask(|r| Req::Read(aumid.to_string(), r)).flatten()
    }

    fn thumbnail(&self, aumid: &str) -> Option<Vec<u8>> {
        self.ask(|r| Req::Thumbnail(aumid.to_string(), r)).flatten()
    }

    fn control(&self, aumid: &str, control: Control) -> Result<bool, String> {
        self.ask(|r| Req::Control(aumid.to_string(), control, r)).unwrap_or_else(|| Err("the media reader did not answer".to_string()))
    }
}

/// What the reader thread keeps for one session it listens to: the session and the tokens of its three events.
struct Subscription {
    session: GlobalSystemMediaTransportControlsSession,
    tokens: [i64; 3],
}

struct Reader {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    allowed: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    on_change: OnChange,
    own_tx: mpsc::Sender<Req>,
    subscriptions: HashMap<String, Subscription>,
    manager_tokens: Option<[i64; 2]>,
}

fn aumid_of(s: &GlobalSystemMediaTransportControlsSession) -> Option<String> {
    s.SourceAppUserModelId().ok().map(|h| h.to_string())
}

impl Reader {
    fn listen_to_the_manager(&mut self) {
        let (tx, on_change) = (self.own_tx.clone(), self.on_change.clone());
        let sessions = self.manager.SessionsChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSessionManager,
            SessionsChangedEventArgs,
        >::new(move |_, _| {
            let _ = tx.send(Req::Resubscribe);
            on_change();
            Ok(())
        }));
        let on_change = self.on_change.clone();
        let current = self.manager.CurrentSessionChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSessionManager,
            CurrentSessionChangedEventArgs,
        >::new(move |_, _| {
            on_change();
            Ok(())
        }));
        match (sessions, current) {
            (Ok(a), Ok(b)) => self.manager_tokens = Some([a, b]),
            (a, b) => log::warn!("the media reader could not listen to the session list ({:?} {:?})", a.err(), b.err()),
        }
    }

    /// Listens to the sessions that are allowed and that it doesn't listen to yet, and stops listening to the ones that are gone. A session that is not allowed
    /// is never subscribed to: no handler, no read.
    fn resubscribe(&mut self) {
        let Ok(sessions) = self.manager.GetSessions() else { return };
        let mut present: HashMap<String, GlobalSystemMediaTransportControlsSession> = HashMap::new();
        for s in &sessions {
            if let Some(id) = aumid_of(&s)
                && (self.allowed)(&id)
            {
                present.entry(id).or_insert(s);
            }
        }
        let gone: Vec<String> = self.subscriptions.keys().filter(|k| !present.contains_key(*k)).cloned().collect();
        for id in gone {
            if let Some(sub) = self.subscriptions.remove(&id) {
                remove_tokens(&sub);
            }
        }
        for (id, session) in present {
            if self.subscriptions.contains_key(&id) {
                continue;
            }
            let cb = self.on_change.clone();
            let a = session.MediaPropertiesChanged(&TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                MediaPropertiesChangedEventArgs,
            >::new({
                let cb = cb.clone();
                move |_, _| {
                    cb();
                    Ok(())
                }
            }));
            let b = session.PlaybackInfoChanged(&TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                PlaybackInfoChangedEventArgs,
            >::new({
                let cb = cb.clone();
                move |_, _| {
                    cb();
                    Ok(())
                }
            }));
            let c = session.TimelinePropertiesChanged(&TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                TimelinePropertiesChangedEventArgs,
            >::new(move |_, _| {
                cb();
                Ok(())
            }));
            if let (Ok(a), Ok(b), Ok(c)) = (a, b, c) {
                self.subscriptions.insert(id, Subscription { session, tokens: [a, b, c] });
            }
        }
    }

    fn unsubscribe_all(&mut self) {
        for (_, sub) in self.subscriptions.drain() {
            remove_tokens(&sub);
        }
        if let Some([a, b]) = self.manager_tokens.take() {
            let _ = self.manager.RemoveSessionsChanged(a);
            let _ = self.manager.RemoveCurrentSessionChanged(b);
        }
    }

    /// The ids of the sessions (all of them: the guard filters), the system's current one first.
    fn list(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(cur) = self.manager.GetCurrentSession().ok().and_then(|s| aumid_of(&s)) {
            out.push(cur);
        }
        if let Ok(all) = self.manager.GetSessions() {
            for s in &all {
                if let Some(id) = aumid_of(&s)
                    && !out.contains(&id)
                {
                    out.push(id);
                }
            }
        }
        out
    }

    fn session(&self, aumid: &str) -> Option<GlobalSystemMediaTransportControlsSession> {
        let all = self.manager.GetSessions().ok()?;
        all.into_iter().find(|s| aumid_of(s).as_deref() == Some(aumid))
    }

    fn read(&self, aumid: &str) -> Option<SessionInfo> {
        let session = self.session(aumid)?;
        let info = session.GetPlaybackInfo().ok()?;
        let status = info.PlaybackStatus().ok()?.0;
        let c = info.Controls().ok()?;
        let controls = Controls {
            play: c.IsPlayEnabled().unwrap_or(false),
            pause: c.IsPauseEnabled().unwrap_or(false),
            toggle: c.IsPlayPauseToggleEnabled().unwrap_or(false),
            next: c.IsNextEnabled().unwrap_or(false),
            prev: c.IsPreviousEnabled().unwrap_or(false),
            position: c.IsPlaybackPositionEnabled().unwrap_or(false),
        };
        let rate_milli = info.PlaybackRate().ok().and_then(|r| r.Value().ok()).map(|r| (r * 1000.0).round().max(0.0) as u64);
        let timeline = session.GetTimelineProperties().ok();
        let ticks = |f: &dyn Fn(&windows::Media::Control::GlobalSystemMediaTransportControlsSessionTimelineProperties) -> Option<i64>| {
            timeline.as_ref().and_then(f).unwrap_or(0)
        };
        let start = ticks(&|t| t.StartTime().ok().map(|d| d.Duration));
        let end = ticks(&|t| t.EndTime().ok().map(|d| d.Duration));
        let position = ticks(&|t| t.Position().ok().map(|d| d.Duration));
        let updated = ticks(&|t| t.LastUpdatedTime().ok().map(|d| d.UniversalTime));
        let duration_ms = ((end - start).max(0) / 10_000) as u64;
        let duration_ms = (duration_ms > 0).then_some(duration_ms);
        let playing = status == super::map::STATUS_PLAYING;
        let position_ms =
            super::position::windows_source_position(position - start, updated, windows_now_ticks(), rate_milli, playing, duration_ms);
        let props = session.TryGetMediaPropertiesAsync().and_then(|op| op.join()).ok();
        let text = |f: &dyn Fn(
            &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
        ) -> windows::core::Result<HSTRING>| {
            props.as_ref().and_then(|p| f(p).ok()).map(|h| h.to_string()).unwrap_or_default()
        };
        Some(SessionInfo {
            aumid: aumid.to_string(),
            status,
            controls,
            duration_ms,
            position_ms,
            rate_milli,
            title: text(&|p| p.Title()),
            artist: text(&|p| p.Artist()),
            album: text(&|p| p.AlbumTitle()),
        })
    }

    fn thumbnail(&self, aumid: &str) -> Option<Vec<u8>> {
        let session = self.session(aumid)?;
        let props = session.TryGetMediaPropertiesAsync().and_then(|op| op.join()).ok()?;
        let reference: IRandomAccessStreamReference = props.Thumbnail().ok()?;
        let stream = reference.OpenReadAsync().and_then(|op| op.join()).ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > MAX_THUMBNAIL_BYTES {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        reader.LoadAsync(size as u32).and_then(|op| op.join()).ok()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        Some(bytes)
    }

    fn control(&self, aumid: &str, control: Control) -> Result<bool, String> {
        let session = self.session(aumid).ok_or_else(|| "the session is gone".to_string())?;
        let e = |err: windows::core::Error| err.to_string();
        let op = match control {
            Control::Play => session.TryPlayAsync(),
            Control::Pause => session.TryPauseAsync(),
            Control::Toggle => session.TryTogglePlayPauseAsync(),
            Control::Next => session.TrySkipNextAsync(),
            Control::Prev => session.TrySkipPreviousAsync(),
            Control::SeekTo(ms) => session.TryChangePlaybackPositionAsync(ms as i64 * 10_000),
        };
        op.map_err(e)?.join().map_err(e)
    }
}

fn remove_tokens(sub: &Subscription) {
    let [a, b, c] = sub.tokens;
    let _ = sub.session.RemoveMediaPropertiesChanged(a);
    let _ = sub.session.RemovePlaybackInfoChanged(b);
    let _ = sub.session.RemoveTimelinePropertiesChanged(c);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(aumid: &str) -> SessionInfo {
        SessionInfo {
            aumid: aumid.into(),
            status: 4,
            controls: Controls { play: true, pause: true, toggle: true, next: true, prev: true, position: true },
            duration_ms: Some(1000),
            position_ms: 0,
            rate_milli: None,
            title: "Real title".into(),
            artist: "Real artist".into(),
            album: String::new(),
        }
    }

    const TEST: &str = "app.kinrelay.dev.mediatest-7";
    const OWN: &str = "app.kinrelay.desktop.dev-abc";

    fn guarded(mode: Mode, backend: &Arc<FakeBackend>) -> Guarded {
        Guarded::new(backend.clone(), mode, Some(OWN.to_string()))
    }

    #[test]
    fn a_test_run_asks_the_backend_for_nothing_about_a_real_session() {
        let backend = Arc::new(FakeBackend::default());
        backend.set(vec![
            (info("MSEdge"), Some(vec![1, 2, 3])),
            (info("Spotify.exe"), None),
            (info(TEST), Some(vec![9])),
            (info(OWN), None),
        ]);
        let g = guarded(Mode::Test { aumid: Some(TEST.into()), real_volume: false }, &backend);
        let snap = g.snapshot();
        assert_eq!(snap.iter().map(|s| s.aumid.as_str()).collect::<Vec<_>>(), [TEST]);
        // Even a caller that asks for a real session by name gets nothing, and the backend is not asked.
        for real in ["MSEdge", "Spotify.exe", OWN] {
            assert!(g.read(real).is_none() && g.thumbnail(real).is_none(), "{real}");
            assert!(g.control(real, Control::Next).is_err(), "{real}");
        }
        assert_eq!(g.thumbnail(TEST), Some(vec![9]));
        for call in backend.calls() {
            assert!(!call.contains("MSEdge") && !call.contains("Spotify") && !call.contains(OWN), "the backend was asked: {call}");
        }
        assert!(backend.controls().is_empty(), "no command reached a session");
    }

    #[test]
    fn a_test_run_with_no_session_named_reads_nothing_at_all() {
        let backend = Arc::new(FakeBackend::default());
        backend.set(vec![(info("MSEdge"), None), (info(TEST), None)]);
        let g = guarded(Mode::Test { aumid: None, real_volume: false }, &backend);
        assert!(g.snapshot().is_empty());
        assert!(backend.calls().iter().all(|c| c == "list "), "only the list of ids was asked for: {:?}", backend.calls());
    }

    #[test]
    fn commands_reach_only_the_test_session_in_a_test_run_and_the_installed_kin_skips_its_own() {
        let backend = Arc::new(FakeBackend::default());
        backend.set(vec![(info("MSEdge"), None), (info(TEST), None), (info(OWN), None)]);
        let g = guarded(Mode::Test { aumid: Some(TEST.into()), real_volume: false }, &backend);
        assert_eq!(g.control(TEST, Control::Toggle), Ok(true));
        assert_eq!(g.control(TEST, Control::SeekTo(5000)), Ok(true));
        assert_eq!(backend.controls(), [(TEST.to_string(), Control::Toggle), (TEST.to_string(), Control::SeekTo(5000))]);
        // The installed Kin reads the user's sessions, but never its own.
        let real = guarded(Mode::Real, &backend);
        let seen: Vec<String> = real.snapshot().into_iter().map(|s| s.aumid).collect();
        assert_eq!(seen, ["MSEdge", TEST]);
        assert!(real.read(OWN).is_none());
    }

    #[test]
    fn app_names_come_from_a_table_and_else_from_the_id() {
        assert_eq!(display_name("Spotify.exe"), "Spotify");
        assert_eq!(display_name("MSEdge"), "Edge");
        assert_eq!(display_name("chrome.exe"), "Chrome");
        assert_eq!(display_name("SomePlayer.exe"), "SomePlayer");
        assert_eq!(display_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "SpotifyMusic");
        assert_eq!(display_name("app.kinrelay.dev.mediatest-7"), "app.kinrelay.dev.mediatest-7");
    }

    #[test]
    fn windows_ticks_of_now_are_in_this_century() {
        let t = windows_now_ticks();
        assert!(t > 133_000_000_000_000_000 && t < 140_000_000_000_000_000, "{t}");
    }
}
