//! The low-battery nudge (design 3.6): a pure state machine, per peer, in memory until Kin quits (not per connection). `protocol/presence.json` `lowBattery[]`.
//!
//! ```text
//! armed starts true
//! if charging or level >= 20      -> armed = true
//! else if level <= 15 and armed   -> NUDGE, armed = false     (not while the receiver is paused: then nothing happens and armed is left alone)
//! ```
//! So one nudge per discharge episode, even across reconnects, re-armed by charging or by reaching 20. A nudge held back by the receiver's own pause does not
//! disarm: it fires on the first snapshot after the pause if the phone is still at 15 % or less and not charging.

pub const NUDGE_AT: u8 = 15;
pub const REARM_AT: u8 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LowBatteryNudge {
    armed: bool,
}

impl Default for LowBatteryNudge {
    fn default() -> Self {
        LowBatteryNudge::new()
    }
}

impl LowBatteryNudge {
    pub fn new() -> LowBatteryNudge {
        LowBatteryNudge { armed: true }
    }

    /// One snapshot with a battery. `true`: show the nudge now. `paused`: this PC is paused (nothing is shown, and the machine stays as it is).
    pub fn snapshot(&mut self, level: u8, charging: bool, paused: bool) -> bool {
        if charging || level >= REARM_AT {
            self.armed = true;
            false
        } else if level <= NUDGE_AT && self.armed && !paused {
            self.armed = false;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: &[(u8, bool, bool)]) -> Vec<usize> {
        let mut n = LowBatteryNudge::new();
        steps.iter().enumerate().filter_map(|(i, (level, charging, paused))| n.snapshot(*level, *charging, *paused).then_some(i)).collect()
    }

    #[test]
    fn one_nudge_per_episode_and_the_edges_are_15_and_20() {
        assert_eq!(run(&[(30, false, false), (20, false, false), (15, false, false), (14, false, false), (10, false, false)]), [2]);
        assert_eq!(run(&[(16, false, false)]), Vec::<usize>::new(), "16 does not nudge");
        assert_eq!(
            run(&[(15, false, false), (19, false, false), (14, false, false), (20, false, false), (15, false, false)]),
            [0, 4],
            "19 does not re-arm, 20 does"
        );
    }

    #[test]
    fn charging_rearms_and_never_nudges() {
        assert_eq!(run(&[(5, true, false), (3, true, false)]), Vec::<usize>::new());
        assert_eq!(run(&[(14, false, false), (14, true, false), (14, false, false)]), [0, 2]);
    }

    #[test]
    fn a_pause_holds_the_nudge_back_without_using_it_up() {
        assert_eq!(run(&[(14, false, true), (14, false, true), (13, false, false)]), [2]);
        assert_eq!(run(&[(14, false, true), (50, true, true), (14, false, false)]), [2]);
    }
}
