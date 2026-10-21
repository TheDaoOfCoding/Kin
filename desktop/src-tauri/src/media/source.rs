//! What the PC sends of its own media (protocol.md "Which session is shown", "Rates, limits, ordering"): a pure state machine. It is told what the reader saw
//! (`on_sessions`), what the volume is (`on_volume`) and what time it is (`tick`), and answers with the messages to send, as [`Out`]s, and when it wants to be
//! called again (`next_due`). It never touches a clock, a socket or Windows, so every rule is tested with times as numbers.
//!
//! * The session shown is chosen by `pick` (Kin's own session never gets here: the guard keeps it out).
//! * A `media.state` goes out through a trailing 250 ms throttle: the first at once, the last of a burst later, an identical one never. "Identical" leaves the
//!   position out: it moves by itself, so a state is re-sent only when something else changed or the position **jumped** (a seek: more than 1,500 ms off the
//!   extrapolation).
//! * No session for 1.5 s is a `media.clear`, and a session paused for 30 minutes is dropped.
use std::collections::HashMap;

use super::limits::Throttle;
use super::map::windows_to_wire;
use super::pick::{Candidate, ClearDebounce, PAUSED_EXPIRY_MS, pick};
use super::position::{jumped, shown};
use super::reader::{SessionInfo, display_name};
use super::volume::VolumeState;
use super::wire::{self, Action, PlayState, State, Volume, sid_of};
use crate::links::clean_link_text;

/// The throttle interval of states and volumes.
pub const THROTTLE_MS: u64 = 250;

/// A message the engine wants sent.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    State(Box<State>),
    Clear,
    Volume(Volume),
}

/// What the engine remembers of the session it shows, and what a command needs of it.
#[derive(Clone, Debug)]
pub struct Active {
    pub aumid: String,
    pub sid: String,
    pub state: State,
    /// When `state.position_ms` was true (the engine's clock).
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    sid: String,
    app: String,
    state: PlayState,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_ms: Option<u64>,
    rate_milli: u64,
    actions: Vec<Action>,
    art_hash: Option<String>,
    /// Counts up with every seek, so a jump is "something changed".
    seek_epoch: u64,
}

/// A state with the key that says whether it is the same as the last one.
#[derive(Clone, Debug)]
struct Outgoing {
    key: Key,
    state: State,
}

impl PartialEq for Outgoing {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

pub struct Engine {
    current: Option<Active>,
    last_sessions: Vec<SessionInfo>,
    /// The picture of the session the caller last named (`on_sessions`): a state built later (a clear that came due, a pause that expired) keeps it.
    art_hash: Option<String>,
    paused_since: HashMap<String, u64>,
    clear: ClearDebounce,
    throttle: Throttle<Outgoing>,
    volume_throttle: Throttle<VolumeState>,
    volume: Option<VolumeState>,
    seek_epoch: u64,
    present: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

/// The text of a title (or artist, album) as the wire will have it: cleaned and cut.
fn text(raw: &str, max: usize) -> Option<String> {
    clean_link_text(raw, max)
}

/// The wire state for a session at `now_ms`. Strings are cleaned and cut here too (a sender must not exceed the limits), and the frame is cut to fit 16 KiB
/// (`album`, then `artist`, then `title`).
pub fn build_state(info: &SessionInfo, volume: Option<VolumeState>, art_hash: Option<String>) -> Option<State> {
    let mapped = windows_to_wire(info.status, info.controls, info.duration_ms.unwrap_or(0));
    let play = mapped.state?;
    let app = text(&display_name(&info.aumid), wire::MAX_APP).unwrap_or_else(|| "Media".to_string());
    let mut state = State {
        sid: sid_of(&info.aumid),
        app,
        state: play,
        title: text(&info.title, wire::MAX_TITLE),
        artist: text(&info.artist, wire::MAX_ARTIST),
        album: text(&info.album, wire::MAX_ALBUM),
        duration_ms: info.duration_ms.map(|d| d.clamp(1, wire::MAX_DURATION_MS)),
        position_ms: Some(info.position_ms.min(wire::MAX_POSITION_MS)),
        rate_milli: info.rate_milli.unwrap_or(1000).min(wire::MAX_RATE_MILLI),
        actions: mapped.actions,
        art_hash,
        icon_hash: None,
        volume: volume.map(|v| Volume { level: v.level, muted: v.muted }),
    };
    if let (Some(d), Some(p)) = (state.duration_ms, state.position_ms) {
        state.position_ms = Some(p.min(d));
    }
    for cut in [|s: &mut State| s.album = None, |s: &mut State| s.artist = None, |s: &mut State| s.title = None] {
        if state.to_frame("0193a5d0-7b1e-7c3a-9d4f-5e6a7b8c0000").to_string().len() <= wire::MAX_STATE_SEND_BYTES {
            break;
        }
        cut(&mut state);
    }
    Some(state)
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            current: None,
            last_sessions: Vec::new(),
            art_hash: None,
            paused_since: HashMap::new(),
            clear: ClearDebounce::default(),
            throttle: Throttle::new(THROTTLE_MS),
            volume_throttle: Throttle::new(THROTTLE_MS),
            volume: None,
            seek_epoch: 0,
            present: false,
        }
    }

