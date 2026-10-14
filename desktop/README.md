# Kin Relay for Windows (desktop)

The Windows app: Tauri v2, a Rust backend and a small TypeScript UI (plain DOM, no component library). **This is milestone 1.1:** identity, the HTTPS server with mutual TLS, pairing by QR code, mDNS, the WebSocket with `hello`, `ack`, `error`, `ping` and `unpair`, a tray, and the logs. Text, clipboard and files come in 1.2 and 1.3. The contract is [docs/protocol.md](../docs/protocol.md), the plan is [docs/phases/phase-1-mvp.md](../docs/phases/phase-1-mvp.md), and what was measured is in [docs/results/phase-1-1-desktop.md](../docs/results/phase-1-1-desktop.md).

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
      tls.rs               TLS 1.3 config, the client certificate verifier (P-256 only, signature always checked)
      server.rs            accept loop (attaches the device ID to every request), POST /v1/pair, GET /v1/ws, 404
      pairing.rs           tokens (single use, 5 min, 5-failure limit), the pairing URL, the QR code as SVG
      netaddr.rs           which addresses go into the QR code
      json.rs, messages.rs strict JSON rules, and the WebSocket receiver as a pure state machine
      ws.rs, connections.rs  one WebSocket per device: hello timer, ping, unpair, close codes, replacing
      storage.rs           SQLite: `devices` and the memory of processed message IDs
      mdns.rs              _kinrelay._tcp advertising
      core.rs              shared state and what the app can do, with no UI in it; emits events
      app.rs, tray.rs, ui.rs   the Tauri side: start-up, single instance, tray, toasts, windows, UI commands
    tests/                 vectors.rs, tls_vectors.rs, fake_phone.rs (+ common/)
```

## Build, test, run

Rust is installed per user. In a shell that predates the install, `. .\_env.ps1` puts `cargo` on the PATH.

```powershell
cd desktop
. .\_env.ps1
bun install
bun run build                       # the UI into dist/. cargo needs dist/ to exist (tauri embeds it)
cd src-tauri
cargo test                          # 88 tests: unit tests, the protocol vectors, and a fake phone over real TLS on loopback
cargo clippy --all-targets -- -D warnings
cd ..
powershell -ExecutionPolicy Bypass -File build.ps1    # release build + NSIS installer (bun tauri build --bundles nsis)
```

Outputs: `src-tauri\target\release\Kin.exe` and `src-tauri\target\release\bundle\nsis\Kin_0.1.0_x64-setup.exe`. If the bundler's own NSIS download times out, stage NSIS by hand as described in [phase-0-tauri.md](../docs/results/phase-0-tauri.md#snag-the-nsis-download-timed-out).

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
| `--dev-no-windows` | `KIN_DEV_NO_WINDOWS=1` | Don't create windows and don't show toasts. A toast is written to the log as `toast: ...` instead |

### Commands (any launch)

The single-instance plugin forwards a second launch's arguments to the running instance, which acts on them, and the second process exits. On a first launch the same commands run once the app is up.

| Command | What it does |
|---|---|
| `--ping` | Ping the connected phone. Same as "Ping phone" in the tray. The result is the toast, the log, and `lastPing` in `status.json` |
| `--dev-start-pairing` | Start a pairing without a window. With `--dev-print-pair-url` set at the first launch, the URL is in `pair-url.txt` |
| `--pair` | Open the pairing window (and start a pairing), like "Pair new phone" |
| `--quit` | Close the WebSockets (1000, "app quit"), stop listening, remove the tray icon and exit |

Unknown arguments are logged and ignored.

### Files in the data directory

| File | What |
|---|---|
| `status.json` | Rewritten on every state change. `state` is `not_paired`, `phone_connected`, `phone_offline` or `stopped` (written on exit). Also `port`, `bind`, `deviceId`, `shortId`, `pcName`, `pairing` (`null`, or `{ active, expiresAtMs }`), `devices[]` (`deviceId`, `name`, `platform`, `connected`, `pairedAt`, `lastSeen`), `lastPing` (`{ at, result, rttMs? }`) and `pid` |
| `pair-url.txt` | The current pairing URL (only with `--dev-print-pair-url`). It holds the token, so it is never written to the log |
| `logs\kin.log` | Rotating log: `kin.log`, `kin.1.log` to `kin.4.log`, 1 MB each |
| `identity.key`, `identity.crt` | The DPAPI-encrypted PKCS#8 key and the certificate (public) |
| `kin.db` | SQLite: `devices`, `seen_messages` |

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

- **Identity:** a P-256 key and a self-signed X.509 v3 certificate (CN=Kin, no extensions) made on the first run. The device ID is `base64url(SHA-256(SPKI))`, and the short ID is the first 8 base32 characters.
- **Server:** TLS 1.3 only, ALPN `http/1.1`, port 47821 or a random one. A client certificate is mandatory in the handshake. The verifier accepts any chain but only an ECDSA P-256 leaf, and always verifies the CertificateVerify signature. Every request carries the device ID computed from the certificate.
- **Pairing:** "Pair new phone" opens a window with the QR code, the PC name, the short ID and a 5-minute countdown. A token is single use and compared in constant time, and five wrong guesses in 60 seconds kill it. Closing the window ends the pairing. On success the window becomes the "New device paired" notice, with a button to remove the device (and a toast).
- **Addresses in the QR code:** private (RFC 1918) addresses of adapters that are up, without loopback, link-local, VPN or virtual adapters, except that the address on the default route is never left out. At most 3, the default route first, then physical adapters.
- **WebSocket:** one per device, a new one replaces the old one (4001). The PC sends `hello` at once. No `hello` in 10 seconds is a 1002 close. A PC `ping` that isn't acked in 5 seconds is "No reply" and the connection is closed. 10 minutes of silence marks the phone offline (the phone's WebSocket ping every 4 minutes keeps it alive). Message IDs are remembered in SQLite, so a repeated `ping` gets the same `ack` and no second toast.
- **Tray:** the icon (a green dot when connected, amber when offline, none when not paired) and the tooltip show the state. The menu has the status line, Pair new phone, Ping phone, Open log folder and Quit. Windows are destroyed when closed.
- **Logs:** every connect and disconnect with its reason or close code, every pairing attempt and its result, every ping, and start and stop. Never a key, a token or the pairing URL.

## Not in this milestone

`text`, `clipboard`, `offer`, `complete` and `cancel` messages (a `text` is validated and acked but goes nowhere, the others answer `unsupported_type`), `POST /v1/transfers` and the file endpoints (they answer `404`), the clipboard, settings, history, the drop window, the hotkey, "Send to", autostart, and the installer work (firewall rule, per-machine install). Those are 1.2 to 1.5.
