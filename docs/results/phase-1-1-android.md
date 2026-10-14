# Phase 1.1 results: Android app

Done 2026-10-14 by the Android coding agent. The code is in `android/` (nothing is committed). How to build, test and drive it: [android/README.md](../../android/README.md).

Summary: the app pairs with a PC, keeps one pinned, mutually authenticated TLS 1.3 WebSocket open from a foreground service, answers and sends `ping`, and logs every connect, disconnect and service start, stop and kill. It was run end to end on the emulator (API 37 and API 29) against an in-JVM fake PC. It has **not** been paired with the real desktop app yet, and nothing was run on a phone.

## Read this first: a spec gap (Android 17 local network permission)

`android.md` lists the permissions, and `ACCESS_LOCAL_NETWORK` is not in it. Apps that target API 37 (ours do, as `targetSdk` was chosen) need this runtime permission ("Nearby devices" group) for **any** connection to a local network address: TCP, UDP, multicast and `NsdManager`, including `10.0.2.2`. Without it a connect just **times out** (no exception that says why). This is what the first emulator run showed: `nc` from the adb shell reached the PC, the app did not.

What the app does now:

- Declares `ACCESS_LOCAL_NETWORK` and asks for it (together with `POST_NOTIFICATIONS`) when the app opens, and before a pairing starts (including from a deep link). Requests are serialized: a second request while a prompt is open comes back empty.
- If pairing then fails with "unreachable" and the permission is missing, it says so ("Kin isn't allowed to reach devices on your network…"). Home shows a card with an "Allow in settings" button while the permission is missing.
- The permission does not exist before Android 17, so API 29 to 36 are unchanged.
- **For scripts:** `adb shell pm grant app.kinrelay.android android.permission.ACCESS_LOCAL_NETWORK` works (verified on API 37). The integration agent must do this once after installing, or tap Allow.

`docs/android.md` and `docs/risks.md` should get this permission, and the Phase 1 test plan should include "permission denied" and "permission revoked later". Also: this permission may matter for the battery and OEM work in 1.4 (it can be revoked in settings), and for the Phase 2 bridge decision (mobile data traffic is not local, so not affected).

## What was built

| Package | Contents |
|---|---|
| `identity` | `Der` (small DER encoder), `CertificateBuilder` (the profile certificate), `CertificateInfo` (DER reader for SPKI and the profile fields, used at runtime and in tests), `DeviceIds`, `Identity`, `IdentityStore`, `KeystoreKeyWrapper` (AES-256-GCM, key in the Android Keystore) |
| `tls` | `PinnedTrustManager` (pin on SHA-256(SPKI), no other trust), `ClientKeyManager`, `KinTls` (TLS 1.3-only OkHttp client, HTTP/1.1, IP-address hostname verifier on this client only), `AddressRacer` |
| `protocol` | `StrictJson`, `Fields` (strict typed reads), `Messages` (frame builders, UUIDv7, UTF-8 size), `MessageReceiver` (framing, validation order, replies), `DedupMemory`, `CloseRules` |
| `pairing` | `PairingUrl` (own parser, both forms), `PairingClient` (race, pin, `POST /v1/pair`, check the answer), `PairingController`, messages |
| `link` | `LinkService` (foreground, `connectedDevice`, `START_STICKY`), `LinkRuntime` (Wi-Fi callback, one `LinkLoop` per Wi-Fi network, sockets bound to that network), `LinkLoop` (connection order, backoff, close actions), `LinkSession` (one WebSocket), `MdnsLookup`, `Notifications`, `BootReceiver`, `Backoff` |
| `data` | Room 3: `devices`, `seen_messages` (de-dup memory, persisted). DataStore: ping interval, counters, kill detection |
| `log` | `FileLog` (rotating, 256 KiB × 2), `Redactor` |
| `ui` | Compose: onboarding (Scan code, paste a link), home (PC status, short IDs, Ping PC with RTT, Unpair), debug (log lines, counts, ping interval choice) |
| `src/debug` | `DebugCommandReceiver` (debug builds only, `DUMP`-protected) |

