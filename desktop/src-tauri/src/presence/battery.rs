//! This PC's battery for the phone (design 6.1) and the maps of the vectors (`protocol/presence.json` `batteryMap[]`).
//!
//! The real source is `GetSystemPowerStatus` (read-only, no permission, no window), polled every 60 s **only while** a phone is connected, "share this PC's battery"
//! is on and the PC has a battery. A PC without one (`BatteryFlag` bit 128) never polls. It is not event driven in 2B.4: `RegisterPowerSettingNotification`
//! needs a window and could not be tried on a PC with no battery. `--dev-battery <level> <charging|unplugged>|none` replaces the source with a fake.
use std::sync::Mutex;

use super::wire::Battery;

/// How often the real battery is read while it matters.
pub const POLL_SECS: u64 = 60;

/// Android's `ACTION_BATTERY_CHANGED` extras to the wire (the phone does this; the PC keeps the map for the vectors): `level` rounded to the nearest percent
/// (halves up) and cut to 100, `charging` = `plugged != 0` (`status` is not looked at), a `scale` of 0 or less or a negative `level` is unknown.
pub fn map_android(level: i32, scale: i32, _status: i32, plugged: i32) -> Option<Battery> {
    if scale <= 0 || level < 0 {
        return None;
    }
    let pct = (i64::from(level) * 200 + i64::from(scale)) / (2 * i64::from(scale));
    Some(Battery { level: pct.min(100) as u8, charging: plugged != 0, saver: None })
}

/// `SYSTEM_POWER_STATUS` to the wire: no battery (`BatteryFlag` bit 128), an unknown flag (255) or an unknown percent (255) is `None`; `charging` is
/// `ACLineStatus == 1` (online); a percent over 100 is cut.
pub fn map_windows(ac_line_status: u8, battery_flag: u8, battery_life_percent: u8) -> Option<Battery> {
    if battery_flag == 255 || battery_flag & 128 != 0 || battery_life_percent == 255 {
        return None;
    }
    Some(Battery { level: battery_life_percent.min(100), charging: ac_line_status == 1, saver: None })
}

/// Where the PC's battery comes from.
pub trait PowerSource: Send + Sync {
    /// Whether this PC has a battery at all: a desktop never polls.
    fn has_battery(&self) -> bool;
    /// The battery now, `None` if there is none or it is unknown.
    fn read(&self) -> Option<Battery>;
}

/// The real thing (`GetSystemPowerStatus`).
pub struct WinPower;

impl WinPower {
    fn status() -> Option<(u8, u8, u8)> {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut s = SYSTEM_POWER_STATUS::default();
        // SAFETY: the call only writes the structure it is given.
        unsafe { GetSystemPowerStatus(&mut s).ok()? };
        Some((s.ACLineStatus, s.BatteryFlag, s.BatteryLifePercent))
    }
}

impl PowerSource for WinPower {
    fn has_battery(&self) -> bool {
        WinPower::status().is_some_and(|(_, flag, _)| flag != 255 && flag & 128 == 0)
    }

    fn read(&self) -> Option<Battery> {
        let (ac, flag, pct) = WinPower::status()?;
        map_windows(ac, flag, pct)
    }
}

/// A fixed battery for a test run (`--dev-battery 9 unplugged`), or none (`--dev-battery none`, and the default of a test that doesn't say). It can be set
/// again while the app runs (`--dev-set devBattery 40:charging`).
#[derive(Default)]
pub struct FakePower(Mutex<Option<Battery>>);

impl FakePower {
    pub fn new(battery: Option<Battery>) -> FakePower {
        FakePower(Mutex::new(battery))
    }

    pub fn set(&self, battery: Option<Battery>) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = battery;
    }
}

impl PowerSource for FakePower {
    fn has_battery(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).is_some()
    }

    fn read(&self) -> Option<Battery> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// `--dev-battery`: `<level> charging`, `<level> unplugged` or `none`, as one word: `62:charging`, `9:unplugged`, `none`.
pub fn parse_dev_battery(text: &str) -> Option<Option<Battery>> {
    if text == "none" {
        return Some(None);
    }
    let (level, state) = text.split_once(':')?;
    let level: u8 = level.parse().ok().filter(|l| *l <= 100)?;
    let charging = match state {
        "charging" => true,
        "unplugged" => false,
        _ => return None,
    };
    Some(Some(Battery { level, charging, saver: None }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_map_knows_no_battery_unknown_and_charging() {
        assert_eq!(map_windows(0, 1, 71), Some(Battery { level: 71, charging: false, saver: None }));
        assert_eq!(map_windows(1, 8, 40).map(|b| b.charging), Some(true));
        assert_eq!(map_windows(1, 128, 255), None, "a desktop PC");
        assert_eq!(map_windows(1, 128, 100), None);
        assert_eq!(map_windows(1, 255, 50), None);
        assert_eq!(map_windows(0, 1, 255), None);
        assert_eq!(map_windows(255, 1, 55).map(|b| b.charging), Some(false), "an unknown line status is not plugged in");
        assert_eq!(map_windows(0, 1, 101).map(|b| b.level), Some(100));
    }

    #[test]
    fn the_android_map_rounds_halves_up_and_knows_unknown() {
        assert_eq!(map_android(128, 255, 3, 0).map(|b| b.level), Some(50));
        assert_eq!(map_android(1, 8, 3, 0).map(|b| b.level), Some(13), "12.5 rounds up");
        assert_eq!(map_android(50, 0, 2, 1), None);
        assert_eq!(map_android(-1, 100, 1, 0), None);
        assert_eq!(map_android(120, 100, 2, 1).map(|b| b.level), Some(100));
        assert_eq!(map_android(80, 100, 4, 2).map(|b| b.charging), Some(true), "plugged in, not charging: still plugged in");
        assert_eq!(map_android(40, 100, 2, 0).map(|b| b.charging), Some(false), "plugged decides");
    }

    #[test]
    fn a_fake_power_source_says_what_it_was_told_and_a_desktop_has_none() {
        let fake = FakePower::new(Some(Battery { level: 9, charging: false, saver: None }));
        assert!(fake.has_battery());
        assert_eq!(fake.read().map(|b| b.level), Some(9));
        fake.set(None);
        assert!(!fake.has_battery() && fake.read().is_none());
    }

    #[test]
    fn the_dev_battery_words() {
        assert_eq!(parse_dev_battery("9:unplugged"), Some(Some(Battery { level: 9, charging: false, saver: None })));
        assert!(parse_dev_battery("100:charging").unwrap().unwrap().charging);
        assert_eq!(parse_dev_battery("none"), Some(None));
        for bad in ["", "9", "101:charging", "x:charging", "9:flying"] {
            assert_eq!(parse_dev_battery(bad), None, "{bad}");
        }
    }

    /// The real call works on this PC (no assertion about the values: a desktop has no battery, a laptop has one).
    #[test]
    fn the_real_power_status_can_be_read() {
        let real = WinPower;
        let _ = real.has_battery();
        if let Some(b) = real.read() {
            assert!(b.level <= 100);
        }
    }
}
