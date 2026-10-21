//! Whether the user is busy in a way that a browser window must not interrupt: a fullscreen game or program, a presentation, or Windows' quiet
//! time. `SHQueryUserNotificationState` is the public API for exactly this (the one Windows' own toasts use to hold back).
//!
//! Used for one decision: a link from the phone opens in the browser at once only when this says it is fine (phonelinks.rs). A browser that
//! appears over a fullscreen game steals its focus and can minimize it, and with an anti-cheat running that can cost the user a match.
use std::sync::Mutex;

/// The values of `QUERY_USER_NOTIFICATION_STATE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserState {
    /// The screen saver is on, the session is locked, or the user is switched away.
    NotPresent,
    /// A fullscreen (non-Direct3D) application is running, or a presentation. Windows calls it "busy".
    Busy,
    /// A Direct3D program (a game) runs fullscreen.
    RunningD3dFullScreen,
    /// Presentation settings are on (PowerPoint's slideshow, `presentationsettings.exe`).
    PresentationMode,
    /// Nothing special: notifications are welcome.
    AcceptsNotifications,
    /// Quiet time (Focus assist "priority only" or "alarms only", or the first hour after a Windows feature update).
    QuietTime,
    /// A Windows Store app runs fullscreen.
    RunningWindowsStoreApp,
    /// The call failed, or a value this code doesn't know. Treated as "free": a failure of this check must not take links away.
    Unknown,
}

impl UserState {
    /// Whether opening a window now would steal the focus of what the user is doing (a fullscreen Direct3D program, a fullscreen or busy
    /// application, presentation mode or quiet time) or would open it where nobody is looking (the session is locked, the screen saver is on or
    /// the user is switched away: `NotPresent`). In these states a link is offered in a toast instead of being opened.
    pub fn holds_back_windows(self) -> bool {
        matches!(
            self,
            UserState::NotPresent | UserState::Busy | UserState::RunningD3dFullScreen | UserState::PresentationMode | UserState::QuietTime
        )
    }

    /// For the log and for `status.json`.
    pub fn name(self) -> &'static str {
        match self {
            UserState::NotPresent => "not_present",
            UserState::Busy => "busy",
            UserState::RunningD3dFullScreen => "fullscreen_d3d",
            UserState::PresentationMode => "presentation",
            UserState::AcceptsNotifications => "free",
            UserState::QuietTime => "quiet_time",
            UserState::RunningWindowsStoreApp => "store_app",
            UserState::Unknown => "unknown",
        }
    }
}

/// Asks Windows what the user is doing. Behind a trait so a test can say "a game is running" without one.
pub trait UserStateProbe: Send + Sync {
    fn state(&self) -> UserState;
}

/// The real thing.
pub struct WindowsUserState;

impl UserStateProbe for WindowsUserState {
    fn state(&self) -> UserState {
        use windows::Win32::UI::Shell::{
            QUNS_ACCEPTS_NOTIFICATIONS, QUNS_APP, QUNS_BUSY, QUNS_NOT_PRESENT, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME,
            QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
        };
        // SAFETY: no pointers are passed; the call returns the state or an HRESULT.
        match unsafe { SHQueryUserNotificationState() } {
            Ok(s) if s == QUNS_NOT_PRESENT => UserState::NotPresent,
            Ok(s) if s == QUNS_BUSY => UserState::Busy,
            Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN => UserState::RunningD3dFullScreen,
            Ok(s) if s == QUNS_PRESENTATION_MODE => UserState::PresentationMode,
            Ok(s) if s == QUNS_ACCEPTS_NOTIFICATIONS => UserState::AcceptsNotifications,
            Ok(s) if s == QUNS_QUIET_TIME => UserState::QuietTime,
            Ok(s) if s == QUNS_APP => UserState::RunningWindowsStoreApp,
            Ok(_) | Err(_) => UserState::Unknown,
        }
    }
}

/// For tests: says what it was told to.
pub struct FixedUserState(Mutex<UserState>);

impl FixedUserState {
    pub fn new(state: UserState) -> FixedUserState {
        FixedUserState(Mutex::new(state))
    }

    pub fn set(&self, state: UserState) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = state;
    }
}

impl UserStateProbe for FixedUserState {
    fn state(&self) -> UserState {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_busy_states_and_a_locked_or_away_user_hold_windows_back_and_nothing_else_does() {
        for held in
            [UserState::NotPresent, UserState::Busy, UserState::RunningD3dFullScreen, UserState::PresentationMode, UserState::QuietTime]
        {
            assert!(held.holds_back_windows(), "{held:?}");
        }
        for free in [UserState::AcceptsNotifications, UserState::RunningWindowsStoreApp, UserState::Unknown] {
            assert!(!free.holds_back_windows(), "{free:?}");
        }
    }

    #[test]
    fn the_real_probe_answers_something() {
        // Whatever the machine is doing right now, the call works and gives a state (a CI box has no fullscreen game either way).
        let _ = WindowsUserState.state();
    }
}
