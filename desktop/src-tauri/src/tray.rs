//! The tray icon: its icon and tooltip show the state (not paired, phone connected with its battery, phone offline with the time it was last seen and its last
//! battery, whether this PC or the phone is paused, and the progress of a transfer), a left click opens the drop window (a Shift-click opens the debug window), and the
//! menu is the one of docs/windows.md: the status line, "Send files...", "Send clipboard to phone", "Pause Kin" (or "Resume Kin"), "Ring my phone" (or "Stop ringing
//! phone"), "Pair new phone", "Unlink phone...", "Open received folder", "History", "Settings" and "Quit". (Pinging the phone, the clipboard sync switch and the log
//! folder are in the windows or on the command line.)
use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Wry};

use crate::core::{LinkState, Snapshot};
use crate::encoding::unix_ms;

pub const TRAY_ID: &str = "kin";
pub const MENU_PAIR: &str = "pair";
pub const MENU_SEND_CLIPBOARD: &str = "send_clipboard";
pub const MENU_SEND_FILES: &str = "send_files";
pub const MENU_OPEN_FOLDER: &str = "open_folder";
pub const MENU_HISTORY: &str = "history";
pub const MENU_SETTINGS: &str = "settings";
pub const MENU_QUIT: &str = "quit";
/// The three items that appear while a phone's media is shown (2B.3): the fallback of the Windows media flyout, and the way to reach it without the mouse over
/// the flyout. They carry no title: the menu and the tooltip are visible to anyone near the PC.
pub const MENU_MEDIA_TOGGLE: &str = "media_toggle";
pub const MENU_MEDIA_NEXT: &str = "media_next";
pub const MENU_MEDIA_PREV: &str = "media_prev";
/// Pause Kin (2B.4): the three choices of the submenu, and the item that replaces it while this PC is paused.
pub const MENU_PAUSE_1H: &str = "pause_1h";
pub const MENU_PAUSE_TOMORROW: &str = "pause_tomorrow";
pub const MENU_PAUSE_RESUMED: &str = "pause_resumed";
pub const MENU_RESUME: &str = "resume";
/// Ring my phone (2B.4), and Stop ringing phone while it rings.
pub const MENU_RING: &str = "ring";
pub const MENU_RING_STOP: &str = "ring_stop";
/// Unlink phone... (2B.4): opens the Settings window with the confirmation already open.
pub const MENU_UNLINK: &str = "unlink";

/// The tray icon is drawn at this size. The shell scales it to 16 to 32 px, and a small image keeps the memory use low.
const ICON_SIZE: usize = 64;

/// What the menu looks like, apart from the text of the status line: when any of it changes the menu is built again (it happens a few times a day at most).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Shape {
    pub media: bool,
    pub paused: bool,
    pub ring_active: bool,
    pub can_ring: bool,
    /// A line under "Ring my phone" that says why it is greyed out (a connected phone that doesn't list `ring`).
    pub ring_reason: Option<String>,
    pub has_phone: bool,
}

impl Shape {
    pub fn of(snapshot: &Snapshot) -> Shape {
        let connected = snapshot.state == LinkState::PhoneConnected;
        Shape {
            media: snapshot.media.is_some(),
            paused: snapshot.presence.pc_pause.is_some(),
            ring_active: snapshot.presence.ring_active,
            can_ring: snapshot.presence.can_ring,
            ring_reason: if connected && !snapshot.presence.can_ring { snapshot.presence.ring_reason.clone() } else { None },
            has_phone: !snapshot.devices.is_empty(),
        }
    }
}

pub struct Tray {
    status: Mutex<MenuItem<Wry>>,
    /// The icon without a state dot, at `ICON_SIZE`.
    base: (Vec<u8>, usize),
    shape: Mutex<Shape>,
}

/// Shrinks an RGBA image to a square of `size` by averaging blocks of pixels.
fn downscale(rgba: &[u8], width: usize, height: usize, size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size * size * 4];
    for oy in 0..size {
        for ox in 0..size {
            let (x0, x1) = (ox * width / size, ((ox + 1) * width / size).max(ox * width / size + 1));
            let (y0, y1) = (oy * height / size, ((oy + 1) * height / size).max(oy * height / size + 1));
            let mut sum = [0u32; 4];
            let mut n = 0;
            for y in y0..y1.min(height) {
                for x in x0..x1.min(width) {
                    // Weight colours by alpha, so transparent pixels don't darken the edges.
                    let p = &rgba[(y * width + x) * 4..][..4];
                    let a = p[3] as u32;
                    sum[0] += p[0] as u32 * a;
                    sum[1] += p[1] as u32 * a;
                    sum[2] += p[2] as u32 * a;
                    sum[3] += a;
                    n += 1;
                }
            }
            let o = &mut out[(oy * size + ox) * 4..][..4];
            for (dst, channel) in o.iter_mut().zip(sum).take(3) {
                *dst = channel.checked_div(sum[3]).unwrap_or(0) as u8;
            }
            o[3] = (sum[3] / n.max(1)) as u8;
        }
    }
    out
}

