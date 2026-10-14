# Phase 1.1 result: the Windows app

Built and measured 2026-10-14 on Windows 11 Pro 10.0.26300, Ryzen 9 7900X, 31 GB RAM (the same PC as [phase-0-tauri.md](phase-0-tauri.md)). Code is in `desktop/` (see [desktop/README.md](../../desktop/README.md)). This is the Windows half of milestone 1.1 ([phase-1-mvp.md](../phases/phase-1-mvp.md)): the Android half is built from the same spec by another agent, and the integration agent pairs the two on the emulator. Nothing here is committed.

## What was built

Tauri v2 with a Rust backend and a TypeScript UI (Vite, plain DOM, no component library). The protocol code has no UI in it, so the tests run the real server in the test process.

| Area | What it does | Where |
|---|---|---|
| Identity | P-256 key and self-signed X.509 v3 certificate (rcgen: `generate_for(&PKCS_ECDSA_P256_SHA256)`, `CertificateParams::default()` with CN=Kin, `is_ca = NoCa`, no key usages, no SAN, no custom extensions, serial left to rcgen). The result has no extensions at all, which a test checks. Key DPAPI-encrypted in `%APPDATA%\Kin\`. Device ID `base64url(SHA-256(SPKI))`, short ID the first 8 base32 characters | `identity.rs`, `dpapi.rs`, `encoding.rs` |
| TLS | TLS 1.3 only (rustls built without the `tls12` feature, and the version list is explicit), ALPN `http/1.1`, no session tickets. The client certificate is mandatory. The verifier accepts any chain but only an ECDSA P-256 leaf (SPKI is `id-ecPublicKey` + `prime256v1`, 91 bytes), offers only `ecdsa_secp256r1_sha256`, and verifies the CertificateVerify signature with a P-256-only algorithm list. It never returns an assertion without verifying | `tls.rs` |
| Server | axum on a custom accept loop: the loop does the handshake, computes the device ID from the leaf certificate and attaches it to every request. `POST /v1/pair`, `GET /v1/ws`, `404` JSON for everything else (also wrong methods, paired or not). No axum `Json` extractor: the body is read with a 1 MiB limit, the Content-Type rule is applied by hand, and everything that fails is `400 bad_request` JSON. Port 47821, or a random one. 64 connections at most | `server.rs` |
| Pairing | Tray "Pair new phone" opens a window with the QR code (SVG from `qrcode`), the PC name, the short ID and the 5-minute countdown. Single-use token (16 random bytes), constant-time compare, 5 failures per token, check and "used" mark under one lock and consumed only after the device is saved. Canonical URL as the spec writes it. On success the window becomes "New device paired" with a "Remove this device" button, and a toast shows | `pairing.rs`, `core.rs`, `ui.rs`, `src/pairing.ts` |
| Addresses | RFC 1918 addresses of adapters that are up, without loopback, link-local, VPN and virtual adapters, except that the default-route address is never left out. At most 3: the default route first, then physical adapters. Read with `GetAdaptersAddresses` and `GetBestInterface` (no packet is sent) | `netaddr.rs` |
| mDNS | `_kinrelay._tcp` with TXT `id` (short ID) and `v=1` (mdns-sd). Not started in developer network mode | `mdns.rs` |
| WebSocket | `hello` at once from both sides, the validation order, strict JSON (depth 64, lone surrogates, duplicate keys), 1 MiB limit, invalid UTF-8 → 1007, binary → 1003, 10 s hello timer → 1002, 4001 replaced, 4002, 4003, `ack`, `error`, `ping` both ways (RTT with a monotonic clock, 5 s ack timeout), `unpair` both ways, de-duplication in SQLite (24 h and at least 1,000 IDs per device). Liveness: WebSocket ping frames are answered, 10 minutes of silence closes the connection | `messages.rs`, `json.rs`, `ws.rs`, `connections.rs` |
| Storage | SQLite (rusqlite, bundled): `devices` (device ID, name, platform, model, app version, paired time, last seen, last addresses) and `seen_messages` | `storage.rs` |
| Tray | Icon and tooltip show the state: no dot and "Kin: not paired", green dot and "Kin: <name> connected", amber dot and "Kin: <name> offline". Menu: status line, Pair new phone, Ping phone, Open log folder, Quit. Single instance (official plugin). Windows are destroyed on close | `tray.rs`, `app.rs` |
| Logs | Rotating files in `%APPDATA%\Kin\logs`, 5 × 1 MB. Every connect and disconnect with the reason or close code, every pairing attempt and result, every ping, start and stop. No key, token or pairing URL ever (a test checks this over a full pair-connect-ping flow) | `filelog.rs` |

## Developer and integration hooks

All documented in [desktop/README.md](../../desktop/README.md). The short version, with a throwaway profile and nothing listening outside loopback:

```powershell
$kin = 'C:\Dev\ecosystem-app\desktop\src-tauri\target\release\Kin.exe'
$data = "$env:TEMP\kin-emulator"
Start-Process $kin -ArgumentList '--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-print-pair-url', '--dev-no-windows'
& $kin --dev-start-pairing                                    # forwarded to the running instance
(Get-Content $data\pair-url.txt).Trim()                       # kinrelay://pair?v=1&a=10.0.2.2:<port>&id=...&tk=...&n=<PC name>
& $kin --ping                                                 # "Ping phone"
Get-Content $data\status.json                                 # state, port, devices[], lastPing, pairing
& $kin --quit
```

| Need | How |
|---|---|
| Loopback only, `10.0.2.2:<port>` in the QR | `--dev-bind 127.0.0.1 --dev-qr-address 10.0.2.2` (or `KIN_DEV_BIND`, `KIN_DEV_QR_ADDRESS`). `--dev-bind` refuses anything that isn't a loopback address, and `--dev-qr-address` alone also means loopback. mDNS is skipped |
| Pairing URL without the GUI | `--dev-print-pair-url` writes it to `pair-url.txt` (and stdout) whenever a pairing starts. `--dev-start-pairing` starts one without a window |
| Headless "Ping phone" | `kin.exe --ping`, forwarded by the single-instance plugin. The answer is `lastPing` in `status.json` and a `toast:` line in the log |
| State | `status.json` in the data directory, rewritten on every state change (`not_paired`, `phone_connected`, `phone_offline`, `stopped`) |
| Throwaway profile | `--data-dir` or `KIN_DATA_DIR` |
| No windows or toasts | `--dev-no-windows` (the toast text goes to the log) |
| Stop it cleanly | `kin.exe --quit` |
| A phone without Android | `cargo run --example fake_phone -- "<pairing URL>" --host 127.0.0.1 --ping-every 5` pairs, says `hello`, acks pings and sends its own |

## Tests

`cargo test` in `desktop/src-tauri`: **88 tests, all pass.** `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean. `bun protocol/check-vectors.ts` still passes (5,246 checks).