Behaviour as specified, in one place:

- **Identity (D15):** a software P-256 key and a hand-rolled X.509 v3 certificate, created on first run. The private key is stored AES-GCM encrypted under a Keystore key (`identity.key`); the certificate is `identity.crt`. If the key file can't be decrypted (a lost Keystore key), a new identity is created, the paired PCs are removed (the PC knows the old ID) and it is logged. Device ID and short ID as specified.
- **Pairing:** deep links `kinrelay://pair?...` and `https://<any host>/pair#...` (an intent filter with `host="*"`, `/pair` and `/pair/`), the Google code scanner, the paste field. Address race with a real TLS 1.3 handshake, pin, `POST /v1/pair` with `Content-Type: application/json; charset=utf-8`, answer checked (`deviceId` equals the pinned id, `protocol` 1), then the PC is saved in Room and the service starts. Pairing a second PC replaces the first one locally (one PC in the MVP).
- **Connection order:** last working addresses raced with a 1.5 s timeout, then mDNS for at most 5 s (match on `id` and `v=1`), then backoff 5 s, 30 s, 2 min, then every 10 min. Reset only by a user action (opening the app, Ping, Reconnect, `CONNECT`), a network change (a new loop per Wi-Fi network), or a connection that kept an accepted `hello` for 60 s (immediate reconnect after that).
- **Close codes:** 4001 waits for a user action, 4002 stops until a user action ("Update the app"), 4003 and `401` on the upgrade stop for good ("Pair again"), everything else reconnects with backoff. On 1002, 1003, 1009 sent by us, the same applies.
- **WebSocket ping:** OkHttp `pingInterval`, 240 s by default; the Debug tab sets 1, 4 or 10 min (`SET_PING_INTERVAL` from adb). A missing pong fails the connection.
- **No wake locks, no polling** while idle. mDNS runs only while reconnecting.
- **Logging:** `kin.log` (+ `kin.log.1`) in `files/logs/`, also mirrored to logcat with the tag `KinRelay`. Pairing tokens (`tk=`, `"token"`), and PEM private keys are redacted before any line is written. Keys are never passed to the logger.

## Library versions (checked on 2026-10-14 against Maven Central and Google Maven)

| Library | Version | Note |
|---|---|---|
| OkHttp (`okhttp`, `mockwebserver3`, `okhttp-tls` for tests) | 5.5.0 | latest stable |
| kotlinx-serialization-json | 1.11.0 | 1.12.0-RC is newer but not stable. Only `JsonElement` is used, so the serialization compiler plugin is not applied |
| kotlinx-coroutines (android, test) | 1.11.0 | |
| Room | **3.0.3** (`androidx.room3`) | stable successor of Room 2.8.5. KSP only, uses `AndroidSQLiteDriver` (no bundled SQLite) |
| KSP | 2.3.12 | works with AGP 9.4.1's built-in Kotlin and Kotlin 2.4.21 |
| DataStore (preferences) | 1.2.1 | |
| play-services-code-scanner | 16.1.0 | |
| androidx.fragment | 1.9.1 | **added on purpose**: the code scanner pulls in `androidx.fragment` 1.0.0, and lint (correctly) flags the Activity Result API against it |
| AGP, Kotlin, Compose BOM, activity, core, lifecycle | unchanged from Phase 0 | |

No dependency injection framework; `AppGraph` wires everything by hand.

## Certificate approach: hand-rolled, no BouncyCastle

`CertificateBuilder` writes the `minimal` layout of `protocol/device-id.json` with a small DER encoder: version `[0] 2`, a 20-byte serial (first byte 1 to 127, so it is positive and stays at 20 bytes), `ecdsa-with-SHA256` without parameters in both places, `CN=Kin` as subject and issuer, validity from 1 day ago for 100 years (UTCTime up to 2049 and GeneralizedTime from 2050, tested at the boundary), the 91-byte SPKI of the key, **no extensions**, signed with `SHA256withECDSA`. The generated certificates are 289 bytes.

