//! Milestone 2B.4 against the real core in the test process, with a fake phone: what the PC sends of its own state and when (the snapshot after the hello, a pause
//! within a second), what a phone's snapshot does here (shown, replaced, forgotten, gating the automatic clipboard), the low-battery nudge and what holds it back,
//! the quiet arrival of manual items while the PC is paused, ring my phone (and that it works while paused, and that the PC makes no sound), auto-sent screenshots
//! and the clipboard formats, unlinking, and that no battery level or pause time reaches a log.
//!
//! Everything listens on 127.0.0.1. The clipboard is the in-memory one: **nothing here touches the real clipboard, and nothing here makes a sound**.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::transfer::*;
use common::*;
use futures_util::StreamExt;
use kin_lib::core::{Core, Event, LinkState};
use kin_lib::presence::battery::FakePower;
use kin_lib::presence::pause::PauseChoice;
use kin_lib::presence::wire::Battery;
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

fn battery(level: u8, charging: bool) -> Option<Battery> {
    Some(Battery { level, charging, saver: None })
}

struct Rig {
    w: World,
    ws: Ws,
}

impl Rig {
    /// A paired phone with these capabilities, a PC whose battery is `power`, no game running, the PC's hello and the phone's hello exchanged.
    async fn new(caps: &[&str], power: Option<Battery>) -> Rig {
        capture_logs();
        let w = World::new().await;
        w.pair().await;
        w.env.core.set_user_state_probe(Arc::new(Fixed(UserState::AcceptsNotifications)));
        w.env.core.set_power_source(Arc::new(FakePower::new(power)));
        let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["type"], "hello");
        assert!(hello["capabilities"].as_array().unwrap().contains(&json!("presence")), "the PC always lists presence");
        assert!(!hello["capabilities"].as_array().unwrap().contains(&json!("ring")), "the PC lists nothing for ring");
        let mut phone = phone_hello();
        phone["capabilities"] = json!(caps);
        send_json(&mut ws, &phone).await;
        w.wait_for_state(LinkState::PhoneConnected).await;
        Rig { w, ws }
    }

    fn core(&self) -> &Arc<Core> {
        &self.w.env.core
    }

    async fn send(&mut self, v: Value) {
        send_json(&mut self.ws, &v).await;
    }

    /// The next frame of this type (others are skipped).
    async fn next_of(&mut self, ty: &str) -> Value {
        for _ in 0..40 {
            let v = next_json(&mut self.ws).await;
            if v["type"] == ty {
                return v;
            }
        }
        panic!("no {ty} frame came");
    }

    /// Nothing of this type comes for `ms` (other frames are skipped).
    async fn none_of(&mut self, ty: &str, ms: u64) {
        let until = tokio::time::Instant::now() + Duration::from_millis(ms);
        loop {
            match tokio::time::timeout_at(until, self.ws.next()).await {
                Err(_) => return,
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    assert_ne!(v["type"], ty, "a {ty} came that should not have: {v}");
                }
                Ok(Some(Ok(_))) => continue,
                Ok(other) => panic!("the connection ended: {other:?}"),
            }
        }
    }

    async fn presence(&mut self, body: Value) {
        let mut v = json!({ "type": "presence", "id": uuid() });
        for (k, val) in body.as_object().unwrap() {
            v[k] = val.clone();
        }
        self.send(v).await;
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
const NO_RING: [&str; 3] = ["link", "media", "presence"];

fn nudges(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToastRedacted { text, .. } if text.contains("% and not charging") => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// ----- the snapshot -------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_pc_sends_its_snapshot_after_the_hello_with_its_battery_and_nothing_when_it_has_none() {
    let mut rig = Rig::new(&NO_RING, battery(62, true)).await;
    let p = rig.next_of("presence").await;
    assert_eq!(p["battery"], json!({ "level": 62, "charging": true }));
    assert!(p.get("pause").is_none(), "not paused: no pause field");
    // A desktop (no battery) sends the empty snapshot, which says "not paused".
    let mut desk = Rig::new(&NO_RING, None).await;
    let p = desk.next_of("presence").await;
    assert!(p.get("battery").is_none() && p.get("pause").is_none(), "{p}");
    // "Share this PC's battery" off: the snapshot has none even on a laptop.
    let mut off = Rig::new(&NO_RING, battery(55, false)).await;
    let first = off.next_of("presence").await;
    assert_eq!(first["battery"]["level"], 55);
    off.core().set_presence_share_battery(false);
    let second = off.next_of("presence").await;
    assert!(second.get("battery").is_none(), "{second}");
    assert_eq!(off.core().presence_status_json()["presence"]["shareBattery"], false);
}

#[tokio::test]
async fn a_phone_without_presence_is_sent_none_and_a_pause_still_works_here() {
    let mut rig = Rig::new(&["link"], battery(62, true)).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    rig.none_of("presence", 1500).await;
    assert!(rig.core().paused(), "the PC's own pause works whatever the phone knows");
    let view = rig.core().presence_view();
    assert!(!view.phone_knows_pause, "the window can say 'update Kin on your phone'");
    assert!(!view.can_ring && view.ring_reason.as_deref().unwrap().contains("needs Kin 0.2.0"), "{:?}", view.ring_reason);
    // A presence from such a phone is dropped silently: nothing is shown, no error comes back.
    rig.presence(json!({ "battery": { "level": 14, "charging": false } })).await;
    rig.none_of("error", 500).await;
    assert!(rig.core().presence_view().phone_battery.is_none());
}

// ----- pause: this PC -------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn pause_goes_to_the_phone_within_a_second_resume_clears_it_and_the_choice_is_kept_in_settings_json() {
    let mut rig = Rig::new(&NO_RING, None).await;
    rig.next_of("presence").await;
    rig.core().set_pause(Some(PauseChoice::OneHour));
    let p = tokio::time::timeout(Duration::from_secs(3), rig.next_of("presence")).await.expect("a pause is told within a second or so");
    let ms = p["pause"]["remainingMs"].as_u64().expect("a one-hour pause is told as the time left");
    assert!((3_590_000..=3_600_000).contains(&ms), "{ms}");
    assert!(rig.core().paused());
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(rig.core().config.data_dir.join("settings.json")).unwrap()).unwrap();
    assert!(saved["pause"]["until"].as_i64().unwrap() > 0, "{saved}");
    // A second pause replaces the first: until resumed.
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    let p = rig.next_of("presence").await;
    assert_eq!(p["pause"], json!({ "untilResumed": true }));
    assert_eq!(rig.core().presence_view().pc_pause_text.as_deref(), Some("until resumed"));
    // Resume.
    rig.core().set_pause(None);
    let p = rig.next_of("presence").await;
    assert!(p.get("pause").is_none(), "{p}");
    assert!(!rig.core().paused());
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(rig.core().config.data_dir.join("settings.json")).unwrap()).unwrap();
    assert!(saved["pause"].is_null());
}