/// The colour of the dot in the corner: **grey while this PC is paused** (so the state shows without hovering), else green when a phone is connected, amber when it
/// is offline, none when not paired.
fn dot_colour(state: &LinkState, paused: bool) -> Option<[u8; 3]> {
    if paused && *state != LinkState::NotPaired {
        return Some([0x9c, 0xa3, 0xaf]);
    }
    match state {
        LinkState::PhoneConnected => Some([0x22, 0xc5, 0x5e]),
        LinkState::PhoneOffline => Some([0xf5, 0x9e, 0x0b]),
        LinkState::NotPaired => None,
    }
}

/// A dot in the bottom right corner.
fn with_dot(base: &[u8], size: usize, state: &LinkState, paused: bool) -> Vec<u8> {
    let mut rgba = base.to_vec();
    let Some(colour) = dot_colour(state, paused) else {
        return rgba;
    };
    let (cx, cy, r) = (size as f32 * 0.74, size as f32 * 0.74, size as f32 * 0.24);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let p = &mut rgba[(y * size + x) * 4..][..4];
            if d <= r {
                p.copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
            } else if d <= r + 2.0 {
                p.copy_from_slice(&[255, 255, 255, 255]); // a white rim, so the dot shows on a dark taskbar too
            }
        }
    }
    rgba
}

fn icon(base: &(Vec<u8>, usize), state: &LinkState, paused: bool) -> Image<'static> {
    Image::new_owned(with_dot(&base.0, base.1, state, paused), base.1 as u32, base.1 as u32)
}

/// "last seen 14:32" for the phone the tray talks about, from Windows' local time. `None` if the phone was never seen.
fn last_seen(snapshot: &Snapshot) -> Option<String> {
    let seen = snapshot.headline_device()?.last_seen?;
    crate::localtime::last_seen_for(seen, unix_ms() as i64)
}

fn queued_note(snapshot: &Snapshot) -> String {
    match snapshot.queued {
        0 => String::new(),
        n => format!(" \u{b7} {n} queued"),
    }
}

/// "62%" or "62% charging".
pub fn battery_text(level: u8, charging: bool) -> String {
    if charging { format!("{level}% charging") } else { format!("{level}%") }
}

/// " \u{b7} 62% charging" for a connected phone whose battery is shown, " \u{b7} 12%" (the last known one) for an offline phone, else nothing.
fn battery_note(snapshot: &Snapshot) -> String {
    let p = &snapshot.presence;
    match snapshot.state {
        LinkState::PhoneConnected => p.phone_battery.map(|(l, c)| format!(" \u{b7} {}", battery_text(l, c))).unwrap_or_default(),
        LinkState::PhoneOffline => p.phone_last_known.map(|(l, c, _)| format!(" \u{b7} {}", battery_text(l, c))).unwrap_or_default(),
        LinkState::NotPaired => String::new(),
    }
}

/// " \u{b7} paused until 14:30" / " \u{b7} paused" when this PC is paused (the open-ended pause says no time) and " \u{b7} phone paused" when the phone is.
fn pause_note(snapshot: &Snapshot) -> String {
    let p = &snapshot.presence;
    let mut out = String::new();
    if p.pc_pause.is_some() {
        match &p.pc_pause_text {
            Some(t) if t != "until resumed" => out.push_str(&format!(" \u{b7} paused {t}")),
            _ => out.push_str(" \u{b7} paused"),
        }
    }
    if snapshot.state == LinkState::PhoneConnected && p.phone_pause.is_some() {
        out.push_str(" \u{b7} phone paused");
    }
    out
}

