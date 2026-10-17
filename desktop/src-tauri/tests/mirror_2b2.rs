//! Milestone 2B.2 against the real core in the test process, with a fake phone and a fake toast layer (no Windows toasts, no COM, no registry except in the
//! one test that turns the switch on for real under a throwaway identity and checks that every key is gone again): the `notif.*` messages, what the PC
//! shows, what a click, a reply and the X do, the rate limits, the one-time code on the clipboard, and that no notification text reaches a log.
//!
//! Everything listens on 127.0.0.1; the clipboard is in memory; the clock is moved by the test.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures_util::StreamExt;
use kin_lib::clipboard::{Clipboard, ManualClock};
use kin_lib::core::{Core, LinkState};
use kin_lib::encoding::base64url;
use kin_lib::mirror::activator::{Activation, Source};
use kin_lib::mirror::args::{ToastAction, ToastArgs};
use kin_lib::mirror::icons::test_png;
use kin_lib::mirror::surface::{DismissReason, FakeSurface, SurfaceEvent};
use kin_lib::mirror::{short_of, wire};
use serde_json::{Value, json};

const NID: &str = "54892d1f759b11f6762ff1bd98cda7df";
const NID2: &str = "36460390475e793c9a9e06dd5e2d421f";
const NID3: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";

fn uuid() -> String {
    kin_lib::encoding::uuid_v7(kin_lib::encoding::unix_ms())
}

fn hello_with(capabilities: Value) -> Value {
    let mut h = phone_hello();
    h["capabilities"] = capabilities;
    h
}

struct Rig {
    w: World,
    surface: Arc<FakeSurface>,
    clock: Arc<ManualClock>,
    ws: Ws,
    short: String,
}

impl Rig {
    /// A paired phone that lists `notif`, a PC whose switch is on, a fake toast layer and a clock at 0.
    async fn new() -> Rig {
        capture_logs();
        let clock = Arc::new(ManualClock::default());
        let w = World::new_with_clock(clock.clone()).await;
        w.pair().await;
        let surface = Arc::new(FakeSurface::default());
        w.core().mirror_for_tests(surface.clone());
        let mut ws = w.try_open_ws().await.expect("the upgrade works for a paired phone");
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["capabilities"], json!(["link", "media", "notif"]), "the PC lists notif while the switch is on");
        send_json(&mut ws, &hello_with(json!(["link", "notif"]))).await;
        w.wait_for_state(LinkState::PhoneConnected).await;
        let short = short_of(&w.phone.device_id).unwrap();
        Rig { w, surface, clock, ws, short }
    }

    fn core(&self) -> &Arc<Core> {
        &self.w.env.core
    }

    async fn send(&mut self, v: Value) {
        send_json(&mut self.ws, &v).await;
    }

    async fn post(&mut self, nid: &str, extra: Value) {
        let mut frame = json!({ "type": "notif.post", "id": uuid(), "nid": nid, "app": "Chat" });
        for (k, v) in extra.as_object().unwrap() {
            frame[k] = v.clone();
        }
        self.send(frame).await;
    }

    /// Waits until a toast with this tag is on the fake screen.
    async fn shown(&self, nid: &str) -> kin_lib::mirror::surface::ToastSpec {
        for _ in 0..100 {
            if let Some(t) = self.surface.find(nid) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no toast for {nid} appeared");
    }

    async fn gone(&self, nid: &str) {
        for _ in 0..100 {
            if self.surface.find(nid).is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the toast for {nid} is still there");
    }

    async fn wait_for(&self, what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..150 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what} never happened");
    }

    /// Nothing goes to the phone for a moment (it would be an `ack`, an `error` or something else).
    async fn silent(&mut self) {
        loop {
            match tokio::time::timeout(Duration::from_millis(400), self.ws.next()).await {
                Err(_) => return,
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(_) | tokio_tungstenite::tungstenite::Message::Pong(_)))) => {
                    continue;
                }
                Ok(other) => panic!("the PC sent something it should not have: {other:?}"),
            }
        }
    }

    fn args(&self, nid: &str, action: ToastAction, index: Option<u8>) -> String {
        ToastArgs::new(&self.short, nid, action, index).to_arguments()
    }

    fn click(&self, arguments: String, inputs: Vec<(&str, &str)>) -> Activation {
        Activation { source: Source::Com, arguments, inputs: inputs.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn toasts_said(&mut self) -> Vec<String> {
        toasts(&drain(&mut self.w.env.events))
    }
}

// ----- the capability ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_pcs_hello_lists_notif_only_while_the_switch_is_on_and_a_new_hello_says_so() {
    capture_logs();
    let w = World::new().await;
    w.pair().await;
    // Off (the default): `link` and `media` (2B.3: every 0.2.0 build lists it), no `notif`.
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["capabilities"], json!(["link", "media"]));
    send_json(&mut ws, &hello_with(json!(["link", "notif"]))).await;
    w.wait_for_state(LinkState::PhoneConnected).await;
    assert!(w.core().capabilities() == ["link", "media"]);
    // On: a new `hello` goes out on the open connection.
    let surface = Arc::new(FakeSurface::default());
    w.core().mirror_for_tests(surface);
    w.core().connections.send_hello_to_all();
    let second = next_json(&mut ws).await;
    assert_eq!(second["type"], "hello");
    assert_eq!(second["capabilities"], json!(["link", "media", "notif"]));
    // Off again: the toasts go and the next `hello` drops `notif`.
    w.env.core.set_mirror_notifications(false);
    let third = next_json(&mut ws).await;
    assert_eq!(third["capabilities"], json!(["link", "media"]));
}