/// A pause that survives a restart: the file is read at the start, and a pause whose time has passed is no pause.
#[tokio::test]
async fn the_pause_is_still_on_after_a_restart_and_an_old_one_is_not() {
    let w = World::new_on_disk().await;
    w.env.core.set_pause(Some(PauseChoice::UntilResumed));
    let w = w.restart().await;
    assert!(w.core().paused(), "until resumed survives a restart");
    let dir = w.env.dir.path().to_path_buf();
    std::fs::write(dir.join("settings.json"), format!(r#"{{ "pause": {{ "until": {} }} }}"#, kin_lib::encoding::unix_ms() as i64 - 1000))
        .unwrap();
    let w = w.restart().await;
    assert!(!w.core().paused(), "an end in the past is simply no pause");
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{ "pause": {{ "until": {} }} }}"#, kin_lib::encoding::unix_ms() as i64 + 9 * 86_400_000),
    )
    .unwrap();
    let w = w.restart().await;
    assert!(!w.core().paused(), "an end more than 8 days ahead is a clock gone wrong: expired");
}

// ----- pause: the phone ------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_phones_snapshot_is_shown_replaced_and_forgotten_with_the_connection_and_its_pause_gates_the_automatic_clipboard() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.next_of("presence").await;
    rig.presence(json!({ "battery": { "level": 62, "charging": true }, "pause": { "remainingMs": 600_000 } })).await;
    wait_until("the snapshot", || rig.core().presence_view().phone_battery == Some((62, true))).await;
    let view = rig.core().presence_view();
    assert!(view.phone_pause.is_some() && view.phone_pause_text.is_some());
    let tooltip = kin_lib::tray::tooltip_with(&rig.core().snapshot(), None);
    assert_eq!(tooltip, "Kin: Test Phone connected \u{b7} 62% charging \u{b7} phone paused");

    // The phone is paused: the PC sends no automatic clipboard (and nothing is queued for it).
    rig.w.env.clipboard.user_copy("auto text while the phone is paused", vec![]);
    rig.core().process_clipboard_change(kin_lib::sync::Tap::now()).await;
    rig.none_of("clipboard", 600).await;

    // A snapshot without a pause replaces it: the phone is back, and the next automatic text goes.
    rig.presence(json!({ "battery": { "level": 61, "charging": true } })).await;
    wait_until("the pause to be forgotten", || rig.core().presence_view().phone_pause.is_none()).await;
    let core = rig.core().clone();
    rig.w.env.clipboard.user_copy("auto text after the phone resumed", vec![]);
    let send = tokio::spawn(async move { core.process_clipboard_change(kin_lib::sync::Tap::now()).await });
    let frame = rig.next_of("clipboard").await;
    assert_eq!(frame["text"], "auto text after the phone resumed");
    rig.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    send.await.unwrap();

    // The connection closing forgets the pause and keeps only the last battery, for the offline line.
    rig.presence(json!({ "battery": { "level": 12, "charging": false }, "pause": { "untilResumed": true } })).await;
    wait_until("the second snapshot", || rig.core().presence_view().phone_pause.is_some()).await;
    let Rig { w, ws } = rig;
    drop(ws);
    w.wait_for_state(LinkState::PhoneOffline).await;
    let view = w.core().presence_view();
    assert!(view.phone_pause.is_none(), "nothing is paused as far as this side knows");
    assert_eq!(view.phone_last_known.map(|k| (k.0, k.1)), Some((12, false)));
    assert!(kin_lib::tray::tooltip_with(&w.core().snapshot(), Some("14:32")).ends_with("last seen 14:32 \u{b7} 12%"));
}

