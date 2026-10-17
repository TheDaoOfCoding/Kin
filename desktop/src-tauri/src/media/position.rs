//! Where a track is (protocol.md "Position and clocks"): the two clocks differ, so a source sends the position **as of the moment it sends** and the viewer
//! extrapolates with its own monotonic clock. Pure: every function takes its times as arguments. `protocol/media.json` (`position[]`, `sourcePosition[]`) pins
//! the arithmetic; all divisions round down.
use super::wire::{MAX_RATE_MILLI, PlayState};

/// A `playing` state this far past its duration, with no new message, is shown paused at the end.
pub const END_GRACE_MS: u64 = 5000;
/// A position that differs by more than this from what the extrapolation says is a seek: the source re-sends.
pub const JUMP_MS: u64 = 1500;

/// What a viewer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    /// `None`: the source sent no position (no progress bar).
    pub position_ms: Option<u64>,
    pub state: PlayState,
}

/// What the viewer shows `elapsed_ms` after a `media.state` arrived. `rate_milli` `None` is 1000, and 0 shows as paused.
pub fn shown(state: PlayState, position_ms: Option<u64>, rate_milli: Option<u64>, duration_ms: Option<u64>, elapsed_ms: u64) -> Shown {
    let rate = rate_milli.unwrap_or(1000).min(MAX_RATE_MILLI);
    let cap = |p: u64| duration_ms.map_or(p, |d| p.min(d));
    let Some(position) = position_ms else {
        let state = if state == PlayState::Playing && rate == 0 { PlayState::Paused } else { state };
        return Shown { position_ms: None, state };
    };
    if state != PlayState::Playing {
        return Shown { position_ms: Some(cap(position)), state };
    }
    if rate == 0 {
        return Shown { position_ms: Some(cap(position)), state: PlayState::Paused };
    }
    let raw = position.saturating_add(elapsed_ms.saturating_mul(rate) / 1000);
    if let Some(d) = duration_ms
        && raw > d.saturating_add(END_GRACE_MS)
    {
        return Shown { position_ms: Some(d), state: PlayState::Paused };
    }
    Shown { position_ms: Some(cap(raw)), state: PlayState::Playing }
}

/// The source's own position for now, on Windows: the timeline's `Position` is at `LastUpdatedTime` (100 ns ticks since 1601), and while `Playing` the time
/// since then goes by at `PlaybackRate`. A session that never updated its timeline (`last_updated_ticks` 0) or a clock that went back adds nothing. Clamped to
/// the duration when there is one.
pub fn windows_source_position(
    position_ticks: i64,
    last_updated_ticks: i64,
    now_ticks: i64,
    rate_milli: Option<u64>,
    playing: bool,
    duration_ms: Option<u64>,
) -> u64 {
    let base = (position_ticks.max(0) / 10_000) as u64;
    let diff = i128::from(now_ticks) - i128::from(last_updated_ticks);
    let extra =
        if playing && last_updated_ticks > 0 && diff > 0 { (diff * i128::from(rate_milli.unwrap_or(1000)) / 10_000_000) as u64 } else { 0 };
    let p = base.saturating_add(extra);
    duration_ms.map_or(p, |d| p.min(d))
}

/// Whether the position a source would send now is a seek compared with the one it sent last (both already extrapolated to the same moment).
pub fn jumped(last_sent_expected_ms: u64, now_ms: u64) -> bool {
    last_sent_expected_ms.abs_diff(now_ms) > JUMP_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_runs_and_the_others_stand_still() {
        assert_eq!(shown(PlayState::Playing, Some(1000), Some(1000), Some(200_000), 5000).position_ms, Some(6000));
        assert_eq!(shown(PlayState::Paused, Some(1000), Some(1000), None, 5000).position_ms, Some(1000));
        assert_eq!(shown(PlayState::Buffering, Some(1000), None, None, 5000).position_ms, Some(1000));
        assert_eq!(shown(PlayState::Playing, Some(0), Some(500), None, 5001).position_ms, Some(2500), "rounds down");
        assert_eq!(shown(PlayState::Playing, Some(0), Some(9000), None, 1000).position_ms, Some(4000), "over 4000 counts as 4000");
    }

    #[test]
    fn rate_zero_is_paused_and_the_end_is_five_seconds_of_grace() {
        assert_eq!(shown(PlayState::Playing, Some(5), Some(0), None, 9999).state, PlayState::Paused);
        let at = |elapsed| shown(PlayState::Playing, Some(9000), Some(1000), Some(10_000), elapsed);
        assert_eq!(at(6000), Shown { position_ms: Some(10_000), state: PlayState::Playing });
        assert_eq!(at(6001), Shown { position_ms: Some(10_000), state: PlayState::Paused });
        assert_eq!(shown(PlayState::Playing, None, Some(1000), Some(10), 5000), Shown { position_ms: None, state: PlayState::Playing });
    }

    #[test]
    fn windows_extrapolates_from_the_last_update_and_never_backwards() {
        let last = 133_700_000_000_000_000i64;
        // The design's example: position 0, updated 103.9 s ago.
        assert_eq!(windows_source_position(0, last, last + 1_039_000_000, None, true, None), 103_900);
        assert_eq!(windows_source_position(100_000_000, last, last + 50_000_000, Some(2000), true, None), 20_000);
        assert_eq!(windows_source_position(100_000_000, last, last + 50_000_000, Some(1000), false, None), 10_000);
        assert_eq!(windows_source_position(150_000_000, last, last + 100_000_000, Some(1000), true, Some(20_000)), 20_000);
        assert_eq!(windows_source_position(50_000_000, 0, last, Some(1000), true, None), 5000, "never updated");
        assert_eq!(windows_source_position(50_000_000, last + 100_000_000, last, Some(1000), true, None), 5000, "a clock that went back");
        assert_eq!(windows_source_position(1_234_567, last, last, Some(1000), false, None), 123, "ticks are 100 ns");
    }

    #[test]
    fn a_jump_is_more_than_1500_ms() {
        assert!(!jumped(10_000, 11_500));
        assert!(jumped(10_000, 11_501));
        assert!(jumped(10_000, 8_499));
    }
}