// ----- post, update, remove ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_post_is_shown_then_replaced_then_removed_and_nothing_is_ever_acked() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "text": "Are you coming tonight?" })).await;
    let t = r.shown(NID).await;
    assert_eq!((t.tag.as_str(), t.group.as_str()), (NID, format!("kin-{}", r.short).as_str()));
    assert!(t.xml.contains("Alice") && t.xml.contains("Are you coming tonight?") && t.xml.contains("Chat \u{b7} Test Phone"), "{}", t.xml);
    assert!(!t.suppress_popup, "an ordinary notification pops up");
    assert_eq!(t.expires_in, kin_lib::mirror::EXPIRY_NORMAL);
    assert!(t.xml.contains("<audio silent=\"true\"/>"), "silent unless the setting says otherwise");
    // The same nid again with new text: the same tag and group, so Windows replaces it.
    r.clock.advance(3000);
    r.post(NID, json!({ "title": "Alice", "text": "Running late" })).await;
    r.wait_for("the replacement", || r.surface.shows().len() == 2).await;
    assert_eq!(r.surface.current().len(), 1);
    assert!(r.surface.find(NID).unwrap().xml.contains("Running late"));
    assert!(r.surface.find(NID).unwrap().generation > t.generation);
    // Removed by the phone.
    r.send(json!({ "type": "notif.remove", "id": uuid(), "nid": NID })).await;
    r.gone(NID).await;
    assert_eq!(r.surface.removals(), vec![(NID.to_string(), format!("kin-{}", r.short))]);
    let c = r.core().mirror_counters();
    assert_eq!((c.shown, c.replaced, c.removed), (1, 1, 1));
    r.silent().await;
}

#[tokio::test]
async fn an_id_is_not_remembered_so_a_repeat_is_processed_again() {
    let mut r = Rig::new().await;
    let id = uuid();
    r.send(json!({ "type": "notif.post", "id": id, "nid": NID, "app": "Chat", "title": "first" })).await;
    r.shown(NID).await;
    r.send(json!({ "type": "notif.post", "id": id, "nid": NID, "app": "Chat", "title": "second" })).await;
    r.wait_for("the second post", || r.surface.shows().len() == 2).await;
    assert!(r.surface.find(NID).unwrap().xml.contains("second"));
    r.silent().await;
}

#[tokio::test]
async fn quiet_and_private_notifications_get_the_right_popup_and_expiry() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "t", "importance": 2 })).await;
    r.post(NID2, json!({ "title": "SECRET", "text": "SECRETTEXT", "sensitive": true })).await;
    r.post(NID3, json!({ "title": "Bank", "text": "Your code is 482913", "code": "482913" })).await;
    let (low, private, code) = (r.shown(NID).await, r.shown(NID2).await, r.shown(NID3).await);
    assert!(low.suppress_popup && low.expires_in == kin_lib::mirror::EXPIRY_SILENT, "importance 2 is quiet and kept 2 hours");
    assert!(private.expires_in == kin_lib::mirror::EXPIRY_PRIVATE && code.expires_in == kin_lib::mirror::EXPIRY_PRIVATE);
    assert!(
        private.xml.contains("Private notification") && !private.xml.contains("SECRET") && !private.xml.contains("<action"),
        "{}",
        private.xml
    );
    assert!(code.xml.contains("Copy 482913"));
    let mut r = r;
    r.post(NID2, json!({ "title": "t", "text": "x", "silent": true })).await;
    r.wait_for("the silent update", || r.surface.shows().len() == 4).await;
    assert!(r.surface.find(NID2).unwrap().suppress_popup, "silent: to the Action Center with no popup");
}

#[tokio::test]
async fn the_settings_shape_what_is_shown() {
    let mut r = Rig::new().await;
    r.core().set_mirror_text_only(true);
    r.post(NID, json!({ "title": "Alice", "text": "hi", "reply": { "i": 1, "hint": "Reply" }, "code": "123456", "actions": [{ "i": 2, "label": "Mark as read" }] })).await;
    let t = r.shown(NID).await;
    assert!(t.xml.contains("Alice") && !t.xml.contains("<action") && !t.xml.contains("<input"), "text-only: {}", t.xml);
    r.core().set_mirror_text_only(false);
    r.core().set_mirror_hide_content(true);
    r.clock.advance(3000);
    r.post(NID, json!({ "title": "Alice", "text": "hi again" })).await;
    r.wait_for("the update", || r.surface.shows().len() == 2).await;
    let t = r.surface.find(NID).unwrap();
    assert!(t.xml.contains("Private notification") && !t.xml.contains("Alice"), "hide content: {}", t.xml);
    r.core().set_mirror_hide_content(false);
    r.core().set_mirror_sound(true);
    r.clock.advance(3000);
    r.post(NID, json!({ "title": "Alice", "text": "third" })).await;
    r.wait_for("the third", || r.surface.shows().len() == 3).await;
    assert!(!r.surface.find(NID).unwrap().xml.contains("<audio"), "the sound setting is on");
}