#[tokio::test]
async fn the_switch_show_my_phones_battery_hides_it() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.presence(json!({ "battery": { "level": 62, "charging": true } })).await;
    wait_until("the snapshot", || rig.core().presence_view().phone_battery.is_some()).await;
    rig.core().set_presence_show_phone(false);
    assert!(rig.core().presence_view().phone_battery.is_none());
    assert!(!kin_lib::tray::tooltip_with(&rig.core().snapshot(), None).contains('%'));
}

// ----- the quiet arrival -----------------------------------------------------------------------------------------------------

async fn upload(w: &World, id: &str, name: &str, bytes: &[u8], origin: Option<&str>) {
    let mut c = http(w).await;
    let mut body = json!({ "id": id, "files": [{ "fileId": "f", "name": name, "size": bytes.len(), "mime": "image/png" }] });
    if let Some(o) = origin {
        body["origin"] = json!(o);
    }
    assert_eq!(c.json("POST", "/v1/transfers", &body).await.status, 200);
    assert_eq!(c.put_bytes(id, "f", bytes).await.unwrap().status, 200);
}

fn files_toast_titles(events: &[Event]) -> Vec<String> {
    events.iter().filter_map(|e| if let Event::FilesToast { title, .. } = e { Some(title.clone()) } else { None }).collect()
}

