//! Milestone 2B.5, Focus and Do Not Disturb, against the real core in the test process, with a fake phone and a **fake Windows source**: when this PC tells the phone it is in Focus
//! (after 3 s of stability, within a second, never while the PC is paused), that nothing is read or registered while the switch is off or no phone is connected, what the PC does
//! with the phone's `dnd` (shows it, holds only its own optional toasts, forgets it with the connection), and the 2B.4 review fixes (the resume race, a hello on an unlinked
//! phone, an unsolicited `ring.status`).
//!
//! **No real Windows Focus or Do Not Disturb is read or changed here** (a fake source replaces the reading, and nothing in Kin changes them), nothing is audible and the
//! clipboard is the in-memory one.
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use futures_util::StreamExt;
use kin_lib::core::{Core, Event, LinkState};
use kin_lib::focus::state::FakeQuiet;
use kin_lib::presence::battery::FakePower;
use kin_lib::presence::pause::PauseChoice;
use kin_lib::presence::wire::Dnd;
use kin_lib::quiet::{UserState, UserStateProbe};
use serde_json::{Value, json};

struct Fixed(UserState);

impl UserStateProbe for Fixed {
    fn state(&self) -> UserState {
        self.0
    }
}

fn uuid() -> String {
    kin_lib::encoding::uuid_v7(kin_lib::encoding::unix_ms())
}

struct Rig {
    w: World,
    ws: Ws,
    quiet: Arc<FakeQuiet>,
}

impl Rig {
    async fn new(caps: &[&str]) -> Rig {
        capture_logs();
        let w = World::new().await;
        w.pair().await;
        w.env.core.set_user_state_probe(Arc::new(Fixed(UserState::AcceptsNotifications)));
        w.env.core.set_power_source(Arc::new(FakePower::new(None)));
        let quiet = Arc::new(FakeQuiet::new(false, UserState::AcceptsNotifications));
        w.env.core.set_quiet_source(quiet.clone());
        let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
        assert_eq!(next_json(&mut ws).await["type"], "hello");
        let mut phone = phone_hello();
        phone["capabilities"] = json!(caps);
        send_json(&mut ws, &phone).await;
        w.wait_for_state(LinkState::PhoneConnected).await;
        Rig { w, ws, quiet }
    }

    fn core(&self) -> &Arc<Core> {
        &self.w.env.core
    }

    async fn send(&mut self, v: Value) {
        send_json(&mut self.ws, &v).await;
    }

    async fn presence_from_phone(&mut self, body: Value) {
        let mut v = json!({ "type": "presence", "id": uuid() });
        for (k, val) in body.as_object().unwrap() {
            v[k] = val.clone();
        }
        self.send(v).await;
    }

    /// The next `presence` frame the PC sends, within `secs` seconds. `None` if none came.
    async fn next_presence(&mut self, secs: u64) -> Option<Value> {
        let until = tokio::time::Instant::now() + Duration::from_secs(secs);
        loop {
            match tokio::time::timeout_at(until, self.ws.next()).await {
                Err(_) => return None,
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["type"] == "presence" {
                        return Some(v);
                    }
                }
                Ok(Some(Ok(_))) => continue,
                Ok(other) => panic!("the connection ended: {other:?}"),
            }
        }
    }

    fn events(&mut self) -> Vec<Event> {
        drain(&mut self.w.env.events)
    }
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..300 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

const CAPS: [&str; 4] = ["link", "media", "presence", "ring"];

