//! Which session is shown (protocol.md "Which session is shown"): the source decides, with a fixed rule, and a vanished session is cleared only after 1.5 s (a
//! track change often destroys and recreates a session). Pure. `protocol/media.json` (`pick[]`, `clearDebounce[]`) pins it.
use super::wire::PlayState;

/// A session that has been paused this long is dropped (and sent again when it plays).
pub const PAUSED_EXPIRY_MS: u64 = 30 * 60 * 1000;
/// A clear goes out this long after the last candidate disappeared.
pub const CLEAR_DEBOUNCE_MS: u64 = 1500;

/// What the choice needs to know about a session. The caller lists them in the platform's priority order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub sid: String,
    /// Kin's own session: never read back (it would echo).
    pub own: bool,
    pub has_title: bool,
    pub state: PlayState,
    /// How long it has been paused, when it is.
    pub paused_ms: Option<u64>,
}

impl Candidate {
    fn eligible(&self) -> bool {
        !self.own
            && self.has_title
            && matches!(self.state, PlayState::Playing | PlayState::Paused | PlayState::Buffering)
            && !(self.state == PlayState::Paused && self.paused_ms.unwrap_or(0) >= PAUSED_EXPIRY_MS)
    }
}

/// The session to show: the current one if it is still a candidate and playing; else the first candidate that is playing; else the current one if it is still a
/// candidate; else the first candidate; else none (a clear).
pub fn pick<'a>(sessions: &'a [Candidate], current: Option<&str>) -> Option<&'a Candidate> {
    let mut candidates = sessions.iter().filter(|s| s.eligible());
    let cur = current.and_then(|c| sessions.iter().find(|s| s.eligible() && s.sid == c));
    if let Some(c) = cur
        && c.state == PlayState::Playing
    {
        return Some(c);
    }
    if let Some(p) = sessions.iter().find(|s| s.eligible() && s.state == PlayState::Playing) {
        return Some(p);
    }
    cur.or_else(|| candidates.next())
}

/// The debounce of a clear. `observe` is called with every look at the sessions (`present`: some candidate exists); it says when a clear is due.
#[derive(Debug)]
pub struct ClearDebounce {
    pending_since: Option<u64>,
    shown: bool,
}

impl Default for ClearDebounce {
    fn default() -> Self {
        // A session is on the viewer when the run starts.
        ClearDebounce { pending_since: None, shown: true }
    }
}

impl ClearDebounce {
    /// Something is on the viewer now (a state was sent): a later disappearance has to be cleared.
    pub fn shown_now(&mut self) {
        self.shown = true;
        self.pending_since = None;
    }

    /// Nothing is on the viewer (a clear went out, or the peer went away).
    pub fn nothing_shown(&mut self) {
        self.shown = false;
        self.pending_since = None;
    }

    /// A look at the sessions at `now_ms`. Returns the time at which a clear was due, when it is due by now (the caller sends it, and `nothing_shown` is already
    /// noted).
    pub fn observe(&mut self, now_ms: u64, present: bool) -> Option<u64> {
        let mut due = None;
        if let Some(since) = self.pending_since
            && now_ms.saturating_sub(since) >= CLEAR_DEBOUNCE_MS
        {
            due = Some(since + CLEAR_DEBOUNCE_MS);
            self.pending_since = None;
            self.shown = false;
        }
        if present {
            self.pending_since = None;
            self.shown = true;
        } else if self.shown && self.pending_since.is_none() {
            self.pending_since = Some(now_ms);
        }
        due
    }

    /// When the timer has to wake up to call `observe` again.
    pub fn next_due(&self) -> Option<u64> {
        self.pending_since.map(|s| s + CLEAR_DEBOUNCE_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(sid: &str, state: PlayState) -> Candidate {
        Candidate { sid: sid.into(), own: false, has_title: true, state, paused_ms: None }
    }

    fn chosen(sessions: &[Candidate], current: Option<&str>) -> Option<String> {
        pick(sessions, current).map(|c| c.sid.clone())
    }

    #[test]
    fn the_current_playing_session_stays_and_a_playing_one_beats_a_paused_current() {
        let two = [s("a", PlayState::Playing), s("b", PlayState::Playing)];
        assert_eq!(chosen(&two, Some("b")).as_deref(), Some("b"));
        assert_eq!(chosen(&two, None).as_deref(), Some("a"));
        let mixed = [s("a", PlayState::Paused), s("b", PlayState::Playing)];
        assert_eq!(chosen(&mixed, Some("a")).as_deref(), Some("b"));
        let buffering = [s("a", PlayState::Buffering), s("b", PlayState::Playing)];
        assert_eq!(chosen(&buffering, Some("a")).as_deref(), Some("b"));
        let paused = [s("a", PlayState::Paused), s("b", PlayState::Paused)];
        assert_eq!(chosen(&paused, Some("b")).as_deref(), Some("b"), "nothing plays: the current one stays");
        assert_eq!(chosen(&paused, None).as_deref(), Some("a"));
        assert_eq!(chosen(&paused, Some("zzz")).as_deref(), Some("a"), "the current one is gone");
    }

    #[test]
    fn kins_own_session_a_missing_title_stopped_and_expired_ones_are_never_chosen() {
        let own = Candidate { own: true, ..s("me", PlayState::Playing) };
        assert_eq!(chosen(&[own.clone(), s("a", PlayState::Paused)], None).as_deref(), Some("a"));
        assert_eq!(chosen(std::slice::from_ref(&own), None), None);
        assert_eq!(chosen(&[own, s("a", PlayState::Playing)], Some("me")).as_deref(), Some("a"));
        let untitled = Candidate { has_title: false, ..s("a", PlayState::Playing) };
        assert_eq!(chosen(&[untitled, s("b", PlayState::Paused)], None).as_deref(), Some("b"));
        assert_eq!(chosen(&[s("a", PlayState::Stopped)], Some("a")), None);
        let paused = |ms| Candidate { paused_ms: Some(ms), ..s("a", PlayState::Paused) };
        assert_eq!(chosen(&[paused(1_799_999)], Some("a")).as_deref(), Some("a"));
        assert_eq!(chosen(&[paused(1_800_000)], Some("a")), None);
        assert_eq!(
            chosen(&[paused(1_800_000), Candidate { paused_ms: Some(10), ..s("b", PlayState::Paused) }], Some("a")).as_deref(),
            Some("b")
        );
    }

    fn clears(events: &[(u64, bool)]) -> Vec<u64> {
        let mut d = ClearDebounce::default();
        events.iter().filter_map(|(t, present)| d.observe(*t, *present)).collect()
    }

    #[test]
    fn a_clear_goes_out_1500_ms_after_the_last_session_went_and_not_if_it_came_back() {
        assert_eq!(clears(&[(0, false), (1600, false)]), [1500]);
        assert_eq!(clears(&[(0, false), (1500, false)]), [1500]);
        assert!(clears(&[(0, false), (1499, true), (9000, true)]).is_empty());
        assert!(clears(&[(0, false), (200, true), (300, false), (500, true), (4000, true)]).is_empty(), "a track change flicker");
        assert_eq!(clears(&[(0, false), (1500, false), (2000, true), (3000, false), (5000, false)]), [1500, 4500]);
        assert_eq!(clears(&[(0, false), (2000, false), (9000, false)]), [1500], "once");
        let mut d = ClearDebounce::default();
        d.observe(100, false);
        assert_eq!(d.next_due(), Some(1600));
    }
}