    /// The session being shown, if any.
    pub fn active(&self) -> Option<&Active> {
        self.current.as_ref()
    }

    /// Candidates in the platform's order, with how long each has been paused.
    fn candidates(&self, now_ms: u64, sessions: &[SessionInfo]) -> Vec<Candidate> {
        sessions
            .iter()
            .map(|s| {
                let mapped = windows_to_wire(s.status, s.controls, s.duration_ms.unwrap_or(0));
                Candidate {
                    sid: sid_of(&s.aumid),
                    own: false,
                    has_title: text(&s.title, wire::MAX_TITLE).is_some(),
                    state: mapped.state.unwrap_or(PlayState::Stopped),
                    paused_ms: (mapped.state == Some(PlayState::Paused))
                        .then(|| now_ms.saturating_sub(self.paused_since.get(&s.aumid).copied().unwrap_or(now_ms))),
                }
            })
            .collect()
    }

    /// Which session would be shown for these sessions, and the picture key of it: `(aumid, title and artist)`. The caller reads the thumbnail only when this key
    /// differs from the last one it read for.
    pub fn preview(&self, now_ms: u64, sessions: &[SessionInfo]) -> Option<(String, String)> {
        let cands = self.candidates(now_ms, sessions);
        let current = self.current.as_ref().map(|c| c.sid.as_str());
        let chosen = pick(&cands, current)?;
        let info = sessions.iter().find(|s| sid_of(&s.aumid) == chosen.sid)?;
        Some((info.aumid.clone(), format!("{}\u{1}{}\u{1}{}", info.title, info.artist, info.album)))
    }

    /// The reader saw these sessions (in priority order). `art_hash` is the picture of the session `preview` named (`None`: no picture).
    pub fn on_sessions(&mut self, now_ms: u64, sessions: Vec<SessionInfo>, art_hash: Option<String>) -> Vec<Out> {
        let seen: Vec<&str> = sessions.iter().map(|s| s.aumid.as_str()).collect();
        self.paused_since.retain(|k, _| seen.contains(&k.as_str()));
        for s in &sessions {
            if windows_to_wire(s.status, s.controls, 0).state == Some(PlayState::Paused) {
                self.paused_since.entry(s.aumid.clone()).or_insert(now_ms);
            } else {
                self.paused_since.remove(&s.aumid);
            }
        }
        // Exactly what the caller says: a track that has no picture does not keep the cover of the one before it.
        self.art_hash = art_hash;
        self.last_sessions = sessions;
        self.evaluate(now_ms)
    }

