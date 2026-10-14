# Phase 2: more features

The order is decided after Phase 1, based on what the friends test shows. Below is the recommended grouping, roughly in priority order. 2A has to be done before a public launch.

## 2A. Works everywhere (before public launch)

- **The bridge.** Used when a direct connection fails: guest Wi-Fi, campus networks, routers with AP isolation, or devices on different networks. The bridge only forwards encrypted bytes. TLS still runs end to end between the devices, so the bridge can't read anything. Options:
  - A WebSocket bridge on Cloudflare Durable Objects, since you already use Cloudflare
  - A small VPS

  Limit size and rate per device to keep costs down.
- **Wake-up push (FCM).** Delivers PC-to-phone sends even after the OEM has killed the service. It needs Google Play services and a small server. Without it, the queue is the fallback.
- **Explain network problems.** Show "this Wi-Fi blocks devices from seeing each other" instead of quietly failing.

## 2B. Ecosystem features

Each of these is cheap because it sits on the pairing and connection layer we already built.

- **Presence:** the phone's battery and name in the tray, and the PC's status in the phone notification
- **Ring my phone** from the PC
- **Open on the other device:** links, YouTube and Maps open directly instead of only being copied
- **Low battery nudge** on the PC
- **Clipboard images** in both directions, plus clipboard history
- **Notification mirroring** (read-only at first). It needs the "Notification access" permission, which worries users, so it's opt-in.
- **Hotspot help.** Android doesn't let regular apps turn on the phone's hotspot. The most we can do is open the hotspot settings page, then have the PC join automatically once it's on. It's worth less than it first sounds.

## 2C. Desktop polish

- An entry in the Windows 11 modern right-click menu. This needs the app to have a package identity, which is more work than the classic "Send to".
- **Drop shelf:** drag a file toward the screen edge and a drop target appears.
- Microsoft Store and winget listings
- Code signing, so SmartScreen stops warning on install

## 2D. More devices

- Multiple phones and PCs, plus tablets
- Phone to phone and PC to PC, which need a different connection role than "PC serves, phone connects"
- macOS and Linux desktop apps. Tauri builds for both, but clipboard, tray and firewall work differs on each OS.

## 2E. Growth

- **Receive in browser:** send to someone who doesn't have the app through a local link. The page offers "Get the app to send back".
- **Pairing link:** scanning the PC's QR with the normal camera app opens our app, or the store if it isn't installed. This needs Android App Links and a domain.
- **Play Store release,** including the foreground service declaration and demo video

## 2F. Make it pretty

- A real design pass with consistent device names and icons
- Arrival sound, haptic feedback and animations