// ----- limits and dropping ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_flood_is_cut_by_the_bucket_and_the_phone_hears_one_rate_limited_error() {
    let mut r = Rig::new().await;
    for k in 0..30u32 {
        let nid = format!("{k:032x}");
        r.post(&nid, json!({ "title": format!("n{k}") })).await;
    }
    r.wait_for("the posts to be judged", || {
        let c = r.core().mirror_counters();
        c.shown + c.dropped_rate == 30
    })
    .await;
    let c = r.core().mirror_counters();
    assert_eq!((c.shown, c.dropped_rate), (20, 10), "capacity 20");
    assert_eq!(r.surface.current().len(), 20);
    let e = next_json(&mut r.ws).await;
    assert_eq!((e["type"].as_str(), e["code"].as_str()), (Some("error"), Some("rate_limited")));
    assert!(e.get("ref").is_none(), "no ref");
    r.silent().await;
    // A second later one token is back.
    r.clock.advance(1000);
    r.post(NID, json!({ "title": "again" })).await;
    r.shown(NID).await;
}

#[tokio::test]
async fn an_identical_post_within_two_seconds_is_ignored() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "text": "same" })).await;
    r.shown(NID).await;
    r.clock.advance(1999);
    r.post(NID, json!({ "title": "Alice", "text": "same" })).await;
    r.wait_for("the repeat to be judged", || r.core().mirror_counters().ignored_duplicates == 1).await;
    assert_eq!(r.surface.shows().len(), 1, "no new popup");
    r.clock.advance(1);
    r.post(NID, json!({ "title": "Alice", "text": "same" })).await;
    r.wait_for("the post after the window", || r.surface.shows().len() == 2).await;
}

#[tokio::test]
async fn thirty_toasts_per_phone_the_oldest_go_first() {
    let mut r = Rig::new().await;
    for k in 0..32u32 {
        r.clock.advance(1000);
        r.post(&format!("{k:032x}"), json!({ "title": format!("n{k}") })).await;
        r.wait_for("the toast", || r.surface.find(&format!("{k:032x}")).is_some()).await;
    }
    assert_eq!(r.surface.current().len(), 30);
    assert!(
        r.surface.find(&format!("{:032x}", 0)).is_none() && r.surface.find(&format!("{:032x}", 1)).is_none(),
        "the two oldest were removed"
    );
    assert!(r.surface.find(&format!("{:032x}", 31)).is_some());
    assert_eq!(r.core().mirror_counters().over_cap_removed, 2);
}

#[tokio::test]
async fn with_the_switch_off_everything_is_dropped_without_a_word() {
    let mut r = Rig::new().await;
    r.core().set_mirror_notifications(false);
    let hello = next_json(&mut r.ws).await;
    assert_eq!(hello["capabilities"], json!(["link", "media"]));
    r.post(NID, json!({ "title": "x" })).await;
    r.send(json!({ "type": "notif.status", "id": uuid(), "enabled": true, "access": "granted", "apps": 1 })).await;
    r.send(json!({ "type": "notif.post", "id": uuid(), "app": "no nid, and invalid on top" })).await;
    r.silent().await;
    assert!(r.surface.shows().is_empty());
    r.wait_for("the drops to be counted", || r.core().mirror_counters().dropped_by_receiver == 3).await;
}

#[tokio::test]
async fn a_phone_that_did_not_list_notif_is_not_obeyed() {
    capture_logs();
    let w = World::new().await;
    w.pair().await;
    let surface = Arc::new(FakeSurface::default());
    w.core().mirror_for_tests(surface.clone());
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["capabilities"], json!(["link", "media", "notif"]));
    send_json(&mut ws, &phone_hello()).await;
    w.wait_for_state(LinkState::PhoneConnected).await;
    send_json(&mut ws, &json!({ "type": "notif.post", "id": uuid(), "nid": NID, "app": "Chat", "title": "x" })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(surface.shows().is_empty());
    assert!(next_nothing(&mut ws).await);
}

async fn next_nothing(ws: &mut Ws) -> bool {
    tokio::time::timeout(Duration::from_millis(300), ws.next()).await.is_err()
}

// ----- the phone's status, sync, icons, unpair --------------------------------------------------------------------------

#[tokio::test]
async fn status_is_kept_for_the_settings_window() {
    let mut r = Rig::new().await;
    r.send(json!({ "type": "notif.status", "id": uuid(), "enabled": true, "access": "granted", "apps": 6 })).await;
    let device = r.w.phone.device_id.clone();
    r.wait_for("the status", || r.core().mirror_view()["phones"][0]["status"] == "mirroring 6 apps").await;
    r.send(json!({ "type": "notif.status", "id": uuid(), "enabled": false, "access": "missing", "apps": 0 })).await;
    r.wait_for("the new status", || r.core().mirror_view()["phones"][0]["status"] == "notification access is off").await;
    assert!(r.core().status_json()["mirror"]["view"]["enabled"] == true);
    // The settings window gets the same part (`mirror`), with each phone's line and the switches.
    let view = r.core().settings_view(None);
    assert!(
        view["mirror"]["enabled"] == true
            && view["mirror"]["dismissToPhone"] == true
            && view["mirror"]["hideContent"] == false
            && view["mirror"]["sound"] == false
    );
    assert_eq!(view["mirror"]["phones"][0]["status"], "notification access is off");
    let _ = device;
}

#[tokio::test]
async fn a_sync_removes_what_the_phone_no_longer_has_also_after_a_restart_of_kin() {
    let mut r = Rig::new().await;
    for nid in [NID, NID2, NID3] {
        r.post(nid, json!({ "title": "t", "text": nid })).await;
        r.shown(nid).await;
    }
    // A restart of Kin: the ledger is gone, the toasts are in Windows' history.
    r.core().mirror_forget_ledger_for_tests();
    r.send(json!({ "type": "notif.sync", "id": uuid(), "nids": [NID] })).await;
    r.gone(NID2).await;
    r.gone(NID3).await;
    assert!(r.surface.find(NID).is_some(), "what the phone still has stays");
    assert_eq!(r.core().mirror_counters().swept, 2);
    // An empty list removes the rest.
    r.send(json!({ "type": "notif.sync", "id": uuid(), "nids": [] })).await;
    r.gone(NID).await;
    r.silent().await;
}