    /// Looks at the last sessions again (a pause that reached 30 minutes, a clear that came due).
    fn evaluate(&mut self, now_ms: u64) -> Vec<Out> {
        let cands = self.candidates(now_ms, &self.last_sessions);
        let current = self.current.as_ref().map(|c| c.sid.clone());
        let chosen_sid = pick(&cands, current.as_deref()).map(|c| c.sid.clone());
        let mut out = Vec::new();
        self.present = chosen_sid.is_some();
        if let Some(at) = self.clear.observe(now_ms, self.present) {
            let _ = at;
            self.current = None;
            self.throttle.reset();
            out.push(Out::Clear);
        }
        let Some(sid) = chosen_sid else {
            return out;
        };
        let Some(info) = self.last_sessions.iter().find(|s| sid_of(&s.aumid) == sid).cloned() else {
            return out;
        };
        let Some(state) = build_state(&info, self.volume, self.art_hash.clone()) else {
            return out;
        };
        // A seek: the position is not where the last observation said it would be by now.
        if let Some(prev) = self.current.as_ref().filter(|c| c.sid == sid)
            && let Some(was) = prev.state.position_ms
        {
            let expected =
                shown(prev.state.state, Some(was), Some(prev.state.rate_milli), prev.state.duration_ms, now_ms.saturating_sub(prev.at_ms));
            if let (Some(e), Some(p)) = (expected.position_ms, state.position_ms)
                && jumped(e, p)
            {
                self.seek_epoch += 1;
            }
        }
        let key = Key {
            sid: state.sid.clone(),
            app: state.app.clone(),
            state: state.state,
            title: state.title.clone(),
            artist: state.artist.clone(),
            album: state.album.clone(),
            duration_ms: state.duration_ms,
            rate_milli: state.rate_milli,
            actions: state.actions.clone(),
            art_hash: state.art_hash.clone(),
            seek_epoch: self.seek_epoch,
        };
        self.current = Some(Active { aumid: info.aumid, sid, state: state.clone(), at_ms: now_ms });
        self.clear.shown_now();
        out.extend(self.throttle.push(now_ms, Outgoing { key, state }).into_iter().map(|(_, o)| Out::State(Box::new(o.state))));
        out
    }

    /// The master volume is now this (a change by the user, or by a command). Goes out through its own trailing throttle.
    pub fn on_volume(&mut self, now_ms: u64, volume: VolumeState) -> Vec<Out> {
        self.volume = Some(volume);
        if let Some(active) = self.current.as_mut() {
            active.state.volume = Some(Volume { level: volume.level, muted: volume.muted });
        }
        self.volume_throttle.push(now_ms, volume).into_iter().map(|(_, v)| Out::Volume(Volume { level: v.level, muted: v.muted })).collect()
    }

