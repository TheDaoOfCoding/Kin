//! Milestone 2B.3 against the real core in the test process, with a fake phone and fakes in place of Windows (no GSMTC, no flyout, no Core Audio): what the PC sends
//! of its own media and when, what a phone's commands do (and that they reach **only the test session** and the **fake volume**), what a phone's session looks like
//! in the flyout (never `Playing` in a test run), what a press in the flyout sends back, the switches, and that nothing about what plays reaches a log or
//! `status.json`.
//!
//! Everything listens on 127.0.0.1. Time is real (the debounce of a read is 300 ms), so a test waits a moment for what it expects.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures_util::StreamExt;
use kin_lib::core::{Core, LinkState};
use kin_lib::media::guard::Mode;
use kin_lib::media::map::{Controls, SmtcStatus};
use kin_lib::media::reader::{Control, FakeBackend, SessionInfo};
use kin_lib::media::surface::{Button, FakeSurface, SurfaceEvent};
use kin_lib::media::volume::{FakeVolume, VolumeEndpoint};
use kin_lib::media::wire::{self, VolumeOp, sid_of};
use serde_json::{Value, json};

const TEST: &str = "app.kinrelay.dev.mediatest-77";
const REAL_EDGE: &str = "MSEdge";
const REAL_SPOTIFY: &str = "Spotify.exe";
const TITLE: &str = "Secret Song 7f3a";
const ARTIST: &str = "Artist X 91c2";

fn uuid() -> String {
    kin_lib::encoding::uuid_v7(kin_lib::encoding::unix_ms())
}

fn controls(prev: bool) -> Controls {
    Controls { play: true, pause: true, toggle: true, next: true, prev, position: true }
}

fn session(aumid: &str, title: &str, status: i32, prev: bool) -> SessionInfo {
    SessionInfo {
        aumid: aumid.to_string(),
        status,
        controls: controls(prev),
        duration_ms: Some(200_000),
        position_ms: 5000,
        rate_milli: Some(1000),
        title: title.to_string(),
        artist: ARTIST.to_string(),
        album: String::new(),
    }
}

struct Rig {
    w: World,
    ws: Ws,
    backend: Arc<FakeBackend>,
    surface: Arc<FakeSurface>,
    volume: Arc<FakeVolume>,
}

impl Rig {
    /// A paired phone that lists `media`, a PC with both switches on and the fakes in place. The PC's own `media.status` has been read.
    async fn new() -> Rig {
        capture_logs();
        let w = World::new().await;
        w.pair().await;
        let backend = Arc::new(FakeBackend::default());
        let surface = Arc::new(FakeSurface::default());
        let volume = Arc::new(FakeVolume::new(50, 51));
        w.env.core.media_for_tests(
            backend.clone(),
            Mode::Test { aumid: Some(TEST.to_string()), real_volume: false },
            Some(surface.clone()),
            volume.clone(),
        );
        let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["type"], "hello");
        assert_eq!(
            hello["capabilities"],
            json!(["link", "media", "presence"]),
            "the PC always lists media and presence (and notif only while its switch is on)"
        );
        let mut phone = phone_hello();
        phone["capabilities"] = json!(["link", "media"]);
        send_json(&mut ws, &phone).await;
        w.wait_for_state(LinkState::PhoneConnected).await;
        let mut rig = Rig { w, ws, backend, surface, volume };
        let status = rig.next_of("media.status").await;
        assert_eq!(
            (status["show"].as_bool(), status["share"].as_bool()),
            (Some(true), Some(true)),
            "the PC tells its switches after the hello"
        );
        rig
    }

    fn core(&self) -> &Arc<Core> {
        &self.w.env.core
    }

    async fn send(&mut self, v: Value) {
        send_json(&mut self.ws, &v).await;
    }

    /// The phone says what it shows and shares.
    async fn phone_status(&mut self, show: bool, share: bool) {
        self.send(json!({ "type": "media.status", "id": uuid(), "show": show, "share": share, "access": "granted" })).await;
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

    async fn wait_for(&self, what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what} never happened");
    }

    /// Nothing goes to the phone for a moment.
    async fn silent(&mut self) {
        loop {
            match tokio::time::timeout(Duration::from_millis(700), self.ws.next()).await {
                Err(_) => return,
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(_) | tokio_tungstenite::tungstenite::Message::Pong(_)))) => {
                    continue;
                }
                Ok(other) => panic!("the PC sent something it should not have: {other:?}"),
            }
        }
    }

    fn set_sessions(&self, sessions: Vec<SessionInfo>) {
        self.backend.set(sessions.into_iter().map(|s| (s, None)).collect());
        self.core().media_changed();
    }

    /// The phone's session on the PC (the PC is the viewer).
    async fn phone_plays(&mut self, extra: Value) {
        let mut frame = json!({
            "type": "media.state", "id": uuid(), "sid": sid_of("com.spotify.music"), "app": "Spotify", "state": "playing",
            "title": "Phone Song 3c9d", "artist": "Phone Artist", "album": "Phone Album", "durationMs": 337000, "positionMs": 42000,
            "actions": ["play", "pause", "next", "prev", "seek"]
        });
        for (k, v) in extra.as_object().unwrap() {
            frame[k] = v.clone();
        }
        self.send(frame).await;
    }
}