#[tokio::test]
async fn manual_items_arrive_quietly_while_the_pc_is_paused_and_normally_after() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.next_of("presence").await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    rig.events();

    // A text: acked, in the history, and **nothing visible**: no toast, and the clipboard is untouched.
    let text_id = uuid();
    rig.send(json!({ "type": "text", "id": text_id, "text": "quiet words", "isUrl": false })).await;
    assert_eq!(rig.next_of("ack").await["ref"], text_id);
    wait_until("the text in the history", || rig.core().db.text_item(&text_id).unwrap().is_some()).await;
    // A link: acked, in the history, **not opened**, no toast.
    let link_id = uuid();
    rig.send(json!({ "type": "link", "id": link_id, "url": "https://example.com/quiet" })).await;
    assert_eq!(rig.next_of("ack").await["ref"], link_id);
    wait_until("the link in the history", || rig.core().db.history_item(&link_id).unwrap().is_some()).await;
    // A screenshot (an auto-sent file): saved, no toast, nothing on the clipboard.
    upload(&rig.w, "shot-1", "Screenshot_1.png", TINY_PNG, Some("screenshot")).await;
    let dir = rig.core().received_dir();
    wait_until("the file", || dir.join("Screenshot_1.png").exists()).await;
    wait_until("the three quiet arrivals", || rig.core().presence_status_json()["pause"]["quietArrivals"] == 3).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = rig.events();
    let visible: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::TextToast { .. } | Event::LinkToast { .. } | Event::FilesToast { .. } | Event::Toast(_)))
        .collect();
    assert!(visible.is_empty(), "nothing is shown while paused: {visible:?}");
    assert!(
        rig.w.env.clipboard.app_writes().is_empty() && rig.w.env.clipboard.app_sensitive_writes().is_empty(),
        "the clipboard was not written"
    );
    assert!(rig.w.env.clipboard.app_images().is_empty(), "no image on the clipboard");
    assert!(rig.w.env.opener.opened().is_empty(), "no browser");

    // Resume: the counter is reset and everything flows again.
    rig.core().set_pause(None);
    assert_eq!(rig.core().presence_status_json()["pause"]["quietArrivals"], 0);
    let text_id2 = uuid();
    rig.send(json!({ "type": "text", "id": text_id2, "text": "loud words", "isUrl": false })).await;
    rig.next_of("ack").await;
    wait_until("the text on the clipboard", || rig.w.env.clipboard.app_writes() == ["loud words".to_string()]).await;
    wait_until("the toast", || rig.events().iter().any(|e| matches!(e, Event::TextToast { .. }))).await;
}

/// Review round 1, item 3: a sensitive text that comes while paused is never kept and never put on the clipboard, but the window says that one came.
#[tokio::test]
async fn a_sensitive_text_while_paused_is_counted_and_nothing_of_it_is_kept_shown_or_logged() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.next_of("presence").await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    rig.events();

    let id = uuid();
    rig.send(json!({ "type": "text", "id": id, "text": "hunter2-the-secret", "isUrl": false, "sensitive": true })).await;
    assert_eq!(rig.next_of("ack").await["ref"], id, "it is acked like any text");
    wait_until("the count", || rig.core().presence_panel()["pause"]["quietSensitive"] == 1).await;
    // One more, and an ordinary text: the ordinary one is a quiet arrival, the sensitive ones only count.
    rig.send(json!({ "type": "text", "id": uuid(), "text": "another-secret", "isUrl": false, "sensitive": true })).await;
    rig.next_of("ack").await;
    let plain = uuid();
    rig.send(json!({ "type": "text", "id": plain, "text": "plain words", "isUrl": false })).await;
    rig.next_of("ack").await;
    wait_until("both counts", || {
        let p = rig.core().presence_panel();
        p["pause"]["quietSensitive"] == 2 && p["pause"]["quietArrivals"] == 1
    })
    .await;

    assert!(rig.core().db.text_item(&id).unwrap().is_none(), "a sensitive text is never stored");
    assert!(rig.core().db.text_item(&plain).unwrap().is_some());
    assert!(
        rig.w.env.clipboard.app_writes().is_empty() && rig.w.env.clipboard.app_sensitive_writes().is_empty(),
        "the clipboard was not written"
    );
    let visible: Vec<Event> =
        rig.events().into_iter().filter(|e| matches!(e, Event::TextToast { .. } | Event::LinkToast { .. } | Event::Toast(_))).collect();
    assert!(visible.is_empty(), "nothing is shown by itself: {visible:?}");
    for text in [rig.core().presence_panel().to_string(), rig.core().status_json().to_string(), captured_lines().join("\n")] {
        assert!(!text.contains("hunter2") && !text.contains("another-secret"), "the content of a sensitive text leaked");
    }
    assert_eq!(rig.core().status_json()["pause"]["quietSensitive"], 2);

    // A resume resets the counts.
    rig.core().set_pause(None);
    let p = rig.core().presence_panel();
    assert_eq!((p["pause"]["quietSensitive"].as_u64(), p["pause"]["quietArrivals"].as_u64()), (Some(0), Some(0)));
}