#[tokio::test]
async fn icons_are_cached_by_hash_and_shown_and_a_bad_one_is_dropped_silently() {
    let mut r = Rig::new().await;
    let png = test_png(24, 24, [200, 30, 30, 255]);
    let hash = wire::hex32(&kin_lib::encoding::sha256(&png));
    r.send(json!({ "type": "notif.icon", "id": uuid(), "hash": hash, "png": base64url(&png) })).await;
    let file = r.core().mirror_icon_dir().join(format!("{hash}.png"));
    r.wait_for("the icon file", || file.is_file()).await;
    assert_eq!(std::fs::read(&file).unwrap(), png);
    r.post(NID, json!({ "title": "Alice", "iconHash": hash })).await;
    let t = r.shown(NID).await;
    assert!(t.xml.contains("<image placement=\"appLogoOverride\"") && t.xml.contains(&format!("notif-icons/{hash}.png")), "{}", t.xml);
    // A hash that is not in the cache: no image, still shown.
    r.post(NID2, json!({ "title": "Bob", "iconHash": "0123456789abcdef0123456789abcdef" })).await;
    assert!(!r.shown(NID2).await.xml.contains("<image"));
    // Wrong hash, and a PNG of 23 pixels: dropped and counted, never an error.
    r.send(json!({ "type": "notif.icon", "id": uuid(), "hash": "0123456789abcdef0123456789abcdef", "png": base64url(&png) })).await;
    let small = test_png(23, 23, [1, 2, 3, 255]);
    r.send(
        json!({ "type": "notif.icon", "id": uuid(), "hash": wire::hex32(&kin_lib::encoding::sha256(&small)), "png": base64url(&small) }),
    )
    .await;
    r.wait_for("the drops", || r.core().mirror_counters().icons_dropped == 2).await;
    assert_eq!(std::fs::read_dir(r.core().mirror_icon_dir()).unwrap().count(), 1);
    r.silent().await;
}

#[tokio::test]
async fn unpairing_the_phone_removes_its_toasts_and_the_icons() {
    let mut r = Rig::new().await;
    let png = test_png(24, 24, [9, 9, 9, 255]);
    let hash = wire::hex32(&kin_lib::encoding::sha256(&png));
    r.send(json!({ "type": "notif.icon", "id": uuid(), "hash": hash, "png": base64url(&png) })).await;
    r.post(NID, json!({ "title": "Alice" })).await;
    r.shown(NID).await;
    r.core().remove_device(&r.w.phone.device_id).unwrap();
    assert!(r.surface.find(NID).is_none(), "its toasts go");
    assert!(!r.core().mirror_icon_dir().exists(), "and the cache, because no phone is left");
}

// ----- what the user does ------------------------------------------------------------------------------------------------