// ----- the PC's media to the phone -------------------------------------------------------------------------------------------

#[tokio::test]
async fn nothing_is_sent_before_the_phone_says_it_shows_and_then_the_session_arrives_afresh() {
    let mut rig = Rig::new().await;
    rig.set_sessions(vec![session(TEST, TITLE, 4, true)]);
    // The reader looked and the engine knows the session, but the phone has not said `show`: it gets nothing.
    rig.wait_for("the engine to see the session", || rig.core().media_view()["pcMedia"] == "playing").await;
    rig.silent().await;
    rig.phone_status(true, false).await;
    let state = rig.next_of("media.state").await;
    assert_eq!(state["sid"], sid_of(TEST));
    assert_eq!((state["title"].as_str(), state["artist"].as_str(), state["state"].as_str()), (Some(TITLE), Some(ARTIST), Some("playing")));
    assert_eq!(state["actions"], json!(["play", "pause", "toggle", "next", "prev", "seek"]));
    assert_eq!(state["durationMs"], 200_000);
    assert!(state["positionMs"].as_u64().unwrap() >= 5000);
    assert_eq!(state["app"], TEST, "an app with no known name is shown by its id, which for a test session is its own");
    assert!(state["id"].as_str().is_some_and(|i| !i.is_empty()));
    let counters = rig.core().media_counters();
    assert_eq!(counters.states_sent, 1);
}

#[tokio::test]
async fn a_session_without_a_picture_is_asked_for_it_twice_at_most() {
    let mut rig = Rig::new().await;
    rig.set_sessions(vec![session(TEST, TITLE, 4, true)]);
    rig.phone_status(true, false).await;
    let state = rig.next_of("media.state").await;
    assert!(state["artHash"].is_null(), "the session has no thumbnail");
    // The first look, the one more after 1.5 s, and then no more (a player with no picture is not asked again and again).
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let asked = rig.backend.calls().iter().filter(|c| c.starts_with("thumbnail")).count();
    assert_eq!(asked, 2, "{:?}", rig.backend.calls());
    // The same track is not asked about again when something else changes.
    rig.set_sessions(vec![SessionInfo { position_ms: 90_000, ..session(TEST, TITLE, 4, true) }]);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(rig.backend.calls().iter().filter(|c| c.starts_with("thumbnail")).count(), 2);
}