| File | Tests | What |
|---|---|---|
| `src/*.rs` (unit) | 45 | encodings, JSON rules, pairing tokens (single use, 300 s expiry, 5 failures in 60 s, the one-hour memory), the URL writer, address selection (VPN, Hyper-V, default route kept, more than 3), CLI and developer flags, SQLite incl. the 24 h / 1,000 IDs rule, log rotation, DPAPI round trip, the certificate profile of a generated certificate, tray icons and text, mDNS record |
| `tests/vectors.rs` | 11 | The vector files in scope for 1.1, run against the real code: `device-id.json` (4 keys with the profile check of every certificate, 5 hashes, 5 tokens, 13 shapes), the 10 canonical URLs of `pairing-urls.json`, `errors[]` (12 codes), `messages.json` (20 valid, 107 invalid, 5 sequences, 3 timers, 24 close codes) and `http.json` (64 exchanges and 2 sequences through the real router with the peer attached as the accept loop does). Every scope pattern of protocol/README.md is checked to match a case |
| `tests/tls_vectors.rs` | 12 | The 10 PC-side `tls[]` cases with real TLS on loopback: no client certificate, unpaired and unknown-issuer and extension-carrying certificates accepted, v1, unknown critical extension, non-P-256, a replayed certificate (valid certificate, wrong key) and a corrupted CertificateVerify refused. Each refusal is read on the client's first read (TLS 1.3 timing), and the server's log line is checked, so each case fails for the reason it is about (`UnsupportedCertVersion`, `UnsupportedCriticalExtension`, `BadSignature`, `ApplicationVerificationFailure` for the P-384 key). Plus a control that accepts the same client when it signs honestly, and a test that picks up an Android certificate fixture when one is added |
| `tests/fake_phone.rs` | 20 | The in-process fake phone: pairs with a real token from the QR URL, 401 before and after pairing, token reuse, 5-failure limit, a 413 over TLS, WebSocket `hello`, `ping` both ways with the RTT, a repeated `ping` (same `ack`, one toast), WebSocket ping frames, 4001 on a second connection, `unpair` from the phone (ack, 1000, then 401) and from the PC (`unpair` message, 1000, then 401), invalid messages keep the connection open, 1003, exactly 1 MiB accepted and 1 MiB + 1 closed, invalid UTF-8 → 1007, 10 s without `hello` → 1002, a message before `hello` → 1002, a future `hello` → 4002, "No reply" after 5 s and a close, app quit → 1000, and the log check (story present, no token, URL or key) |