fn nudges(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToastRedacted { text, .. } if text.contains("% and not charging") => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// ----- this PC's Focus, to the phone ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_pc_tells_the_phone_it_is_in_focus_after_three_seconds_and_when_it_leaves() {
    let mut rig = Rig::new(&CAPS).await;
    let first = rig.next_presence(3).await.expect("the snapshot after the hello");
    assert!(first.get("focus").is_none(), "nothing before the switch is on: {first}");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    // Focus starts (Windows says so): the phone hears it once it has been stable for 3 s, and within a second after that.
    let started = Instant::now();
    rig.quiet.set(true, UserState::AcceptsNotifications);
    rig.core().focus_event();
    let p = rig.next_presence(9).await.expect("a snapshot with focus");
    assert_eq!(p["focus"], true, "{p}");
    assert!(started.elapsed() >= Duration::from_millis(2_900), "it was sent after {:?}, before the 3 s were over", started.elapsed());
    assert!(p.get("dnd").is_none(), "the PC never sends dnd");
    assert_eq!(rig.core().focus_status_json()["on"], true);
    // Focus ends: a snapshot without it, again after 3 s.
    let ended = Instant::now();
    rig.quiet.set(false, UserState::AcceptsNotifications);
    rig.core().focus_event();
    let p = rig.next_presence(9).await.expect("a snapshot without focus");
    assert!(p.get("focus").is_none(), "{p}");
    assert!(ended.elapsed() >= Duration::from_millis(2_900), "{:?}", ended.elapsed());
    assert_eq!(rig.core().focus_status_json()["on"], false);
    // The log has the words and nothing else about it.
    let lines: Vec<String> = captured_lines().into_iter().filter(|l| l.contains("focus:")).collect();
    assert!(lines.iter().any(|l| l.ends_with("focus: on (focus session)")), "{lines:?}");
    assert!(lines.iter().any(|l| l.ends_with("focus: off")), "{lines:?}");
}

#[tokio::test]
async fn a_flip_back_inside_three_seconds_sends_nothing() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    rig.quiet.set(true, UserState::AcceptsNotifications);
    rig.core().focus_event();
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    rig.quiet.set(false, UserState::AcceptsNotifications);
    rig.core().focus_event();
    assert!(rig.next_presence(5).await.is_none(), "a Focus of 1 s never reached the phone");
    assert_eq!(rig.core().focus_status_json()["on"], false);
}

#[tokio::test]
async fn presenting_counts_only_with_the_checkbox() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    rig.quiet.set(false, UserState::RunningD3dFullScreen);
    rig.core().focus_event();
    assert!(rig.next_presence(4).await.is_none(), "a full-screen program is not Focus without the checkbox");
    rig.core().set_focus_also_presenting(true);
    rig.core().focus_event();
    let p = rig.next_presence(9).await.expect("with the checkbox it counts");
    assert_eq!(p["focus"], true);
}

#[tokio::test]
async fn nothing_is_read_or_registered_while_the_switch_is_off_or_no_phone_is_connected() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!rig.core().focus_source_active(), "the switch is off: no source");
    assert_eq!(rig.core().focus_status_json()["counts"]["reads"], 0);
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    wait_until("a first read", || rig.core().focus_status_json()["counts"]["reads"].as_u64().unwrap_or(0) >= 1).await;
    // The switch off: the source is dropped and nothing is read again.
    rig.core().set_focus_share(false);
    assert!(!rig.core().focus_source_active());
    let reads = rig.core().focus_status_json()["counts"]["reads"].clone();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(rig.core().focus_status_json()["counts"]["reads"], reads);
    // The switch on and the phone gone: no source either.
    rig.core().set_focus_share(true);
    wait_until("the source again", || rig.core().focus_source_active()).await;
    rig.ws.close(None).await.unwrap();
    wait_until("the source is dropped with the connection", || !rig.core().focus_source_active()).await;
}

#[tokio::test]
async fn a_phone_without_presence_gets_no_focus_and_nothing_is_read() {
    let rig = Rig::new(&["link"]).await;
    rig.core().set_focus_share(true);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!rig.core().focus_source_active(), "an old phone has no presence: nothing to tell it");
}