/// Review round 1, item 4: the phone's `reason` is text from the phone; only the protocol's words reach the log.
#[tokio::test]
async fn the_rings_stop_reason_is_logged_only_when_it_is_a_known_word() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.send(json!({ "type": "ring.status", "id": uuid(), "state": "stopped", "reason": "timeout" })).await;
    wait_until("the known word in the log", || captured_lines().iter().any(|l| l.contains("stopped (timeout)"))).await;
    rig.send(json!({ "type": "ring.status", "id": uuid(), "state": "stopped", "reason": "free text HOMEWIFI-NAME" })).await;
    wait_until("the unknown word as 'other'", || captured_lines().iter().any(|l| l.contains("stopped (other)"))).await;
    assert!(!captured_lines().iter().any(|l| l.contains("HOMEWIFI-NAME") || l.contains("free text")), "the phone's text is not in the log");
}

// ----- the low-battery nudge --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_low_battery_phone_gets_one_toast_per_episode_and_the_toast_never_names_the_level_in_the_log() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.events();
    rig.presence(json!({ "battery": { "level": 14, "charging": false } })).await;
    wait_until("the nudge", || !nudges(&rig.events()).is_empty()).await;
    // Still low: no second toast. Charging and low again: one more.
    rig.presence(json!({ "battery": { "level": 13, "charging": false } })).await;
    rig.presence(json!({ "battery": { "level": 40, "charging": true } })).await;
    rig.presence(json!({ "battery": { "level": 15, "charging": false } })).await;
    wait_until("the second episode", || !nudges(&rig.events()).is_empty()).await;
    assert!(!captured_lines().iter().any(|l| l.contains("14%") || l.contains("is at 15")), "the level is not logged");
    assert_eq!(rig.core().presence_status_json()["presence"]["counts"]["nudges"], 2);
}

#[tokio::test]
async fn a_pause_a_game_or_the_switch_holds_the_nudge_back_without_using_it_up() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.events();
    // A fullscreen game: no toast over it.
    rig.core().set_user_state_probe(Arc::new(Fixed(UserState::RunningD3dFullScreen)));
    rig.presence(json!({ "battery": { "level": 14, "charging": false } })).await;
    wait_until("the snapshot", || rig.core().presence_view().phone_battery == Some((14, false))).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(nudges(&rig.events()).is_empty(), "a game is never interrupted");
    // The game is over: the next snapshot nudges (the machine was not disarmed).
    rig.core().set_user_state_probe(Arc::new(Fixed(UserState::AcceptsNotifications)));
    rig.presence(json!({ "battery": { "level": 13, "charging": false } })).await;
    wait_until("the held nudge", || !nudges(&rig.events()).is_empty()).await;
    // A pause: held back too, and the switch off: never.
    rig.presence(json!({ "battery": { "level": 50, "charging": true } })).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    rig.presence(json!({ "battery": { "level": 14, "charging": false } })).await;
    wait_until("the paused snapshot", || rig.core().presence_view().phone_battery == Some((14, false))).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(nudges(&rig.events()).is_empty(), "no toast while paused");
    rig.core().set_pause(None);
    rig.core().set_low_battery_nudge(false);
    rig.presence(json!({ "battery": { "level": 13, "charging": false } })).await;
    wait_until("the snapshot", || rig.core().presence_view().phone_battery == Some((13, false))).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(nudges(&rig.events()).is_empty(), "the switch is off");
}

#[tokio::test]
async fn a_burst_of_presence_is_dropped_after_ten_with_one_rate_limited_error() {
    let mut rig = Rig::new(&CAPS, None).await;
    for i in 0..14 {
        rig.presence(json!({ "battery": { "level": 50 + (i % 2), "charging": false } })).await;
    }
    let e = rig.next_of("error").await;
    assert_eq!(e["code"], "rate_limited");
    assert!(e.get("ref").is_none(), "no ref");
    rig.none_of("error", 700).await;
    wait_until("the counts", || rig.core().presence_status_json()["presence"]["counts"]["droppedRate"].as_u64().unwrap_or(0) >= 4).await;
}

