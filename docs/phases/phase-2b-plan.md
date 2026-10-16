# Phase 2B plan: ecosystem features (v0.1.1 → v0.2.0)

Written 2026-10-16. The user chose to build the 2B ecosystem features before 2A (the bridge). Everything here still runs on the local network only, with no account, no server and no telemetry.

## Goal

Turn Kin from "a better LocalSend" into "the private Phone Link that works on any phone": notifications, media, links and small "it just knows" features, on top of the pairing and connection layer from Phase 1.

## How it's built

- **Coders:** Claude Sonnet 5.5 (high). **Reviewer:** Claude Opus 5.5 (high). The reviewer only reviews. After clearance it commits and pushes, staging only the paths of its own milestone.
- **Accounts:** Support, Sam and Dmca. Each milestone runs a desktop agent, an Android agent and, when needed, an extension or integration agent in parallel. If an account hits its rate limit, the task re-runs on another one.
- **Review rule:** Opus blocks only on real bugs, security or privacy problems. Polish goes to a cleanup list in the results file.
- **Ownership:**
  - The desktop agent owns `protocol/` and `docs/protocol.md`. Each milestone's protocol additions come with test vectors, and `bun protocol/check-vectors.ts` stays green.
  - The Android agent implements against the same spec. If it finds a spec problem, it writes the problem in its results file instead of changing the spec.
- **Testing:** there's no real phone attached, so all phone testing uses the emulator (`tools/emulator/`, AVDs `kin_api37` and `kin_api29`). Only one agent drives the emulator at a time.
  - Each milestone ends with an integration agent that runs the real desktop build against the emulator and writes `tools/e2e-2b-N.ps1` plus `docs/results/phase-2b-N-*.md`.
  - The user re-tests on real hardware at v0.2.0.
- **Results:** each agent writes `docs/results/phase-2b-N-{desktop,android,extension,integration}.md`.

## Rules for every agent (unchanged from Phase 1, plus new ones)

1. **Code:** coders write code; the orchestrator never does. No admin and no UAC prompts.
2. **Hands off the user's real setup:**
   - Never touch the user's Run key, SendTo, Downloads or firewall rules.
   - Never touch the user's installed Kin or its data. Never kill a Kin process you didn't start.
3. **Dev desktop:** run only with `--dev-bind 127.0.0.1`, a throwaway data dir and a non-default port.
4. **Clipboard:** save it in all formats before any test that changes it, and restore it afterwards.
5. **Secrets:** never print the keystore password, `signing.properties` or LocalSend's `settings.json`. Never commit them.
6. **No disruption to the user:**
   - No OS input injection on the PC.
   - Before anything that could pop up a window, check that no game (or EasyAntiCheat) is running. If one is, skip that step and note it.
   - Browser tests run headless with a throwaway profile, never in the user's own browser profile.
7. **New for 2B:**
   - **Native messaging:** tests may register native messaging host keys only under HKCU, with a dev-only host name (`app.kinrelay.dev`). They must remove the keys at the end, even on failure.
   - **Media:** tests must never send play, pause, skip or volume commands to media sessions they didn't create, and never change the PC's system volume, mute state, Focus or Do Not Disturb.
   - **Sound:** tests must not play audible sound on the PC.
8. **Git:** commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never force-push.

## Milestones

### v0.1.1: speed and cleanup (S)

- **Parallel files in a batch:**
  - The phone is the HTTP client in both directions. It downloads (PC→phone) and uploads (phone→PC) up to 4 files of one offer at the same time.
  - Small files are pipelined, so a 50-photo batch no longer pays one round trip per file.
  - Resume, cancel, retry, the queue and progress all keep working.
  - Target: 50 × 3.4 MB in about 3–4 s on fast Wi-Fi, against 6–9 s today. LocalSend does it in about 3 s.
  - The protocol doc states the allowed concurrency.
  - The PC server, progress UI and history handle several active files per transfer.