/// The tooltip, at most 127 characters (what Windows shows). `seen` is the "last seen" time of an offline phone.
pub fn tooltip_with(snapshot: &Snapshot, seen: Option<&str>) -> String {
    let base = match (&snapshot.state, snapshot.headline_device()) {
        (LinkState::NotPaired, _) | (_, None) => "Kin: not paired".to_string(),
        (LinkState::PhoneConnected, Some(d)) => format!("Kin: {} connected", d.name),
        (LinkState::PhoneOffline, Some(d)) => match seen {
            Some(t) => format!("Kin: {} offline, last seen {t}", d.name),
            None => format!("Kin: {} offline", d.name),
        },
    };
    // "Kin: Pixel 8 connected \u{b7} 62% charging \u{b7} paused until 14:30": the battery and the pause, then (as before) the media state.
    let base = format!("{base}{}{}", battery_note(snapshot), pause_note(snapshot));
    // "Kin: Pixel 8 connected, media playing": the state, never what plays.
    let base = match snapshot.media {
        Some(word) => format!("{base}, media {word}"),
        None => base,
    };
    let text = match &snapshot.transfers {
        Some(progress) => format!("{base}, {progress}"),
        None => base,
    };
    let text = format!("{text}{}", queued_note(snapshot));
    text.chars().take(127).collect()
}

pub fn tooltip(snapshot: &Snapshot) -> String {
    tooltip_with(snapshot, last_seen(snapshot).as_deref())
}

/// The status line of the menu: "Phone offline \u{2013} last seen 14:32" when the phone is away, with the battery and the pause as in the tooltip.
pub fn status_line_with(snapshot: &Snapshot, seen: Option<&str>) -> String {
    let line = match (&snapshot.state, snapshot.headline_device()) {
        (LinkState::NotPaired, _) | (_, None) => "Not paired".to_string(),
        (LinkState::PhoneConnected, Some(d)) => format!("Phone connected: {}{}", d.name, queued_note(snapshot)),
        (LinkState::PhoneOffline, Some(d)) => match seen {
            Some(t) => format!("Phone offline \u{2013} last seen {t}{}", queued_note(snapshot)),
            None => format!("Phone offline: {}{}", d.name, queued_note(snapshot)),
        },
    };
    let extra = format!("{}{}", battery_note(snapshot), pause_note(snapshot));
    let ringing = match &snapshot.presence.ring_text {
        Some(t) => format!(" \u{b7} {t}"),
        None => String::new(),
    };
    format!("{line}{extra}{ringing}")
}

/// The status line as the tray menu shows it right now (also in `status.json`, for scripts).
pub fn status_text(snapshot: &Snapshot) -> String {
    status_line(snapshot)
}

fn status_line(snapshot: &Snapshot) -> String {
    status_line_with(snapshot, last_seen(snapshot).as_deref())
}

/// The menu for a shape. The status item is returned too: its text changes without the menu being built again.
fn build_menu<M: Manager<Wry>>(app: &M, shape: &Shape, status_text: &str) -> tauri::Result<(Menu<Wry>, MenuItem<Wry>)> {
    let status = MenuItem::with_id(app, "status", status_text, false, None::<&str>)?;
    let send_files = MenuItem::with_id(app, MENU_SEND_FILES, "Send files\u{2026}", true, None::<&str>)?;
    let send = MenuItem::with_id(app, MENU_SEND_CLIPBOARD, "Send clipboard to phone", true, None::<&str>)?;
    let pair = MenuItem::with_id(app, MENU_PAIR, "Pair new phone", true, None::<&str>)?;
    let open_folder = MenuItem::with_id(app, MENU_OPEN_FOLDER, "Open received folder", true, None::<&str>)?;
    let history = MenuItem::with_id(app, MENU_HISTORY, "History", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let separator = || PredefinedMenuItem::separator(app);

    // "Pause Kin >" with its three choices, or "Resume Kin" while paused.
    let pause_submenu = Submenu::with_items(
        app,
        "Pause Kin",
        true,
        &[
            &MenuItem::with_id(app, MENU_PAUSE_1H, "1 hour", true, None::<&str>)?,
            &MenuItem::with_id(app, MENU_PAUSE_TOMORROW, "Until tomorrow", true, None::<&str>)?,
            &MenuItem::with_id(app, MENU_PAUSE_RESUMED, "Until I resume", true, None::<&str>)?,
        ],
    )?;
    let resume = MenuItem::with_id(app, MENU_RESUME, "Resume Kin", true, None::<&str>)?;
    let ring = if shape.ring_active {
        MenuItem::with_id(app, MENU_RING_STOP, "Stop ringing phone", true, None::<&str>)?
    } else {
        MenuItem::with_id(app, MENU_RING, "Ring my phone", shape.can_ring, None::<&str>)?
    };
    let ring_reason = shape.ring_reason.as_ref().map(|r| MenuItem::with_id(app, "ring_reason", r, false, None::<&str>)).transpose()?;
    let unlink = MenuItem::with_id(app, MENU_UNLINK, "Unlink phone\u{2026}", true, None::<&str>)?;
    let media_items = [
        MenuItem::with_id(app, MENU_MEDIA_TOGGLE, "\u{25b6} Play/Pause phone media", true, None::<&str>)?,
        MenuItem::with_id(app, MENU_MEDIA_NEXT, "\u{23ed} Next", true, None::<&str>)?,
        MenuItem::with_id(app, MENU_MEDIA_PREV, "\u{23ee} Previous", true, None::<&str>)?,
    ];
    let first_separator = separator()?;
    let second_separator = separator()?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&status, &first_separator];
    if shape.media {
        items.extend(media_items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<Wry>));
    }
    items.push(&send_files);
    items.push(&send);
    if shape.paused {
        items.push(&resume);
    } else {
        items.push(&pause_submenu);
    }
    items.push(&ring);
    if let Some(r) = &ring_reason {
        items.push(r);
    }
    items.push(&pair);
    if shape.has_phone {
        items.push(&unlink);
    }
    items.push(&open_folder);
    items.push(&history);
    items.push(&settings);
    items.push(&second_separator);
    items.push(&quit);
    let menu = Menu::with_items(app, &items)?;
    Ok((menu, status))
}

