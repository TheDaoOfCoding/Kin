//! What "the PC is in Focus" means (protocol.md "Do Not Disturb, Focus and alerts", design 3.5) and the 3 s filter that keeps a flapping value off the phone.
//! Pure: no Windows, no clock of its own (the time is an argument), so `protocol/alerts.json` (`focusState[]`, `focusTimeline[]`, `focusSnapshot[]`) drives it.
//!
//! `focus = focusSession || quietTime || (alsoPresenting && presentingOrFullscreen)`:
//! * `focusSession`: a Windows Focus session (`FocusSessionManager.IsFocusActive`, read-only).
//! * `quietTime`: `SHQueryUserNotificationState == QUIET_TIME` (Windows "Do Not Disturb"), read-only.
//! * `presentingOrFullscreen`: `BUSY`, `RUNNING_D3D_FULL_SCREEN` or `PRESENTATION_MODE`, counted only with the checkbox "Also when I'm presenting or playing full
//!   screen" (off by default).
//!
//! A locked PC (`NOT_PRESENT`) is not Focus **by itself**: it counts neither as quiet time nor as presenting. A Focus *session* still counts while the PC is locked (a Windows
//! setting that outlives locking). Nothing here reads or changes a Windows setting.
use crate::quiet::UserState;

/// A change of the value is sent after it has stayed the same for this long.
pub const STABLE_MS: i64 = 3_000;
/// The poll of the quiet state, while the feature is on and a phone that lists `presence` is connected (there is no event for `SHQueryUserNotificationState`).
pub const POLL_MS: i64 = 15_000;

/// Where the two Windows facts come from. Behind a trait so the tests (and `--dev-focus on|off`) never touch the user's real settings.
pub trait QuietSource: Send + Sync {
    /// Whether a Windows Focus session is active. `false` where the API doesn't exist (Windows 10).
    fn focus_session(&self) -> bool;
    /// `SHQueryUserNotificationState`.
    fn quiet_state(&self) -> UserState;
    /// Whether `FocusSessionManager` exists here (Windows 11 22H2 or later). The Settings row says "Windows 10 only reports Do Not Disturb" when it doesn't.
    fn focus_sessions_supported(&self) -> bool {
        true
    }
}

/// The rule of 3.5 over what Windows says now.
pub fn is_focus(focus_session: bool, state: UserState, also_presenting: bool) -> bool {
    focus_session
        || state == UserState::QuietTime
        || (also_presenting && matches!(state, UserState::Busy | UserState::RunningD3dFullScreen | UserState::PresentationMode))
}

/// A short word for the log ("focus: on (focus session)"): which part made it. Words only, never a history.
pub fn reason_word(focus_session: bool, state: UserState, also_presenting: bool) -> &'static str {
    if focus_session {
        "focus session"
    } else if state == UserState::QuietTime {
        "quiet time"
    } else if also_presenting && is_focus(false, state, true) {
        "presenting or full screen"
    } else {
        "off"
    }
}

/// The 3 s stability filter. The stable value starts `false`; a value that differs from it is reported after it has stayed the same for [`STABLE_MS`], counted from the first
/// sample that gave it. A flip back before that cancels it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StableFocus {
    stable: bool,
    candidate: bool,
    since: i64,
}

impl StableFocus {
    pub fn new() -> StableFocus {
        StableFocus::default()
    }

    /// The value that was last reported (what goes into a snapshot).
    pub fn stable(&self) -> bool {
        self.stable
    }

    /// When the pending change becomes stable, if there is one.
    pub fn next_due(&self) -> Option<i64> {
        (self.candidate != self.stable).then_some(self.since + STABLE_MS)
    }

    /// Reports the pending change if it has been stable for 3 s at `now`: `Some((when, value))`, `when` being the moment the 3 s ended.
    pub fn advance(&mut self, now: i64) -> Option<(i64, bool)> {
        let due = self.next_due().filter(|due| *due <= now)?;
        self.stable = self.candidate;
        Some((due, self.stable))
    }

    /// One reading (an event, a poll tick, a phone connecting). Anything that was already due is reported first.
    pub fn feed(&mut self, now: i64, raw: bool) -> Option<(i64, bool)> {
        let reported = self.advance(now);
        if raw != self.candidate {
            self.candidate = raw;
            self.since = now;
        }
        reported
    }

    /// The first reading after nothing was being read (a phone connects while the feature is on): there is no flapping history to filter, so it is the stable value at once.
    /// The phone's first snapshot of a connection then already tells the truth (a reconnect after a Wi-Fi blip must not look like the PC left Focus and came back).
    pub fn seed(&mut self, raw: bool) {
        *self = StableFocus { stable: raw, candidate: raw, since: 0 };
    }