fn action_frame(v: &Value) -> (String, u64, String, Option<String>) {
    assert_eq!(v["type"], "notif.action");
    (
        v["nid"].as_str().unwrap().to_string(),
        v["i"].as_u64().unwrap(),
        v["label"].as_str().unwrap().to_string(),
        v["text"].as_str().map(str::to_string),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_a_button_and_a_copy_reach_the_phone_and_the_user_is_told() {
    let mut r = Rig::new().await;
    r.post(
        NID,
        json!({ "title": "Alice", "text": "hi", "reply": { "i": 1, "hint": "Reply to Alice" }, "code": "482913", "actions": [{ "i": 2, "label": "Mark as read" }] }),
    )
    .await;
    r.shown(NID).await;

    // A reply: the text trimmed, the label is the hint, the index is the phone's.
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Reply, Some(1)), vec![("reply", "  hello there \n")]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(action_frame(&frame), (NID.to_string(), 1, "Reply to Alice".to_string(), Some("hello there".to_string())));
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    assert!(r.toasts_said().iter().any(|t| t == "Reply sent to Test Phone."));

    // A button, and the phone says the notification changed: nothing was pressed.
    r.clock.advance(5000);
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Button, Some(2)), vec![]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(action_frame(&frame), (NID.to_string(), 2, "Mark as read".to_string(), None));
    r.send(json!({ "type": "error", "ref": frame["id"], "code": "stale", "message": "The notification changed." })).await;
    task.await.unwrap();
    assert!(r.toasts_said().iter().any(|t| t.contains("changed on Test Phone")), "stale is said plainly");

    // Copy: the code goes to the clipboard as sensitive, and leaves it after 60 seconds if nothing else was copied.
    let click = r.click(r.args(NID, ToastAction::Copy, None), vec![]);
    r.core().mirror_activation(click).await;
    assert_eq!(r.w.env.clipboard.app_sensitive_writes(), ["482913"]);
    assert_eq!(r.w.env.clipboard.text().as_deref(), Some("482913"));
    r.clock.advance(59_999);
    r.core().mirror_clear_code_if_due();
    assert_eq!(r.w.env.clipboard.text().as_deref(), Some("482913"), "not yet");
    r.clock.advance(1);
    r.core().mirror_clear_code_if_due();
    assert_eq!(r.w.env.clipboard.text(), None, "emptied");
    r.silent().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copied_code_is_left_alone_when_the_user_copied_something_else() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Bank", "code": "482913" })).await;
    r.shown(NID).await;
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Copy, None), vec![])).await;
    r.w.env.clipboard.user_copy("my own text", vec![]);
    r.clock.advance(60_000);
    r.core().mirror_clear_code_if_due();
    assert_eq!(r.w.env.clipboard.text().as_deref(), Some("my own text"));
    // Kin's own clipboard sync never sees the code: it was written with the sensitive formats.
    let formats_at_copy = {
        r.w.env.clipboard.write_text_sensitive("x").unwrap();
        r.w.env.clipboard.read().unwrap().formats
    };
    assert!(kin_lib::clipboard::exclusion(&formats_at_copy).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_is_checked_before_anything_is_sent() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "reply": { "i": 1 }, "actions": [{ "i": 2, "label": "Mark as read" }] })).await;
    r.shown(NID).await;
    let refused = |r: &Rig| r.core().mirror_counters().activations_refused;
    // Arguments that are not ours, a phone nobody paired, a notification that is not on screen, a button that was not offered.
    r.core().mirror_activation(r.click("kin=1&d=ABCD2345".into(), vec![])).await;
    assert_eq!(refused(&r), 1);
    r.core().mirror_activation(r.click(ToastArgs::new("AAAAAAAA", NID, ToastAction::Button, Some(2)).to_arguments(), vec![])).await;
    assert_eq!(refused(&r), 2);
    r.core().mirror_activation(r.click(r.args(NID2, ToastAction::Button, Some(2)), vec![])).await;
    assert_eq!(refused(&r), 3);
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Button, Some(9)), vec![])).await;
    assert_eq!(refused(&r), 4);
    // A click on the toast itself does nothing visible and sends nothing.
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Tap, None), vec![])).await;
    assert_eq!(refused(&r), 4);
    // An empty reply asks for text and sends nothing.
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Reply, Some(1)), vec![("reply", "   ")])).await;
    assert!(r.toasts_said().iter().any(|t| t.contains("Type your reply")));
    // With the switch off no click is believed.
    r.core().set_mirror_notifications(false);
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Button, Some(2)), vec![])).await;
    assert_eq!(refused(&r), 5);
    // Nothing of that reached the phone (except the `hello` that the switch sent).
    let hello = next_json(&mut r.ws).await;
    assert_eq!(hello["type"], "hello");
    r.silent().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_click_twice_and_too_many_clicks_send_one_and_stop() {
    let mut r = Rig::new().await;
    r.post(
        NID,
        json!({ "title": "Alice", "actions": [{ "i": 0, "label": "One" }, { "i": 1, "label": "Two" }, { "i": 2, "label": "Three" }] }),
    )
    .await;
    r.shown(NID).await;
    // COM and the toast object both report one click: it is one click.
    let click = r.click(r.args(NID, ToastAction::Button, Some(0)), vec![]);
    let (a, b) = (r.core().clone(), r.core().clone());
    let (c1, c2) = (click.clone(), Activation { source: Source::InProcess, ..click });
    let t1 = tokio::spawn(async move { a.mirror_activation(c1).await });
    let frame = next_json(&mut r.ws).await;
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    t1.await.unwrap();
    b.mirror_activation(c2).await;
    r.silent().await;
    // Five presses in ten seconds are fine (the same button again after 3 s, so it is not the same click); the sixth is refused with a word.
    let _ = r.toasts_said();
    for (advance, button, sent) in [(0u64, 1u8, true), (0, 2, true), (3001, 0, true), (0, 1, true), (0, 2, false)] {
        r.clock.advance(advance);
        let core = r.core().clone();
        let click = r.click(r.args(NID, ToastAction::Button, Some(button)), vec![]);
        let task = tokio::spawn(async move { core.mirror_activation(click).await });
        let got = tokio::time::timeout(Duration::from_millis(700), next_json(&mut r.ws)).await;
        assert_eq!(got.is_ok(), sent, "button {button} at +{advance} ms");
        if let Ok(frame) = got {
            r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
        }
        task.await.unwrap();
    }
    assert!(r.toasts_said().iter().any(|t| t.starts_with("That is too fast")), "the refused press says why");
    // Ten seconds on, a press is fine again.
    r.clock.advance(10_000);
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Button, Some(2)), vec![]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_answer_in_five_seconds_is_said_and_never_retried_and_the_connection_stays() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "actions": [{ "i": 0, "label": "One" }] })).await;
    r.shown(NID).await;
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Button, Some(0)), vec![]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(frame["type"], "notif.action");
    // The phone says nothing.
    tokio::time::timeout(Duration::from_secs(9), task).await.expect("the press ends within 9 s").unwrap();
    assert!(r.toasts_said().iter().any(|t| t.starts_with("No answer from Test Phone")));
    // It is not sent again, and the connection is still there: a post still works.
    r.post(NID2, json!({ "title": "still here" })).await;
    r.shown(NID2).await;
    assert_eq!(r.core().snapshot().state, LinkState::PhoneConnected);
    r.silent().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_restart_of_kin_a_click_is_checked_against_windows_history() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "reply": { "i": 3, "hint": "Reply to Alice" }, "code": "G-123", "actions": [{ "i": 1, "label": "Mute" }] })).await;
    r.shown(NID).await;
    r.core().mirror_forget_ledger_for_tests();
    // Reply: the label is the hint that the toast in the history shows.
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Reply, Some(3)), vec![("reply", "yes")]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(action_frame(&frame), (NID.to_string(), 3, "Reply to Alice".to_string(), Some("yes".to_string())));
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    // The code too.
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Copy, None), vec![])).await;
    assert_eq!(r.w.env.clipboard.text().as_deref(), Some("G-123"));
    // A toast that is not in the history is not believed.
    r.core().mirror_activation(r.click(r.args(NID2, ToastAction::Reply, Some(3)), vec![("reply", "x")])).await;
    assert!(r.core().mirror_counters().activations_refused >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn pressing_x_dismisses_on_the_phone_only_for_the_exact_signal_and_only_when_asked() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "one" })).await;
    r.post(NID2, json!({ "title": "two" })).await;
    r.post(NID3, json!({ "title": "three" })).await;
    for n in [NID, NID2, NID3] {
        r.shown(n).await;
    }
    let group = format!("kin-{}", r.short);
    let ev = |nid: &str, generation: u64, reason: DismissReason| SurfaceEvent::Dismissed {
        tag: nid.into(),
        group: group.clone(),
        generation,
        reason,
    };
    let gen_of = |r: &Rig, n: &str| r.surface.find(n).unwrap().generation;

    // A popup that timed out and a toast the app hid say nothing.
    r.core().mirror_surface_event(ev(NID, gen_of(&r, NID), DismissReason::TimedOut)).await;
    r.core().mirror_surface_event(ev(NID, gen_of(&r, NID), DismissReason::ApplicationHidden)).await;
    r.silent().await;
    // The X, from the toast layer itself (the way the Windows handler reports it).
    let x = r.surface.user_closes(NID).unwrap();
    r.core().mirror_surface_event(x).await;
    let frame = next_json(&mut r.ws).await;
    assert_eq!((frame["type"].as_str(), frame["nid"].as_str()), (Some("notif.dismiss"), Some(NID)));
    assert_eq!(frame["id"].as_str().unwrap().len(), 36);
    assert_eq!(r.core().mirror_counters().dismissals_sent, 1);
    // The same X again: the ledger no longer has it.
    let again = ev(NID, 1, DismissReason::UserCanceled);
    r.core().mirror_surface_event(again).await;
    r.silent().await;
    // A toast that was replaced: the old object's X is not the user's.
    let old_generation = gen_of(&r, NID2);
    r.clock.advance(3000);
    r.post(NID2, json!({ "title": "two, updated" })).await;
    r.wait_for("the update", || gen_of(&r, NID2) > old_generation).await;
    r.core().mirror_surface_event(ev(NID2, old_generation, DismissReason::UserCanceled)).await;
    r.silent().await;
    // A toast the phone removed.
    r.send(json!({ "type": "notif.remove", "id": uuid(), "nid": NID3 })).await;
    r.gone(NID3).await;
    r.core().mirror_surface_event(ev(NID3, 3, DismissReason::UserCanceled)).await;
    r.silent().await;
    // With the setting off nothing is sent.
    r.core().set_mirror_dismiss_to_phone(false);
    let x = r.surface.user_closes(NID2).unwrap();
    r.core().mirror_surface_event(x).await;
    r.silent().await;
    assert_eq!(r.core().mirror_counters().dismissals_sent, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_developer_commands_call_the_same_functions() {
    capture_logs();
    let clock = Arc::new(ManualClock::default());
    let w = World::new_with_clock(clock).await;
    // (the test core has developer settings, so the hooks are allowed)
    w.pair().await;
    let surface = Arc::new(FakeSurface::default());
    w.core().mirror_for_tests(surface.clone());
    let mut ws = w.try_open_ws().await.unwrap();
    let _ = next_json(&mut ws).await;
    send_json(&mut ws, &hello_with(json!(["link", "notif"]))).await;
    w.wait_for_state(LinkState::PhoneConnected).await;
    send_json(&mut ws, &json!({ "type": "notif.post", "id": uuid(), "nid": NID, "app": "Chat", "title": "Alice", "reply": { "i": 1, "hint": "Reply" }, "code": "482913" })).await;
    for _ in 0..100 {
        if surface.find(NID).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let core = w.env.core.clone();
    let task = tokio::spawn(async move { core.mirror_dev_toast("reply", NID, Some("dev reply".into())).await });
    let frame = next_json(&mut ws).await;
    assert_eq!(action_frame(&frame).3.as_deref(), Some("dev reply"));
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    w.env.core.mirror_dev_toast("copy", NID, None).await;
    assert_eq!(w.env.clipboard.text().as_deref(), Some("482913"));
    w.env.core.mirror_dev_toast("dismiss", NID, None).await;
    let dismiss = next_json(&mut ws).await;
    assert_eq!(dismiss["type"], "notif.dismiss");
}

// ----- privacy ----------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn nothing_a_notification_says_is_written_to_a_log_or_to_disk() {
    let mut r = Rig::new().await;
    let secrets =
        ["ZQTITLEZQ", "ZQTEXTZQ", "ZQBIGZQ", "ZQSENDERZQ", "ZQREPLYZQ", "ZQLABELZQ", "ZQHINTZQ", "ZQAPPZQ", "ZQCONVZQ", "ZQLINEZQ"];
    r.send(json!({
        "type": "notif.post", "id": uuid(), "nid": NID, "app": "ZQAPPZQ", "title": "ZQTITLEZQ", "text": "ZQTEXTZQ", "bigText": "ZQTEXTZQ ZQBIGZQ",
        "conversation": "ZQCONVZQ", "messages": [{ "sender": "ZQSENDERZQ", "text": "ZQTEXTZQ" }], "lines": ["ZQLINEZQ"], "code": "918273",
        "reply": { "i": 1, "hint": "ZQHINTZQ" }, "actions": [{ "i": 2, "label": "ZQLABELZQ" }]
    }))
    .await;
    r.shown(NID).await;
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Reply, Some(1)), vec![("reply", "ZQREPLYZQ")]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Copy, None), vec![])).await;
    r.core().mirror_activation(r.click(r.args(NID, ToastAction::Button, Some(2)), vec![])).await;
    let x = r.surface.user_closes(NID).unwrap();
    r.core().mirror_surface_event(x).await;
    r.core().mirror_clear_code_now();
    let log = captured_lines().join("\n");
    for s in secrets.iter().chain(&["918273", NID]) {
        assert!(!log.contains(s), "{s} is in a log line");
    }
    assert!(log.contains("phone notification"), "there are log lines, with counts and tags only");
    // Nothing is stored in the data directory but the icon cache's folder (not even made here) and the app's own files.
    let mut found = Vec::new();
    let mut stack = vec![r.w.env.dir.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = std::fs::read(&p)
                && secrets.iter().any(|s| bytes.windows(s.len()).any(|w| w == s.as_bytes()))
            {
                found.push(p);
            }
        }
    }
    assert!(found.is_empty(), "notification text found in files: {found:?}");
    // The status file's numbers have no text either.
    let status = r.core().status_json().to_string();
    assert!(secrets.iter().all(|s| !status.contains(s)));
}