- **1.5 cleanup list:**
  - Fix the uninstall hook order: if the user cancels the uninstall, nothing should already be removed.
  - Remove the real PC and user names from `docs/results/phase-1-5-desktop.md` (around lines 113, 115 and 122) and `phase-1-5-android.md` (around line 133).
  - Fix the stale "QR only" text.
  - `release-smoke.ps1` (around line 250): kill only the Kin it started.
  - `Save-Clipboard`: keep the earlier backup instead of overwriting it.
  - Add a lock file so the e2e, smoke and capture scripts can't run at the same time.
  - Clear the copied pairing link from the clipboard after pairing, if it's still the clipboard content.
  - Optional: limit the installer's firewall rule to the local subnet.
  - Drop the x86 and x86_64 ABIs from the release APK only (debug builds keep x86_64 for the emulator).
- **Speed test:** commit the results (`docs/results/phase-1-speed.md` and `tools/speedtest/`).
- **Release:** build the installer and APK as 0.1.1, then tag `v0.1.1`.

### 2B.1: links and the browser extension (L)

**Protocol.** A new `link` message: `{url, title?, sourceApp?}`.
- Only `http` and `https` are allowed. Everything else is rejected on both sides.
- The title is capped at 300 characters and cleaned like file names (bidi characters replaced).
- YouTube and other timestamped URLs pass through unchanged, including `t=`.

**Phone, receiving a link:**
- **The notification** shows the title, with the real domain underneath (always visible, so a page can't spoof where the link goes).
- **Tapping it opens the right app straight away.** On API 30+ it uses `FLAG_ACTIVITY_REQUIRE_NON_BROWSER` first and falls back to the default browser. On API 29 it uses an intent resolve check.
- **Buttons:** the only one is Copy link. The user rejected a separate "Open in browser" button as redundant.
- **Label:** "Open in YouTube" when a non-browser app handles the link. Use `<queries>` for ACTION_VIEW https, not QUERY_ALL_PACKAGES.
- **No auto-open:** no "display over other apps" permission. The user chose "one tap after the notification".
- **Grouping:** several links in a row are grouped.
- **Lock screen:** a setting for "Show link previews on the lock screen" (default: title hidden).

**Phone, receiving text** (from the extension or the PC): a notification with a Copy button. Check what Phase 1 already does and keep it. Add the "copy automatically" setting if it's missing.

**Phone → PC links:**
- Sharing a single URL to Kin opens it straight away in the PC's default browser. There's a setting, "Open links from phone automatically", which is on by default.
- The toast says "Opened link from phone", with a Copy button. When the setting is off, the toast has Open and Copy.

**Browser extension** (`extension/`, Manifest V3, one codebase for Chromium browsers and Firefox):
- **Right-click menu:** "Send to Kin" on selected text, links, images and the page. If there's only one paired device, it's a single item; with several, a submenu.
- **Toolbar button:** sends the current tab. A small popup shows the device, its online, queued or offline state, and the last sends.
- **Keyboard shortcut** (default Alt+Shift+K).
- **YouTube timestamp:** on youtube.com, the extension reads the video's current time with `activeTab` + `scripting` at click time and adds `t=`. No permanent site access.
- **Permissions:** `contextMenus`, `nativeMessaging`, `activeTab`, `scripting`, `storage`. No "read all sites" permission.
- **Images:** the desktop downloads by URL first, capped at 64 MiB. If that fails (auth-protected), the extension fetches the image in the page context and passes the bytes. The image then goes through the normal file transfer and lands in the phone's gallery.
- **Kin not running:** the extension shows a clear error with a "Start Kin" hint.

**Desktop:**
- **Native host:** a small native messaging host binary (`kin-native-host.exe`, part of the Rust workspace). It forwards stdio messages over a named pipe to the running Kin.
  - The pipe is restricted to the current user, with `PIPE_REJECT_REMOTE_CLIENTS`.
  - If Kin isn't running, the host starts it.
  - The host ↔ extension message contract is documented in `docs/extension.md`.
- **Installer:** registers the host for Chrome, Edge, Brave, Vivaldi and Opera (the Chromium keys) and for Firefox. Uninstall removes those keys.
- **Extension IDs:** stable dev IDs (the manifest `key`; Firefox `browser_specific_settings.gecko.id`). The store IDs are added later.
- **Stores:** publishing to the Chrome, Edge and Firefox stores needs the user's developer accounts (Chrome's has a one-time $5 fee). It isn't done by agents. They build store-ready zips and write load-unpacked instructions.

