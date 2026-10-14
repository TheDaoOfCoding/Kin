# Kin Relay for Windows (desktop)

The Windows app: Tauri v2, a Rust backend and a small TypeScript UI (plain DOM, no component library). **This is milestone 1.2.** Milestone 1.1 was identity, the HTTPS server with mutual TLS, pairing by QR code, mDNS, the WebSocket with `hello`, `ack`, `error`, `ping` and `unpair`, a tray, and the logs. **1.2 adds text, links and the clipboard:** automatic clipboard sync to the phone, a global hotkey (and a tray item) that sends the clipboard text, receiving `text` from the phone (clipboard, toast, links that open from the toast), waiting for the `ack`, and the arrival times. Files come in 1.3. The contract is [docs/protocol.md](../docs/protocol.md), the plan is [docs/phases/phase-1-mvp.md](../docs/phases/phase-1-mvp.md), and what was measured is in [docs/results/phase-1-1-desktop.md](../docs/results/phase-1-1-desktop.md) and [docs/results/phase-1-2-desktop.md](../docs/results/phase-1-2-desktop.md).

## Layout

```
desktop/
  index.html, src/          the UI: the pairing window (src/pairing.ts)
  src-tauri/
    src/
      main.rs, lib.rs      entry point, module list, `run()`
      cli.rs               command line and KIN_* environment: data dir, developer settings, commands
      filelog.rs           rotating log files (5 x 1 MB)
      encoding.rs          base64url, base32, device ID, short ID, tokens, UUIDv7
      identity.rs, dpapi.rs   P-256 key + certificate (rcgen), key kept DPAPI-encrypted
      tls.rs               TLS 1.3 config, the client certificate verifier (P-256 only, signature always checked, one SPKI for the signature and the device ID)
      server.rs            accept loop (attaches the device ID to every request), connection limits and timeouts, POST /v1/pair, GET /v1/ws, 404
      pairing.rs           tokens (single use, 5 min, 5-failure limit), the pairing URL, the QR code as SVG
      netaddr.rs           which addresses go into the QR code
      json.rs, messages.rs strict JSON rules, and the WebSocket receiver as a pure state machine
      ws.rs, connections.rs  one WebSocket per device: hello timer, ping, unpair, close codes, replacing
      storage.rs           SQLite: `devices` and the memory of processed message IDs
      mdns.rs              _kinrelay._tcp advertising
      core.rs              shared state and what the app can do, with no UI in it; emits events
      sync.rs              1.2: sending `text` and `clipboard`, waiting for the `ack`, the debounced clipboard loop, receiving `text`, arrival times
      clipboard.rs         1.2: the opt-out format checks, loop guard, debounce, text helpers; the Win32 clipboard (hidden listener window) and an in-memory one for tests
      links.rs             1.2: what counts as a link (`isUrl`), which links may be opened (http, https), opening one in the browser
      settings.rs          1.2: `settings.json` (clipboard sync on or off, the hotkey)
      app.rs, tray.rs, ui.rs   the Tauri side: start-up, single instance, tray, global hotkey, toasts and clicks on them, windows, UI commands
    tests/                 vectors.rs, tls_vectors.rs, fake_phone.rs, server_limits.rs (+ common/)
```

## Build, test, run

Rust is installed per user. In a shell that predates the install, `. .\_env.ps1` puts `cargo` on the PATH.

```powershell
cd desktop
. .\_env.ps1
bun install
bun run build                       # the UI into dist/. cargo needs dist/ to exist (tauri embeds it)
cd src-tauri
cargo test                          # 158 tests (and 1 ignored): unit tests, the protocol vectors, and a fake phone over real TLS on loopback (the real clipboard is never touched)
cargo clippy --all-targets -- -D warnings
cd ..
powershell -ExecutionPolicy Bypass -File build.ps1    # release build + NSIS installer (bun tauri build --bundles nsis)
```