pub fn build(app: &App, snapshot: &Snapshot) -> tauri::Result<Tray> {
    let shape = Shape::of(snapshot);
    let (menu, status) = build_menu(app, &shape, &status_line(snapshot))?;

    let window_icon = app.default_window_icon().expect("the bundle has an icon");
    let base = (downscale(window_icon.rgba(), window_icon.width() as usize, window_icon.height() as usize, ICON_SIZE), ICON_SIZE);
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(&base, &snapshot.state, snapshot.presence.pc_pause.is_some()))
        .tooltip(tooltip(snapshot))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| crate::app::on_menu(app, event.id().as_ref()))
        // A left click opens the drop window: Windows doesn't support dropping files on a tray icon, so this window is the target. With
        // Shift held it opens the debug window instead (it is also reachable from the Settings window).
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if shift_is_down() {
                    crate::ui::open_debug_window(tray.app_handle());
                } else {
                    crate::ui::open_drop_window(tray.app_handle());
                }
            }
        })
        .build(app)?;
    Ok(Tray { status: Mutex::new(status), base, shape: Mutex::new(shape) })
}

/// Whether Shift is held down right now (the tray click that opens the debug window).
fn shift_is_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
    // SAFETY: `GetKeyState` only reads the state of a key; a negative value means it is down.
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