// ----- ring ----------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn ring_my_phone_asks_the_phone_waits_for_its_status_stops_on_request_and_works_while_paused() {
    let mut rig = Rig::new(&CAPS, None).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    assert!(rig.core().presence_view().can_ring, "a phone that lists ring can be rung, paused or not");
    let core = rig.core().clone();
    let start = tokio::spawn(async move { core.ring_start().await });
    let frame = rig.next_of("ring.start").await;
    assert!(frame["id"].as_str().unwrap().len() == 36);
    rig.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    assert_eq!(start.await.unwrap(), "Ringing Test Phone", "an ack alone is not 'ringing'");
    rig.send(json!({ "type": "ring.status", "id": uuid(), "state": "ringing", "audible": true })).await;
    wait_until("ringing", || rig.core().presence_view().ring_text.as_deref() == Some("Test Phone is ringing")).await;
    assert!(rig.core().presence_view().ring_active);
    assert_eq!(rig.core().presence_status_json()["ring"]["state"], "ringing");

    // Stop: a ring.stop goes, the PC stops showing "ringing" at once, the phone's status says why.
    let core = rig.core().clone();
    let stop = tokio::spawn(async move { core.ring_stop().await });
    let frame = rig.next_of("ring.stop").await;
    rig.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    stop.await.unwrap();
    assert!(!rig.core().presence_view().ring_active);
    rig.send(json!({ "type": "ring.status", "id": uuid(), "state": "stopped", "reason": "pc" })).await;
    wait_until("stopped", || rig.core().presence_status_json()["ring"]["state"] == "stopped").await;
    assert_eq!(rig.core().presence_status_json()["ring"]["counts"]["started"], 1);
    // The PC never makes a sound and never touches the volume: the only things that went out were messages.
    assert!(rig.w.env.clipboard.app_writes().is_empty());
}

#[tokio::test]
async fn ring_is_refused_at_most_once_every_10_s_and_the_phones_errors_have_words() {
    let mut rig = Rig::new(&CAPS, None).await;
    // The phone answers `disabled`: the user is told it is turned off there.
    let core = rig.core().clone();
    let first = tokio::spawn(async move { core.ring_start().await });
    let frame = rig.next_of("ring.start").await;
    rig.send(json!({ "type": "error", "ref": frame["id"], "code": "disabled", "message": "off" })).await;
    assert_eq!(first.await.unwrap(), "Ringing is turned off on Test Phone");
    // A second press inside 10 s is not sent at all.
    let text = rig.core().ring_start().await;
    assert!(text.starts_with("Wait ") && text.ends_with("before ringing Test Phone again"), "{text}");
    rig.none_of("ring.start", 400).await;
}

#[tokio::test]
async fn ring_with_no_answer_is_told_so_and_never_sent_again() {
    let mut rig = Rig::new(&CAPS, None).await;
    let core = rig.core().clone();
    let first = tokio::spawn(async move { core.ring_start().await });
    rig.next_of("ring.start").await;
    // The phone says nothing: after 5 s the user hears "no answer", the connection stays open, and nothing is re-sent.
    let text = first.await.unwrap();
    assert_eq!(text, "No answer from Test Phone. It may be asleep or off Wi-Fi.");
    assert_eq!(rig.w.core().snapshot().state, LinkState::PhoneConnected, "a missing ack to a ring does not close the connection");
    rig.none_of("ring.start", 300).await;
}

#[tokio::test]
async fn ring_is_not_offered_when_the_phone_does_not_list_it_and_says_what_to_do() {
    let mut rig = Rig::new(&NO_RING, None).await;
    let view = rig.core().presence_view();
    assert!(!view.can_ring);
    assert_eq!(view.ring_reason.as_deref(), Some("Ringing isn't available: turn it on in Kin on Test Phone"));
    let text = rig.core().ring_start().await;
    assert!(text.contains("turn it on in Kin on Test Phone"), "{text}");
    rig.none_of("ring.start", 400).await;
    // The phone turns ringing on with a new hello: the button is there.
    let mut hello = phone_hello();
    hello["capabilities"] = json!(CAPS);
    rig.send(hello).await;
    wait_until("the ring capability", || rig.core().presence_view().can_ring).await;
}