#[tokio::test]
async fn a_paused_pc_puts_no_focus_in_its_snapshot_and_resuming_brings_it_back() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    rig.quiet.set(true, UserState::AcceptsNotifications);
    rig.core().focus_event();
    assert_eq!(rig.next_presence(9).await.expect("focus")["focus"], true);
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    let paused = rig.next_presence(4).await.expect("the pause");
    assert!(paused.get("pause").is_some() && paused.get("focus").is_none(), "a paused PC asks not to be acted on: {paused}");
    rig.core().set_pause(None);
    let resumed = rig.next_presence(4).await.expect("the resume");
    assert!(resumed.get("pause").is_none() && resumed["focus"] == true, "{resumed}");
}

#[tokio::test]
async fn a_phone_that_connects_while_the_pc_is_in_focus_hears_it_in_its_first_snapshot() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    rig.quiet.set(true, UserState::AcceptsNotifications);
    rig.core().focus_event();
    assert_eq!(rig.next_presence(9).await.expect("focus")["focus"], true);
    // The phone's Wi-Fi blips: a new connection starts with a snapshot that already says it.
    rig.ws.close(None).await.unwrap();
    wait_until("offline", || rig.core().snapshot().state == LinkState::PhoneOffline).await;
    let mut ws = rig.w.try_open_ws().await.expect("a second connection");
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    let mut phone = phone_hello();
    phone["capabilities"] = json!(CAPS);
    send_json(&mut ws, &phone).await;
    let first = loop {
        let f = next_json(&mut ws).await;
        if f["type"] == "presence" {
            break f;
        }
    };
    assert_eq!(first["focus"], true, "{first}");
}

/// Review round 3, blocker 1: a reconnect that races the pump can create a connection whose first snapshot says `focus: true` after this PC's stable value went back to false
/// (the pump counted no phone, reset, and told only the senders that existed). The reading afterwards equals the reset value, so no *change* is ever seen: without the pump
/// telling every sender the stable value on each turn the phone stayed silent for good. Here the stale connection is made directly; the pump must put it right.
#[tokio::test]
async fn a_connection_created_with_a_stale_focus_true_is_told_the_truth_by_the_next_turn() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    assert_eq!(rig.core().focus_status_json()["on"], false, "Windows says: not in Focus");
    let id = rig.core().snapshot().devices[0].device_id.clone();
    rig.core().presence_force_stale_focus(&id, true);
    let stale = rig.next_presence(3).await.expect("the stale snapshot");
    assert_eq!(stale["focus"], true, "{stale}");
    // Windows still says "not in Focus": the reading equals the stable value, so no change is ever seen. Only the pump's next turn can put it right.
    rig.core().focus_event();
    let healed = rig.next_presence(5).await.expect("the phone must hear that there is no Focus");
    assert!(healed.get("focus").is_none(), "{healed}");
    assert!(rig.next_presence(3).await.is_none(), "and then nothing more: an unchanged value sends nothing");
}

/// Review round 3, cleanup 4: a clean quit tells the phone first (a snapshot without `focus`), so it does not wait 60 s to let go.
#[tokio::test]
async fn a_clean_quit_sends_one_snapshot_without_focus_before_the_sockets_close() {
    let mut rig = Rig::new(&CAPS).await;
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    rig.quiet.set(true, UserState::AcceptsNotifications);
    rig.core().focus_event();
    assert_eq!(rig.next_presence(9).await.expect("focus")["focus"], true);
    let core = rig.core().clone();
    let quit = tokio::spawn(async move { core.shutdown().await });
    let mut last_presence = None;
    loop {
        match tokio::time::timeout(Duration::from_secs(5), rig.ws.next()).await.expect("the socket closes") {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                if v["type"] == "presence" {
                    last_presence = Some(v);
                }
            }
            Some(Ok(_)) => {}
            Some(Err(_)) | None => break,
        }
    }
    quit.await.unwrap();
    let last = last_presence.expect("a last snapshot before the close");
    assert!(last.get("focus").is_none(), "{last}");
}

