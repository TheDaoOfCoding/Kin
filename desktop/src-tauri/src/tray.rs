//! The tray icon: its icon and tooltip show the state (not paired, phone connected, phone offline with the time it was last seen, and the
//! progress of a transfer), a left click opens the drop window (a Shift-click opens the debug window), and the menu is the one of
//! docs/windows.md: the status line, "Send files...", "Send clipboard to phone", "Pair new phone", "Open received folder", "History",
//! "Settings" and "Quit". (Pinging the phone, the clipboard sync switch and the log folder are in the windows or on the command line.)
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Wry};

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

/// The tray icon is drawn at this size. The shell scales it to 16 to 32 px, and a small image keeps the memory use low.
const ICON_SIZE: usize = 64;

pub struct Tray {
    status: MenuItem<Wry>,
    /// The icon without a state dot, at `ICON_SIZE`.
    base: (Vec<u8>, usize),
    menu: Menu<Wry>,
    /// Play/pause, next and previous of the phone's media, in the menu only while a phone's media is shown.
    media_items: [MenuItem<Wry>; 3],
    media_shown: std::sync::atomic::AtomicBool,
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

/// A dot in the bottom right corner: green when a phone is connected, amber when it is offline, none when not paired.
fn with_dot(base: &[u8], size: usize, state: &LinkState) -> Vec<u8> {
    let mut rgba = base.to_vec();
    let colour = match state {
        LinkState::PhoneConnected => [0x22, 0xc5, 0x5e],
        LinkState::PhoneOffline => [0xf5, 0x9e, 0x0b],
        LinkState::NotPaired => return rgba,
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

fn icon(base: &(Vec<u8>, usize), state: &LinkState) -> Image<'static> {
    Image::new_owned(with_dot(&base.0, base.1, state), base.1 as u32, base.1 as u32)
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

/// The status line of the menu: "Phone offline – last seen 14:32" when the phone is away.
pub fn status_line_with(snapshot: &Snapshot, seen: Option<&str>) -> String {
    match (&snapshot.state, snapshot.headline_device()) {
        (LinkState::NotPaired, _) | (_, None) => "Not paired".to_string(),
        (LinkState::PhoneConnected, Some(d)) => format!("Phone connected: {}{}", d.name, queued_note(snapshot)),
        (LinkState::PhoneOffline, Some(d)) => match seen {
            Some(t) => format!("Phone offline \u{2013} last seen {t}{}", queued_note(snapshot)),
            None => format!("Phone offline: {}{}", d.name, queued_note(snapshot)),
        },
    }
}

/// The status line as the tray menu shows it right now (also in `status.json`, for scripts).
pub fn status_text(snapshot: &Snapshot) -> String {
    status_line(snapshot)
}

fn status_line(snapshot: &Snapshot) -> String {
    status_line_with(snapshot, last_seen(snapshot).as_deref())
}

pub fn build(app: &App, snapshot: &Snapshot) -> tauri::Result<Tray> {
    let status = MenuItem::with_id(app, "status", status_line(snapshot), false, None::<&str>)?;
    let send_files = MenuItem::with_id(app, MENU_SEND_FILES, "Send files\u{2026}", true, None::<&str>)?;
    let send = MenuItem::with_id(app, MENU_SEND_CLIPBOARD, "Send clipboard to phone", true, None::<&str>)?;
    let pair = MenuItem::with_id(app, MENU_PAIR, "Pair new phone", true, None::<&str>)?;
    let open_folder = MenuItem::with_id(app, MENU_OPEN_FOLDER, "Open received folder", true, None::<&str>)?;
    let history = MenuItem::with_id(app, MENU_HISTORY, "History", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let media_items = [
        MenuItem::with_id(app, MENU_MEDIA_TOGGLE, "\u{25b6} Play/Pause phone media", true, None::<&str>)?,
        MenuItem::with_id(app, MENU_MEDIA_NEXT, "\u{23ed} Next", true, None::<&str>)?,
        MenuItem::with_id(app, MENU_MEDIA_PREV, "\u{23ee} Previous", true, None::<&str>)?,
    ];
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &send_files,
            &send,
            &pair,
            &open_folder,
            &history,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let window_icon = app.default_window_icon().expect("the bundle has an icon");
    let base = (downscale(window_icon.rgba(), window_icon.width() as usize, window_icon.height() as usize, ICON_SIZE), ICON_SIZE);
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(&base, &snapshot.state))
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
    let tray = Tray { status, base, menu, media_items, media_shown: std::sync::atomic::AtomicBool::new(false) };
    tray.show_media_items(snapshot.media.is_some());
    Ok(tray)
}

/// Whether Shift is held down right now (the tray click that opens the debug window).
fn shift_is_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
    // SAFETY: `GetKeyState` only reads the state of a key; a negative value means it is down.
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

impl Tray {
    /// Shows the state: tooltip, icon and the status line of the menu.
    pub fn update(&self, app: &AppHandle, snapshot: &Snapshot) {
        let Some(tray) = app.tray_by_id(TRAY_ID) else {
            return;
        };
        let _ = tray.set_tooltip(Some(tooltip(snapshot)));
        let _ = tray.set_icon(Some(icon(&self.base, &snapshot.state)));
        let _ = self.status.set_text(status_line(snapshot));
        self.show_media_items(snapshot.media.is_some());
    }

    /// The three media items go under the status line while a phone's media is shown, and out of the menu again when it isn't.
    fn show_media_items(&self, show: bool) {
        use std::sync::atomic::Ordering;
        if self.media_shown.swap(show, Ordering::SeqCst) == show {
            return;
        }
        for (i, item) in self.media_items.iter().enumerate() {
            let _ = if show { self.menu.insert(item, 1 + i) } else { self.menu.remove(item) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DeviceRow;

    fn device(name: &str, connected: bool) -> DeviceRow {
        DeviceRow { device_id: "x".into(), name: name.into(), platform: "android".into(), connected, paired_at: 1, last_seen: None }
    }

    fn s(state: LinkState, devices: Vec<DeviceRow>) -> Snapshot {
        Snapshot { state, devices, auto_clipboard_sync: true, transfers: None, queued: 0, media: None }
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

    #[test]
    fn dot_colours_differ_by_state() {
        let base = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
        let none = with_dot(&base, ICON_SIZE, &LinkState::NotPaired);
        let green = with_dot(&base, ICON_SIZE, &LinkState::PhoneConnected);
        let amber = with_dot(&base, ICON_SIZE, &LinkState::PhoneOffline);
        assert_eq!(none, base);
        assert_ne!(green, amber);
        let centre = (ICON_SIZE * 3 / 4 * ICON_SIZE + ICON_SIZE * 3 / 4) * 4;
        assert_eq!(&green[centre..centre + 4], &[0x22, 0xc5, 0x5e, 255]);
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