impl Tray {
    /// Shows the state: tooltip, icon and the status line of the menu. The menu is built again when its shape changed (media shown, paused, ringing, ...).
    pub fn update(&self, app: &AppHandle, snapshot: &Snapshot) {
        let Some(tray) = app.tray_by_id(TRAY_ID) else {
            return;
        };
        let _ = tray.set_tooltip(Some(tooltip(snapshot)));
        let _ = tray.set_icon(Some(icon(&self.base, &snapshot.state, snapshot.presence.pc_pause.is_some())));
        let shape = Shape::of(snapshot);
        let changed = {
            let mut current = self.shape.lock().unwrap_or_else(|p| p.into_inner());
            let changed = *current != shape;
            *current = shape.clone();
            changed
        };
        if changed {
            match build_menu(app, &shape, &status_line(snapshot)) {
                Ok((menu, status)) => {
                    let _ = tray.set_menu(Some(menu));
                    *self.status.lock().unwrap_or_else(|p| p.into_inner()) = status;
                }
                Err(e) => log::warn!("building the tray menu failed: {e}"),
            }
        } else {
            let _ = self.status.lock().unwrap_or_else(|p| p.into_inner()).set_text(status_line(snapshot));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DeviceRow;
    use crate::presence::PhonePause;
    use crate::presence::PresenceView;
    use crate::presence::pause::PauseState;

    fn device(name: &str, connected: bool) -> DeviceRow {
        DeviceRow { device_id: "x".into(), name: name.into(), platform: "android".into(), connected, paired_at: 1, last_seen: None }
    }

    fn s(state: LinkState, devices: Vec<DeviceRow>) -> Snapshot {
        Snapshot { state, devices, auto_clipboard_sync: true, transfers: None, queued: 0, media: None, presence: PresenceView::default() }
    }

    fn with_presence(mut snap: Snapshot, f: impl FnOnce(&mut PresenceView)) -> Snapshot {
        f(&mut snap.presence);
        snap
    }

    #[test]
    fn the_tooltip_names_the_state_of_the_phones_media_and_never_what_plays() {
        let playing = Snapshot { media: Some("playing"), ..s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]) };
        assert_eq!(tooltip_with(&playing, None), "Kin: Pixel 8 connected, media playing");
        let paused = Snapshot {
            media: Some("paused"),
            transfers: Some("sending 5%".into()),
            ..s(LinkState::PhoneConnected, vec![device("Pixel 8", true)])
        };
        assert_eq!(tooltip_with(&paused, None), "Kin: Pixel 8 connected, media paused, sending 5%");
        assert_eq!(tooltip_with(&s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]), None), "Kin: Pixel 8 connected");
    }

    #[test]
    fn text_follows_the_state() {
        assert_eq!(tooltip_with(&s(LinkState::NotPaired, vec![]), None), "Kin: not paired");
        assert_eq!(tooltip_with(&s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]), None), "Kin: Pixel 8 connected");
        assert_eq!(status_line_with(&s(LinkState::PhoneOffline, vec![device("Pixel 8", false)]), None), "Phone offline: Pixel 8");
        assert_eq!(status_line_with(&s(LinkState::NotPaired, vec![]), None), "Not paired");
        let sending = Snapshot { transfers: Some("sending 45%".into()), ..s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]) };
        assert_eq!(tooltip_with(&sending, None), "Kin: Pixel 8 connected, sending 45%");
    }

    #[test]
    fn an_offline_phone_shows_when_it_was_last_seen() {
        let offline = s(LinkState::PhoneOffline, vec![device("Pixel 8", false)]);
        assert_eq!(status_line_with(&offline, Some("14:32")), "Phone offline \u{2013} last seen 14:32");
        assert_eq!(tooltip_with(&offline, Some("yesterday 09:10")), "Kin: Pixel 8 offline, last seen yesterday 09:10");
        // A connected phone never shows it.
        let online = s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]);
        assert_eq!(status_line_with(&online, Some("14:32")), "Phone connected: Pixel 8");
    }

    #[test]
    fn queued_sends_are_counted_in_the_status_and_the_tooltip() {
        let mut offline = s(LinkState::PhoneOffline, vec![device("Pixel 8", false)]);
        offline.queued = 2;
        assert_eq!(status_line_with(&offline, Some("14:32")), "Phone offline \u{2013} last seen 14:32 \u{b7} 2 queued");
        assert!(tooltip_with(&offline, Some("14:32")).ends_with("\u{b7} 2 queued"));
        let long = Snapshot { queued: 99, ..s(LinkState::PhoneOffline, vec![device(&"N".repeat(200), false)]) };
        assert!(tooltip_with(&long, Some("14:32")).chars().count() <= 127, "Windows shows at most 127 characters");
    }

    /// 2B.4: the phone's battery, and the pause of either side, are in the tooltip and the status line.
    #[test]
    fn the_tooltip_carries_the_battery_and_the_pause() {
        let connected = s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]);
        let charging = with_presence(connected.clone(), |p| p.phone_battery = Some((62, true)));
        assert_eq!(tooltip_with(&charging, None), "Kin: Pixel 8 connected \u{b7} 62% charging");
        assert_eq!(status_line_with(&charging, None), "Phone connected: Pixel 8 \u{b7} 62% charging");
        let unplugged = with_presence(connected.clone(), |p| p.phone_battery = Some((14, false)));
        assert_eq!(tooltip_with(&unplugged, None), "Kin: Pixel 8 connected \u{b7} 14%");
        // This PC paused until a time, until resumed, and the phone paused.
        let until = with_presence(unplugged.clone(), |p| {
            p.pc_pause = Some(PauseState::Until(5));
            p.pc_pause_text = Some("until 14:30".into());
        });
        assert_eq!(tooltip_with(&until, None), "Kin: Pixel 8 connected \u{b7} 14% \u{b7} paused until 14:30");
        let forever = with_presence(connected.clone(), |p| {
            p.pc_pause = Some(PauseState::UntilResumed);
            p.pc_pause_text = Some("until resumed".into());
        });
        assert_eq!(tooltip_with(&forever, None), "Kin: Pixel 8 connected \u{b7} paused");
        let phone_paused = with_presence(connected.clone(), |p| p.phone_pause = Some(PhonePause { ends_ms: None }));
        assert_eq!(tooltip_with(&phone_paused, None), "Kin: Pixel 8 connected \u{b7} phone paused");
        // An offline phone shows its last known level, and never "phone paused" (a pause is forgotten with the connection).
        let offline = with_presence(s(LinkState::PhoneOffline, vec![device("Pixel 8", false)]), |p| {
            p.phone_last_known = Some((12, false, 1));
            p.phone_pause = Some(PhonePause { ends_ms: None });
        });
        assert_eq!(tooltip_with(&offline, Some("14:32")), "Kin: Pixel 8 offline, last seen 14:32 \u{b7} 12%");
        // A ring in progress is in the status line.
        let ringing = with_presence(connected, |p| p.ring_text = Some("Pixel 8 is ringing".into()));
        assert_eq!(status_line_with(&ringing, None), "Phone connected: Pixel 8 \u{b7} Pixel 8 is ringing");
    }

    #[test]
    fn the_tooltip_stays_within_127_characters_with_everything_on() {
        let busy = Snapshot {
            media: Some("playing"),
            transfers: Some("sending 45%".into()),
            queued: 12,
            presence: PresenceView {
                phone_battery: Some((100, true)),
                pc_pause: Some(PauseState::Until(5)),
                pc_pause_text: Some("until tomorrow 08:00".into()),
                phone_pause: Some(PhonePause { ends_ms: None }),
                ..Default::default()
            },
            ..s(LinkState::PhoneConnected, vec![device(&"N".repeat(80), true)])
        };
        assert!(tooltip_with(&busy, None).chars().count() <= 127);
    }

    #[test]
    fn dot_colours_differ_by_state_and_a_paused_pc_has_a_grey_one() {
        let base = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
        let none = with_dot(&base, ICON_SIZE, &LinkState::NotPaired, false);
        let green = with_dot(&base, ICON_SIZE, &LinkState::PhoneConnected, false);
        let amber = with_dot(&base, ICON_SIZE, &LinkState::PhoneOffline, false);
        let grey = with_dot(&base, ICON_SIZE, &LinkState::PhoneConnected, true);
        assert_eq!(none, base);
        assert_ne!(green, amber);
        assert_ne!(green, grey);
        let centre = (ICON_SIZE * 3 / 4 * ICON_SIZE + ICON_SIZE * 3 / 4) * 4;
        assert_eq!(&green[centre..centre + 4], &[0x22, 0xc5, 0x5e, 255]);
        assert_eq!(&grey[centre..centre + 4], &[0x9c, 0xa3, 0xaf, 255]);
        assert_eq!(with_dot(&base, ICON_SIZE, &LinkState::PhoneOffline, true), grey, "paused is grey whether the phone is there or not");
        assert_eq!(with_dot(&base, ICON_SIZE, &LinkState::NotPaired, true), base, "nothing paired, nothing to pause");
    }

    #[test]
    fn the_menu_changes_shape_with_the_pause_the_ring_and_the_phone() {
        let idle = s(LinkState::PhoneConnected, vec![device("Pixel 8", true)]);
        let a = Shape::of(&idle);
        assert!(!a.paused && !a.ring_active && a.has_phone && !a.media);
        let paused = Shape::of(&with_presence(idle.clone(), |p| p.pc_pause = Some(PauseState::UntilResumed)));
        assert!(paused.paused && paused != a);
        let ringing = Shape::of(&with_presence(idle.clone(), |p| p.ring_active = true));
        assert!(ringing.ring_active && ringing != a);
        let reason = Shape::of(&with_presence(idle.clone(), |p| {
            p.ring_reason = Some("Ringing isn't available: turn it on in Kin on Pixel 8".into())
        }));
        assert!(reason.ring_reason.is_some(), "a connected phone that can't be rung says why");
        let offline =
            Shape::of(&with_presence(s(LinkState::PhoneOffline, vec![device("Pixel 8", false)]), |p| p.ring_reason = Some("x".into())));
        assert!(offline.ring_reason.is_none(), "an offline phone is in the status line already");
        assert!(!Shape::of(&s(LinkState::NotPaired, vec![])).has_phone, "no Unlink without a phone");
    }

    #[test]
    fn downscaling_keeps_colour_and_ignores_transparent_pixels() {
        // 2x2 image: three transparent pixels and one opaque red one, shrunk to 1x1.
        let img = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255];
        let out = downscale(&img, 2, 2, 1);
        assert_eq!(&out[..3], &[255, 0, 0]);
        assert_eq!(out[3], 63);
    }
}