- **Size impact:** about 330 lines of Kotlin (`Der`, `CertificateBuilder`, `CertificateInfo`) and no dependency. The whole release APK is 3.3 MB with R8, so BouncyCastle (which adds several MB before shrinking, and keep rules) was not needed and not measured.
- **Checked three ways:** (1) a unit test against the profile (v3, curve, algorithm with no parameters inside and outside, serial length and sign, time encodings, no critical extension, no extensions); (2) `java.security.cert.X509Certificate` parses it and `verify()` passes; (3) the committed fixture was also parsed by Bun's `node:crypto` `X509Certificate`: `CN=Kin`, 20-byte serial, `prime256v1`, self-signature verifies, the device ID matches, and the AlgorithmIdentifier without parameters occurs twice. `bun protocol/check-vectors.ts` passes (5246 checks) with the current vectors.
- **Real TLS:** Conscrypt on API 37 and API 29 presented it to a JSSE TLS 1.3 server with a mandatory client certificate, and the server took the device ID from it. **What this does not prove** is that rustls-webpki accepts it; that is what the fixture is for.
- **Fixture for the desktop tests:** `android/app/src/test/resources/fixtures/android-generated-cert.der` (also `.pem`), and `android-generated-cert.json` with the expected `deviceId` (`nrOqgVDbtcLFKV2mp2ScXpkjV82JOMfeh4JSL7rP61Q`, short ID `T2Z2VAKQ`) and the SPKI in hex. I did not touch `protocol/`; the desktop agent can copy or reference the files. A test checks that the committed fixture still fits the profile.

## Automation hooks

Full list and exact commands: [android/README.md](../../android/README.md). In short:

```
adb shell pm grant app.kinrelay.android android.permission.ACCESS_LOCAL_NETWORK
adb shell pm grant app.kinrelay.android android.permission.POST_NOTIFICATIONS
adb shell am start -a android.intent.action.VIEW -d "kinrelay://pair?v=1&a=10.0.2.2:47821&id=...&tk=...&n=Desk%20PC"
adb shell am broadcast -a app.kinrelay.android.debug.PING  -n app.kinrelay.android/.debug.DebugCommandReceiver
adb shell am broadcast -a app.kinrelay.android.debug.STATE -n app.kinrelay.android/.debug.DebugCommandReceiver
adb logcat -d -s KinRelay
adb shell run-as app.kinrelay.android cat files/logs/kin.log
```

Other commands: `CONNECT`, `UNPAIR`, `PAIR --es url`, `SET_PING_INTERVAL --ei seconds N`. The receiver is declared in `src/debug/AndroidManifest.xml`, so it is not in the release APK (checked: the broadcast completes with `result=0` and no log line on the release build). It is exported with `android:permission="android.permission.DUMP"`, which the adb shell holds and apps don't.

## Tests

`gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` passes. **108 JVM tests, 0 failed, 1 skipped** (the skipped one is the opt-in fake PC server, `FakePcSession`). Lint: **0 issues**. Suppressions, each justified in the code or manifest: `CustomX509TrustManager` (the pin is the trust model, it never skips a check), `AppLinkWarning` on the `https://*/pair` filter (no domain to verify yet; the PC is pinned by ID). The `androidx.fragment` warning was fixed by a dependency, not suppressed.