The DER fixtures for the negative cases are in `protocol/fixtures/` (`client-v1.der`, `client-unknown-critical-extension.der`, `client-not-p256.der` and its key), made by `protocol/fixtures/make-fixtures.ts`, documented in `protocol/README.md`, and checked by `check-vectors.ts` (each leaves the profile in exactly the way its name says and keeps a valid self-signature).

**Checked by hand on the release build** (not part of `cargo test`): the real `Kin.exe` with the developer flags; a second launch forwarding `--dev-start-pairing`, `--ping` and `--quit`; listening only on `127.0.0.1` (no other TCP or any UDP endpoint); the pairing window showing the QR code, the name, the short ID and the countdown (screenshots read by the agent); the fake phone pairing, connecting, acking a PC ping (`Ping: 2 ms`, `lastPing` in `status.json`) and sending its own pings (`Ping from Fake Phone` in the log); "Remove this device" through UI Automation (the phone gets `unpair`, then 1000 "unpaired", the window shows "Device removed", Done destroys it and no WebView2 process is left); and the tray tooltip changing with the state, read from the overflow flyout through UI Automation: `Kin: not paired`, then `Kin: Fake Phone connected`, then `Kin: Fake Phone offline`.

## Idle memory (release build)

Release `Kin.exe`, tray only, no window ever opened, **60 s after launch**, measured as in [phase-0-tauri.md](phase-0-tauri.md): private working set from `Win32_PerfFormattedData_PerfProc_Process`, working set and private bytes from `Get-Process`. `desktop/measure-idle.ps1`, three runs with a throwaway profile, all identical to 0.1 MB:

| Working set | **Private working set** | Private bytes | Processes | Threads | Handles |
|---|---|---|---|---|---|
| 16.4 MB | **2.7 MB** | 3.9 MB | 1 | 28 | 216 |

That is the number for [targets.md](../targets.md)' "under 30 MB active private working set". The spike (tray only, no server) was 1.5 MB, so the server, SQLite, TLS and the tokio threads cost about 1.2 MB. **Caveats:** it was measured with the developer network settings (loopback only, no mDNS), because a normal start binds `0.0.0.0` and starts mDNS, and that would raise a Windows Firewall prompt (not allowed for this task). The mDNS daemon of a normal run is not in the number. It is also not the 1-hour figure of the target, and no phone was connected. A window costs what the spike measured (about 73 MB private working set while open, freed when it is destroyed): with the pairing window open, `Kin.exe` itself was at about 38 MB working set, and no WebView2 process was left after closing it.

## Sizes

| Artifact | Size |
|---|---|
| `Kin.exe` (release: `lto`, `opt-level = "s"`, `strip`) | 10,822,144 bytes = **10.32 MiB** |
| NSIS installer `Kin_0.1.0_x64-setup.exe` | 3,319,464 bytes = **3.17 MiB** (built, not run) |

The spike's hello world was 3.04 MiB and 1.08 MiB. The rest is what the real app adds: bundled SQLite (C), rustls with aws-lc-rs, axum and hyper, mdns-sd, rcgen and x509-parser, qrcode and the `windows` crate. The installer is about a third of the "about 10 MB" target. The WebView2 runtime is not in it (the default bootstrapper). A release build took 1 m 40 s to 3 m when the crate changed (LTO), and about 4 minutes from a cold `target\`.

## Spec ambiguities and how they were resolved

Nothing in protocol.md was changed in behaviour. These are the places where the spec or the vectors were silent, disagreed, or needed a reading:

