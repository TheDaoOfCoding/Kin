# Windows app

## Basics

- **Tauri v2.** The Rust backend does all the real work. The TypeScript UI is only for windows: pairing, sending, history, settings and debug.
- Windows 10 and 11, x64. ARM64 comes later.

## Rust crates (planned)

| Job | Crate |
|---|---|
| Async runtime | `tokio` |
| HTTPS and WebSocket server | `axum` (with its WebSocket support), `rustls`, `tokio-rustls` |
| Certificates | `rcgen` |
| Encrypting the private key | DPAPI through the `windows` crate |
| mDNS advertising | `mdns-sd` |
| Database | `rusqlite` (bundled SQLite) |
| Clipboard read and write | `arboard`, with change events from `AddClipboardFormatListener` through the `windows` crate |
| QR rendering | `qrcode` (rendered as SVG for the UI) |
| Tray, hotkey, notifications, autostart, single instance | Tauri's built-in tray, plus the `global-shortcut`, `notification`, `autostart` and `single-instance` plugins |
| JSON | `serde`, `serde_json` |

Handlers need to know which device made each request, but axum doesn't pass the client certificate to them by default. That needs a custom TLS accept loop that attaches the peer certificate to every request. It's a known pattern, and it gets built in 1.1.

## Process model

- One process. It starts when you sign in and lives in the tray.
- Windows are created when opened and **destroyed** when closed, not just hidden. An open WebView2 costs tens of MB, and destroying it keeps idle memory low.
- Single instance: a second launch (for example from "Send to") passes its arguments to the running instance and exits.

## Tray

- The icon shows whether the phone is connected.
- **Left-click** opens the drop window. Windows doesn't support dropping files onto a tray icon, so this window is the drop target.
- **Menu:** status line, Send files…, Send clipboard to phone, Pair new phone, Open received folder, History, Settings, Quit.

## Sending to the phone

- **Drop window:** drag files in, or click to pick them.
- **Explorer "Send to":** a shortcut in `%APPDATA%\Microsoft\Windows\SendTo` runs the app with `--send <paths>`. On Windows 11 it's under "Show more options". The modern menu entry comes in Phase 2.
- **Hotkey** (configurable, default picked in 1.2 after checking for conflicts): sends the current clipboard text as a `text` message.
- **Automatic clipboard sync** (on by default, can be turned off): each clipboard change sends a `clipboard` message.
  - Debounced by 300 ms.
  - Text only in the MVP.
  - **Skips password-manager content.** That means content carrying any of the formats apps use to opt out: `ExcludeClipboardContentFromMonitorProcessing`, or `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` set to 0.
  - **Loop prevention:** remember a hash of the text last set from the phone, and ignore the change event it causes.

## Receiving from the phone

- **Files** go to `%USERPROFILE%\Downloads\Kin\` (configurable). Each one is written as `name.ext.part` and renamed when complete. Name collisions get ` (1)`, ` (2)` and so on.
- **Images** are also put on the clipboard (on by default), so Ctrl+V works right away.
- **Text and links** go to the clipboard, with a toast. Clicking a link's toast opens it in the browser.
- **Toast actions:** clicking a toast opens the file or its folder. "Open" and "Show in folder" buttons only if they're practical: toast buttons need a registered app ID when the app isn't packaged, so check this in 1.3. If they aren't practical, toasts are click-only.

## Install

- **Tauri NSIS installer, per-machine.** It needs admin rights to add the firewall rule:

  ```
  netsh advfirewall firewall add rule name="Kin Relay" dir=in action=allow program="<install path>\kin.exe" enable=yes profile=private,public
  ```

  Which profiles to include is decision D12. The uninstaller removes the rule.
- Also adds the "Send to" shortcut and start-with-Windows.
- **Code signing:** unsigned in the MVP, so SmartScreen shows a warning. Sign before the public launch.

## Data locations

`%APPDATA%\Kin\` holds:

- Settings
- The private key (DPAPI-encrypted)
- The SQLite database
- Logs (rotating, 5 files of 1 MB)

## UI

- Vite with TypeScript, using plain DOM or Svelte. No component library.
- Windows: Pairing (QR), Drop/send, History, Settings, Debug.