Outputs: `src-tauri\target\release\Kin.exe` and `src-tauri\target\release\bundle\nsis\Kin_0.1.0_x64-setup.exe`. If the bundler's own NSIS download times out, stage NSIS by hand as described in [phase-0-tauri.md](../docs/results/phase-0-tauri.md#snag-the-nsis-download-timed-out).

One more test touches the **real clipboard** and so is ignored by default: `cargo test real_clipboard -- --ignored --test-threads=1`. It saves the clipboard text, uses the clipboard, and puts the text back. It does nothing if the clipboard holds more than plain text (an image, files, rich text), which it couldn't restore.

`bun tauri dev` works too (Vite on `localhost:1420`), but the developer settings below are what you want for anything that listens.

## Developer and integration settings

Everything here is also available in a release build, so the integration agent can drive the real `Kin.exe` without clicking. **Every setting that has to do with the network is loopback only: with `--dev-bind` or `--dev-qr-address` the app never binds to anything but `127.0.0.1` (or `::1`), and it doesn't start mDNS** (which would open multicast sockets on every interface). No Windows Firewall prompt can appear.

### Settings (first launch)

A flag wins over its environment variable.

| Flag | Environment variable | Meaning |
|---|---|---|
| `--data-dir <path>` | `KIN_DATA_DIR` | Use this folder instead of `%APPDATA%\Kin`: a throwaway profile with its own key, database and logs. Quote it if it has a space. If the app can't start (for example it can't create this folder), it writes the reason to `kin-startup-error.txt` in the temp folder and exits with 1 |
| `--dev-bind <ip>` | `KIN_DEV_BIND` | Bind the HTTPS server to this address. Only a loopback address is accepted, anything else is an error at start |
| `--dev-qr-address <ipv4>` | `KIN_DEV_QR_ADDRESS` | Put `<ipv4>:<port>` in the QR code and the pairing URL instead of the PC's own addresses (`10.0.2.2` is the PC's loopback as seen from the Android emulator). Implies `--dev-bind 127.0.0.1` when no `--dev-bind` is given |
| `--dev-port <n>` | `KIN_DEV_PORT` | Port to try first (default 47821). If it is taken, a random port is used. `0` means random. **Read the real port from the URL or from `status.json`** |
| `--dev-print-pair-url` | `KIN_DEV_PRINT_PAIR_URL=1` | Whenever a pairing starts (tray menu, `--pair` or `--dev-start-pairing`), write the URL to `pair-url.txt` in the data directory and to stdout |
| `--dev-no-windows` | `KIN_DEV_NO_WINDOWS=1` | Don't create windows and don't show toasts. A toast is written to the log as `toast: ...` instead (for "Text from <phone>" the log has the title and the length of the preview, never the text) |

### Commands (any launch)

The single-instance plugin forwards a second launch's arguments to the running instance, which acts on them, and the second process exits. On a first launch the same commands run once the app is up.

| Command | What it does |
|---|---|
| `--ping` | Ping the connected phone. Same as "Ping phone" in the tray. The result is the toast, the log, and `lastPing` in `status.json` |
| `--dev-start-pairing` | Start a pairing without a window. With `--dev-print-pair-url` set at the first launch, the URL is in `pair-url.txt` |
| `--pair` | Open the pairing window (and start a pairing), like "Pair new phone" |
| `--send-clipboard` | Send the text on the clipboard to the phone as a `text` message: **the hotkey, headless.** The tray item "Send clipboard to phone" and the hotkey run exactly the same code. The result is `lastSent.text` in `status.json` (with `trigger` `command`, `hotkey` or `tray`) and a log line |
| `--dev-set-clipboard <text>` | Developer hook: put this text on the PC clipboard, as if the user had copied it. It goes through the normal pipeline (change event, 300 ms debounce, automatic sync), so this is how to trigger a PC to phone `clipboard` message. Add `--dev-clipboard-mark exclude`, `no-history`, `no-cloud` or `viewer-ignore` to add the opt-out format a password manager would add (the text is then not sent) |
| `--dev-get-clipboard` | Developer hook: write the PC clipboard text to `dev-clipboard.txt` in the data directory (UTF-8, exactly the text; the file is deleted if there is no text). Use it to check that a `text` from the phone arrived |
| `--quit` | Close the WebSockets (1000, "app quit"), stop listening, remove the tray icon and exit |