// ----- the real switch, under a throwaway identity ---------------------------------------------------------------------------

/// Turns the switch on for real: the keys are written under this instance's own names, the toast layer starts and the COM class is registered; turning it off
/// (and removing the keys) leaves nothing behind, including the two keys that Windows makes by itself for an AUMID.
#[tokio::test(flavor = "multi_thread")]
async fn turning_the_switch_on_registers_a_dev_identity_and_turning_it_off_removes_every_key() {
    use kin_lib::mirror::registration::{self, Identity, Kind};
    capture_logs();
    let w = World::new().await;
    let core = w.env.core.clone();
    let identity = Identity::for_config(&core.config);
    assert_eq!(identity.kind, Kind::Dev, "a test's data directory is never the installed Kin's");
    assert_ne!(identity.aumid, registration::RELEASE_AUMID);
    let result = std::panic::AssertUnwindSafe(async {
        assert!(core.capabilities() == ["link", "media"], "off by default");
        assert!(registration::leftovers(&identity).is_empty());
        core.set_mirror_notifications(true);
        assert!(core.mirror_view()["running"] == true, "{}", core.mirror_view());
        assert!(core.mirror_view()["activator"] == true, "the COM class is registered: {}", core.mirror_view());
        assert!(core.capabilities() == ["link", "media", "notif"]);
        let (command, activator) = registration::registered(&identity);
        assert!(command.unwrap().ends_with("--toast-activated"));
        assert_eq!(activator.as_deref(), Some(identity.clsid.as_str()));
        // The class is really there: calling it the way the shell does reaches the app (the click is refused, as nothing is on screen, but it arrived).
        let arguments = ToastArgs::new("ABCD2345", NID, ToastAction::Tap, None).to_arguments();
        kin_lib::mirror::activator::call_class(identity.clsid_u128(), &identity.aumid, &arguments, &[]).expect("the COM class answers");
        core.set_mirror_notifications(false);
        assert!(core.capabilities() == ["link", "media"]);
        assert!(core.mirror_view()["running"] == false);
    });
    let outcome = futures_util::FutureExt::catch_unwind(result).await;
    core.mirror_stop(true);
    let _ = registration::unregister(&identity);
    assert!(registration::leftovers(&identity).is_empty(), "no key of this identity is left: {:?}", registration::leftovers(&identity));
    outcome.expect("the checks above");
}