#[tokio::test]
async fn a_real_session_is_never_read_or_controlled_in_a_test_run() {
    let mut rig = Rig::new().await;
    rig.backend.set(vec![
        (session(REAL_EDGE, "Real Edge Title", 4, true), Some(vec![1, 2, 3])),
        (session(REAL_SPOTIFY, "Real Spotify Title", 4, true), None),
        (session(TEST, TITLE, 4, true), None),
    ]);
    rig.core().media_changed();
    rig.phone_status(true, true).await;
    let state = rig.next_of("media.state").await;
    assert_eq!(state["title"], TITLE, "only the test session is looked at");
    // The phone tries to control the real ones: their `sid`s were never announced, so it is `stale`, and the backend is not asked.
    for real in [REAL_EDGE, REAL_SPOTIFY] {
        rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid_of(real), "cmd": "next" })).await;
        let e = rig.next_of("error").await;
        assert_eq!(e["code"], "stale");
    }
    for call in rig.backend.calls() {
        assert!(
            !call.contains("MSEdge") && !call.contains("Spotify") || call == "list ",
            "the backend was asked for a real session: {call}"
        );
    }
    assert!(rig.backend.controls().is_empty(), "no command reached a session: {:?}", rig.backend.controls());
}

#[tokio::test]
async fn a_commands_from_the_phone_reach_the_session_it_names_and_have_no_reply_on_success() {
    let mut rig = Rig::new().await;
    rig.set_sessions(vec![session(TEST, TITLE, 4, false)]);
    rig.phone_status(true, true).await;
    let state = rig.next_of("media.state").await;
    let sid = state["sid"].as_str().unwrap().to_string();
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid, "cmd": "toggle" })).await;
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid, "cmd": "seek", "seekMs": 90_000 })).await;
    rig.wait_for("the commands", || rig.backend.controls().len() == 2).await;
    assert_eq!(rig.backend.controls(), [(TEST.to_string(), Control::Toggle), (TEST.to_string(), Control::SeekTo(90_000))]);
    // `prev` is not offered by this session: `stale`, with the id of the command as `ref`.
    let id = uuid();
    rig.send(json!({ "type": "media.cmd", "id": id, "sid": sid, "cmd": "prev" })).await;
    let e = rig.next_of("error").await;
    assert_eq!((e["code"].as_str(), e["ref"].as_str()), (Some("stale"), Some(id.as_str())));
    // Another `sid` is `stale` too.
    let id = uuid();
    rig.send(json!({ "type": "media.cmd", "id": id, "sid": sid_of("someone.else"), "cmd": "next" })).await;
    assert_eq!(rig.next_of("error").await["code"], "stale");
    // The player refuses: `action_failed`.
    *rig.backend.refuse.lock().unwrap() = true;
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid, "cmd": "next" })).await;
    assert_eq!(rig.next_of("error").await["code"], "action_failed");
    assert_eq!(rig.backend.controls().len(), 3);
}

#[tokio::test]
async fn a_command_with_nothing_playing_is_not_found_and_a_source_with_share_off_says_disabled() {
    let mut rig = Rig::new().await;
    rig.phone_status(true, true).await;
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid_of(TEST), "cmd": "next" })).await;
    assert_eq!(rig.next_of("error").await["code"], "not_found");
    // Share off: one `media.clear` to a phone that shows, then `disabled` for a command, and nothing else.
    rig.core().set_media_share(false);
    let clear = rig.next_of("media.clear").await;
    assert!(clear["id"].is_string());
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid_of(TEST), "cmd": "next" })).await;
    let status = rig.next_of("media.status").await;
    assert_eq!((status["share"].as_bool(), status["show"].as_bool()), (Some(false), Some(true)));
    assert_eq!(rig.next_of("error").await["code"], "disabled");
    assert!(rig.backend.controls().is_empty());
}

#[tokio::test]
async fn the_volume_command_changes_the_fake_and_the_new_level_comes_back() {
    let mut rig = Rig::new().await;
    rig.phone_status(true, true).await;
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "cmd": "volume", "level": 40 })).await;
    let v = rig.next_of("media.volume").await;
    assert_eq!((v["level"].as_u64(), v["muted"].as_bool()), (Some(40), Some(false)));
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "cmd": "volume", "delta": 5 })).await;
    let v = rig.next_of("media.volume").await;
    assert_eq!(v["level"], 46, "one step of 2 % is moved at least; 5 % is three");
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "cmd": "volume", "mute": true })).await;
    let v = rig.next_of("media.volume").await;
    assert_eq!(v["muted"], true);
    assert_eq!(rig.volume.writes(), [VolumeOp::Level(40), VolumeOp::Level(46), VolumeOp::Mute(true)]);
    assert_eq!(rig.volume.get().unwrap().level, 46);
}