Unknown arguments are logged and ignored. **The log names a command and never prints its text.**

**Which clipboard hook to use.** For ordinary text, **PowerShell's `Set-Clipboard` and `Get-Clipboard` are the better choice**: they work on the real clipboard like any app does, so the whole path is tested, including the Windows change event, and they need no running Kin to answer. The two things PowerShell can't do are why the hooks exist: `--dev-set-clipboard ... --dev-clipboard-mark ...` sets the password-manager formats, and `--dev-get-clipboard` reads the clipboard *as Kin sees it* (UTF-16 to UTF-8, the same function that sends it). Note that a `text` from the phone is stored with `\r\n` line ends (see below), so `Get-Clipboard -Raw` shows `\r\n` where the phone sent `\n`.

```powershell
# PC -> phone, automatic: copy something. The phone gets a `clipboard` message about 300 ms later.
Set-Clipboard -Value 'hello from the PC'
# PC -> phone, explicit (the hotkey, headless):
& $kin --send-clipboard
# A password manager's copy is not sent:
& $kin --dev-set-clipboard 'hunter2' --dev-clipboard-mark exclude
# Phone -> PC: after the phone sent a `text`
Get-Clipboard -Raw
& $kin --dev-get-clipboard; Get-Content "$data\dev-clipboard.txt" -Raw
# What happened, with the times:
(Get-Content "$data\status.json" -Raw | ConvertFrom-Json) | Select-Object lastSent, lastReceived, clipboardSync, hotkey | ConvertTo-Json -Depth 6
```

### Files in the data directory

