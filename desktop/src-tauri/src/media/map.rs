//! The Windows side of the wire: a GSMTC session's status and `Controls` as a wire state and `actions`, and a wire state as what the Windows media flyout
//! (`SystemMediaTransportControls`) shows. Pure. `protocol/media.json` (`windowsMap[]`, `toSmtc[]`) pins it.
use super::wire::{Action, PlayState};

/// GSMTC's `PlaybackStatus`: Closed 0, Opened 1, Changing 2, Stopped 3, Playing 4, Paused 5.
pub const STATUS_CHANGING: i32 = 2;
pub const STATUS_PLAYING: i32 = 4;
pub const STATUS_PAUSED: i32 = 5;

/// The flags of a session's `Controls`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub play: bool,
    pub pause: bool,
    pub toggle: bool,
    pub next: bool,
    pub prev: bool,
    /// `IsPlaybackPositionEnabled`.
    pub position: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapped {
    /// `None`: not a candidate (closed, opened or stopped).
    pub state: Option<PlayState>,
    pub actions: Vec<Action>,
}

/// A session's status and controls as the wire has them. `seek` needs the position control **and** a duration.
pub fn windows_to_wire(status: i32, controls: Controls, duration_ms: u64) -> Mapped {
    let state = match status {
        STATUS_CHANGING => Some(PlayState::Buffering),
        STATUS_PLAYING => Some(PlayState::Playing),
        STATUS_PAUSED => Some(PlayState::Paused),
        _ => None,
    };
    let has = |a: Action| match a {
        Action::Play => controls.play,
        Action::Pause => controls.pause,
        Action::Toggle => controls.toggle,
        Action::Next => controls.next,
        Action::Prev => controls.prev,
        Action::Seek => controls.position && duration_ms > 0,
    };
    Mapped { state, actions: super::wire::ACTIONS.into_iter().filter(|a| has(*a)).collect() }
}

/// `MediaPlaybackStatus` of the flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmtcStatus {
    Changing,
    Stopped,
    Playing,
    Paused,
}

/// What the flyout is told: the status and which of the four buttons are enabled (`seek` is not a button: it comes with the timeline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmtcView {
    pub status: SmtcStatus,
    pub play: bool,
    pub pause: bool,
    pub next: bool,
    pub prev: bool,
}

/// A wire state as the flyout shows it. **In a test run (`test_mode`) it is never `Playing`**: a `Playing` Windows session takes the keyboard's media keys.
/// `toggle` enables play and pause.
pub fn wire_to_smtc(state: PlayState, actions: &[Action], test_mode: bool) -> SmtcView {
    let status = match state {
        PlayState::Playing if test_mode => SmtcStatus::Paused,
        PlayState::Playing => SmtcStatus::Playing,
        PlayState::Paused => SmtcStatus::Paused,
        PlayState::Buffering => SmtcStatus::Changing,
        PlayState::Stopped => SmtcStatus::Stopped,
    };
    let has = |a: Action| actions.contains(&a);
    SmtcView {
        status,
        play: has(Action::Play) || has(Action::Toggle),
        pause: has(Action::Pause) || has(Action::Toggle),
        next: has(Action::Next),
        prev: has(Action::Prev),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_edge_session_maps_the_way_the_probe_saw_it() {
        let c = Controls { play: false, pause: true, toggle: true, next: true, prev: true, position: true };
        let m = windows_to_wire(STATUS_PLAYING, c, 285_500);
        assert_eq!(m.state, Some(PlayState::Playing));
        assert_eq!(m.actions, [Action::Pause, Action::Toggle, Action::Next, Action::Prev, Action::Seek]);
        assert_eq!(
            windows_to_wire(STATUS_PLAYING, c, 0).actions,
            [Action::Pause, Action::Toggle, Action::Next, Action::Prev],
            "no duration, no seek"
        );
        assert!(!windows_to_wire(STATUS_PLAYING, Controls { position: false, ..c }, 5000).actions.contains(&Action::Seek));
    }

    #[test]
    fn only_changing_playing_and_paused_are_candidates() {
        for (status, want) in [
            (0, None),
            (1, None),
            (2, Some(PlayState::Buffering)),
            (3, None),
            (4, Some(PlayState::Playing)),
            (5, Some(PlayState::Paused)),
            (6, None),
        ] {
            assert_eq!(windows_to_wire(status, Controls::default(), 0).state, want, "{status}");
        }
    }

    #[test]
    fn a_test_run_never_shows_playing() {
        for state in [PlayState::Playing, PlayState::Paused, PlayState::Buffering, PlayState::Stopped] {
            assert_ne!(wire_to_smtc(state, &[Action::Pause], true).status, SmtcStatus::Playing);
        }
        assert_eq!(wire_to_smtc(PlayState::Playing, &[], false).status, SmtcStatus::Playing);
        assert_eq!(wire_to_smtc(PlayState::Playing, &[Action::Pause], true).status, SmtcStatus::Paused);
    }

    #[test]
    fn toggle_enables_play_and_pause_and_seek_is_no_button() {
        let v = wire_to_smtc(PlayState::Paused, &[Action::Toggle], false);
        assert!(v.play && v.pause && !v.next && !v.prev);
        let v = wire_to_smtc(PlayState::Paused, &[Action::Seek, Action::Next], false);
        assert!(!v.play && !v.pause && v.next);
    }
}
