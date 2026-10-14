# Phase 1.1 result: integration (real Windows app + real Android app)

Run 2026-10-14 by the integration agent. The two halves of milestone 1.1 were built independently from [protocol.md](../protocol.md), by [the desktop agent](phase-1-1-desktop.md) and [the Android agent](phase-1-1-android.md). This is the first time they talked to each other. Nothing is committed.

**Setup:** the real release `Kin.exe` (`desktop/build.ps1` output, unchanged) in developer mode (`--dev-bind 127.0.0.1 --dev-qr-address 10.0.2.2`, loopback only, a throwaway data dir, never `%APPDATA%\Kin`), and the real debug APK, on the `kin_api37` (Android 17) and `kin_api29` (Android 10) emulators. The PC is reached as `10.0.2.2:47821`. `ACCESS_LOCAL_NETWORK` was granted with `adb shell pm grant` on API 37.

**Result in one line: no cross-side bug was found.** The first real pairing between the two independently written halves worked at the first attempt, and every scenario below passes. The code changes of this run are tests, a script, docs and one small fix in `tools/emulator/start.ps1`.

## Scenarios

| # | Scenario | API | Result | Evidence |
|---|---|---|---|---|
| 1 | Android-generated certificate accepted by the PC verifier | host | **PASS** | `cargo test` (`android_generated_certificate_is_accepted_when_the_fixture_exists`): profile check, `P256ClientVerifier`, **rustls-webpki `EndEntityCert` parse**, device ID equals the sidecar. `check-vectors.ts`: profile, self-signature, SPKI, device ID. The real handshake with this certificate is scenario 2 (the app's key can't leave the app, so there is no key fixture) |
| 2a | Pair through the deep link | 37 | **PASS** | Link opened, phone "Connected", PC `phone_connected`, 3.6 s from the link to connected (cold app, two TLS handshakes) |
| 2b | Device rows match | 37 | **PASS** | PC ID on the phone `laxXyRYe…` = `status.json deviceId`; phone ID `2RQ3ypsR…` = the PC's `devices[0].deviceId`; Home shows `PC ID: SWWF-PSIW` and `This phone: 3EKD-PSU3`, and both short IDs were recomputed from the full IDs independently |
| 2c | Ping phone → PC | 37 | **PASS** | RTT 9.3 ms (first), 8.3 ms; PC logs `toast: Ping from sdk_gphone64_x86_64` |
| 2d | Ping PC → phone | 37 | **PASS** | RTT 4.1 ms (`Ping: 4 ms`, `lastPing.rttMs` in `status.json`); phone notification "Ping from EMPIRE" (`dumpsys notification`) and `link: ping received from 'EMPIRE'` |
| 2e | Kill the desktop, restart it: the phone reconnects through its backoff | 37 | **PASS** | PC killed (`Stop-Process -Force`), the phone saw `Connection reset` and logged `next attempt in 5 s` (first backoff step). The PC was listening again 4.0 s after the kill, and the phone was connected **6.1 s after the kill, 2.1 s after the PC was listening again** (the 5 s step ran out 2 s later) |
| 2f | Emulator Wi-Fi off, then on | 37 | **PASS** | `Wi-Fi lost` → `closing with 1000 (Wi-Fi lost)` → state `NotOnWifi` (read with `STATE`). `Wi-Fi available` → connected **0.1 s later** (a new loop per network, no backoff). See open issue 1: the PC didn't learn the phone had gone |
| 2g | `kill -9` of the app process | 37 | **PASS** | `START_STICKY` restarted the service (new pid, `isForeground=true types=0x10`), `kill detected`, `kills=1`, connected again 1.9 s after the kill. The PC logged the broken connection (`peer closed connection without sending TLS close_notify`) and accepted the new one |
| 2h | `am force-stop` | 37 | **PASS** (Android's rule) | Force-stop ends the service and nothing restarts it, as Android defines. The PC shows `phone_offline`. Opening the app starts the service: `kill detected`, `kills=2`, connected |
| 2i | Replacement (4001) | 37 | **PARTIAL** | The PC side is seen for real: it closes the old socket with 4001 and logs `The phone will: reconnect_on_user_action`. The phone side can't be triggered end to end, because a second connection needs the phone's own key (the Keystore wraps it). The phone's 4001 rule (no auto reconnect, a user action reconnects) is covered by `LinkLoopTest`, `LinkSessionTest`, `closeCodes[]` and the earlier emulator run with the fake PC. Re-pairing while connected does not open a second WebSocket, so it doesn't trigger it either |
| 2j | Unpair from the phone | 37 | **PASS** | `closing with 1000 (unpair)`, PC `device … unpaired itself`, `devices: []`, `not_paired`, phone service stopped, onboarding |
| 2k | Unpair from the PC | 37 | **PASS** | "Remove this device" in the pairing window (UI Automation `Invoke`): PC sends `unpair`, closes 1000 `unpaired`, phone `the PC unpaired this phone`, row removed, service stopped, `Stopped paired=null` |
| 2l | Re-pair after unpairing | 37 | **PASS** | New token, deep link, connected in 2.2 s, ping 8.3 ms. Pairing again while paired just updates the record (`pairedAt`) |
| 2m | Reused token | 37 | **PASS** | HTTP 401 `token_used`, phone shows "That code was already used. Show a new one on your PC." (screenshot). When the phone is already paired, Home doesn't show the message (it only goes to the log and `pairing failed: TOKEN_USED`); see open issue 4 |
| 2n | Expired token | 37 | **PASS** | Two cases. A token **invalidated** by starting a new pairing, and a token that really **timed out after 300 s** (used at +5 min 43 s): both HTTP 401 `token_expired`, phone shows "That code has expired. Show a new one on your PC." |
| 2o | Logs: a reason for every connect and disconnect, no tokens, no keys | 37 | **PASS** | All PC `websocket ended` lines and all phone `connection ended` lines carry a close code or an error and the action. Searched both logs and the whole logcat for the text of the two tokens that were recorded, for `tk=`, and for `PRIVATE KEY`: no match. The word "token" appears only in the `token_used` / `token_expired` outcomes. The e2e script repeats this check |
| 2p | Liveness over 3.5 minutes | 37 | **PASS** | Phone ping interval set to 30 s (`SET_PING_INTERVAL`), connection held 3.5 min: no disconnect on either side, then a PC ping answered in 6.3 ms. (WebSocket ping frames from OkHttp are answered by tungstenite) |
| 3 | Pair, ping both ways (`tools/e2e-1-1.ps1 -Avd kin_api29`) | 29 | **PASS** | Paired in 3.2 s, phone → PC 8.0 ms, PC → phone 7.5 ms, "Ping from" shown on both sides, 15 checks. No permission prompts on API 29 |
| 4 | The e2e script itself | 37 and 29 | **PASS** | 16 checks on API 37, 15 on API 29, exit code 0 and nothing left running; exit code 1 with a bad `-Kin` path |

RTT summary (loopback through the emulator's NAT, so these are small): phone → PC 8.0 to 9.3 ms (38 ms for the very first ping after a cold start of the app), PC → phone 4.1 to 7.5 ms.

Reconnect summary: desktop restart 6.1 s after the kill (5 s backoff step); network back 0.1 s; process kill 1.9 s; re-pair 2.2 s; first pairing 3.2 to 3.6 s.

## Bugs found and fixed

No bug in `desktop/` or `android/`: the two codebases agreed on everything the run touched (certificate profile, handshake, pairing, `hello`, `ping`, `unpair`, close codes, backoff). What did need fixing was around them:

| Where | What | Fix |
|---|---|---|
| `tools/emulator/start.ps1:25` | When the same AVD is already running the script printed the serial and then `return`ed. Run with `-File`, PowerShell then exits with the exit code of the last native command (`adb … emu avd name`), which was non-zero, so a caller saw a failed start | `return` became `exit 0` |
| `desktop/src-tauri/tests/tls_vectors.rs:268` | The Android fixture test only checked the profile and the SPKI check of the verifier. That doesn't parse the certificate the way `verify_tls13_signature` does | Also parses it with `webpki::EndEntityCert::try_from` (version, extensions, encodings) and compares the device ID with the sidecar |
| `protocol/fixtures/android-client.{der,json}` | New fixture, as `protocol/README.md` asked for the first Android-generated certificate | Copied from `android/app/src/test/resources/fixtures/android-generated-cert.der`, the sidecar has `deviceId`, `shortId`, `spkiHex`. `check-vectors.ts` (end of the `fixtures` block) checks the profile, the self-signature and the sidecar. `protocol/README.md` (the "Android fixture" paragraph) describes it |

A trap for automation, handled in the script but not a bug in a script: **`start.ps1` starts the adb daemon, which inherits the caller's output pipe**, so a caller that reads the pipe (an agent shell, a CI step) never sees `start.ps1` end, although the emulator booted in 26 s. `tools/e2e-1-1.ps1` runs the emulator scripts as child processes with output in files and waits for the process only.

## Spec problems

None found, and `docs/protocol.md` wasn't touched. `docs/android.md` got the missing permission (the Android agent's finding, now in the permissions table): `ACCESS_LOCAL_NETWORK`, runtime, API 37, "Nearby devices", with the reason. Without it, on API 37, a connect to `10.0.2.2` just times out.

## Tests after the changes

| Suite | Result |
|---|---|
| `cargo test` (`desktop/src-tauri`) | 88 passed (45 unit, 20 fake phone, 12 `tls_vectors`, 11 `vectors`), 0 failed |
| `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` | clean |
| `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` | BUILD SUCCESSFUL, 108 tests, 0 failed, 1 skipped (the opt-in fake PC) |
| `bun protocol/check-vectors.ts` | OK, 5,250 checks (4 more than before: the Android fixture) |
| `tools/e2e-1-1.ps1` | OK on `kin_api37` (16 checks) and `kin_api29` (15 checks) |

## `tools/e2e-1-1.ps1`

```
powershell -ExecutionPolicy Bypass -File tools\e2e-1-1.ps1                 # kin_api37
powershell -ExecutionPolicy Bypass -File tools\e2e-1-1.ps1 -Avd kin_api29
powershell -ExecutionPolicy Bypass -File tools\e2e-1-1.ps1 -Build          # builds Kin.exe and the debug APK first
powershell -ExecutionPolicy Bypass -File tools\e2e-1-1.ps1 -KeepRunning    # leaves emulator and Kin up, for debugging
```

It starts the emulator, installs the debug APK, grants the permissions (`ACCESS_LOCAL_NETWORK` only on API 37+), starts `Kin.exe` in developer mode (loopback only, a data dir under `%TEMP%`), pairs through the deep link, checks that the device rows match, pings both ways, checks `status.json`, the phone state and both logs (no token, no `PRIVATE KEY`), unpairs from the phone and checks that the PC cleaned up. It tears everything down in a `finally` (Kin `--quit`, `stop.ps1 -All`, adb server) and removes the data dir (kept on failure, the path is printed). Exit code 0 only if every check passed. It refuses to start if a `Kin.exe` is already running. About 50 s once the emulator is up. The place for later milestones is marked `EXTEND HERE`.

## Open issues

1. **The PC shows a stale "connected" after the phone loses Wi-Fi.** The phone aborts its socket locally (`Software caused connection abort`) and the close frame never reaches the PC, so `status.json` and the tray stay `phone_connected` until the phone comes back (the PC then closes the old socket with 4001) or the 10 minutes of silence of the PC pass. The spec allows it (10 minutes of silence, 4 minute phone pings). For a user it means "connected" on the PC while the phone is away. Options for 1.4: the PC sends its own WebSocket ping every 30 to 60 s and treats a missing pong as offline.
2. **4001 on the phone side was not driven end to end** (see 2i). A way to do it later: a debug-only command that opens a second connection with the phone's own identity.
3. **`BOOT_COMPLETED` showed up once after `am force-stop` + launch** on API 37 (`boot: BOOT_COMPLETED received`, a few ms after `app: process started`). It is harmless (the receiver only starts the service when a PC is paired), but it wasn't investigated, and the real-phone soak should look at how the OEM handles it.
4. **Pairing errors aren't shown when the phone is already paired.** The reused-token and expired-token messages are shown on the onboarding screen. If the phone is already paired and a stale link is opened, Home shows nothing and the error is only in the log. Probably fine for the MVP (pairing a second PC isn't a flow yet).
5. **Cosmetic log lines on the phone:** after a phone-initiated unpair the loop logs `-> RECONNECT_WITH_BACKOFF` and `PC not reachable; next attempt in 5 s` for a moment before the service stops (`link/LinkLoop.kt:111` and `:138`; `peerUnpaired` is only set when the PC unpairs). Nothing reconnects.
6. **`status.json` is a snapshot:** `pairing` stays `{ active: true, expiresAtMs }` after the expiry time, until the next state change. The pairing itself is refused correctly (2n).
7. **Not tested here:** mDNS (the emulator can't), a normal (non-developer) start of the Windows app (it binds `0.0.0.0` and would raise a firewall prompt), a real phone and its Wi-Fi, Doze, OEM killers, the camera scanner, Windows 10, the installed app. The phone does two TLS handshakes per connection (the address race, then the WebSocket); on a cold process the first one took about 510 ms every time (22 ms on a warm one), which is worth a look in the soak.
8. **Docs still to do (not asked):** `docs/risks.md` and the Phase 1 test plan should list `ACCESS_LOCAL_NETWORK` ("permission denied" and "revoked later"), as the Android agent suggested.