// ----- review round 1 -----------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_over_two_thousand_characters_is_refused_on_the_pc_and_two_thousand_goes() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "reply": { "i": 1, "hint": "Reply" } })).await;
    r.shown(NID).await;
    let args = r.args(NID, ToastAction::Reply, Some(1));
    let long = "é".repeat(2001);
    r.core().mirror_activation(r.click(args.clone(), vec![("reply", long.as_str())])).await;
    let said = r.toasts_said();
    assert!(said.iter().any(|t| t.contains("too long") && t.contains("2001") && t.contains("2000")), "{said:?}");
    assert_eq!(r.core().mirror_counters().actions_sent + r.core().mirror_counters().actions_failed, 0);
    r.silent().await;
    // Shortened at once and pressed again: not taken for the click that was just refused. 2,000 characters is the limit and is sent.
    let exact = "é".repeat(2000);
    let core = r.core().clone();
    let click = r.click(args, vec![("reply", exact.as_str())]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(frame["text"].as_str().unwrap().chars().count(), 2000);
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_different_reply_within_three_seconds_is_not_dropped_silently() {
    let mut r = Rig::new().await;
    r.post(NID, json!({ "title": "Alice", "reply": { "i": 1, "hint": "Reply" } })).await;
    r.shown(NID).await;
    let args = r.args(NID, ToastAction::Reply, Some(1));
    let core = r.core().clone();
    let click = r.click(args.clone(), vec![("reply", "first")]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    let _ = r.toasts_said();
    // The same click reported again (the other route): one reply, nothing said.
    r.core().mirror_activation(r.click(args.clone(), vec![("reply", "first")])).await;
    assert!(r.toasts_said().is_empty());
    // Another text: not sent, and the user is told.
    r.core().mirror_activation(r.click(args.clone(), vec![("reply", "second")])).await;
    let said = r.toasts_said();
    assert!(said.iter().any(|t| t.starts_with("A reply to this notification was sent a moment ago")), "{said:?}");
    r.silent().await;
    // Three seconds on, the second reply is sent.
    r.clock.advance(3000);
    let core = r.core().clone();
    let click = r.click(args, vec![("reply", "second")]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(frame["text"], "second");
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_action_limit_counts_from_the_first_press_also_for_a_phone_the_pc_has_no_entry_for() {
    let mut r = Rig::new().await;
    let nids: Vec<String> = (0..6).map(|k| format!("{k:032x}")).collect();
    for n in &nids {
        r.post(n, json!({ "title": "t", "actions": [{ "i": 0, "label": "One" }] })).await;
        r.shown(n).await;
        r.clock.advance(1100);
    }
    // After a restart of Kin there is no entry for the phone at all (no status, no ledger): the toasts are in Windows' history.
    r.core().mirror_forget_ledger_for_tests();
    let _ = r.toasts_said();
    for (k, n) in nids.iter().enumerate() {
        r.clock.advance(100);
        let core = r.core().clone();
        let click = r.click(r.args(n, ToastAction::Button, Some(0)), vec![]);
        let task = tokio::spawn(async move { core.mirror_activation(click).await });
        let got = tokio::time::timeout(Duration::from_millis(700), next_json(&mut r.ws)).await;
        assert_eq!(got.is_ok(), k < 5, "press {k}");
        if let Ok(frame) = got {
            r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
        }
        task.await.unwrap();
    }
    assert!(r.toasts_said().iter().any(|t| t.starts_with("That is too fast")));
}

#[tokio::test]
async fn a_change_of_the_switch_sends_a_hello_only_to_a_peer_that_lists_notif() {
    capture_logs();
    let w = World::new().await;
    w.pair().await;
    let surface = Arc::new(FakeSurface::default());
    w.core().mirror_for_tests(surface);
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    // A phone of 0.1.x: its hello lists `link` only.
    send_json(&mut ws, &hello_with(json!(["link"]))).await;
    w.wait_for_state(LinkState::PhoneConnected).await;
    w.env.core.set_mirror_notifications(false);
    assert!(next_nothing(&mut ws).await, "a phone that doesn't list notif is not sent a hello about it");
    // The name of the PC changing still tells every phone.
    w.core().connections.send_hello_to_all();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
}

#[tokio::test]
async fn the_too_large_error_names_the_limit_of_the_type() {
    let mut r = Rig::new().await;
    r.send(json!({ "type": "notif.post", "id": uuid(), "nid": NID, "app": "Chat", "title": "x", "text": "a".repeat(33_000) })).await;
    let e = next_json(&mut r.ws).await;
    assert_eq!((e["type"].as_str(), e["code"].as_str()), (Some("error"), Some("too_large")));
    assert!(e["message"].as_str().unwrap().contains("32 KiB"), "{e}");
    r.send(json!({ "type": "notif.icon", "id": uuid(), "hash": "0".repeat(32), "png": "a".repeat(50_000) })).await;
    let e = next_json(&mut r.ws).await;
    assert_eq!(e["code"], "too_large");
    assert!(e["message"].as_str().unwrap().contains("48 KiB"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hint_and_a_label_that_windows_reads_back_with_spaces_still_match_after_a_restart() {
    let mut r = Rig::new().await;
    let hint = "To\u{2028}Alice\u{ffff}";
    r.post(NID, json!({ "title": "A", "reply": { "i": 1, "hint": hint }, "actions": [{ "i": 2, "label": "Mu\u{2029}te" }] })).await;
    let toast = r.shown(NID).await;
    // What is on the toast, and what the ledger holds, is the same form: it is also what goes back to the phone.
    assert!(toast.xml.contains("placeHolderContent=\"To Alice\""), "{}", toast.xml);
    // A restart: the toast comes back from Windows with those characters as the spaces the XML had.
    r.core().mirror_forget_ledger_for_tests();
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Reply, Some(1)), vec![("reply", "yes")]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(action_frame(&frame).2, "To Alice");
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
    let core = r.core().clone();
    let click = r.click(r.args(NID, ToastAction::Button, Some(2)), vec![]);
    let task = tokio::spawn(async move { core.mirror_activation(click).await });
    let frame = next_json(&mut r.ws).await;
    assert_eq!(action_frame(&frame).2, "Mu te");
    r.send(json!({ "type": "ack", "ref": frame["id"] })).await;
    task.await.unwrap();
}

/// 2B.3 carry-over from the 2B.2 review: a device ID that came from the arguments of a click never makes the ledger an entry unless it is a paired phone.
#[tokio::test]
async fn pressing_a_button_for_a_device_that_is_not_paired_makes_no_ledger_entry() {
    let r = Rig::new().await;
    let before = r.core().mirror_press_for_tests(&r.w.phone.device_id, NID).await;
    let refused_before = r.core().mirror_counters().activations_refused;
    let after = r.core().mirror_press_for_tests("a-device-id-that-nobody-paired-----------------", NID).await;
    assert_eq!(before, after, "an unknown device gets no entry (and the press is refused)");
    assert_eq!(
        r.core().mirror_counters().activations_refused,
        refused_before + 1,
        "the press of the unknown device is refused exactly once"
    );
}
