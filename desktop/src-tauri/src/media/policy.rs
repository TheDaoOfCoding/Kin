//! Who may send what, and what a source answers to a command (protocol.md "Roles, capability and status" and "Rates, limits, ordering"). Pure: the core asks these
//! functions and acts on the answer, so the rules are the same ones `protocol/media.json` (`sendRule[]`, `viewerRule[]`, `cmdResult[]`) pins.
use super::wire::{Action, Cmd, CmdKind};

/// The kinds of message the rules talk about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    State,
    Image,
    Volume,
    Clear,
    Cmd,
    Status,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "state" => Kind::State,
            "image" => Kind::Image,
            "volume" => Kind::Volume,
            "clear" => Kind::Clear,
            "cmd" => Kind::Cmd,
            "status" => Kind::Status,
            _ => return None,
        })
    }
}

/// What a peer's latest `media.status` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerStatus {
    pub show: bool,
    pub share: bool,
}

/// May this side send a message of `kind` to the peer now?
///
/// * A **source** sends `state`, `image`, `volume` and `clear` only if its own `share` is on, the peer's latest `hello` lists `media` and the peer's latest status says
///   `show: true` (before any status arrived: nothing). The one exception is the single `clear` a source sends when its `share` has just been turned off, which needs
///   the peer to show.
/// * A **viewer** sends a `cmd` only to a peer whose latest `hello` lists `media` and whose latest status says `share: true`.
/// * A `status` goes to any peer that lists `media`.
pub fn may_send(kind: Kind, own_share: bool, share_just_turned_off: bool, peer_lists_media: bool, peer: Option<PeerStatus>) -> bool {
    if !peer_lists_media {
        return false;
    }
    match kind {
        Kind::Status => true,
        Kind::Cmd => peer.is_some_and(|p| p.share),
        Kind::Clear if share_just_turned_off => peer.is_some_and(|p| p.show),
        Kind::State | Kind::Image | Kind::Volume | Kind::Clear => own_share && peer.is_some_and(|p| p.show),
    }
}

/// A viewer whose `show` is off drops `state`, `image`, `clear` and `volume` silently (counted); everything else is kept.
pub fn viewer_accepts(kind: Kind, own_show: bool) -> bool {
    own_show || !matches!(kind, Kind::State | Kind::Image | Kind::Clear | Kind::Volume)
}

/// What a source answers to a `media.cmd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It is done (or being done): no reply.
    Ok,
    Disabled,
    RateLimited,
    NotFound,
    Stale,
    ActionFailed,
}

impl Verdict {
    /// The `error` code, or `None` for success.
    pub fn code(self) -> Option<&'static str> {
        match self {
            Verdict::Ok => None,
            Verdict::Disabled => Some("disabled"),
            Verdict::RateLimited => Some("rate_limited"),
            Verdict::NotFound => Some("not_found"),
            Verdict::Stale => Some("stale"),
            Verdict::ActionFailed => Some("action_failed"),
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        Some(match s {
            "ok" => Verdict::Ok,
            "disabled" => Verdict::Disabled,
            "rate_limited" => Verdict::RateLimited,
            "not_found" => Verdict::NotFound,
            "stale" => Verdict::Stale,
            "action_failed" => Verdict::ActionFailed,
            _ => return None,
        })
    }
}