// ----- the phone's Do Not Disturb, here ----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_phones_dnd_is_shown_holds_only_the_pcs_own_optional_toasts_and_is_forgotten_with_the_connection() {
    let mut rig = Rig::new(&CAPS).await;
    rig.events();
    rig.presence_from_phone(json!({ "dnd": "priority" })).await;
    wait_until("the dnd", || rig.core().presence_view().phone_dnd == Some(Dnd::Priority)).await;
    assert_eq!(rig.core().presence_panel()["phone"]["dnd"], "Do Not Disturb");
    assert_eq!(rig.core().presence_status_json()["presence"]["phone"]["dnd"], "priority", "a current word, never a history");
    let snapshot = rig.core().snapshot();
    assert!(kin_lib::tray::tooltip(&snapshot).contains("phone: Do Not Disturb"), "{}", kin_lib::tray::tooltip(&snapshot));
    // Kin's own optional toast (the low-battery warning) is held; the machine disarms, so it does not come later either.
    rig.presence_from_phone(json!({ "dnd": "priority", "battery": { "level": 14, "charging": false } })).await;
    wait_until("the battery", || rig.core().presence_view().phone_battery == Some((14, false))).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(nudges(&rig.events()).is_empty(), "held while the phone is in Do Not Disturb");
    assert_eq!(rig.core().focus_status_json()["counts"]["heldToasts"], 1);
    assert_eq!(
        rig.core().presence_status_json()["presence"]["counts"]["nudges"],
        0,
        "a held nudge was not shown, so it is not counted as shown"
    );
    rig.presence_from_phone(json!({ "battery": { "level": 13, "charging": false } })).await;
    wait_until("DND over", || rig.core().presence_view().phone_dnd.is_none()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(nudges(&rig.events()).is_empty(), "the held nudge is not shown later");
    // With the setting off the toast is shown even in Do Not Disturb; a new episode.
    rig.presence_from_phone(json!({ "dnd": "total", "battery": { "level": 50, "charging": true } })).await;
    rig.core().set_hold_toasts_on_phone_dnd(false);
    rig.presence_from_phone(json!({ "dnd": "total", "battery": { "level": 12, "charging": false } })).await;
    wait_until("the shown nudge", || !nudges(&rig.events()).is_empty()).await;
    assert_eq!(rig.core().presence_status_json()["presence"]["counts"]["nudges"], 1);
    // A wrong type is an error and changes nothing; the connection closing forgets the phone's state.
    rig.presence_from_phone(json!({ "dnd": true })).await;
    let mut got_error = false;
    for _ in 0..10 {
        let f = next_json(&mut rig.ws).await;
        if f["type"] == "error" {
            assert_eq!(f["code"], "bad_request");
            got_error = true;
            break;
        }
    }
    assert!(got_error);
    assert_eq!(rig.core().presence_phone_dnd(), Some(Dnd::Total), "unchanged by a bad frame");
    rig.ws.close(None).await.unwrap();
    wait_until("forgotten", || rig.core().presence_phone_dnd().is_none()).await;
}

/// Review round 1: while this PC is paused its Kin does nothing on its own, so the phone's Do Not Disturb is neither used (no holding) nor shown; it counts again at resume.
#[tokio::test]
async fn the_phones_dnd_is_ignored_while_the_pc_is_paused_and_counts_again_at_resume() {
    let mut rig = Rig::new(&CAPS).await;
    rig.events();
    rig.presence_from_phone(json!({ "dnd": "priority" })).await;
    wait_until("the dnd", || rig.core().presence_phone_dnd() == Some(Dnd::Priority)).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    assert_eq!(rig.core().presence_phone_dnd(), None, "not used while paused");
    assert_eq!(rig.core().presence_view().phone_dnd, None, "not shown while paused");
    assert_eq!(rig.core().presence_panel()["phone"]["dnd"], Value::Null);
    rig.core().set_pause(None);
    assert_eq!(rig.core().presence_phone_dnd(), Some(Dnd::Priority), "kept, and used again after the pause");
    assert_eq!(rig.core().presence_view().phone_dnd, Some(Dnd::Priority));
}