#[tokio::test]
async fn the_commands_a_phone_may_send_are_limited_to_twenty_and_one_error() {
    let mut rig = Rig::new().await;
    rig.set_sessions(vec![session(TEST, TITLE, 4, true)]);
    rig.phone_status(true, true).await;
    let state = rig.next_of("media.state").await;
    let sid = state["sid"].as_str().unwrap().to_string();
    for _ in 0..30 {
        rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid, "cmd": "toggle" })).await;
    }
    let e = rig.next_of("error").await;
    assert_eq!(e["code"], "rate_limited");
    assert!(e["ref"].is_string(), "with the ref of the command that went over");
    rig.wait_for("the accepted commands", || rig.backend.controls().len() >= 20).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let n = rig.backend.controls().len();
    assert!((20..=22).contains(&n), "20 in the bucket plus what refilled meanwhile: {n}");
    rig.silent().await;
}

// ----- the phone's media on the PC ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_phones_session_shows_in_the_flyout_and_never_as_playing_in_a_test_run() {
    let mut rig = Rig::new().await;
    rig.phone_status(true, true).await;
    rig.phone_plays(json!({})).await;
    rig.wait_for("the flyout", || rig.surface.current().is_some()).await;
    let view = rig.surface.current().unwrap();
    assert_eq!((view.title.as_str(), view.artist.as_str()), ("Phone Song 3c9d", "Phone Artist"));
    assert_eq!(view.album_line, format!("{PHONE_NAME} \u{b7} Spotify"));
    assert_eq!(view.buttons.status, SmtcStatus::Paused, "the phone plays, but a test run never shows Playing");
    assert!(view.buttons.pause && view.buttons.next && view.buttons.prev && view.buttons.play);
    let t = view.timeline.expect("a duration and a position");
    assert_eq!((t.duration_ms, t.seek), (337_000, true));
    assert!(t.position_ms >= 42_000 && t.position_ms < 50_000);
    assert_eq!(rig.core().media_tray_state(), Some("playing"), "the tray says the state, never the title");
    // A clear removes it.
    rig.send(json!({ "type": "media.clear", "id": uuid() })).await;
    rig.wait_for("the flyout to clear", || rig.surface.current().is_none()).await;
    assert_eq!(rig.core().media_tray_state(), None);
    // Back, then the connection closes: it goes with it.
    rig.phone_plays(json!({})).await;
    rig.wait_for("the flyout", || rig.surface.current().is_some()).await;
    drop(rig.ws);
    for _ in 0..100 {
        if rig.surface.current().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("the flyout still shows the phone's session after the connection closed");
}

#[tokio::test]
async fn a_picture_and_the_volume_of_the_phones_session_arrive_and_a_bad_picture_is_dropped() {
    let mut rig = Rig::new().await;
    rig.phone_status(true, true).await;
    let art = fixture("media-art-16.png");
    let hash = wire::hash_of(&art);
    rig.send(wire::image_frame(&uuid(), wire::ImageKind::Art, &art)).await;
    rig.phone_plays(json!({ "artHash": hash, "volume": { "level": 35, "muted": false } })).await;
    rig.wait_for("the flyout", || rig.surface.current().is_some()).await;
    assert_eq!(rig.surface.current().unwrap().thumbnail.as_deref().map(Vec::as_slice), Some(art.as_slice()));
    assert_eq!(rig.core().media_panel()["phone"]["volume"]["level"], 35);
    rig.send(json!({ "type": "media.volume", "id": uuid(), "level": 80, "muted": true })).await;
    rig.wait_for("the volume", || rig.core().media_panel()["phone"]["volume"]["level"] == 80).await;
    // A picture whose hash is wrong is dropped silently and counted.
    let mut bad = wire::image_frame(&uuid(), wire::ImageKind::Art, &art);
    bad["hash"] = json!("0".repeat(32));
    rig.send(bad).await;
    rig.wait_for("the drop to be counted", || rig.core().media_counters().dropped_by_receiver >= 1).await;
}