// ----- auto-sent screenshots -----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_screenshot_goes_on_the_clipboard_with_the_sensitive_formats_and_a_camera_photo_never() {
    let mut rig = Rig::new(&CAPS, None).await;
    let dir = rig.core().received_dir();
    rig.events();
    upload(&rig.w, "s1", "Screenshot_a.png", TINY_PNG, Some("screenshot")).await;
    wait_until("the screenshot on the clipboard", || rig.w.env.clipboard.app_sensitive_images() == [dir.join("Screenshot_a.png")]).await;
    assert_eq!(rig.w.env.clipboard.app_images(), [dir.join("Screenshot_a.png")]);
    wait_until("the toast", || !files_toast_titles(&rig.events()).is_empty()).await;
    upload(&rig.w, "s2", "Screenshot_b.png", TINY_PNG, Some("screenshot")).await;
    wait_until("the second screenshot's toast", || {
        rig.events().iter().any(|e| matches!(e, Event::FilesToast { title, .. } if title == "Screenshot from Test Phone"))
    })
    .await;

    // A camera photo: "Photo from", saved, never on the clipboard.
    upload(&rig.w, "c1", "IMG_1.png", TINY_PNG, Some("camera")).await;
    wait_until("the photo's toast", || {
        rig.events().iter().any(|e| matches!(e, Event::FilesToast { title, .. } if title == "Photo from Test Phone"))
    })
    .await;
    // A manual one (no origin) or an origin nobody knows: as before, "1 file from", an image on the clipboard but not with the sensitive formats.
    upload(&rig.w, "m1", "manual.png", TINY_PNG, None).await;
    upload(&rig.w, "m2", "selfie.png", TINY_PNG, Some("selfie")).await;
    wait_until("the manual ones", || rig.w.env.clipboard.app_images().len() == 4).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rig.w.env.clipboard.app_sensitive_images(),
        [dir.join("Screenshot_a.png"), dir.join("Screenshot_b.png")],
        "only the screenshots carry the sensitive formats; the camera photo is not on the clipboard at all"
    );
    assert!(!rig.w.env.clipboard.app_images().contains(&dir.join("IMG_1.png")), "a camera photo never goes to the clipboard");
    // With "images also go to the clipboard" off, a screenshot is still shown and saved, and not copied.
    rig.core().set_images_to_clipboard(false);
    upload(&rig.w, "s3", "Screenshot_c.png", TINY_PNG, Some("screenshot")).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!rig.w.env.clipboard.app_images().contains(&dir.join("Screenshot_c.png")));
    // The origin is kept with the transfer.
    assert_eq!(rig.core().db.transfer_origin("s1").as_deref(), Some("screenshot"));
    assert_eq!(rig.core().db.transfer_origin("m1"), None);
    assert_eq!(rig.core().db.transfer_origin("m2"), None, "an unknown origin is absent");
}

// ----- unlinking -------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn unlinking_forgets_the_phones_battery_ring_and_overrides_and_clears_a_pause_with_no_phone_left() {
    let mut rig = Rig::new(&CAPS, None).await;
    let id = rig.w.phone.device_id.clone();
    rig.presence(json!({ "battery": { "level": 30, "charging": false } })).await;
    wait_until("the snapshot", || rig.core().presence_view().phone_battery.is_some()).await;
    rig.core().set_feature_override(&id, kin_lib::features::MEDIA_SHOW, Some(true));
    assert!(rig.core().feature_enabled(kin_lib::features::MEDIA_SHOW, Some(&id)), "the override beats the default (off)");
    assert!(!rig.core().feature_enabled(kin_lib::features::MEDIA_SHOW, None));
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    assert!(rig.core().remove_device(&id).await.unwrap());
    assert!(!rig.core().feature_enabled(kin_lib::features::MEDIA_SHOW, Some(&id)), "its overrides are deleted");
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(rig.core().config.data_dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["featureOverrides"], json!({}));
    assert!(!rig.core().paused(), "a pause belongs to a PC that has someone to pause for");
    let view = rig.core().presence_view();
    assert!(view.phone_battery.is_none() && view.phone_last_known.is_none() && !view.can_ring);
}

// ----- what is written down ----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn nothing_about_the_phones_battery_or_pause_is_in_a_log_and_status_json_has_numbers() {
    let mut rig = Rig::new(&CAPS, battery(91, true)).await;
    rig.presence(json!({ "battery": { "level": 73, "charging": false }, "pause": { "remainingMs": 123_456 } })).await;
    wait_until("the snapshot", || rig.core().presence_view().phone_battery == Some((73, false))).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let status = rig.core().status_json();
    assert_eq!(status["presence"]["phone"]["level"], 73, "status.json is for scripts: numbers are fine");
    assert_eq!(status["presence"]["phone"]["paused"], true);
    assert_eq!(status["pause"]["on"], true);
    let text = status.to_string();
    assert!(!text.contains("123456"), "no pause time of the phone");
    for line in captured_lines() {
        assert!(!line.contains("73%") && !line.contains("level 73") && !line.contains("\"level\":73"), "a level in a log line: {line}");
        assert!(!line.contains("123456") && !line.contains("remainingMs"), "a pause time in a log line: {line}");
    }
}
