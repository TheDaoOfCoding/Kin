//! The tray icon: its icon and tooltip show the state (not paired, phone connected, phone offline), and the menu has the
//! status line, "Send clipboard to phone", "Pair new phone", "Ping phone", the clipboard sync check item, "Open log folder"
//! and "Quit".
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Wry};

use crate::core::{LinkState, Snapshot};

pub const TRAY_ID: &str = "kin";
pub const MENU_PAIR: &str = "pair";
pub const MENU_PING: &str = "ping";
pub const MENU_SEND_CLIPBOARD: &str = "send_clipboard";
pub const MENU_AUTO_SYNC: &str = "auto_sync";
pub const MENU_LOGS: &str = "logs";
pub const MENU_QUIT: &str = "quit";

/// The tray icon is drawn at this size. The shell scales it to 16 to 32 px, and a small image keeps the memory use low.
const ICON_SIZE: usize = 64;

pub struct Tray {
    status: MenuItem<Wry>,
    /// "Sync clipboard automatically". Windows flips the check by itself when it is clicked.
    pub auto_sync: CheckMenuItem<Wry>,
    /// The icon without a state dot, at `ICON_SIZE`.
    base: (Vec<u8>, usize),
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

pub fn tooltip(snapshot: &Snapshot) -> String {
    match (&snapshot.state, snapshot.headline_device()) {
        (LinkState::NotPaired, _) | (_, None) => "Kin: not paired".to_string(),
        (LinkState::PhoneConnected, Some(d)) => format!("Kin: {} connected", d.name),
        (LinkState::PhoneOffline, Some(d)) => format!("Kin: {} offline", d.name),
    }
}

fn status_line(snapshot: &Snapshot) -> String {
    match (&snapshot.state, snapshot.headline_device()) {
        (LinkState::NotPaired, _) | (_, None) => "Not paired".to_string(),
        (LinkState::PhoneConnected, Some(d)) => format!("Phone connected: {}", d.name),
        (LinkState::PhoneOffline, Some(d)) => format!("Phone offline: {}", d.name),
    }
}

pub fn build(app: &App, snapshot: &Snapshot) -> tauri::Result<Tray> {
    let status = MenuItem::with_id(app, "status", status_line(snapshot), false, None::<&str>)?;
    let send = MenuItem::with_id(app, MENU_SEND_CLIPBOARD, "Send clipboard to phone", true, None::<&str>)?;
    let pair = MenuItem::with_id(app, MENU_PAIR, "Pair new phone", true, None::<&str>)?;
    let ping = MenuItem::with_id(app, MENU_PING, "Ping phone", true, None::<&str>)?;
    let auto_sync =
        CheckMenuItem::with_id(app, MENU_AUTO_SYNC, "Sync clipboard automatically", true, snapshot.auto_clipboard_sync, None::<&str>)?;
    let logs = MenuItem::with_id(app, MENU_LOGS, "Open log folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &send,
            &pair,
            &ping,
            &auto_sync,
            &logs,
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
        .build(app)?;
    Ok(Tray { status, auto_sync, base })
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
        let _ = self.auto_sync.set_checked(snapshot.auto_clipboard_sync);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DeviceRow;

    fn device(name: &str, connected: bool) -> DeviceRow {
        DeviceRow { device_id: "x".into(), name: name.into(), platform: "android".into(), connected, paired_at: 1, last_seen: None }
    }

    #[test]
    fn text_follows_the_state() {
        let s = |state, devices| Snapshot { state, devices, auto_clipboard_sync: true };
        assert_eq!(tooltip(&s(LinkState::NotPaired, vec![])), "Kin: not paired");
        assert_eq!(tooltip(&s(LinkState::PhoneConnected, vec![device("Pixel 8", true)])), "Kin: Pixel 8 connected");
        assert_eq!(status_line(&s(LinkState::PhoneOffline, vec![device("Pixel 8", false)])), "Phone offline: Pixel 8");
        assert_eq!(status_line(&s(LinkState::NotPaired, vec![])), "Not paired");
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