    /// Back to "not in Focus" with nothing pending: the feature was switched off, or no phone is connected, so nothing is read any more.
    pub fn reset(&mut self) {
        *self = StableFocus::default();
    }
}

/// Does `focus: true` go into the PC's snapshot? Only when the share setting is on, the stable value is true, the PC is not paused (a paused PC asks not to be acted on) and
/// the phone's `hello` lists `presence`.
pub fn snapshot_focus(share_on: bool, stable: bool, paused: bool, peer_has_presence: bool) -> bool {
    share_on && stable && !paused && peer_has_presence
}

/// A source that says what it was told, for tests and `--dev-focus on|off`.
pub struct FakeQuiet {
    inner: std::sync::Mutex<(bool, UserState)>,
    supported: bool,
}

impl FakeQuiet {
    pub fn new(focus_session: bool, state: UserState) -> FakeQuiet {
        FakeQuiet { inner: std::sync::Mutex::new((focus_session, state)), supported: true }
    }

    /// A source of a Windows without `FocusSessionManager`.
    pub fn without_focus_sessions(state: UserState) -> FakeQuiet {
        FakeQuiet { inner: std::sync::Mutex::new((false, state)), supported: false }
    }

    pub fn set(&self, focus_session: bool, state: UserState) {
        *self.inner.lock().unwrap_or_else(|p| p.into_inner()) = (focus_session, state);
    }
}

impl QuietSource for FakeQuiet {
    fn focus_session(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).0
    }

    fn quiet_state(&self) -> UserState {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).1
    }

    fn focus_sessions_supported(&self) -> bool {
        self.supported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule() {
        assert!(!is_focus(false, UserState::AcceptsNotifications, false));
        assert!(is_focus(true, UserState::AcceptsNotifications, false));
        assert!(is_focus(false, UserState::QuietTime, false));
        assert!(!is_focus(false, UserState::Busy, false), "presenting counts only with the checkbox");
        assert!(is_focus(false, UserState::Busy, true));
        assert!(!is_focus(false, UserState::NotPresent, true), "a locked PC is not Focus by itself");
        assert!(is_focus(true, UserState::NotPresent, false), "a Focus session outlives locking");
        assert!(!is_focus(false, UserState::RunningWindowsStoreApp, true));
        assert!(!is_focus(false, UserState::Unknown, true));
    }

    #[test]
    fn a_change_is_reported_after_3_seconds_and_a_flip_back_cancels_it() {
        let mut f = StableFocus::new();
        assert_eq!(f.feed(0, true), None);
        assert_eq!(f.next_due(), Some(3_000));
        assert_eq!(f.advance(2_999), None);
        assert_eq!(f.advance(3_000), Some((3_000, true)));
        assert!(f.stable());
        // a flicker of 1 s while on is nothing
        assert_eq!(f.feed(10_000, false), None);
        assert_eq!(f.feed(11_000, true), None);
        assert_eq!(f.next_due(), None);
        // off for good: reported 3 s after the first sample that said off
        assert_eq!(f.feed(20_000, false), None);
        assert_eq!(
            f.feed(35_000, false),
            Some((23_000, false)),
            "the poll that finds it already due reports it at the 3 s mark, not at the poll"
        );
        assert!(!f.stable());
    }

    #[test]
    fn a_seed_is_stable_at_once_and_leaves_nothing_pending() {
        let mut f = StableFocus::new();
        f.seed(true);
        assert!(f.stable());
        assert_eq!(f.next_due(), None);
        assert_eq!(f.feed(1_000, true), None, "the same reading changes nothing");
        assert_eq!(f.feed(2_000, false), None, "a change after the seed is filtered like any other");
        assert_eq!(f.advance(5_000), Some((5_000, false)));
    }

    #[test]
    fn reset_forgets_everything() {
        let mut f = StableFocus::new();
        f.feed(0, true);
        f.advance(3_000);
        assert!(f.stable());
        f.reset();
        assert!(!f.stable());
        assert_eq!(f.next_due(), None);
    }

    #[test]
    fn a_snapshot_carries_focus_only_when_every_condition_holds() {
        assert!(snapshot_focus(true, true, false, true));
        assert!(!snapshot_focus(false, true, false, true), "switch off");
        assert!(!snapshot_focus(true, false, false, true));
        assert!(!snapshot_focus(true, true, true, true), "a paused PC sends none");
        assert!(!snapshot_focus(true, true, false, false), "a phone without presence");
    }

    #[test]
    fn the_reason_words_name_the_part_that_made_it() {
        assert_eq!(reason_word(true, UserState::QuietTime, false), "focus session");
        assert_eq!(reason_word(false, UserState::QuietTime, false), "quiet time");
        assert_eq!(reason_word(false, UserState::Busy, true), "presenting or full screen");
        assert_eq!(reason_word(false, UserState::Busy, false), "off");
    }
}
