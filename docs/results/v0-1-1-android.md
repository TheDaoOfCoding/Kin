# v0.1.1, Android: parallel files in a batch, ARM-only release APK

Date: 2026-10-16. Built by the Android agent (not yet reviewed or committed). Version 0.1.1, `versionCode` 2.

## 1. What changed

| Part | Change |
|---|---|
| Parallel files, both directions | Up to 4 files of one transfer are on the wire at once (`TransferTimings.maxParallelFiles`, protocol.md "Concurrent file requests"). One request per file: a file is never split, so a single big file goes exactly as before. Still one transfer at a time per direction. `DownloadRunner.downloadAll`, `UploadRunner.uploadAll` |
| Pipelining, PC → phone | One producer makes the gallery entries of the next files while the earlier ones download (a channel as long as the pool). It also hands out names one at a time (files of one offer may share a name), and the names already in the folder / claimed by other transfers are read once per attempt, not once per file |
| Pipelining, phone → PC | A producer opens the source of the next files at the PC's offset ahead of their `PUT`; a source that was opened and not used is closed again |
| The "tap to start" delay | `enqueueUpload` looked at each shared file one after the other (a provider round trip each), and the first attempt did it all over again. The looks now run 8 at a time (`mapParallel`), and the first attempt of a transfer made a moment ago skips the second look (`sourcesJustChecked`). Every later attempt (reconnect, queue, Retry) still checks that the files are unchanged, also 8 at a time |
| Failure handling with several in flight | A file that **fails for good** stops the files in flight and fails the transfer as before. A file whose **connection broke** fails only itself for this attempt: the others finish, and the next attempt resumes only what is missing (`Range` / a new `POST`) |
| Progress | One aggregate counter over all files in flight (a `200` to a `Range` request takes the file's bytes back), `LiveProgress` per file as before. The notification says "N of M files done · P%" (there is no "current file" any more) |
| HTTP client | A connection pool of its own for the file requests: 8 idle connections, kept 20 s (the PC closes a connection idle for 30 s and `retryOnConnectionFailure` is off, so a stale reused connection would fail a request). Dispatcher limits set to 16 (the file calls are synchronous `execute()`, which OkHttp doesn't limit per host anyway) |
| Release APK | `ndk.abiFilters` to `arm64-v8a` and `armeabi-v7a` on the release build type only (section 3) |
| Version | `versionName` 0.1.1, `versionCode` 2 |

Not changed: Room schema (still version 4), resume rules, cancel, retry (only the files without a `200`, under a new ID), the offline queue and its 7-day expiry, MediaStore folders, the owed `complete` / `cancel` messages, the wake-lock rule (held only around a request that moves bytes, counted per request).

Files: `transfer/DownloadRunner.kt`, `UploadRunner.kt`, `TransferEngine.kt`, `TransferPorts.kt` (timings), new `Parallel.kt`, `TransferNotifications.kt`, `link/LinkSession.kt`, `app/build.gradle.kts`; tests and fakes under `app/src/test`; `docs/android.md` (new "Parallel files" section), `android/README.md`; scripts in `android/tools/` (section 5).

## 2. Measurements (emulator, kin_api37, real dev desktop)

**Read this first.** The emulator reaches the PC through its own NAT (`10.0.2.2`) at about 2 to 9 MB/s, whatever the phone or the PC could do. A batch of 50 × 3.4 MB is limited by that pipe and cannot show the change; the numbers below are **indicative only** and do **not** show whether the 3 to 4 s target is met on a real phone on Wi-Fi. That needs the real phone (`tools/speedtest`). What the emulator can show is the cost per file, with many small files, and that a single file did not get slower.

Method: `android\tools\bench-batch.ps1`. The "before" APK is built from the committed source (0.1.0), the "after" APK is this one; same dev desktop, same files (random bytes), the phone's files made by the app itself. Each APK ran twice, **each time on a freshly booted emulator**, in the order before, after, before, after. Wall time = from the adb command (phone → PC) or `Kin.exe --send` (PC → phone) until the PC's record says the transfer is done. Median of 10 runs for the small batches (5 per APK run), 2 runs for the big ones.

| Case | Before (0.1.0) median (min to max) | After (0.1.1) median (min to max) | Change |
|---|---|---|---|
| phone → PC, 50 × 20 KB | 2.46 s (2.41 to 2.76) | 2.10 s (1.93 to 2.23) | -15% |
| phone → PC, 50 × 100 KB | 3.19 s (3.00 to 3.98) | 2.47 s (2.30 to 2.96) | -23% |
| PC → phone, 50 × 20 KB | 2.62 s (2.22 to 2.97) | 1.41 s (1.17 to 1.97) | -46% |
| PC → phone, 50 × 100 KB | 2.35 s (2.21 to 8.46) | 1.49 s (1.36 to 2.04) | -37% |
| phone → PC, 50 × 3.4 MB (170 MB) | 35.7 s, 36.3 s | 30.3 s, 29.5 s | -17% (pipe-bound, about 5 to 6 MB/s) |
| PC → phone, 50 × 3.4 MB | 20.0 s, 21.3 s | 20.7 s, 19.1 s | same (pipe-bound, about 8.5 MB/s) |
| phone → PC, one 30 MiB file | 6.20 s, 5.99 s | 6.79 s, 5.82 s | same (one request, not split) |
| PC → phone, one 30 MiB file | 3.58 s, 4.31 s | 4.69 s, 3.51 s | same |

- The PC's own count of files moving at the same moment (`filesActive` in its `status.json`) peaked at **4** with the new APK in both directions and at 1 with the old one.
- After the change a 50-file batch of small files is bounded by what is left per file: the PC-side handling and the emulator's NAT (phone → PC still costs about 40 ms per file with four in flight), not by the phone's own work.
- **The "tap to start" delay (about 4 s for 50 photos on the real phone)** can't be measured here: the files of the debug commands come from Kin's own provider in the same process, so a look at a file costs almost nothing (the PC's record starts 0.1 s after the command in both builds). It is covered by a unit test instead: 40 files with a 40 ms look each are looked at in under 60% of the one-by-one time (one by one 1.6 s, before this change twice that, because of the second look), and each file is looked at once before the first attempt (`ParallelFilesTest.aShareOfManyFilesIsLookedAtSeveralAtATimeAndOnlyOnceBeforeItStarts`).
- **A first benchmark run was thrown away.** Two APKs measured back to back on one long-running emulator gave 3 to 20 s per 50-file batch with no difference between the builds: the emulator's gallery slowed down as the session aged (PC → phone went from 5 s to 20 s over five runs). That is why every number above comes from freshly booted emulators in alternating order. The same lesson is in `android\tools\bench-batch.ps1`.

## 3. Release APK: ABIs

- The app has **no native code of its own**. The release APK held eight small `.so` files, from CameraX (`libimage_processing_util_jni`, `libsurface_util_jni`), AndroidX graphics-path (`libandroidx.graphics.path`) and DataStore's shared counter (`libdatastore_shared_counter`), each for four ABIs (`arm64-v8a`, `armeabi-v7a`, `x86`, `x86_64`).
- `ndk.abiFilters += arm64-v8a, armeabi-v7a` is set on the **release** build type only. The debug build keeps all four (the emulator is x86_64).
- Result: 8 `.so` files (two ABIs), `aapt2 dump badging` says `native-code: 'arm64-v8a' 'armeabi-v7a'`. Size **4,447,746 bytes (4.24 MiB)**, down from 4,662,856 (-215,110 bytes). versionCode 2, versionName 0.1.1, minSdk 29, targetSdk 37.
- Signed with the same key as 0.1.0: `apksigner verify` says "Verifies" (v2), certificate SHA-256 `83fd45c8402978c061c784d1f5dd5f7f1cc91c6165faa89a767c5bc19ec7d419`. `android\tools\check-release.ps1`: **RESULT: clean** (no gms, firebase, datatransport or mlkit anywhere).
- **Consequence:** the release APK **no longer installs on the x86_64 emulators** (no matching ABI). `android\tools\release-smoke.ps1` (the signed-APK smoke of 1.5) needs an ARM emulator, or the debug build, from now on. It was not run on this build for that reason. The `x86` emulator images can't run it either. Nothing in the app needs the x86 libraries on a phone.
- Release APK SHA-256: `0baf937bd06c738d7abe1c3eb98d8e4edc1bb12173330fa560f6b8c7c3546526` (`android\app\build\outputs\apk\release\app-release.apk`; the debug APK is next to it under `debug`).

## 4. Tests and checks

| Check | Result |
|---|---|
| `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` | passes; lint clean |
| Unit tests | **474 run, 0 failures, 1 skipped** (`FakePcSession.serve`, a helper that JUnit skips by assumption, as before). 17 new tests: `ParallelFilesTest` (16) and one more in `UploadFlowTest` |
| Repeated, to find flaky tests | `transfer.*` and `link.*` suites 12 times in a row. Two of the first four runs failed in `UploadFlowTest.aFailureOnAPutIsFinalToo`, which assumed the second file is never started after the first fails (now pinned to `maxParallelFiles = 1`, with a new test for the parallel case); the next 8 runs were green. Two more tests that assumed the old order were changed while writing (the order of the `PUT`s in a multi-file upload, and `a409SourceChangedFailsTheOfferAndKeepsTheFilesAlreadySaved`, where the failing file now answers late so the first file is saved before the offer fails) |
| `bun protocol/check-vectors.ts` | OK: all 6413 checks passed (the desktop agent's new vectors included) |
| `android\tools\e2e-v011.ps1` on kin_api37 (final run) | **OK: all 36 checks passed** (4.1) |
| Same, kin_api29 (S1, S2, S3, S7) | **OK: all 22 checks passed** (4.1) |

`ParallelFilesTest`: a download and an upload run exactly 4 files at once and never 5 (the fake PC holds the first four requests until all four are in flight); `maxParallelFiles = 1` gives the old order; a single big file is one `GET` / `PUT`; same-name files get distinct names and each keeps its own bytes; **one file breaks mid-batch while the others finish, and only it resumes** (`Range: bytes=30000-` / `PUT` from `offset=30000`, one extra `POST`); a file that fails for good stops the others and nothing is left behind; **cancel mid-batch** (user, several files writing; for an upload the `DELETE` goes out, all sources are closed, the wake lock is released); aggregate progress never passes the total; the parallel share look; `mapParallel` keeps order and bounds concurrency. The shared HTTP vector replays (`HttpVectorsTest`) run with one file at a time, because a vector is one fixed sequence of requests.

### 4.1 End to end on the emulator against a real desktop

`android\tools\e2e-v011.ps1` (real debug APK, the dev desktop of section 5, loopback only, throwaway data dir, non-default port, no PC window):

- S1 phone → PC, 50 files through the share activity: all there, byte for byte (SHA-256), no `.part`. About 2.8 s from the tap.
- S2 PC → phone, 50 files: all in `Pictures/Kin`, byte for byte, no `(1)` copies, no pending entry (about 1.7 s).
- S3 8 files named `IMG.jpg` from 8 folders: `IMG.jpg`, `IMG (1).jpg` to `IMG (7).jpg`, 8 different contents (names are made one at a time).
- S4 PC → phone, 12 × 1 MB, user cancel on the phone with several in flight: canceled on both sides, no pending entry, what is left is complete and right.
- S5 phone → PC, 16 × 2 MB, user cancel on the phone: canceled on both sides, no `.part`, what is left is complete and right.
- S6 phone → PC, the PC cancels: the phone ends canceled (`peer_canceled`), not failed.
- S7 both directions, 10 × 3 MB, Wi-Fi off for 6 s in the middle: everything resumes and arrives byte for byte. Repeated 15 + 20 + 30 times in one session each (new APK) and 20 times with the old APK as a control.
- No crash in logcat, wake lock not held when idle.

**Final run on the final debug APK: 36 of 36 checks on kin_api37 (Android 17) and 22 of 22 on kin_api29 (Android 10; S1, S2, S3, S7).**

## 5. Scripts added (all in `android/tools/`, none touches the repo's `desktop/`)

- **`build-dev-pc.ps1`**: builds a private copy of the desktop source with another app identifier (`TAURI_CONFIG`), no LTO, into `D:\kin-v011-android` (about 2 minutes).
- **`e2e-v011.ps1`**: the scenarios above (`-Avd`, `-Only S1,S2`, `-S7Runs N`).
- **`bench-batch.ps1`**: the before / after timing of section 2.

**Why a private desktop build (an environment finding for everyone who tests on this PC).** Kin is a single-instance app and the slot is named after the app identifier. With the user's own Kin running (as it was), a second `Kin.exe` (even with `--data-dir`, `--dev-bind` and a free port) hands its arguments to that instance and exits, so **no dev instance starts**; and a script that checks `Get-Process Kin` is also fooled the other way. The only way to run a dev desktop next to the user's Kin without touching it was a copy built with another identifier; it has its own data dir, port and `--quit`, and processes are recognised by path, never by name. The user's Kin (PID 25708) was never touched, and the exe is built from the desktop agent's working tree at the time (it includes their concurrency changes). The desktop agent and the integration agent will meet the same wall: `build-dev-pc.ps1` is the way around it.

## 6. Problems for the protocol / desktop agents

- **No spec change needed.** protocol.md "Concurrent file requests" says a client MAY pipeline several requests on one connection for small files. OkHttp (HTTP/1.1) does not pipeline, and the phone does not need to: four connections, each reused for the next file (kept 20 s), are enough. The phone never opens more than 4 file connections (plus the WebSocket).
- Item 53 (b), "a `POST` for an existing transfer no longer stops the other files' writers", is what the phone relies on when one file of a batch is retried: in this build the retry `POST` only goes out once every other file of the attempt has ended (the attempt is the unit), so the phone is safe either way.
- The desktop's own idle rule (30 s) is why the phone's pool keeps connections only 20 s.

## 7. Open issues

> **Update from the integration run ([v0-1-1-integration.md](v0-1-1-integration.md)):** `build-dev-pc.ps1` is retired (the normal `Kin.exe` runs beside the installed Kin), `e2e-v011.ps1` and `bench-batch.ps1` use that. Issue 1 (the stall) was investigated: 140 Wi-Fi cuts, no stall; the mechanism (a silent drop of about a minute fails the transfer, the link only ends after the 240 s WebSocket ping) was reproduced and is the same in 0.1.0.

1. **One unexplained stall (new APK only: 1 time in about 80 phone → PC Wi-Fi cuts; 0 in 20 with the old APK as a control, and 0 in the 65 phone → PC cuts that followed with the new one).** After the Wi-Fi was cut for 6 s in the middle of a 10 × 3 MB upload, four files were complete, four connections went idle and the PC closed them after 30 s, and then nothing moved for 3.5 minutes: the PC still had the phone's old WebSocket open (237 s) and the phone only reconnected when the script switched Wi-Fi again. It looks like the link layer (the phone's WebSocket not noticing a silent drop, then its reconnect backoff), which the file code only waits for, but the run happened with the new APK and the phone's log was not captured at that time (the script now captures it). Not reproduced in 65 more cuts in each direction. Worth a look when someone has a real phone and a router to pull the plug on.
2. **Real-phone speed is not measured.** The 50 × 3.4 MB target (3 to 4 s on fast Wi-Fi) needs the real phone: `tools/speedtest` with `Kin.exe --send` and the share path, old and new APK, alternating. The emulator can't carry that many bytes.
3. ~~**`release-smoke.ps1` can't run on the x86_64 emulators** with the ARM-only release APK (section 3).~~ Fixed: see "Release smoke (R8)" below.
4. `adb shell sha256sum *` through Windows PowerShell returned nothing at all twice in about 60 runs under load (the files were fine: pulled and compared byte for byte). `e2e-v011.ps1` asks again when a listing is empty. Nothing to do in the app.
5. The progress notification no longer says which file is current ("N of M files done"). If someone misses the old text, the per-file numbers are in `LiveProgress`.
6. Cleanup list: `D:\kin-v011-android` holds the private desktop build (`target`), the baseline source and the benchmark outputs (about a few GB): delete it when the integration agent is done (`Remove-Item -Recurse D:\kin-v011-android`), or keep `target\release\kin.exe` and `build-dev-pc.ps1` for the next milestone's tests.


## 8. Silent drop fix (P1/P2)

Date: 2026-10-17. Fixes P1 and P2 of [v0-1-1-integration.md](v0-1-1-integration.md). Android agent (Claude Sonnet 5.5); not reviewed, **not committed**. `versionName` stays 0.1.1.

### What was wrong

- **P1.** A network that goes silent without a reset (packets just stop) left the WebSocket "ready" for 4 to 8 minutes (ping every 240 s, then the pong is waited for 240 s). Every attempt of a transfer reused that session, each attempt ended in about 5.5 s, and after 5 attempts with no byte (`maxStalledAttempts`), about a minute in, the transfer failed with `connection_lost` and needed a manual Retry.
- **P2.** `LinkLoop`'s backoff (5 s, 30 s, 2 min, 10 min) kept climbing while a transfer was waiting for the link; a PC that was unreachable for 100 s after Wi-Fi returned left the phone asleep for 2 minutes.

### What changed

| Where | Change |
|---|---|
| `link/LinkSession.kt` | New `LinkConnection.checkAlive(timeoutMillis)` and `foundDead`. It asks the network (a new TLS handshake to the PC, through `AddressRacer`, wired in `DefaultLinkEnvironment`); if the PC doesn't answer, the session is abandoned (`socket.cancel()`, which ends it; a fallback ends it by hand), so `LinkLoop` reconnects and nobody is handed a dead "ready" session. |
| `transfer/StallGuard.kt` (new) | Runs beside every attempt of an upload and of a download: no byte moved for `stallProbeAfterMillis` (15 s) → `checkAlive(probeTimeoutMillis = 10 s)`. Never asks while bytes move or while nothing is transferred. |
| `UploadRunner.kt`, `DownloadRunner.kt` | The guard is started in `whileConnected`. After any attempt that ended in a retry on a connection that is still "ready", the runner asks too. A connection found dead is the network's doing: the attempts it cost do **not** count towards `maxStalledAttempts` (reset to 0) and the next wait for a connection kicks the loop. The transfer waits for the new connection (`reconnectWaitMillis`, 15 min, then `queued`; the 7-day queue rules are untouched) and resumes with `Range` / a new `POST`. A PC that answers the check but refuses every request still fails after 5 attempts, as before. |
| `link/LinkLoop.kt` | New `workWaiting` parameter and `workIsWaiting()`. While something waits for the link a backoff step is at most **10 s** (`WAITING_RETRY_MILLIS`), and the moment something starts waiting a loop that sleeps a long step tries **now**. Nothing waiting: the steps are exactly the old ones. |
| `link/LinkController.kt`, `LinkRuntime.kt` | `connection()` counts its callers (`transferWaiting`), wakes the loop on entry. |
| `TransferTimings` | `stallProbeAfterMillis = 15_000`, `probeTimeoutMillis = 10_000`. |

### Decisions you should know about

- **No shorter WebSocket ping while a transfer is active.** Asked for; not done, deliberately. (1) OkHttp takes `pingInterval` once per client and has no way to change it on an open socket or to send a ping frame by hand. (2) The PC sends no ping frames, so the phone's pong is the only one. (3) The JSON `ping` message is "a debug and latency tool" that the receiver shows as a notification / toast ("Ping from <name>", protocol.md), so using it as a probe would put a toast on the PC every few seconds. The closest compliant approach is the event-driven check above: a new TLS connection, invisible on the PC (nothing is sent after the handshake, and the PC logs nothing for it; see below), sent only when a transfer has stalled. The idle keepalive is **unchanged at 240 s** (battery).
- **The backoff cap is a deviation from architecture.md's step list, only while something waits.** The cost while a transfer waits: one race (1.5 s) plus, if the last addresses fail, one mDNS lookup (5 s) per attempt, every 10 s, for at most the 15 minutes a transfer waits (the phone log shows about one attempt per 16.5 s). A PC that is off with no transfer waiting costs nothing more than before.
- Neither change touches the protocol, the PC, the database, or the permissions.

### Tests

| Check | Result |
|---|---|
| `gradlew.bat lintDebug testDebugUnitTest` (once) | passes; lint reports only the existing `ChromeOsAbiSupport` warning. **486 tests, 0 failures, 1 skipped** (474 before; 12 new) |
| `SilentDropTest` (8, new) | an upload and a download over a silent network (requests held, WebSocket open): dropped after the stall, waited out, resumed on the new connection, no error code, bytes right; the same when every attempt fails at once and the PC doesn't answer the check (`maxStalledAttempts` 2 would have failed it); a PC that answers the check is never torn down by slow requests (same connection to the end); a transfer that keeps moving bytes is never checked (0 checks); the control: a PC that answers the check but ends every request still fails with `connection_lost`; `checkAlive` keeps an answering connection and abandons a silent one (`isReady` false, it ends) |
| `LinkLoopTest` (4 new) | with a transfer waiting every step is at most 10 s (0, 5, 15, 25, 35, … s); the usual steps return as soon as nothing waits (…, 625 s); a transfer that starts waiting wakes a loop that sleeps a 2 min step (tries at once, then every 10 s); neither a connected loop nor one that stopped on purpose (4001) is woken. The existing `backoffIs5s30s2minThenEvery10Min` (the idle steps) is unchanged and passes |

### On the emulator (kin_api37 / Android 17, rooted, new debug APK, standard dev Kin beside the installed Kin)

`android\tools\e2e-v011.ps1 -Only S14` (a 10 x 3 MB batch; `iptables` drops everything between the phone and the PC for N seconds with the Wi-Fi "up"; no Retry, no user):

| Direction | Drop | Batch done after the packets flowed again | Before the fix (v0-1-1-integration.md) |
|---|---|---|---|
| phone → PC | 90 s | **8 s**, byte for byte | failed at 75 s and 100 s |
| phone → PC | 120 s | **10 s**, byte for byte | failed at 100 s |
| phone → PC | 300 s | **18 s**, byte for byte | (failed at 100 s) |
| PC → phone | 90 s | **11 s**, byte for byte | (finished by itself at 100 s) |
| PC → phone | 300 s | **16 s**, byte for byte | |

5 runs, all pass; "no crash in logcat" and "the wake lock is not held when idle" pass. Then `-Only S1,S2` on a fresh boot: **12 of 12 checks** (50 files each way, byte for byte, 1.2 s and 1.6 s). No full regression and no benchmarks, as agreed.

Timeline of the 120 s upload (phone log): the drop starts at t = 0; t + 15 s `upload …: no byte moved for 15.0 s; checking that the PC answers`; t + 25 s `the PC doesn't answer a new connection within 10000 ms; treating the connection as dead` → `connection ended` → `the PC doesn't answer; waiting for a new connection`; then the loop retries every 5 s, then every 10 s `(a transfer is waiting)` until the packets flow again; the batch is done 10 s later. In the 300 s download the loop woke from a 600 s step 2 s after the runner started waiting and then tried every 16.5 s (10 s step + about 6.5 s of race and mDNS); about 16 attempts in 4.5 minutes. The PC log of these runs shows only its own `idle for 30 s` closes of the four dead connections: no TLS handshake error and no `too many connections` for the check's handshakes.

### What is not covered

- **A real router.** The emulator's `iptables` drop is the same kind of silence (no reset, Wi-Fi up), but nothing here has run on a real phone and a real access point. The integration agent's open item (the one-off 3.5 min stall) is explained by P1/P2; whether it was that stays unproven.
- **The window the guard can't see:** a silent drop that starts while no request is in flight and no transfer is waiting (an idle link) is still noticed by the 240 s ping, as before. The next transfer's first request then stalls and the guard takes over after 15 s plus the 10 s check.
- Release APK (ARM only) not run on a device. Size 4,447,746 bytes, SHA-256 `3fe99e9ce346e6d01521a51cc77247171c5ef11ababcc19d0cf82f00197b3611`, `versionCode 2`, `versionName 0.1.1`, signed with the same key as 0.1.0 (`certificate SHA-256 83fd45c8…7d419`, `apksigner verify` ok). Debug APK 47,206,015 bytes, SHA-256 `bc8869125026fd0d1ea5be06567459099baea5433c7ddf54629557a521320d8e`. Both rebuilt after the last source change.
- `e2e-v011.ps1` S14's final check uses `$phoneFailed`, which is never set in that block (the check is therefore only "the PC's record says done, byte for byte"). A batch that the phone had failed and the user Retry had finished isn't flagged; no run here needed a Retry. Not changed (the script belongs to the integration agent).

## 9. Release smoke (R8)

Done 2026-10-17. Nothing is committed.

- **Opt-in smoke build.** `gradlew assembleSmoke -PkinSmokeAbis=x86_64` (comma-separated ABIs) creates a `smoke` build type that is `initWith(release)`: same R8 (`isMinifyEnabled`, `isShrinkResources`), same `proguard-android-optimize.txt` + `proguard-rules.pro`, same signing config and the release-only sources (`src/release`), plus the extra ABIs. Without the property the build type doesn't exist and `release` is unchanged. Output: `app/build/outputs/apk/smoke/app-smoke.apk` (4,564,225 bytes; `native-code: arm64-v8a armeabi-v7a x86_64`, versionCode 2, versionName 0.1.1, not debuggable, same certificate `83fd45c8…7d419`). It is for smoke tests only and is never `app-release.apk`.
- **The default release build is unchanged:** `assembleRelease` after the change: `app-release.apk` 4,447,746 bytes, SHA-256 `3fe99e9c…b3611` (identical to section 3), `native-code: arm64-v8a armeabi-v7a`, signed with the same key.
- **`android	oolselease-smoke.ps1`** now builds the smoke APK itself (`-NoBuild` to reuse it, `-Apk` to test another file), checks versionName 0.1.1 / versionCode 2 (it still said 0.1.0 / 1), and has a small batch both ways: 3 files PC → phone (`--send` with 3 paths), and 3 files phone → PC through the `share-source` test app (SEND_MULTIPLE with grants, the way a gallery shares; adb can't build that intent), SHA-256 compared for each file.
- **Run (once): 39 checks, 0 failed**, emulator `kin_api37`, against a dev Kin (`--dev-bind 127.0.0.1`, own data directory, own port) beside the user's installed Kin, which was never touched (PID 25796 before and after). Passed: install of the R8 APK, not debuggable, camera permission flow of "Scan code" (deny, ask again, allow, scan screen with the camera open, cancel), pairing through the link and Confirm, ping, text both ways (the PC clipboard holds the phone's text; the phone shows "Text from DESK-PC"), a 700 KB file PC → phone and a 500 KB file phone → PC (SHA-256 equal), the 3-file batch both ways (SHA-256 equal), and **logcat with no `FATAL EXCEPTION`, `ClassNotFoundException`, `NoSuchMethodError`, `NoSuchFieldError` or `Could not find class`** for the app. The PC clipboard was saved and restored (written only through `Set-QuietClipboardText`).
- **No R8 runtime problem found, so no rule changed and no second run was needed.**
- **What this does not cover:** the smoke APK differs from the real release APK only by the x86_64 libraries (the Kotlin/Java code, R8 output and resources come from the same configuration), so the ARM native libraries themselves (CameraX/DataStore's few `.so` files) are not exercised; a phone is still the only test for that.