| Test class | Tests | What |
|---|---|---|
| `DeviceIdVectorsTest` | 7 | `device-id.json`: `keys[]` (IDs, SPKI from the PEM certificates, private key matches certificate, profile, extension variants), `hashVectors[]`, `tokenVectors[]`, `shapeVectors[]` |
| `PairingUrlVectorsTest` | 3 | all 33 `valid[]` and all 88 `invalid[]` of `pairing-urls.json` |
| `MessageVectorsTest` | 13 | the 1.1 cases of `messages.json` through the real `MessageReceiver`: **20 `valid[]`, 106 `invalid[]`**, the 5 `sequences[]`, `timers[]` (3), `closeCodes[]` (24, with the backoff reset). Plus de-dup, 1 MiB measured in bytes, frame builders, UUIDv7 |
| `StrictJsonTest` | 12 | all `strict-*`, `nesting-*`, lone-surrogate vectors through the app's decoder; depth 64 vs 65 and 100,000; the integer, boolean and ID reads; a list of RFC 8259 violations and valid texts |
| `LinkSessionTest` | 18 | real OkHttp WebSocket over TLS 1.3 against the fake PC: hello fields and order, ping RTT, no-ack, incoming ping once, 4001, protocol 2 hello, hello timeout, message before hello, garbage before hello, 1 MiB + 1, binary frame, unpair both ways, 401 on upgrade, refused client certificate, wrong pin |
| `LinkLoopTest` | 13 | virtual time: backoff steps, connection order and timeouts, the 60 s rule, instant reconnect after a stable connection, 4001 / 4002 / 4003 / 401, user action during backoff, unpair ends the loop, clean stop |
| `PairingClientTest` | 9 | success and request fields, content type, name cut by code points, **pin mismatch aborts before any request**, unreachable, race with dead addresses, TLS 1.2-only server refused, refused client certificate, bad answers, every `errors[]` entry of `http.json` |
| `TlsVectorsTest` | 6 | the phone-side `tls[]` cases: pinned self-signed accepted, expired and wrong-hostname accepted, different key rejected, no fallback trust, our certificate is what the server sees. (The optional invalid-CertificateVerify case is not written: Conscrypt guarantees it) |
| `CertificateBuilderTest` | 8 | profile, JDK parse and verify, device ID, serials, time encodings at 2049/2050, DER helpers, the committed fixture |
| `IdentityStoreTest` | 3 | reuse, not in clear, unreadable key file creates a new identity |
| `DedupMemoryTest`, `LogTest`, `PreviousRunTest` | 6, 5, 4 | 24 h / 1000 rule, redaction and rotation, kill versus reboot |