**Testing:**
- The extension is tested with headless Chromium (Playwright, throwaway profile) against a dev Kin and the emulator.
- Firefox is tested with `web-ext` in headless mode if available. Otherwise they test with a manual checklist.

### 2B.2: notification mirroring, phone → PC (L)

**Phone:**
- **NotificationListenerService,** opt-in, with an onboarding page.
  - On Android 13+, sideloaded installs may show "Restricted setting". Onboarding detects this and walks the user through App info → ⋮ → "Allow restricted settings".
- **Per-app allow list:**
  - First-run picker; messaging apps are suggested.
  - Banking, password manager and authenticator apps are off by default and flagged.
  - The list can be changed later.
- **Skipped:** Kin's own notifications, ongoing/foreground-service notifications, group summaries, media notifications (2B.3 handles those) and notifications the user has hidden on the lock screen (`VISIBILITY_SECRET`, unless the user allows them).

**What's sent:**
- the app name and app icon (small PNG, sent once and cached by hash on the PC)
- title, text and big text, plus the last few conversation messages (MessagingStyle)
- time, notification key, and the actions that can be replied to or are simple buttons

**PC:**
- **Toasts:** a Windows toast with the app icon, title and text. Updates to the same notification replace the old toast.
- **Inline reply** when the notification supports it. The PC sends the text, and the phone fills the RemoteInput and fires the PendingIntent.
- **Action buttons:** up to 3.
- **Dismiss sync** both ways: dismissing on the PC cancels it on the phone (`cancelNotification(key)`), and removal on the phone removes the toast from the PC's Action Center.

**One-time codes:**
- The phone detects codes (4–8 digits, or short alphanumerics next to "code", "OTP", "verification" and similar words in common languages).
- The toast gets a "Copy 123456" button. Copy clears the clipboard after 60 s if it still holds the code.
- On Android 15+, Android hides codes from listener apps like Kin. Detect redacted content and show "Code hidden by Android" instead of a broken button. Verify this on the API 37 emulator. Research whether any legitimate exemption applies, without hacks.

**Privacy:**
- Content is never written to the history database or the logs. Only counts and app names are logged.
- On the PC, content lives only in the toast. Nothing about notifications stays in the PC's data dir except icon caches.
- The sensitive flag is honoured.

**Limits:**
- Rate limiting and dedupe, so a chatty app can't flood the PC.
- Offline: notifications are not queued for later. If the PC is offline, they're dropped. A stale toast is worse than none.

**Testing:** the emulator posts test notifications through `adb shell cmd notification post`. A debug-only screen in the debug build posts a notification with RemoteInput and a button, to test reply and actions.

### 2B.3: media controls in both directions (M)

**Phone → PC:**
- **Reading the phone's media:** MediaSessionManager `getActiveSessions` reuses the notification-listener permission from 2B.2. It reads the title, artist, a small album-art thumbnail, duration, position, play state and the available actions.
- **Showing it on the PC:** in the Windows media flyout via SystemMediaTransportControls, using an HWND interop with Kin's hidden window. It also shows in Kin's tray popup.
- **Commands back:** play/pause, next, previous, seek and phone media volume.

**PC → phone:**
- **Reading the PC's media:** GlobalSystemMediaTransportControlsSessionManager. First verify it works for an unpackaged app; if not, record it as a blocker.
- **Showing it on the phone:** as a MediaStyle notification backed by a Kin MediaSession ("PC · Spotify — Song"), so it also appears in the quick-settings media carousel.
- **Commands back:** play/pause, next, previous and PC master volume (Core Audio).

**Both directions:**
- Battery: send updates only on change. The position is worked out from the timestamp and playback rate, not streamed.
- A setting on each side turns each direction on or off.

**Testing:**
- Phone media: a debug-only test player session on the emulator.
- PC media: a silent test media session created by the test itself.
- Per the rules, never touch the user's real sessions or volume.

### 2B.4: presence and quick wins (M)

- **Presence:**
  - The phone's battery level and charging state show in the tray tooltip and popup.
  - The PC's status (and battery, for laptops) shows in the phone's persistent notification.
  - A low-battery nudge on the PC when the phone drops to 15% or below.
