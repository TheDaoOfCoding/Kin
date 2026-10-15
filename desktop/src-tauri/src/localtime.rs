//! Local time for the tray's "Phone offline – last seen <time>". The conversion from a Unix time to the user's local time (with their
//! time zone and daylight saving at that moment) is Windows' (`SystemTimeToTzSpecificLocalTime`); what is written is plain code, and is
//! tested with times given by hand.

/// A time of day on a calendar day, in the user's time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    /// 1 to 31
    pub day: u32,
    /// 0 (Sunday) to 6 (Saturday)
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Days since 0000-03-01 style counter for a calendar day, to count whole days between two dates.
fn day_number(t: &LocalTime) -> i64 {
    let (y, m) = if t.month <= 2 { (t.year as i64 - 1, t.month as i64 + 12) } else { (t.year as i64, t.month as i64) };
    365 * y + y / 4 - y / 100 + y / 400 + (153 * (m - 3) + 2) / 5 + t.day as i64
}

/// "14:32" today, "yesterday 14:32", "Mon 14:32" within the last week, else "12 Oct 14:32".
pub fn last_seen_text(seen: &LocalTime, now: &LocalTime) -> String {
    let clock = format!("{:02}:{:02}", seen.hour, seen.minute);
    match day_number(now) - day_number(seen) {
        0 => clock,
        1 => format!("yesterday {clock}"),
        2..=6 => format!("{} {clock}", WEEKDAYS[(seen.weekday % 7) as usize]),
        _ => format!("{} {} {clock}", seen.day, MONTHS[((seen.month.clamp(1, 12)) - 1) as usize]),
    }
}

/// The user's local time for a Unix time in milliseconds, from Windows.
pub fn local_time(unix_ms: i64) -> Option<LocalTime> {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // FILETIME counts 100 ns from 1601-01-01.
    let ticks = u64::try_from((unix_ms + 11_644_473_600_000) * 10_000).ok()?;
    let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    // SAFETY: both calls only read and write the structures given to them.
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(LocalTime {
        year: local.wYear as i32,
        month: local.wMonth as u32,
        day: local.wDay as u32,
        weekday: local.wDayOfWeek as u32,
        hour: local.wHour as u32,
        minute: local.wMinute as u32,
    })
}

/// "last seen" text for a time in Unix milliseconds, as of `now_ms`.
pub fn last_seen_for(seen_ms: i64, now_ms: i64) -> Option<String> {
    Some(last_seen_text(&local_time(seen_ms)?, &local_time(now_ms)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(year: i32, month: u32, day: u32, weekday: u32, hour: u32, minute: u32) -> LocalTime {
        LocalTime { year, month, day, weekday, hour, minute }
    }

    #[test]
    fn today_is_just_the_time() {
        assert_eq!(last_seen_text(&t(2026, 10, 15, 4, 9, 5), &t(2026, 10, 15, 4, 21, 40)), "09:05");
    }

    #[test]
    fn yesterday_the_weekday_and_the_date() {
        let now = t(2026, 10, 15, 4, 8, 0); // a Thursday
        assert_eq!(last_seen_text(&t(2026, 10, 14, 3, 23, 59), &now), "yesterday 23:59", "a different day, not 24 hours");
        assert_eq!(last_seen_text(&t(2026, 10, 12, 1, 14, 32), &now), "Mon 14:32");
        assert_eq!(last_seen_text(&t(2026, 10, 9, 5, 14, 32), &now), "Fri 14:32", "6 days ago is still a weekday");
        assert_eq!(last_seen_text(&t(2026, 10, 8, 4, 14, 32), &now), "8 Oct 14:32", "a week ago has the date");
        assert_eq!(last_seen_text(&t(2025, 12, 31, 3, 7, 0), &t(2026, 1, 1, 4, 7, 0)), "yesterday 07:00", "across a year");
        assert_eq!(last_seen_text(&t(2024, 2, 29, 4, 7, 0), &t(2024, 3, 1, 5, 7, 0)), "yesterday 07:00", "across a leap day");
    }

    #[test]
    fn windows_gives_a_plausible_local_time() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let lt = local_time(now).expect("the conversion works");
        assert!(lt.year >= 2024 && (1..=12).contains(&lt.month) && lt.hour < 24 && lt.minute < 60);
        assert_eq!(last_seen_for(now, now).map(|s| s.len()), Some(5), "now is \"HH:MM\"");
    }
}