/// 2B.4: a pause acts as both switches off for as long as it lasts: the PC's source sends one `media.clear` and says so in `media.status`, and what the phone
/// showed here is taken down and the rest is dropped.
#[tokio::test]
async fn a_pause_takes_the_phones_media_down_and_stops_sharing_the_pcs() {
    use kin_lib::presence::pause::PauseChoice;
    let mut rig = Rig::new().await;
    rig.phone_status(true, true).await;
    rig.phone_plays(json!({})).await;
    rig.wait_for("the flyout", || rig.surface.current().is_some()).await;
    rig.core().set_pause(Some(PauseChoice::UntilResumed));
    // The source: one `media.clear`, and the new status says it neither shows nor shares.
    let _clear = rig.next_of("media.clear").await;
    let status = rig.next_of("media.status").await;
    assert_eq!((status["show"].as_bool(), status["share"].as_bool()), (Some(false), Some(false)));
    // The viewer: the flyout session is gone, and what the phone still sends is dropped.
    rig.wait_for("the flyout to go", || rig.surface.current().is_none()).await;
    rig.phone_plays(json!({})).await;
    rig.wait_for("the drop to be counted", || rig.core().media_counters().dropped_viewer_off >= 1).await;
    assert!(rig.surface.current().is_none());
    assert!(!rig.core().media_share_on() && !rig.core().media_show_on(), "a pause acts as the switches off");
}

#[tokio::test]
async fn a_viewer_with_show_off_drops_what_arrives_silently() {
    let mut rig = Rig::new().await;
    rig.phone_status(true, true).await;
    rig.core().set_media_show(false);
    let status = rig.next_of("media.status").await;
    assert_eq!(status["show"], false, "the PC says so on the open connection, without a new hello");
    rig.phone_plays(json!({})).await;
    rig.wait_for("the drop to be counted", || rig.core().media_counters().dropped_viewer_off >= 1).await;
    assert!(rig.surface.current().is_none());
    rig.silent().await;
}

#[tokio::test]
async fn a_press_in_the_flyout_is_a_command_to_the_phone_when_it_shares_and_not_when_it_doesnt() {
    let mut rig = Rig::new().await;
    rig.phone_status(true, false).await;
    rig.phone_plays(json!({})).await;
    rig.wait_for("the flyout", || rig.surface.current().is_some()).await;
    // The phone does not share: nothing goes.
    rig.core().media_surface_event(SurfaceEvent::Button(Button::Next)).await;
    rig.silent().await;
    rig.phone_status(true, true).await;
    rig.wait_for("the PC to learn the phone shares", || {
        !rig.core().media_view()["phones"][0]["status"].as_str().unwrap().contains("isn't sharing")
    })
    .await;
    rig.core().media_surface_event(SurfaceEvent::Button(Button::Play)).await;
    let cmd = rig.next_of("media.cmd").await;
    assert_eq!((cmd["cmd"].as_str(), cmd["sid"].as_str()), (Some("play"), Some(sid_of("com.spotify.music").as_str())));
    rig.core().media_surface_event(SurfaceEvent::Button(Button::Prev)).await;
    assert_eq!(rig.next_of("media.cmd").await["cmd"], "prev");
    // A dragged bar is one command at once and the last value after 150 ms.
    for ms in [10_000u64, 20_000, 30_000] {
        rig.core().media_surface_event(SurfaceEvent::Seek(ms)).await;
    }
    let first = rig.next_of("media.cmd").await;
    assert_eq!((first["cmd"].as_str(), first["seekMs"].as_u64()), (Some("seek"), Some(10_000)));
    let last = rig.next_of("media.cmd").await;
    assert_eq!(last["seekMs"], 30_000, "the last of the burst");
    rig.silent().await;
    // The developer command is the same thing.
    rig.core().media_press("toggle");
    assert_eq!(rig.next_of("media.cmd").await["cmd"], "pause", "the phone's session is playing and lists no toggle: pause");
}

// ----- the switches, the numbers and privacy --------------------------------------------------------------------------------------