// ----- the 2B.4 review ------------------------------------------------------------------------------------------------------------------

/// Resume race: `was_paused` changes in the same critical section as the pause, so the pump's tick never sees a pause that "ended" while a user's resume is still being
/// written. (Without the fix a tick in that gap ran `pause_ended` and the media were started twice.) Probabilistic by nature: a thread hammers the tick while the main
/// thread pauses and resumes; no "pause: over" line may appear (nothing here expires).
#[tokio::test]
async fn a_resume_never_races_with_the_pump_into_a_second_pause_ended() {
    let rig = Rig::new(&CAPS).await;
    let core = rig.core().clone();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hammer = {
        let (core, stop) = (core.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                core.presence_tick(kin_lib::encoding::unix_ms() as i64);
            }
        })
    };
    let core2 = core.clone();
    tokio::task::spawn_blocking(move || {
        for _ in 0..60 {
            core2.set_pause(Some(PauseChoice::UntilResumed));
            core2.set_pause(None);
        }
    })
    .await
    .unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    hammer.join().unwrap();
    let overs = captured_lines().iter().filter(|l| l.contains("pause: over")).count();
    assert_eq!(overs, 0, "the pump saw a pause end that a user's resume was still writing");
}

#[tokio::test]
async fn a_hello_on_a_socket_of_an_unlinked_phone_sends_no_snapshot() {
    let rig = Rig::new(&CAPS).await;
    let id = rig.core().snapshot().devices[0].device_id.clone();
    wait_until("a presence connection", || rig.core().presence_connection_count() == 1).await;
    rig.core().remove_device(&id).await.unwrap();
    // A `hello` handled after the removal (the socket was still closing): the phone is not paired, so nothing is made for it and nothing is sent.
    rig.core().presence_peer_hello(&id, true);
    assert_eq!(rig.core().presence_connection_count(), 0);
}

#[tokio::test]
async fn an_unsolicited_ring_status_is_ignored_and_one_after_a_press_is_taken() {
    let mut rig = Rig::new(&CAPS).await;
    rig.send(json!({ "type": "ring.status", "id": uuid(), "state": "ringing", "audible": true })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(rig.core().presence_status_json()["ring"]["state"], "idle", "nobody asked, so the PC does not show a ring");
    assert!(!rig.core().presence_view().ring_active);
    assert!(captured_lines().iter().any(|l| l.contains("a status nobody asked for was ignored")));
    // After a press the same status is taken.
    let core = rig.core().clone();
    let start = tokio::spawn(async move { core.ring_start().await });
    let frame = loop {
        let f = next_json(&mut rig.ws).await;
        if f["type"] == "ring.start" {
            break f;
        }
    };
    rig.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    start.await.unwrap();
    rig.send(json!({ "type": "ring.status", "id": uuid(), "state": "ringing", "audible": true })).await;
    wait_until("ringing", || rig.core().presence_status_json()["ring"]["state"] == "ringing").await;
}

#[tokio::test]
async fn on_a_windows_without_focus_sessions_only_quiet_time_is_read_and_the_panel_says_so() {
    let mut rig = Rig::new(&CAPS).await;
    rig.core().set_quiet_source(Arc::new(FakeQuiet::without_focus_sessions(UserState::QuietTime)));
    rig.next_presence(3).await.expect("the snapshot after the hello");
    rig.core().set_focus_share(true);
    wait_until("the source", || rig.core().focus_source_active()).await;
    assert_eq!(rig.core().focus_panel()["sessionsSupported"], false, "Settings says: Windows 10 only reports Do Not Disturb");
    assert_eq!(rig.next_presence(9).await.expect("quiet time is Focus")["focus"], true);
}
