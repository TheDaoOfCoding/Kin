# Changelog

What changed in each release, for people who use Kin Relay. The Windows app and the Android app share a version number; the browser extension is versioned on its own.

## 0.2.2

Open-source release. No change in how the apps behave.

- The source code is now public under the AGPL-3.0 (a commercial license is available), with a security policy, contribution guide and third-party notices.
- Build hygiene: pinned toolchains (Rust, Tauri CLI, extension type packages), `cargo fmt` and `clippy` clean, and a continuous-integration workflow.
- The browser extension stays at 0.2.0.

## 0.2.1

Fixes for three problems found on real devices.

- **Phone:** no more crash when you unlink the PC from the Settings tab.
- **Phone:** Kin's connection service now starts again by itself after the app is updated. Before, it stayed off until you opened the app.
- **PC:** updating Kin no longer offers to delete your data. An update installs over the existing copy without running the uninstaller, so your pairing, history, "Start with Windows", "Send to" entry and firewall rules are kept.

## 0.2.0

The "phone link" release: more than files and text.

- **Links:** send a web link between phone and PC, always showing the real address (with a warning for look-alike addresses), one-tap open, "Copy link".
- **Browser extension** for Chrome, Edge, Brave, Vivaldi, Opera and Firefox: right-click "Send to Kin", a toolbar popup, `Alt+Shift+K`, YouTube time, images, offline queue.
- **Phone notifications on the PC** (opt-in, per app), with reply, buttons, dismiss in both directions, and "Copy code" for one-time codes.
- **Media controls in both directions:** the phone's player in the Windows media flyout, the PC's player on the phone, with play, pause, next, previous, seek and volume.
- **Presence:** battery of the other device and a low-battery warning.
- **Pause** (one hour, until tomorrow, until you resume), **Ring my phone**, and opt-in **auto-sent screenshots** with a 5-second cancel window.
- **Unlinking** on both sides now removes the pairing for good and stops anything in flight.
- **Do Not Disturb sync:** the phone goes quiet while the PC is in Focus.
- **Alerts:** `Kin.exe --notify "text"` and "Tell my phone when a download finishes" in the browser extension.
- Test instances of Kin can no longer touch the real clipboard (a developer-facing change).

## 0.1.1

Speed and robustness.

- **Up to 4 files of a transfer move at the same time**, in both directions, which makes batches of many small files much faster.
- **A network that goes silent** (a router that drops packets without telling anyone) no longer fails a whole batch for good: the phone notices sooner and retries.
- **The firewall rule** now only allows connections from your local subnet.
- Uninstalling asks before removing anything.
- A test copy of Kin can now run beside an installed one (a developer-facing change).

## 0.1.0

First version.

- Pair a phone and a PC with a QR code (or a pasted link). Mutual TLS 1.3 with pinned keys; everything stays on the local network.
- Send files, text, links and the clipboard in both directions: share sheet, text selection menu, quick settings tile and notification action on the phone; tray, hotkey, drop window and "Send to" on the PC.
- A persistent queue (sends wait while the other device is offline), a history, settings and a debug screen on both sides.
- Windows installer (per-machine, with a firewall rule and "Start with Windows") and a signed Android APK. The QR scanner needs no Google services.