/// Found by the real-Windows check: at the start of the app the media is started from a thread that has no Tokio runtime (the app hands one over instead), and a
/// `tokio::task::spawn_blocking` there panics. This starts it the same way, with the real reader (it lists the AUMIDs of the sessions, which a test run filters out
/// before it asks for anything else: it names no session, so it reads none).
#[test]
fn starting_the_media_from_a_thread_with_no_runtime_does_not_panic_and_a_test_run_reads_nothing() {
    capture_logs();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let world = rt.block_on(World::new());
    let core = world.env.core.clone();
    core.media_runtime_for_tests(rt.handle().clone());
    assert!(tokio::runtime::Handle::try_current().is_err(), "this thread has no runtime, like the app's setup");
    core.set_media_share(true);
    let view = core.media_view();
    assert_eq!(view["share"], true);
    assert_eq!(view["mode"], "test");
    assert!(view["reading"] == true || view["readerUnavailable"].is_string(), "{view}");
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(core.media_view()["pcMedia"], "none", "no test session was named: nothing is read, nothing is shared");
    core.set_media_share(false);
    core.media_stop();
    drop(world);
}

#[tokio::test]
async fn both_directions_are_off_by_default_and_the_pc_still_lists_the_capability() {
    capture_logs();
    let w = World::new().await;
    let core = w.core();
    assert!(!core.media_share_on() && !core.media_show_on());
    assert_eq!(core.capabilities(), ["link", "media", "presence"]);
    assert_eq!(core.media_view()["share"], false);
    assert_eq!(core.media_view()["show"], false);
    assert_eq!(core.media_view()["reading"], false, "nothing is read until share is on");
    assert_eq!(core.media_view()["mode"], "test", "a debug build is never the installed Kin");
}

#[tokio::test]
async fn the_pc_tells_a_phone_its_switches_and_a_phone_that_never_does_is_waiting_in_the_settings() {
    let mut rig = Rig::new().await;
    let lines = rig.core().media_view()["phones"].clone();
    assert_eq!(lines[0]["status"], "waiting for the phone");
    rig.send(json!({ "type": "media.status", "id": uuid(), "show": false, "share": false, "access": "missing" })).await;
    rig.wait_for("the status", || rig.core().media_view()["phones"][0]["status"] != "waiting for the phone").await;
    let line = rig.core().media_view()["phones"][0]["status"].as_str().unwrap().to_string();
    assert!(line.contains("isn't showing your media") && line.contains("isn't sharing"), "{line}");
}

#[tokio::test]
async fn nothing_about_what_plays_reaches_a_log_or_status_json() {
    let mut rig = Rig::new().await;
    rig.set_sessions(vec![session(TEST, TITLE, 4, true)]);
    rig.phone_status(true, true).await;
    rig.next_of("media.state").await;
    rig.phone_plays(json!({})).await;
    rig.wait_for("the flyout", || rig.surface.current().is_some()).await;
    rig.send(json!({ "type": "media.cmd", "id": uuid(), "sid": sid_of(TEST), "cmd": "toggle" })).await;
    rig.wait_for("the command", || !rig.backend.controls().is_empty()).await;
    let status = rig.core().status_json().to_string();
    let log = captured_lines().join("\n");
    for secret in [TITLE, ARTIST, "Phone Song 3c9d", "Phone Artist", "Phone Album", TEST, "com.spotify.music"] {
        assert!(!log.contains(secret), "the log has {secret:?}");
        assert!(!status.contains(secret), "status.json has {secret:?}");
    }
    // What is in status.json is numbers and words.
    let media = &rig.core().status_json()["media"];
    assert_eq!(media["view"]["pcMedia"], "playing");
    assert!(media["counts"]["sent"]["states"].as_u64().unwrap() >= 1);
    let data_dir_files: Vec<_> = std::fs::read_dir(rig.w.env.dir.path()).unwrap().filter_map(Result::ok).collect();
    for f in data_dir_files {
        if f.path().is_file() && f.metadata().unwrap().len() < 5_000_000 {
            let bytes = std::fs::read(f.path()).unwrap();
            for secret in [TITLE, ARTIST, "Phone Song 3c9d"] {
                assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "{} holds {secret:?}", f.path().display());
            }
        }
    }
}
