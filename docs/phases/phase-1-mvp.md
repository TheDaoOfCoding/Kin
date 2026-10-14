# Phase 1: MVP

Goal: one Android phone and one Windows PC on the same Wi-Fi feel like one setup. Prove the three claims in [product.md](../product.md) (nothing to open, paired not strangers, clipboard), and prove they hold up reliably.

## Scope

You pair the phone once by scanning a QR code shown on the PC.

| | Phone → PC | PC → phone |
|---|---|---|
| **Files and photos** | Share sheet, with the PC shown in the direct-share row | Drop window opened from the tray, or Explorer's "Send to" |
| **Text and links** | Share sheet, or "Send to PC" in the text selection menu | Hotkey sends the current clipboard |
| **Clipboard (text only)** | Manual: quick tile or notification button | Automatic (can be turned off) |
| **On arrival** | Toast. Text and links go to the clipboard. Images also go to the clipboard so Ctrl+V works. Clicking the toast opens the file or link | Notification. Text goes to the clipboard. Photos and videos appear in the gallery. Links get an "Open" button |

Also included:

- **Resume:** an interrupted transfer continues from where it stopped.
- **Queue:** sends made while the other side is unreachable go out automatically when it's back.
- **History:** a list of recent transfers on both sides. Tap to open, retry if failed.
- **Status:** the tray icon and the phone notification show whether the devices are connected.
- **Onboarding:** walks the user through notifications and the phone's battery settings.

Phone-to-PC clipboard is manual because Android 10+ blocks background apps from reading the clipboard (see [android.md](../android.md)).

Out of scope: everything in [Phase 2](phase-2-features.md) and [Later](later.md). That includes the bridge, Bluetooth, multiple phones, clipboard images, notification mirroring, screen mirroring and a pretty UI.

## Milestones

Sizes are relative: S is a few days, M is about a week, L is more than a week. They're guesses for one person learning Kotlin and Rust along the way.

### 1.1 Connection spike: go/no-go gate (L)

Build only what's needed to test the riskiest assumption: that an always-on connection survives on real phones without hurting battery.

- Identity keys and certificates on both sides
- PC: HTTPS server, mDNS advertising, a pairing window with the QR code
- Phone: QR scan, pairing, the foreground service holding the WebSocket, and reconnect logic
- A "Ping" button on both sides. The arrival shows as a notification or toast with the measured latency
- A local log on both sides of every connect, disconnect (with its reason), and service start or stop

**Soak test:** leave it running for at least 3 days on each test phone, during normal use. Record:

- Battery used by the app per day
- Disconnects per day
- How many times the OS killed the service
- PC-to-phone ping latency after the screen has been off for over an hour

**Gate:**

| Result | What happens |
|---|---|
| Battery under 1%/day, service survives (with the battery exemption) on most phones, screen-off ping arrives in under 3 s | Continue with 1.2 |
| Battery too high | Lengthen the heartbeat interval and re-measure. If still too high, revisit the architecture |
| Service gets killed often | Keep the design. Treat PC-to-phone as best effort (the queue covers gaps) and move the FCM wake-up push from Phase 2 into Phase 1 |
| Both fail | Rethink: the phone only connects when the user does something, and PC-to-phone waits in the queue until then |

### 1.2 Text, links and clipboard (M)

- WebSocket messages `text`, `clipboard` and `ack` ([protocol.md](../protocol.md))
- Phone: share target for text, "Send to PC" in the text selection menu, quick tile and notification button for sending the clipboard
- PC: watches the clipboard and syncs it automatically, skipping content from password managers. Also the global hotkey, and receiving to the clipboard with a toast
- Loop prevention, so a clipboard set by the other device isn't sent straight back

**Done when:** copying on the PC puts the text on the phone's clipboard in under 1 s, and selecting text on the phone → "Send to PC" → Ctrl+V on the PC works.

### 1.3 Files in both directions (L)

- **Phone → PC:** share target for single and multiple files, plus a direct-share shortcut. Upload with progress notification. The PC saves to `Downloads\Kin\`, writing a `.part` file and renaming it when complete. Toast to open the file or show it in its folder. Images also go to the PC clipboard (on by default).
- **PC → phone:** drop window and "Send to". The phone downloads and saves photos and videos to the gallery and everything else to `Download/Kin`. Notification with "Open".
- Resume after a dropped connection, cancel from either side, a free disk space check, and safe file names

**Done when:** a 2 GB video and a 50-photo batch work in both directions; turning Wi-Fi off mid-transfer and back on resumes it; and speed is at least as good as LocalSend on the same network.

### 1.4 Queue, history, notifications, onboarding (M)

- **Queue:** anything sent while disconnected is saved first and goes out on reconnect
- **History:** a screen on both sides. Tap to open, retry failed items
- **Onboarding (phone):** notification permission, battery optimization settings, and autostart steps for each OEM
- **Settings (basic):** device name, auto clipboard sync on/off, images to clipboard on/off, start with Windows, unpair
- **Debug screen:** success rate, recent errors, latency, "Export logs"

**Done when:** sending from the PC while the phone's Wi-Fi is off arrives within seconds of turning Wi-Fi back on.

### 1.5 Package and test with friends (M)

- **Android:** a signed release APK (minified with R8), installed directly. Play Store submission waits for Phase 2 because of the foreground service review.
- **Windows:** a Tauri NSIS installer that adds the firewall rule, the "Send to" shortcut and start-with-Windows, and removes them on uninstall.
- Measure every target in [targets.md](../targets.md) and write the results in `docs/results/phase-1.md`.
- Have 3–5 friends with different phones and networks use it for 2 weeks. Collect what failed, which features they used most, and whether they kept using it.

## Exit criteria

- [ ] Every target in [targets.md](../targets.md) is met on the reference devices, or there's a written reason why not
- [ ] No known bug that loses or corrupts data
- [ ] At least some friends are still using it after 2 weeks without being reminded
- [ ] The Phase 2 order is decided based on what the friends test showed