Not run in the app (by the README's own rule): `text-frame-invalid-utf8`. The `text`, `clipboard`, `offer`, `complete`, `cancel` cases are 1.2/1.3 and not run.

**The fake PC** (`testing/FakePc.kt`) is a real TLS 1.3 server with mandatory client certificates (any chain accepted, as the PC does), `POST /v1/pair`, and a WebSocket, built on MockWebServer. It also runs as a loopback-only server for the emulator (`FakePcSession`, see the README).

## APK sizes

| Build | Size |
|---|---|
| Debug | 37,083,589 bytes (35.4 MiB; Compose tooling, no shrinking) |
| Release, R8 on, unsigned | **3,416,423 bytes (3.26 MiB)** against the ~10 MB target |

The Phase 0 hello world was 1.31 MiB with R8; OkHttp, Room, DataStore, kotlinx and the Play services scanner account for the rest. No extra keep rules were needed: the R8 build ran pairing, the service and the WebSocket on the emulator (signed with the debug key for the test only).

## Emulator smoke results

`tools/emulator/start.ps1`, `install.ps1`, `screenshot.ps1`; the fake PC on `127.0.0.1:47831` (loopback only), reached as `10.0.2.2:47831`.

**API 37 (`kin_api37`, debug build)**

| Check | Result |
|---|---|
| Onboarding renders, notification and Nearby-devices prompts appear | yes |
| Deep link opens pairing | yes. Without the permission the pairing timed out (the finding above); with it: TCP, TLS 1.3, `POST /v1/pair` (the fake PC saw the right fields and took the phone's ID from the certificate), service started, WebSocket open, `hello` accepted |
| Home | "Connected", PC short ID and phone short ID |
| Service notification | "Connected to Fake PC", importance low, silent, ongoing, foreground service type `connectedDevice` (`types=0x10`) |
| Ping PC | `Rtt(millis=6.6129)` |
| Ping from the PC | notification "Ping from Fake PC" |
| 4001 | state Replaced, no reconnect; `CONNECT` reconnects |
| Future-protocol `hello` from the PC | `error unsupported_version` with `ref`, close 4002, state "Update the app"; a user action reconnects |
| 4003 | state "Pair again", Home shows "Remove this PC" |
| `unpair` from the PC | acked, the PC is removed, the service stops, onboarding returns |
| Wi-Fi off / on | "Not on Wi-Fi", clean close with 1000; Wi-Fi on reconnects |
| Kill (`kill -9`) | `START_STICKY` restarts the service, "kill detected", kills = 1, reconnects |
| Reboot | `BOOT_COMPLETED` received, service started, reconnected (see the finding below) |
| R8 release build (debug-signed) | pairs, runs the service, connects; no debug receiver |
| Scan code | the Google scanner isn't available (`MlKitException: Waiting for the Barcode UI module to be downloaded`); the app shows "The QR scanner isn't available on this phone. Paste the pairing link instead." |

**API 29 (`kin_api29`, Android 10, debug build):** install, launch, deep link, pairing, TLS 1.3 (Conscrypt on Android 10), service in the foreground, "connected". No permission prompts (neither permission exists there). Home and Debug screens render.

Not tested on any emulator: mDNS discovery (multicast doesn't reach the PC's network, as the emulator README says), a real phone's Wi-Fi, Doze, OEM battery managers, the camera scanner.

## The API 35+ boot-start finding

On **API 37** a `connectedDevice` foreground service **can** be started from a `BOOT_COMPLETED` receiver. After `adb reboot` with a paired PC: `boot: BOOT_COMPLETED received (sdk 37)`, `boot: service start requested`, then `service started` and a connected WebSocket about a second later; `dumpsys` shows `isForeground=true types=0x10`. This matches the platform rule that Android 15 blocks boot-started foreground services only for `dataSync`, `camera`, `mediaPlayback`, `phoneCall`, `mediaProjection` and `microphone`. `BootReceiver` uses `goAsync()`, and logs a refusal (`start refused: …`) instead of crashing if a future Android or an OEM refuses. API 35, 36 weren't run separately. On API 29 `BOOT_COMPLETED` was received and the service started too. A real phone (OEM autostart rules) still needs a check in the soak.

A reboot is not counted as a kill: the service stores the system boot counter with its "running" flag (`PreviousRun` is `CLEAN`, `KILLED` or `REBOOT`).

## Did kotlinx need the strict pre-validation? Yes, badly

`Json { isLenient = false }.parseToJsonElement` on its own accepted **29 of the 33** raw cases that must be rejected (the `strict-*`, `nesting-*` and lone-surrogate vectors):

- **Accepted by kotlinx alone:** every unquoted literal (`yes`, `True`, `NaN`, `Infinity`, `-Infinity`, `nul`, `undefined`), `01`, `+1`, `.5`, `1.`, `0x10`, **single-quoted strings**, nesting of 65 and of 1000 (arrays and objects), and **all six lone-surrogate escape cases** (`strict-unquoted-literal-in-known-field`, `-in-unknown-field`, `strict-capitalised-boolean-*`, `strict-leading-zero-*`, `strict-plus-sign-in-unknown-field`, `strict-leading-dot-…`, `strict-trailing-dot-…`, `strict-hex-number-…`, `strict-nan-*`, `strict-infinity-…`, `strict-negative-infinity-…`, `strict-null-misspelled-…`, `strict-undefined-…`, `strict-single-quoted-string-…`, `strict-unquoted-literal-before-hello`, `nesting-depth-65-*`, `nesting-depth-1000`, `text-lone-high-surrogate-escape`, `text-lone-low-surrogate-escape`, `text-high-surrogate-followed-by-non-low-escape`, `text-reversed-surrogate-pair-escape`, `lone-surrogate-escape-in-a-key`, `lone-surrogate-escape-in-id`).
- **Rejected by kotlinx alone:** `strict-unquoted-key`, `strict-trailing-comma-in-object`, `strict-trailing-comma-in-array`, `strict-comment`.
- With the app's `StrictJson` (a one-pass RFC 8259 checker with a depth counter, then kotlinx builds the tree, then a surrogate walk over strings and keys) **all of them are rejected**, and depth 64 and brackets inside strings are accepted. The same test file also covers duplicate keys (last wins: kotlinx keeps the last value) and the integer, boolean and ID reads that never call `.long` or `.boolean` on an unchecked primitive.
- `StrictJsonTest.measureWhichVectorsKotlinxAloneAccepts` prints this list on every run.

## Spec ambiguities, choices and deviations

1. **`ACCESS_LOCAL_NETWORK`** (above). Not in the spec; needed on API 37.
2. **Answer to `POST /v1/pair` that doesn't match.** The spec says that if `deviceId` isn't the pinned `id` or `protocol` isn't 1, the phone treats it "as `unsupported_version`" and saves nothing. I followed it literally for both cases (the log says which one it was). For a `deviceId` mismatch, "update the app" is a misleading message; a separate result would be kinder, but the spec says what it says.
3. **4002 and a user action.** "Don't retry automatically": the app doesn't, but a user action (opening the app, Reconnect) tries again, because the user may just have updated the PC. 4003 and `401` never retry until the user pairs again.
4. **The address race is a handshake probe.** It does a TLS 1.3 handshake (with the client certificate) to every address and takes the first whose key matches the pin; it sends nothing after that (so the PC never sees a second WebSocket, which would be answered with 4001). The WebSocket then does its own handshake: two handshakes per connection. On the emulator a handshake took about 500 ms in several runs and 10 to 50 ms in others, so I couldn't separate JIT warm-up from crypto cost; it is worth looking at in the soak. A probe cannot see a refused client certificate (the spec's TLS 1.3 timing note): that shows up on the first read of the WebSocket, which is handled (the session fails, no `hello`, backoff).
5. **JSSE and Conscrypt differ on that timing.** On the JVM the client's handshake call can already fail when the server refuses the certificate; on Conscrypt it returns. The tests accept either, and require that a refusal is never "connected" or "paired".
6. **Message types of later milestones.** The 1.1 receiver answers `text`, `clipboard`, `offer`, `complete` and `cancel` with `unsupported_type`, as for an unknown type, until 1.2/1.3. The desktop app must not send them to the 1.1 build.
7. **WebSocket liveness.** OkHttp's `pingInterval` closes the connection when no pong arrives within one interval, so a dead connection is noticed after up to two intervals (8 min at the default 4 min). The PC's 10-minute rule is the stricter one anyway.
8. **Pairing a second PC** replaces the first one locally; the old PC isn't told.
9. **Device name** is `Settings.Global.DEVICE_NAME`, or the model; cut to 64 code points without splitting a surrogate pair. Unpaired surrogates are replaced by U+FFFD before sending.
10. **Logs also go to logcat in release builds** (redacted). No secrets reach them; remove the mirror if friends' phones shouldn't have it.
11. **Close reason texts** are short and never matched.
12. **mDNS** uses `NsdManager.resolveService`, deprecated on API 34 but the only call that works from API 29 (the replacement needs API 34). It also needs the local network permission on API 37.

## Known gaps

- Not run against the real desktop app (that is the integration agent's job) and not on a real phone: mDNS, Wi-Fi sleep, Doze and OEM killers, the camera scanner, battery.
- No OEM battery onboarding, no `WAKE_LOCK` use: those are 1.4 and 1.3.
- No history, queue, settings screen, share targets or clipboard: 1.2 to 1.4.
- The pairing UI shows no PC address list or rescan; "Reconnect now" and Ping are the only manual controls.
- Identity: if the Keystore key is lost (for example after a restored backup, which `allowBackup=false` prevents), a new identity is created and the paired PC rows are deleted, so the user sees onboarding. The store-level part is unit-tested (`IdentityStoreTest`); the whole path wasn't triggered on the emulator.
- `PAIR` over a debug broadcast may be refused on Android 12+ when it has to start the foreground service from the background; use the deep link.
- Release builds are unsigned (no keystore in the repo); the R8 test used the debug keystore from `~/.android` only to install.