1. **The 1.1 scope list needs a `text` handler.** protocol/README.md says the 1.1 `invalid[]` cases need no `text` handler, but the 1.1 sequence `garbage-before-hello-keeps-the-connection-open` ends with a valid `text` that must be acked (`deliver_text`). Resolved: the receiver validates `text` (id, non-empty `text`, boolean `isUrl`), acks it and remembers its ID, and the app logs the length and does nothing else (the text is never logged). Delivering it is 1.2. A note was added to protocol/README.md. `clipboard`, `offer`, `complete` and `cancel` still answer `unsupported_type` (the first two are the wrong direction for the PC anyway).
2. **`closeCodes[]` describes the phone.** The PC never reconnects, so `client_action` and `resets_backoff` are pure functions tested against the vectors, and the PC uses the first one to write what the phone will do into the disconnect log line. `timers[]` is the 10 s hello rule: the driver's timer calls `Receiver::hello_timeout`.
3. **"Before the body is read" (protocol.md, Content-Type paragraph) contradicts the 1 MiB `413`**, which needs the body size. Changed the sentence to "after `401` and the 1 MiB `413`, before the body is parsed" (a reviewer nit asked for the "1 MiB `413`" wording). Order implemented: 413, then Content-Type, then JSON, then fields, then protocol, then token.
4. **A `413` the client can read.** A body over the limit is read to the end (up to 16 MiB, then the connection just ends) and thrown away, so the phone can read the `413` instead of a reset.
5. **1009 may look like a reset.** The spec says to set the library limit to 1 MiB so it stops reading early. A client still writing the rest of an oversize frame can then see a connection reset before it reads the close frame. The PC closes with 1009 and no reply either way. The test accepts either.
6. **A pairing that is closed or replaced.** Closing the pairing window, or starting a new pairing, invalidates the token on screen. It then answers `token_expired` (protocol.md says "expired or was invalidated"). After a success the token answers `token_used`, and no pairing is in progress.
7. **Which token a wrong guess counts against.** The active token, as written. With no pairing in progress nothing is counted (it answers `bad_token`).
8. **Close codes the spec doesn't assign.** 10 minutes of silence, an unacked PC `ping`, and app quit all close with 1000 and a reason text. Removing a device whose `hello` isn't accepted yet closes with 4003 (an accepted one gets the `unpair` message first and then 1000).
9. **`GET /v1/ws` from a paired device without upgrade headers** is `400 bad_request` (the spec doesn't say). From an unpaired one it is `401`, as the vector says.
10. **`/v1/transfers` and the file routes answer `404`** for everyone in 1.1, although the spec has `401` for an unpaired device on them. They are 1.3. Unknown routes and methods are `404` for everyone, as the spec says.
11. **`closeCodes` and 4001.** The PC doesn't retry, so no state is kept. The old connection ending after a newer one took its place doesn't touch the newer one (a test checks this).
12. **Certificate profile at the verifier.** `verify_client_cert` checks only the SPKI (the profile's key rule). Version, critical extensions and encodings are checked by rustls-webpki inside `verify_tls13_signature`, as protocol.md says. `certificate_profile_violations` (used by tests) checks all of the profile.
13. **No session tickets.** Resumed TLS 1.3 sessions would skip the CertificateVerify. Every connection proves its key, so tickets are off.
14. **P-384 under the P-256 label.** The one signature that the default rustls list would wrongly accept (P-384 key, SHA-256) can't be made with aws-lc-rs, so the handshake test can't show the default list failing. The P-256-only list and the SPKI check are each tested: the verifier refuses the P-384 certificate before looking at a signature (`ApplicationVerificationFailure` in the log), and a unit test calls it directly.
15. **PC name.** `COMPUTERNAME`. There is no setting for it before 1.4.

## Known gaps and not verified

- **mDNS was never run on this PC**, because it opens multicast sockets on every interface and that would raise a Windows Firewall prompt. Only the record (service type, instance, host, TXT `id` and `v`, auto addresses) is unit tested. The first real test is on a phone. The socket-sharing question with Windows' own responder on port 5353 (Phase 0) is still open.
- **A normal (non-developer) start was never run**, for the same reason: it binds `0.0.0.0`. Address selection was run against this PC's real adapters (a test prints them) but no QR code with real LAN addresses was shown.
- **Windows 10** and the **installed app** were not tested. The installer was built but not run. Toasts from an unpackaged exe may carry PowerShell's name until the app is installed (see phase-0-tauri.md). The toast code path was exercised only through the log.
- **Tray:** the tooltip was read through UI Automation for all three states (see above). The dot colours are unit tested but not looked at on screen, and the menu items were not clicked through UI Automation this time. "Pair new phone", "Ping phone" and "Quit" use the same handlers as `--pair`, `--ping` and `--quit`, which were run. "Open log folder" starts `explorer.exe` and was not run. Left-click does nothing (the drop window is 1.2).
- **Several phones** work in the data model (one WebSocket each), but the tray and the notice show only one (a connected one first).
- **Not in 1.1:** text delivery, clipboard, files, settings, history, autostart, "Send to", the hotkey, the firewall rule and the per-machine install (1.5).
- A dev instance and an installed Kin share the single-instance slot (same app identifier).
- The soak test of 1.1 needs the real phones and is not part of this.