- **Ring my phone:** from the tray or the PC app, a loud ring on the alarm audio stream, even when the phone is silent. A full-screen "Stop" action ends it, and it stops on its own after 30 s.
- **Pause Kin:**
  - A quick-settings tile on the phone and a tray toggle on the PC, with 1 hour, until tomorrow, or until resumed.
  - While paused, all automatic features stop (clipboard sync, mirroring, media, screenshots). Manual sends still arrive, quietly: they go to history with no notification or toast.
  - The other side shows "Paused until …".
- **Easy unlinking:** an obvious "Unlink" on both sides that removes the pairing on both (the protocol already has `unpair`), with a confirmation. A lost phone can be unlinked from the PC alone; its key stops working straight away.
- **Auto-send screenshots** (opt-in):
  - New screenshots go to the PC automatically and land as a toast with the image on the clipboard. "Also send new camera photos" is a second toggle.
  - It needs photo access (READ_MEDIA_IMAGES on 33+, storage read on 29–32), asked only when turned on.
  - This adds a new risk, R18: Play Store photo-permission policy.
- **Per-feature settings model:** data that's ready for multiple devices later (2D), shown as per-feature toggles for now.

### 2B.5: Focus/DND sync and PC → phone alerts (M)

- **Do Not Disturb sync:**
  - Phone → PC: the listener's `onInterruptionFilterChanged`. When the phone is on DND, Kin holds its own PC toasts. Kin can't set Windows Focus through a public API, so it doesn't try.
  - PC → phone: research public APIs for reading Windows Focus or quiet-hours state (`SHQueryUserNotificationState`, `Windows.UI.Shell.FocusSessionManager`). If one exists, turn on phone DND with `requestInterruptionFilter` while the PC is in Focus.
  - Either way: public APIs only, no undocumented Windows state. A toggle on each side. Document what's possible.
- **PC → phone alerts ("tell me when it's done"):**
  - `Kin.exe --notify "text"` sends a notification to the phone, for scripts and renders.
  - The extension can say "Download finished" (optional `downloads` permission, asked only when turned on).
  - The PC's battery is low (laptops).
- **Full Windows notification mirroring to the phone:** not in this phase. It needs package identity (a sparse MSIX package plus a paid code-signing certificate). Record this as a decision and leave it for later.

### 2B.6: release v0.2.0 (M)

- Build the installer and APK as 0.2.0, plus the extension zips for Chrome/Edge and Firefox.
- The installer registers and unregisters the native host.
- Update `docs/TESTING.md` with a checklist for the user's real-hardware test of every 2B feature.
- Write the phase results to `docs/results/phase-2b.md`.
- Update `docs/decisions.md` (use the next free D numbers; D17 is already taken by "dev Kin beside the installed Kin"):
  - 2B before 2A
  - links open after one notification tap, no overlay permission
  - the browser extension uses native messaging through a pipe
  - notification mirroring is opt-in and per app, and content is never stored
  - no Windows → phone notification mirroring until code signing
- Update `docs/risks.md` (R18 and new risks), `docs/android.md`, `docs/windows.md` and `docs/phases/phase-2-features.md` (mark what 2B delivered).
- Tag `v0.2.0`.

## Order and parallelism

| Step | Support | Dmca | Sam |
|---|---|---|---|
| 1 | v0.1.1 desktop + cleanup | v0.1.1 Android (parallel files) | 2B.1 extension scaffold (`extension/` + `docs/extension.md`) |
| 2 | v0.1.1 integration | | Opus review v0.1.1 → commit, tag |
| 3 | 2B.1 desktop (protocol, native host, link handling) | 2B.1 Android (link notifications, phone→PC links) | 2B.1 extension finish |
| 4 | 2B.1 integration | | Opus review 2B.1 → commit |
| 5–8 | 2B.2 to 2B.5: desktop | 2B.2 to 2B.5: Android | Integration, then Opus review and commit per milestone |
| 9 | 2B.6 release | | Opus final review → commit, tag v0.2.0 |

Reviews alternate accounts based on load.

## Not in this plan

- The bridge, FCM and network-problem messages (2A, next).
- Clipboard images and clipboard history.
- SMS, calls, webcam, screen mirroring (Windows or scrcpy already cover them, or they're blocked by Android).
- Unlocking the PC with the phone, and hotspot toggling (not possible).
- Multiple phones (2D).
- Store publishing (needs the user's accounts).