| File | What |
|---|---|
| `status.json` | Rewritten on every state change. `state` is `not_paired`, `phone_connected`, `phone_offline` or `stopped` (written on exit). Also `port`, `bind`, `deviceId`, `shortId`, `pcName`, `pairing` (`null`, or `{ active, expiresAtMs }`: it is `null` as soon as the code expires or five wrong guesses shut it down), `devices[]` (`deviceId`, `name`, `platform`, `connected`, `pairedAt`, `lastSeen`), `lastPing` (`{ at, result, rttMs? }`) and `pid`. **1.2 adds** the objects described under [status.json in 1.2](#statusjson-in-12) |
| `settings.json` | The settings: `{ "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+K" }`. Written when the tray check item is toggled. Edit it to change the hotkey (it is read at start). Missing keys are the defaults |
| `dev-clipboard.txt` | The clipboard text, after `--dev-get-clipboard` |
| `pair-url.txt` | The current pairing URL (only with `--dev-print-pair-url`). It holds the token, so it is never written to the log |
| `logs\kin.log` | Rotating log: `kin.log`, `kin.1.log` to `kin.4.log`, 1 MB each |
| `identity.key`, `identity.crt` | The DPAPI-encrypted PKCS#8 key and the certificate (public) |
| `kin.db` | SQLite: `devices`, `seen_messages` |

### status.json in 1.2

```json
{
  "settings": { "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+K" },
  "hotkey": { "accelerator": "Ctrl+Alt+Shift+K", "registered": true, "error": null },
  "lastSent": {
    "text":      { "id": "0199...", "at": 1760443200123, "result": "acked", "chars": 17, "trigger": "hotkey", "tapToAckMs": 41.3, "rttMs": 38.9 },
    "clipboard": { "id": "0199...", "at": 1760443199000, "result": "acked", "chars": 17, "trigger": "clipboard", "tapToAckMs": 352.8, "rttMs": 40.2 }
  },
  "lastReceived": { "text": { "id": "...", "at": 1760443201000, "chars": 20, "isUrl": false, "openable": false, "clipboardSet": true } },
  "clipboardSync": { "enabled": true, "sent": 3, "skipped": 1, "lastSkip": { "at": 1760443198000, "reason": "password_manager:ExcludeClipboardContentFromMonitorProcessing" } }
}
```

- **`result`** is `acked`, `no_reply` (no `ack` in 5 s: the connection was closed and the phone is offline), `phone_offline`, `no_phone_paired`, `error:<code>` (the phone answered with an `error`) or `too_large` (over 1 MiB as a message: a `.txt` file in 1.3, so for now it is skipped and logged).
- **`tapToAckMs`** is the arrival time of targets.md: from the tap to the phone's `ack`. For the hotkey the tap is the key press (or the command arriving). For the automatic sync it is the **first** clipboard change of a burst, so the 300 ms debounce is included (target: under 1 s). **`rttMs`** is only from sending the message to the `ack`. Both are monotonic-clock measurements made on the PC, and the same two numbers are in the log (`acked, tap to ack 352.8 ms (round trip 40.2 ms)`).
- **`lastSkip.reason`** is `password_manager:<format>`, `echo_of_phone_text` (loop prevention: the text the phone just sent), `not_text`, `empty`, `too_large` or `read_failed`.
- **No clipboard text is in `status.json` or in the log.** Both have an id, a length, a result and times. A log line also carries the first 8 hex digits of the SHA-256 of the text, so one text can be followed through the log without reading it.

### Running against the Android emulator

```powershell
$kin = 'C:\Dev\ecosystem-app\desktop\src-tauri\target\release\Kin.exe'
$data = "$env:TEMP\kin-emulator"            # a throwaway profile
# Start-Process doesn't quote the items of an argument array, so quote a path that could have a space in it.
Start-Process $kin -ArgumentList '--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-print-pair-url', '--dev-no-windows'
# wait until $data\status.json exists, then:
& $kin --dev-start-pairing                      # a second launch: forwarded to the running instance
$url = (Get-Content $data\pair-url.txt).Trim()  # kinrelay://pair?v=1&a=10.0.2.2:<port>&id=...&tk=...&n=<PC name>
adb shell am start -a android.intent.action.VIEW -d "'$url'"   # quote it: & is special in the shell
Get-Content $data\status.json                   # state: phone_connected once the phone has paired and sent hello
& $kin --ping                                   # then read lastPing in status.json, or the toast line in the log
Get-Content $data\logs\kin.log -Tail 20
& $kin --quit
```

- A dev instance and a normally installed Kin share the single-instance slot (same app identifier). Quit one before starting the other.
- `10.0.2.2` is a valid address in the pairing URL (it is in `10.0.0.0/8`).
- The pairing token lasts 5 minutes. `--dev-start-pairing` again makes a new one (and the old one stops working).

## How it behaves

- **Automatic clipboard sync (PC to phone).** On by default, switched in the tray ("Sync clipboard automatically") or with `autoClipboardSync` in `settings.json`. A hidden message-only window listens with `AddClipboardFormatListener`. Every change restarts a 300 ms timer. When it runs out the text on the clipboard goes to the phone as a `clipboard` message and the PC waits for the `ack` (5 s). Text only. Not sent: content marked by a password manager (`ExcludeClipboardContentFromMonitorProcessing`, or `CanIncludeInClipboardHistory` or `CanUploadToCloudClipboard` with a value of 0, and the older `Clipboard Viewer Ignore`), the text the phone just sent (loop prevention: a hash of it is remembered for 10 s and the change event it causes is ignored), anything that isn't text, and a text over 1 MiB as a message (skipped and logged: a `.txt` file is 1.3). Never stored in a history (D10), never logged as text. While the phone is offline an automatic send fails quietly (the persistent queue is 1.4); an explicit one shows a toast.
- **The hotkey.** `Ctrl+Alt+Shift+K` by default ([why](../docs/windows.md#hotkey)), `hotkey` in `settings.json` to change it (read at start, empty turns it off). It sends the current clipboard text as a `text` message, with `isUrl` set when the whole text is one http(s) link. The tray's "Send clipboard to phone" and `--send-clipboard` do the same. A toast says "Sent to <phone>", or why not.
- **Receiving `text` from the phone.** Acked first (the `ack` doesn't wait for the clipboard or the toast), then put on the clipboard with `\r\n` line ends (a bare `\n` shows as one long line in older Windows apps), without being sent back, and a toast "Text from <phone>" shows the first 100 characters on one line. If `isUrl` is set **and** the text really is one `http` or `https` URL, clicking the toast opens it in the default browser. The scheme is checked when the toast is made and again when it opens (nothing but `http` and `https` ever reaches `ShellExecute`), so `file:`, `javascript:`, `ms-msdt:` and the like never open, whatever the phone says. A click is only reported while the toast is on screen. A repeated message `id` gets the same `ack` and no second toast.
- **Connection limits.** TLS handshake 10 s, request headers 10 s, a JSON body 15 s (a slow client gets `400`), 30 s without a byte on a connection that isn't a WebSocket. 64 connections: 16 for handshaking, 24 for strangers (a client with a certificate of its own that isn't paired yet) and 24 that only a paired device can use, and at most 8 handshaking or unpaired connections per IP address. A flood of strangers can use up its own slots and never the paired phone's.
- **Identity:** a P-256 key and a self-signed X.509 v3 certificate (CN=Kin, no extensions) made on the first run. The device ID is `base64url(SHA-256(SPKI))`, and the short ID is the first 8 base32 characters.
- **Server:** TLS 1.3 only, ALPN `http/1.1`, port 47821 or a random one. A client certificate is mandatory in the handshake. The verifier accepts any chain but only an ECDSA P-256 leaf, and always verifies the CertificateVerify signature. Every request carries the device ID computed from the certificate.
- **Pairing:** "Pair new phone" opens a window with the QR code, the PC name, the short ID and a 5-minute countdown. A token is single use and compared in constant time, and five wrong guesses in 60 seconds kill it. Closing the window ends the pairing. On success the window becomes the "New device paired" notice, with a button to remove the device (and a toast).
- **Addresses in the QR code:** private (RFC 1918) addresses of adapters that are up, without loopback, link-local, VPN or virtual adapters, except that the address on the default route is never left out. At most 3, the default route first, then physical adapters.
- **WebSocket:** one per device, a new one replaces the old one (4001). The PC sends `hello` at once. No `hello` in 10 seconds is a 1002 close. A PC `ping` that isn't acked in 5 seconds is "No reply" and the connection is closed. 10 minutes of silence marks the phone offline (the phone's WebSocket ping every 4 minutes keeps it alive). Message IDs are remembered in SQLite, so a repeated `ping` gets the same `ack` and no second toast.
- **Tray:** the icon (a green dot when connected, amber when offline, none when not paired) and the tooltip show the state. The menu has the status line, Pair new phone, Ping phone, Open log folder and Quit. Windows are destroyed when closed.
- **Logs:** every connect and disconnect with its reason or close code, every pairing attempt and its result, every ping, and start and stop. Never a key, a token or the pairing URL.

## Not in this milestone

`offer`, `complete` and `cancel` messages (they answer `unsupported_type`), `POST /v1/transfers` and the file endpoints (they answer `404`), a clipboard text over 1 MiB (it needs the `.txt` file of 1.3), the persistent queue and the history (an explicit `text` is not stored anywhere yet), a settings window (`settings.json` instead), the drop window, "Send to", autostart, and the installer work (firewall rule, per-machine install). Those are 1.3 to 1.5.
