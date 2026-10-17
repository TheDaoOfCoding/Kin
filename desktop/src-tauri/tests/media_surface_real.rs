//! The Windows media flyout session of Kin, for real and silent (its own test process, because it sets the process AUMID): a hidden top-level window, a
//! `SystemMediaTransportControls` on it with the status `Paused` (never `Playing`, no audio), read back through GSMTC by **its own AUMID only**. It checks what the unit
//! tests can't: that the session is really there, that a lock of the Windows session takes it away and an unlock brings it back (the same message Windows
//! **sends** to the window, `simulate_lock_for_tests`), and that clearing it removes it from the list.
use std::sync::Arc;
use std::time::Duration;

use kin_lib::media::map::{SmtcStatus, SmtcView};
use kin_lib::media::reader::{SessionBackend, WinBackend};
use kin_lib::media::surface::{MediaSurface, SurfaceView, Timeline, WinSurface};

fn view(title: &str) -> SurfaceView {
    SurfaceView {
        buttons: SmtcView { status: SmtcStatus::Paused, play: true, pause: false, next: true, prev: false },
        title: title.to_string(),
        artist: "Kin test artist".to_string(),
        album_line: "Test Phone \u{b7} Test".to_string(),
        thumbnail: Some(Arc::new(kin_lib::mirror::icons::test_png(64, 64, [10, 200, 90, 255]))),
        timeline: Some(Timeline { duration_ms: 200_000, position_ms: 42_000, seek: true }),
        rate: 1.0,
    }
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("{what} never happened");
}

/// `#[ignore]`: it talks to the real Windows media stack, so it runs from `tools\check-2b3-desktop.ps1` (`cargo test --test media_surface_real -- --ignored`),
/// not in a plain `cargo test`.
#[test]
#[ignore = "real Windows media session: run by tools/check-2b3-desktop.ps1"]
fn the_flyout_session_is_there_goes_when_the_pc_locks_and_returns_when_it_unlocks() {
    let aumid = format!("app.kinrelay.dev.mediatest-{}", std::process::id());
    kin_lib::integration::set_process_app_user_model_id(&aumid).expect("the process AUMID");
    // The reader looks at exactly this session: it listens to no other and reads no other.
    let wanted = aumid.clone();
    let backend = WinBackend::start(Arc::new(move |a| a == wanted), Arc::new(|| {})).expect("GSMTC is available");
    let surface = WinSurface::start(Arc::new(|_| {})).expect("GetForWindow works on a hidden top-level window");
    let title = format!("flyout-test-{}", std::process::id());
    let seen = |backend: &WinBackend| backend.list().contains(&aumid).then(|| backend.read(&aumid)).flatten();

    surface.show(view(&title)).expect("shown");
    wait_for("the session to appear in GSMTC", || seen(&backend).is_some());
    let info = seen(&backend).unwrap();
    assert_eq!((info.title.as_str(), info.artist.as_str()), (title.as_str(), "Kin test artist"));
    assert_eq!(info.status, 5, "Paused (5): a test session is never Playing");
    assert_eq!(info.duration_ms, Some(200_000));
    assert!(info.controls.play && info.controls.next && !info.controls.prev, "{:?}", info.controls);
    assert!(backend.thumbnail(&aumid).is_some_and(|t| !t.is_empty()), "the picture made it into the session");

    // A change goes through.
    surface.show(view("changed-title")).expect("shown again");
    wait_for("the new title", || seen(&backend).is_some_and(|i| i.title == "changed-title"));

    // The Windows session locks: nothing of the phone's media stays. It unlocks: it is back.
    surface.simulate_lock_for_tests(true);
    wait_for("the session to go while locked", || seen(&backend).is_none());
    surface.show(view("while-locked")).expect("a show while locked is accepted and kept");
    std::thread::sleep(Duration::from_millis(500));
    assert!(seen(&backend).is_none(), "a locked PC shows nothing, even for a new state");
    surface.simulate_lock_for_tests(false);
    wait_for("the session to return after the unlock", || seen(&backend).is_some_and(|i| i.title == "while-locked"));

    // Clearing removes it.
    surface.clear();
    wait_for("the session to leave the list", || seen(&backend).is_none());
    surface.stop();
    backend.stop();
}