    /// Time passed: what a throttle held goes out, a clear that came due is sent, a pause that reached 30 minutes is dropped.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Out> {
        let mut out: Vec<Out> = Vec::new();
        if let Some((_, o)) = self.throttle.flush(now_ms) {
            out.push(Out::State(Box::new(o.state)));
        }
        if let Some((_, v)) = self.volume_throttle.flush(now_ms) {
            out.push(Out::Volume(Volume { level: v.level, muted: v.muted }));
        }
        if self.clear.next_due().is_some_and(|d| d <= now_ms) || self.expiry_due().is_some_and(|d| d <= now_ms) {
            out.extend(self.evaluate(now_ms));
        }
        out
    }

    /// When a paused active session has been paused for 30 minutes.
    fn expiry_due(&self) -> Option<u64> {
        let active = self.current.as_ref()?;
        if active.state.state != PlayState::Paused {
            return None;
        }
        self.paused_since.get(&active.aumid).map(|since| since + PAUSED_EXPIRY_MS)
    }

    /// When the engine wants `tick` to be called (the earliest of what it is waiting for).
    pub fn next_due(&self) -> Option<u64> {
        [self.throttle.next_due(), self.volume_throttle.next_due(), self.clear.next_due(), self.expiry_due()].into_iter().flatten().min()
    }

    /// The session as a phone that just connected should see it, with the position worked out for `now_ms`. `None` when nothing is shown.
    pub fn current_wire(&self, now_ms: u64) -> Option<State> {
        let active = self.current.as_ref()?;
        let s = &active.state;
        let at = shown(s.state, s.position_ms, Some(s.rate_milli), s.duration_ms, now_ms.saturating_sub(active.at_ms));
        let mut state = s.clone();
        state.position_ms = at.position_ms;
        state.state = at.state;
        state.volume = self.volume.map(|v| Volume { level: v.level, muted: v.muted });
        Some(state)
    }

    /// The current volume the engine knows (`None` before it was told).
    pub fn volume(&self) -> Option<Volume> {
        self.volume.map(|v| Volume { level: v.level, muted: v.muted })
    }

    /// The viewer has nothing now (the peer went away): the next state goes out at once, and a vanished session need not be cleared there.
    pub fn viewer_reset(&mut self) {
        self.throttle.reset();
        self.volume_throttle.reset();
        self.clear.nothing_shown();
        if self.current.is_some() {
            self.clear.shown_now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::map::Controls;

    fn controls() -> Controls {
        Controls { play: true, pause: true, toggle: true, next: true, prev: true, position: true }
    }

    fn session(aumid: &str, status: i32, title: &str, position_ms: u64) -> SessionInfo {
        SessionInfo {
            aumid: aumid.into(),
            status,
            controls: controls(),
            duration_ms: Some(200_000),
            position_ms,
            rate_milli: Some(1000),
            title: title.into(),
            artist: "Artist".into(),
            album: String::new(),
        }
    }

    const PLAYING: i32 = 4;
    const PAUSED: i32 = 5;

    fn states(out: &[Out]) -> Vec<&State> {
        out.iter().filter_map(|o| if let Out::State(s) = o { Some(s.as_ref()) } else { None }).collect()
    }

    #[test]
    fn the_first_state_goes_at_once_with_everything_the_wire_has() {
        let mut e = Engine::new();
        let out = e.on_sessions(0, vec![session("Spotify.exe", PLAYING, "Song", 5000)], Some("a".repeat(32)));
        let s = states(&out)[0];
        assert_eq!((s.app.as_str(), s.title.as_deref(), s.state, s.position_ms), ("Spotify", Some("Song"), PlayState::Playing, Some(5000)));
        assert_eq!(s.sid, sid_of("Spotify.exe"));
        assert_eq!(s.actions, [Action::Play, Action::Pause, Action::Toggle, Action::Next, Action::Prev, Action::Seek]);
        assert_eq!(s.art_hash.as_deref(), Some("a".repeat(32).as_str()));
        assert_eq!(e.active().map(|a| a.aumid.as_str()), Some("Spotify.exe"));
    }

    #[test]
    fn the_position_moving_by_itself_sends_nothing_but_a_seek_does() {
        let mut e = Engine::new();
        e.on_sessions(0, vec![session("A", PLAYING, "Song", 0)], None);
        // 10 s later the source says 10.2 s: the extrapolation explains it.
        assert!(states(&e.on_sessions(10_000, vec![session("A", PLAYING, "Song", 10_200)], None)).is_empty());
        assert!(e.tick(10_300).is_empty());
        // 20 s later it says 60 s: a seek.
        let out = e.on_sessions(20_000, vec![session("A", PLAYING, "Song", 60_000)], None);
        assert_eq!(states(&out).len(), 1);
        assert_eq!(states(&out)[0].position_ms, Some(60_000));
    }

    #[test]
    fn a_burst_of_changes_sends_the_first_and_the_last_250_ms_later() {
        let mut e = Engine::new();
        assert_eq!(states(&e.on_sessions(0, vec![session("A", PLAYING, "One", 0)], None)).len(), 1);
        assert!(states(&e.on_sessions(50, vec![session("A", PLAYING, "Two", 0)], None)).is_empty());
        assert!(states(&e.on_sessions(100, vec![session("A", PLAYING, "Three", 0)], None)).is_empty());
        assert_eq!(e.next_due(), Some(250));
        assert!(e.tick(249).is_empty());
        let out = e.tick(250);
        assert_eq!(states(&out)[0].title.as_deref(), Some("Three"));
        assert_eq!(e.next_due(), None);
    }

    #[test]
    fn a_state_that_is_the_same_is_not_sent_again_and_pause_is_a_change() {
        let mut e = Engine::new();
        e.on_sessions(0, vec![session("A", PLAYING, "Song", 0)], None);
        assert!(states(&e.on_sessions(1000, vec![session("A", PLAYING, "Song", 1000)], None)).is_empty());
        let out = e.on_sessions(2000, vec![session("A", PAUSED, "Song", 2000)], None);
        assert_eq!(states(&out)[0].state, PlayState::Paused);
    }

    #[test]
    fn a_session_that_disappears_is_cleared_after_1500_ms_and_not_if_it_comes_back() {
        let mut e = Engine::new();
        e.on_sessions(0, vec![session("A", PLAYING, "Song", 0)], None);
        assert!(e.on_sessions(1000, vec![], None).is_empty());
        assert_eq!(e.next_due(), Some(2500));
        assert_eq!(e.tick(2500), [Out::Clear]);
        assert!(e.active().is_none());
        // The next session is sent at once (the viewer has nothing).
        assert_eq!(states(&e.on_sessions(3000, vec![session("B", PLAYING, "Next", 0)], None)).len(), 1);
        // A flicker of 200 ms clears nothing.
        e.on_sessions(4000, vec![], None);
        let out = e.on_sessions(4200, vec![session("B", PLAYING, "Next", 200)], None);
        assert!(states(&out).is_empty() && !out.contains(&Out::Clear));
        assert!(e.tick(9000).is_empty());
    }

    #[test]
    fn two_playing_sessions_do_not_flip_flop_and_a_playing_one_beats_a_paused_one() {
        let mut e = Engine::new();
        e.on_sessions(0, vec![session("A", PLAYING, "a", 0), session("B", PLAYING, "b", 0)], None);
        assert_eq!(e.active().unwrap().aumid, "A");
        e.on_sessions(1000, vec![session("B", PLAYING, "b", 1000), session("A", PLAYING, "a", 1000)], None);
        assert_eq!(e.active().unwrap().aumid, "A", "the current playing session stays");
        e.on_sessions(2000, vec![session("A", PAUSED, "a", 2000), session("B", PLAYING, "b", 2000)], None);
        assert_eq!(e.active().unwrap().aumid, "B");
    }

    #[test]
    fn a_session_paused_for_30_minutes_is_dropped_and_comes_back_when_it_plays() {
        let mut e = Engine::new();
        e.on_sessions(0, vec![session("A", PAUSED, "Song", 0)], None);
        assert!(e.active().is_some());
        let due = e.next_due().expect("the expiry is armed");
        assert_eq!(due, 30 * 60 * 1000);
        assert!(e.tick(due - 1).is_empty());
        assert!(e.tick(due).is_empty(), "no candidate now: the clear waits 1.5 s");
        assert_eq!(e.tick(due + 1500), [Out::Clear]);
        let out = e.on_sessions(due + 2000, vec![session("A", PLAYING, "Song", 0)], None);
        assert_eq!(states(&out).len(), 1, "sent again when it plays");
    }

    #[test]
    fn a_session_with_no_title_or_not_playing_is_not_shown() {
        let mut e = Engine::new();
        assert!(e.on_sessions(0, vec![session("A", PLAYING, "  ", 0)], None).is_empty());
        assert!(e.active().is_none());
        assert!(e.on_sessions(10, vec![session("A", 3, "Song", 0)], None).is_empty(), "stopped");
        assert!(e.preview(10, &[session("A", 3, "Song", 0)]).is_none());
    }

    #[test]
    fn the_volume_goes_out_through_its_own_trailing_throttle() {
        let mut e = Engine::new();
        let v = |level| VolumeState { level, muted: false };
        assert_eq!(e.on_volume(0, v(10)), [Out::Volume(Volume { level: 10, muted: false })]);
        assert!(e.on_volume(100, v(20)).is_empty() && e.on_volume(200, v(30)).is_empty());
        assert_eq!(e.tick(250), [Out::Volume(Volume { level: 30, muted: false })]);
        assert!(e.on_volume(1000, v(30)).is_empty(), "the same level is not sent");
        // A state built after that carries the volume.
        let out = e.on_sessions(2000, vec![session("A", PLAYING, "Song", 0)], None);
        assert_eq!(states(&out)[0].volume, Some(Volume { level: 30, muted: false }));
    }

    #[test]
    fn a_phone_that_just_connected_gets_the_session_at_the_position_it_has_now() {
        let mut e = Engine::new();
        assert!(e.current_wire(0).is_none());
        e.on_sessions(1000, vec![session("A", PLAYING, "Song", 5000)], None);
        let s = e.current_wire(11_000).unwrap();
        assert_eq!((s.position_ms, s.state), (Some(15_000), PlayState::Playing));
        e.viewer_reset();
        // After a reset the next state goes at once, even though it is the same.
        let out = e.on_sessions(12_000, vec![session("A", PLAYING, "Song", 16_000)], None);
        assert_eq!(states(&out).len(), 1);
    }

    #[test]
    fn a_title_in_markup_or_with_control_characters_is_cleaned_and_cut_by_the_sender_too() {
        let long = "x".repeat(400);
        let mut info = session("A", PLAYING, "Line\none\u{202e}", 0);
        info.artist = long.clone();
        let s = build_state(&info, None, None).unwrap();
        assert_eq!(s.title.as_deref(), Some("Line one_"));
        assert_eq!(s.artist.as_ref().map(|a| a.chars().count()), Some(100));
        assert!(s.to_frame("id").to_string().len() <= wire::MAX_STATE_SEND_BYTES);
    }

    #[test]
    fn a_new_track_without_a_picture_does_not_keep_the_cover_of_the_one_before() {
        let mut e = Engine::new();
        let first = e.on_sessions(0, vec![session("A", PLAYING, "One", 0)], Some("a".repeat(32)));
        assert_eq!(states(&first)[0].art_hash.as_deref(), Some("a".repeat(32).as_str()));
        let second = e.on_sessions(100, vec![session("A", PLAYING, "Two", 0)], None);
        assert!(states(&second).is_empty(), "held by the throttle");
        let out = e.tick(250);
        assert_eq!(states(&out)[0].art_hash, None, "the track changed and has no picture");
        assert_eq!(states(&out)[0].title.as_deref(), Some("Two"));
    }

    #[test]
    fn a_state_built_by_a_tick_keeps_the_picture_the_caller_named() {
        let mut e = Engine::new();
        e.on_sessions(0, vec![session("A", PAUSED, "Song", 0)], Some("b".repeat(32)));
        // The pause reaches 30 minutes: no candidate, the clear comes 1.5 s later; before that a tick re-evaluates, and a playing session coming back keeps it.
        let due = e.next_due().expect("the expiry");
        assert!(e.tick(due).is_empty());
        let out = e.on_sessions(due + 10, vec![session("A", PLAYING, "Song", 0)], Some("b".repeat(32)));
        assert_eq!(states(&out)[0].art_hash.as_deref(), Some("b".repeat(32).as_str()));
    }

    #[test]
    fn the_preview_names_the_session_and_a_key_that_changes_with_the_track() {
        let e = Engine::new();
        let (aumid, key1) = e.preview(0, &[session("A", PLAYING, "One", 0)]).unwrap();
        let (_, key2) = e.preview(0, &[session("A", PLAYING, "Two", 0)]).unwrap();
        assert_eq!(aumid, "A");
        assert_ne!(key1, key2);
    }
}
