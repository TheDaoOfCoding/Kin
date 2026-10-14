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
| Clipboard read and write | The Win32 clipboard API through the `windows` crate: `AddClipboardFormatListener` on a hidden message-only window for the change events, `CF_UNICODETEXT` for the text, and `EnumClipboardFormats` for the opt-out formats. (`arboard` was in this plan. It would only have covered the text, and the listener and the format checks need Win32 anyway, so it was left out: one dependency fewer.) |
| QR rendering | `qrcode` (rendered as SVG for the UI) |
| Tray, hotkey, notifications, autostart, single instance | Tauri's built-in tray, plus the `global-shortcut`, `notification`, `autostart` and `single-instance` plugins |
| JSON | `serde`, `serde_json` |

Handlers need to know which device made each request, but axum doesn't pass the client certificate to them by default. That needs a custom TLS accept loop that attaches the peer certificate to every request. It's a known pattern, and it gets built in 1.1. Two rules from [protocol.md](protocol.md#transport) apply to it. The client certificate is mandatory in the handshake, and the custom verifier accepts any chain but must still verify the TLS 1.3 signature (`verify_tls13_signature`), so a replayed public certificate can't pass. And axum's default `Json` extractor answers `415`/`422` in plain text, so the app uses its own extractor that returns `400 bad_request` JSON (see [protocol/README.md](../protocol/README.md#notes-for-the-implementers)).

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
- **Hotkey** (default `Ctrl+Alt+Shift+K`, configurable): sends the current clipboard text as a `text` message. The tray's "Send clipboard to phone" and `kin.exe --send-clipboard` do the same. See [Hotkey](#hotkey).
- **Automatic clipboard sync** (on by default, can be turned off): each clipboard change sends a `clipboard` message.
  - Debounced by 300 ms.
  - Text only in the MVP.
  - **Skips password-manager content.** That means content carrying any of the formats apps use to opt out: `ExcludeClipboardContentFromMonitorProcessing`, or `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` set to 0.
  - **Loop prevention:** remember a hash of the text last set from the phone, and ignore the change event it causes.

### Hotkey

**Default: `Ctrl+Alt+Shift+K`** ("K" for Kin). It is picked to stay out of the way of what people already use:

- **Windows keeps every `Win+<key>` for itself** (`Win+V` is the clipboard history, `Win+Shift+S` the snipping tool, `Win+K` casting, and so on), so none of those is used.
- **Two-modifier shortcuts are what apps use** (`Ctrl+Shift+K` opens the web console in Firefox, for one, and editors bind many `Ctrl+Alt+<letter>` commands). A global hotkey wins over the app in front, so it would break those. With three modifiers it sits where far fewer commands are bound.
- **`Ctrl+Alt+<letter>` is `AltGr+<letter>` on many keyboard layouts**, which is how they type characters (`€`, `@`, Polish `ł` and `ś`, and so on). A global `Ctrl+Alt+<letter>` takes that character away from the people who type it. Adding `Shift` keeps it clear of the common ones.
- **Checked on the development PC:** Windows accepted the registration, so no other program holds it there. The check is repeated on every start (a program that holds the key makes the registration fail, which shows in the log, in `status.json` and in a toast). Every program on every PC can't be checked, which is why it is configurable.

**Changing it.** Put the key in `%APPDATA%\Kin\settings.json` (a settings window comes in 1.4):

```json
{ "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+F9" }
```

The syntax is the accelerator syntax of the Tauri global-shortcut plugin: modifiers `Ctrl`, `Alt`, `Shift` and `Super`, a letter, a digit or a name such as `F9`, joined with `+`. An empty string turns the hotkey off. The change takes effect the next time the app starts. If the text isn't a hotkey, or another program already has it, the app starts without a hotkey and says why once (a toast, the log, and `hotkey.error` in `status.json`).

**What the hotkey does.** It reads the clipboard at the moment of the press and sends the text as a `text` message, which the phone treats as an explicit send (a notification, and later an entry in the history). Two rules differ from the automatic sync, because someone pressed a key on purpose: it also sends content that a password manager marked "don't record" (the person asked), and a failure (the phone isn't connected, no `ack` within 5 s) gets a toast. The toast on success is "Sent to <phone>". A clipboard with no text gets "The clipboard has no text to send."

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

  The rule allows both the Private and Public profiles (D12). Windows 11 marks new networks Public by default, and every request is authenticated (see R8 in [risks.md](risks.md)). The uninstaller removes the rule.
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