/// What a source answers, **in this order**: its `share` is off → `Disabled`; over its budget → `RateLimited`; (for every command but the volume) no active session →
/// `NotFound`, a `sid` that isn't the active one or an action the session doesn't list → `Stale`; the system refusing (`system_ok` false) → `ActionFailed`.
pub fn cmd_result(own_share: bool, rate_ok: bool, active: Option<(&str, &[Action])>, cmd: &Cmd, system_ok: bool) -> Verdict {
    if !own_share {
        return Verdict::Disabled;
    }
    if !rate_ok {
        return Verdict::RateLimited;
    }
    if !matches!(cmd.kind, CmdKind::Volume(_)) {
        let Some((sid, actions)) = active else {
            return Verdict::NotFound;
        };
        let listed = cmd.needs().is_some_and(|needs| needs.iter().any(|n| actions.contains(n)));
        if cmd.sid.as_deref() != Some(sid) || !listed {
            return Verdict::Stale;
        }
    }
    if system_ok { Verdict::Ok } else { Verdict::ActionFailed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::wire::VolumeOp;

    fn cmd(kind: CmdKind, sid: Option<&str>) -> Cmd {
        Cmd { sid: sid.map(str::to_string), kind }
    }

    #[test]
    fn nothing_is_sent_before_the_peer_says_show_and_a_peer_without_media_gets_nothing() {
        let shows = Some(PeerStatus { show: true, share: false });
        assert!(may_send(Kind::State, true, false, true, shows));
        assert!(!may_send(Kind::State, true, false, true, None), "before any status");
        assert!(!may_send(Kind::State, true, false, true, Some(PeerStatus { show: false, share: true })));
        assert!(!may_send(Kind::State, false, false, true, shows), "own share off");
        assert!(!may_send(Kind::State, true, false, false, shows), "the peer doesn't list media");
        for k in [Kind::Image, Kind::Volume, Kind::Clear] {
            assert!(may_send(k, true, false, true, shows) && !may_send(k, false, false, true, shows), "{k:?}");
        }
    }

    #[test]
    fn a_command_goes_only_to_a_peer_that_shares_and_a_status_to_any_peer_with_media() {
        assert!(may_send(Kind::Cmd, false, false, true, Some(PeerStatus { show: false, share: true })));
        assert!(!may_send(Kind::Cmd, false, false, true, Some(PeerStatus { show: true, share: false })));
        assert!(!may_send(Kind::Cmd, false, false, true, None));
        assert!(may_send(Kind::Status, false, false, true, None));
        assert!(!may_send(Kind::Status, true, false, false, None));
    }

    #[test]
    fn the_one_clear_after_share_goes_off_needs_a_viewer_that_shows() {
        let st = |show| Some(PeerStatus { show, share: false });
        assert!(may_send(Kind::Clear, false, true, true, st(true)));
        assert!(!may_send(Kind::Clear, false, true, true, st(false)));
        assert!(!may_send(Kind::Clear, false, true, true, None));
        assert!(!may_send(Kind::Clear, false, false, true, st(true)), "without the one-time flag a source with share off sends nothing");
    }

    #[test]
    fn a_viewer_that_does_not_show_drops_the_four_and_keeps_the_rest() {
        for k in [Kind::State, Kind::Image, Kind::Clear, Kind::Volume] {
            assert!(!viewer_accepts(k, false) && viewer_accepts(k, true), "{k:?}");
        }
        assert!(viewer_accepts(Kind::Status, false) && viewer_accepts(Kind::Cmd, false));
    }

    #[test]
    fn a_source_answers_in_the_order_disabled_rate_not_found_stale_failed() {
        let sid = "a".repeat(32);
        let listed = [Action::Play, Action::Pause, Action::Next];
        let active = Some((sid.as_str(), &listed[..]));
        let next = cmd(CmdKind::Next, Some(&sid));
        assert_eq!(cmd_result(false, false, None, &next, false), Verdict::Disabled);
        assert_eq!(cmd_result(true, false, active, &next, true), Verdict::RateLimited);
        assert_eq!(cmd_result(true, true, None, &next, true), Verdict::NotFound);
        assert_eq!(cmd_result(true, true, active, &cmd(CmdKind::Next, Some(&"b".repeat(32))), true), Verdict::Stale);
        assert_eq!(cmd_result(true, true, active, &cmd(CmdKind::Prev, Some(&sid)), true), Verdict::Stale, "prev is not listed");
        assert_eq!(cmd_result(true, true, active, &cmd(CmdKind::Seek(5), Some(&sid)), true), Verdict::Stale);
        assert_eq!(cmd_result(true, true, active, &next, false), Verdict::ActionFailed);
        assert_eq!(cmd_result(true, true, active, &next, true), Verdict::Ok);
        assert_eq!(Verdict::Ok.code(), None);
        assert_eq!(Verdict::Stale.code(), Some("stale"));
    }

    #[test]
    fn play_and_pause_are_met_by_toggle_and_the_volume_needs_no_session() {
        let sid = "a".repeat(32);
        let toggle_only = [Action::Toggle];
        let active = Some((sid.as_str(), &toggle_only[..]));
        assert_eq!(cmd_result(true, true, active, &cmd(CmdKind::Play, Some(&sid)), true), Verdict::Ok);
        assert_eq!(cmd_result(true, true, active, &cmd(CmdKind::Pause, Some(&sid)), true), Verdict::Ok);
        let play_only = [Action::Play];
        assert_eq!(cmd_result(true, true, Some((sid.as_str(), &play_only[..])), &cmd(CmdKind::Pause, Some(&sid)), true), Verdict::Stale);
        let volume = cmd(CmdKind::Volume(VolumeOp::Level(10)), None);
        assert_eq!(cmd_result(true, true, None, &volume, true), Verdict::Ok);
        assert_eq!(cmd_result(true, true, None, &volume, false), Verdict::ActionFailed);
        let other = cmd(CmdKind::Volume(VolumeOp::Mute(true)), Some(&"z".repeat(32)));
        assert_eq!(cmd_result(true, true, active, &other, true), Verdict::Ok, "a sid on a volume command is not checked");
    }
}
