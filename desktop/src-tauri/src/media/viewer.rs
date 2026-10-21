//! The PC as the **viewer** of a phone's media (protocol.md "Showing it"): what a `media.state` becomes in the Windows flyout and in the tray panel, with the
//! pictures it arrived with kept in memory only. Pure: no Windows, no clock. The surface and the window are told what to show; nothing here shows anything.
use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::{Value, json};

use super::map::wire_to_smtc;
use super::position::{Shown, shown};
use super::surface::{SurfaceView, Timeline};
use super::wire::{Action, ImageKind, PlayState, State, Volume};

/// Pictures kept per phone, in memory only (album art and an app icon are personal).
pub const MAX_IMAGES: usize = 8;

/// A phone's pictures by hash, the oldest dropped first.
#[derive(Default)]
pub struct ImageStore {
    items: VecDeque<(String, ImageKind, Arc<Vec<u8>>)>,
}

impl ImageStore {
    pub fn put(&mut self, hash: String, kind: ImageKind, data: Vec<u8>) {
        self.items.retain(|(h, _, _)| *h != hash);
        self.items.push_back((hash, kind, Arc::new(data)));
        while self.items.len() > MAX_IMAGES {
            self.items.pop_front();
        }
    }

    pub fn get(&self, hash: &str) -> Option<Arc<Vec<u8>>> {
        self.items.iter().find(|(h, _, _)| h == hash).map(|(_, _, d)| d.clone())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

/// The session a phone sent, with the moment it arrived on this PC's monotonic clock (the anchor of the extrapolation).
#[derive(Clone, Debug)]
pub struct ViewSession {
    pub state: State,
    pub anchor_ms: u64,
}

impl ViewSession {
    /// What is shown `now_ms`: the position and the state (a playing state past its end by more than 5 s is shown paused).
    pub fn shown(&self, now_ms: u64) -> Shown {
        let s = &self.state;
        shown(s.state, s.position_ms, Some(s.rate_milli), s.duration_ms, now_ms.saturating_sub(self.anchor_ms))
    }

    pub fn can(&self, action: Action) -> bool {
        self.state.actions.contains(&action)
    }

    /// The timeline of the flyout: only with a duration and a position.
    pub fn timeline(&self, now_ms: u64) -> Option<Timeline> {
        let d = self.state.duration_ms?;
        let p = self.shown(now_ms).position_ms?;
        Some(Timeline { duration_ms: d, position_ms: p, seek: self.can(Action::Seek) })
    }
}

/// The flyout's content for a session: the title line is the title (else the app), the artist line the artist, the album line `<phone> · <app>`; only the
/// controls the source listed are enabled; in a test run it is never `Playing`.
pub fn surface_view(session: &ViewSession, now_ms: u64, phone_name: &str, art: Option<Arc<Vec<u8>>>, test_mode: bool) -> SurfaceView {
    let s = &session.state;
    let at = session.shown(now_ms);
    SurfaceView {
        buttons: wire_to_smtc(at.state, &s.actions, test_mode),
        title: s.title.clone().unwrap_or_else(|| s.app.clone()),
        artist: s.artist.clone().unwrap_or_default(),
        album_line: format!("{phone_name} \u{b7} {}", s.app),
        thumbnail: art,
        timeline: session.timeline(now_ms),
        rate: s.rate_milli as f64 / 1000.0,
    }
}

/// The word for a state in a status line (never the title: it is on screen for anyone near the PC).
pub fn state_word(state: PlayState) -> &'static str {
    match state {
        PlayState::Playing => "playing",
        PlayState::Paused => "paused",
        PlayState::Buffering => "buffering",
        PlayState::Stopped => "stopped",
    }
}

/// What the panel of the drop window shows for a phone's session (the title and the artist are shown **there**, in a window the user opened).
pub fn panel_json(
    session: &ViewSession,
    now_ms: u64,
    phone_name: &str,
    art_data_url: Option<String>,
    icon_data_url: Option<String>,
) -> Value {
    let s = &session.state;
    let at = session.shown(now_ms);
    let mut v = json!({
        "from": phone_name,
        "app": s.app,
        "title": s.title.clone().unwrap_or_else(|| s.app.clone()),
        "artist": s.artist,
        "state": state_word(at.state),
        "positionMs": at.position_ms,
        "durationMs": s.duration_ms,
        "rateMilli": s.rate_milli,
        "actions": s.actions.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
        "art": art_data_url,
        "icon": icon_data_url,
    });
    if let Some(Volume { level, muted }) = s.volume {
        v["volume"] = json!({ "level": level, "muted": muted });
    }
    v
}

/// What `--dev-dump-view media` writes of the drop window's panel: the same shape, but no title, artist, app name or picture of what plays (a file in the data
/// directory must never hold them). A field that is `null` stays `null`.
pub fn redact_panel(panel: &mut Value) {
    for side in ["phone", "pc"] {
        let Some(obj) = panel.get_mut(side).and_then(Value::as_object_mut) else { continue };
        for key in ["title", "artist", "app", "art", "icon"] {
            if let Some(v) = obj.get_mut(key).filter(|v| !v.is_null()) {
                *v = Value::String("<redacted>".to_string());
            }
        }
    }
}

/// The `data:` URL a picture goes to the web view as (the content security policy already allows `img-src data:`).
pub fn data_url(bytes: &[u8]) -> String {
    use base64::Engine;
    let mime = match super::wire::image_info(bytes).map(|i| i.format) {
        Some(super::wire::Format::Jpeg) => "image/jpeg",
        _ => "image/png",
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::map::SmtcStatus;

    #[test]
    fn the_dumped_panel_keeps_its_shape_and_loses_what_plays() {
        let mut panel = json!({
            "phone": { "from": "Pixel", "title": "Secret Song", "artist": "Secret Artist", "app": "Spotify", "art": "data:image/png;base64,AAAA", "icon": null, "state": "paused", "positionMs": 5 },
            "pc": { "title": "Other Song", "artist": null, "app": "Edge", "state": "playing" },
            "volume": { "level": 30, "muted": false },
            "testMode": true
        });
        redact_panel(&mut panel);
        let text = panel.to_string();
        for leaked in ["Secret", "Spotify", "Other Song", "Edge", "base64"] {
            assert!(!text.contains(leaked), "{leaked} is still in {text}");
        }
        assert_eq!(panel["phone"]["title"], "<redacted>");
        assert_eq!(panel["phone"]["icon"], Value::Null);
        assert_eq!(panel["pc"]["artist"], Value::Null);
        assert_eq!((panel["phone"]["state"].as_str(), panel["phone"]["positionMs"].as_u64()), (Some("paused"), Some(5)));
        assert_eq!(panel["volume"]["level"], 30);
        let mut none = json!({ "phone": null, "pc": null, "volume": null });
        redact_panel(&mut none);
        assert_eq!(none, json!({ "phone": null, "pc": null, "volume": null }));
    }

    fn session(state: PlayState, position: Option<u64>, actions: &[Action]) -> ViewSession {
        ViewSession {
            state: State {
                sid: "a".repeat(32),
                app: "Spotify".into(),
                state,
                title: Some("Blue in Green".into()),
                artist: Some("Miles Davis".into()),
                album: Some("Kind of Blue".into()),
                duration_ms: Some(337_000),
                position_ms: position,
                rate_milli: 1000,
                actions: actions.to_vec(),
                art_hash: None,
                icon_hash: None,
                volume: Some(Volume { level: 35, muted: false }),
            },
            anchor_ms: 1000,
        }
    }

    #[test]
    fn the_flyout_names_the_phone_and_the_app_and_enables_what_the_source_listed() {
        let s = session(PlayState::Playing, Some(42_000), &[Action::Pause, Action::Next, Action::Seek]);
        let v = surface_view(&s, 11_000, "Pixel 8", None, false);
        assert_eq!(
            (v.title.as_str(), v.artist.as_str(), v.album_line.as_str()),
            ("Blue in Green", "Miles Davis", "Pixel 8 \u{b7} Spotify")
        );
        assert_eq!(v.buttons.status, SmtcStatus::Playing);
        assert!(v.buttons.pause && v.buttons.next && !v.buttons.play && !v.buttons.prev);
        assert_eq!(v.timeline, Some(Timeline { duration_ms: 337_000, position_ms: 52_000, seek: true }), "10 s after it arrived");
    }

    #[test]
    fn a_test_run_shows_paused_even_while_the_phone_plays() {
        let s = session(PlayState::Playing, Some(0), &[Action::Pause]);
        assert_eq!(surface_view(&s, 2000, "Pixel 8", None, true).buttons.status, SmtcStatus::Paused);
    }

    #[test]
    fn without_a_title_the_app_stands_in_and_without_a_duration_there_is_no_bar() {
        let mut s = session(PlayState::Paused, Some(5), &[Action::Play]);
        s.state.title = None;
        s.state.artist = None;
        s.state.duration_ms = None;
        let v = surface_view(&s, 5000, "Pixel 8", None, false);
        assert_eq!((v.title.as_str(), v.artist.as_str(), v.timeline), ("Spotify", "", None));
        s.state.duration_ms = Some(100);
        s.state.position_ms = None;
        assert_eq!(surface_view(&s, 5000, "Pixel 8", None, false).timeline, None, "no position, no bar");
    }

    #[test]
    fn a_state_that_ran_past_its_end_is_shown_paused_at_the_end() {
        let mut s = session(PlayState::Playing, Some(330_000), &[Action::Pause]);
        s.state.duration_ms = Some(337_000);
        let at = s.shown(1000 + 20_000);
        assert_eq!((at.state, at.position_ms), (PlayState::Paused, Some(337_000)));
        assert_eq!(surface_view(&s, 1000 + 20_000, "P", None, false).buttons.status, SmtcStatus::Paused);
    }

    #[test]
    fn the_image_store_keeps_eight_and_drops_the_oldest() {
        let mut store = ImageStore::default();
        for i in 0..10 {
            store.put(format!("{i:032}"), ImageKind::Art, vec![i as u8]);
        }
        assert_eq!(store.len(), MAX_IMAGES);
        assert!(store.get(&format!("{:032}", 0)).is_none() && store.get(&format!("{:032}", 1)).is_none());
        assert_eq!(store.get(&format!("{:032}", 9)).as_deref(), Some(&vec![9u8]));
        store.put(format!("{:032}", 2), ImageKind::Icon, vec![42]);
        assert_eq!(store.len(), MAX_IMAGES, "the same hash is replaced, not added");
        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn the_panel_has_the_title_but_the_status_word_has_not() {
        let s = session(PlayState::Paused, Some(0), &[Action::Play]);
        let p = panel_json(&s, 2000, "Pixel 8", Some(data_url(&[0x89, 0x50])), None);
        assert_eq!(
            (p["title"].as_str(), p["from"].as_str(), p["state"].as_str()),
            (Some("Blue in Green"), Some("Pixel 8"), Some("paused"))
        );
        assert_eq!(p["volume"]["level"], 35);
        assert!(p["art"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(state_word(PlayState::Playing), "playing");
    }
}
