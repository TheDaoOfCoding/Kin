//! `cargo run --example focus_probe [--seconds N] [--also-presenting] [--cost]`: what Kin reads to decide "this PC is in Focus" (milestone 2B.5, docs/windows.md, R45), for the person who
//! wants to check it on their own PC, for N seconds (30 by default).
//!
//! **Read-only.** It uses the source Kin itself uses (`kin_lib::focus::source::WindowsQuiet`): `FocusSessionManager.IsFocusActive` and its `IsFocusActiveChanged` event, and
//! `SHQueryUserNotificationState`. It never starts or ends a Focus session and never changes Do Not Disturb or any other setting; **you** do that, in Windows, while it runs:
//!
//! * start a Focus session from the Clock app, then end it, or
//! * turn Do Not Disturb on in the notification center (the bell), then off, or
//! * open a full-screen video or a presentation.
//!
//! It prints one line at the start and one line whenever something changes (the Windows event, or a change it sees on its once-a-second look), with the second it happened:
//! `focusSession=<bool> quiet=<state> event=<count> focus=<bool> (<reason>)`. `focus` is what Kin would tell the phone (after its 3 s of stability); `--also-presenting` counts
//! a full-screen or presenting state as Focus, like the checkbox in Kin's Settings. It prints nothing about notifications and writes nothing anywhere.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kin_lib::focus::source::WindowsQuiet;
use kin_lib::focus::state::{QuietSource, is_focus, reason_word};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args.iter().position(|a| a == "--seconds").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(30);
    let also = args.iter().any(|a| a == "--also-presenting");

    let events = Arc::new(AtomicU64::new(0));
    let source = {
        let events = events.clone();
        WindowsQuiet::new(Arc::new(move || {
            events.fetch_add(1, Ordering::SeqCst);
        }))
    };
    println!(
        "FocusSessionManager {} on this Windows; reading for {seconds} s (read-only: change Focus or Do Not Disturb yourself and watch the lines)",
        if source.focus_sessions_supported() {
            "exists"
        } else {
            "is NOT available (Windows 10 or an older Windows 11): only Do Not Disturb is read"
        }
    );
    // `--cost`: what one poll costs (1000 readings of both facts), so the 15 s tick can be judged.
    if args.iter().any(|a| a == "--cost") {
        let t = Instant::now();
        for _ in 0..1000 {
            let _ = (source.focus_session(), source.quiet_state());
        }
        println!(
            "one reading (IsFocusActive through the worker thread + SHQueryUserNotificationState): {:.1} microseconds",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    let start = Instant::now();
    let mut last: Option<(bool, String, u64)> = None;
    while start.elapsed() < Duration::from_secs(seconds) {
        let session = source.focus_session();
        let state = source.quiet_state();
        let count = events.load(Ordering::SeqCst);
        let now = (session, state.name().to_string(), count);
        if last.as_ref() != Some(&now) {
            println!(
                "t={:>3}s focusSession={session} quiet={} event={count} focus={} ({})",
                start.elapsed().as_secs(),
                state.name(),
                is_focus(session, state, also),
                reason_word(session, state, also)
            );
            last = Some(now);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    println!("done: {} Windows event(s) in {seconds} s", events.load(Ordering::SeqCst));
}
