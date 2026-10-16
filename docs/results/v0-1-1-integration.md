# v0.1.1, integration: the real desktop build against the emulator

Done 2026-10-16 and 2026-10-17 by the integration agent (Claude Sonnet 5.5) on Windows 11 Pro 10.0.26300 with the emulators `kin_api37` (Android 17) and `kin_api29` (Android 10). **Nothing is committed.** Inputs: [v0-1-1-desktop.md](v0-1-1-desktop.md), [v0-1-1-android.md](v0-1-1-android.md), `desktop\build.ps1` (0.1.1 installer 3.60 MiB, built again for this run), the debug APK of the Android agent's tree (`versionName 0.1.1`).

**Summary.**

- Every test Kin ran **beside the user's installed Kin** with the standard mechanism of D17 (own `--data-dir`, own port, loopback, no windows). The private build of `android\tools\build-dev-pc.ps1` is retired.
- **No product bug in the v0.1.1 changes was found.** 4 files at once work in both directions, byte for byte, with cancel, Retry, the queue and Wi-Fi cuts. The desktop's `POST` change (a resume `POST` no longer stops the other writers) held in 140 Wi-Fi cuts and in the S7, S10 and S11 resumes.
- **The stall the Android agent saw was not reproduced by Wi-Fi cuts** (140 cuts, 0 stalls), but its mechanism was found and reproduced with a *silent* network drop: a drop of about one minute or more kills a phone → PC batch for good (`failed: connection_lost`) while the WebSocket still looks alive for 4 to 8 minutes. **It is not new in v0.1.1:** the committed 0.1.0 app does exactly the same (control run). Details in "The stall".
- Two **product observations for a coder** are written up precisely below (P1 and P2). Both are about how the phone reacts to a network that is gone without telling it; neither is a regression.
- The test scripts had **ten problems**, all fixed (list below). One of them (the `$1` in three scenario files) made the 1.2 to 1.4 PC → phone text checks run with the wrong text.

## What ran

All on the emulator, against the real `Kin.exe` of `tauri build`, with the installed Kin untouched. Counts are checks.

| Run | Result |
|---|---|
| `desktop\check-dev-beside-installed.ps1` (first, and again at the end after the `kin-dev.ps1` change) | **31 passed, 0 failed**, both times |
| `desktop\check-1-3.ps1`, `check-1-4.ps1` (end of the run, after the `kin-dev.ps1` change) | **39** and **63** passed, 0 failed |
| `tools\e2e-1-1.ps1` on kin_api37 | **16** passed |
| `tools\e2e-1-2.ps1` on kin_api37 | **78** passed |
| `tools\e2e-1-3.ps1` on kin_api37 | **142** passed |
| `tools\e2e-1-4.ps1` on kin_api37 (burst 20 texts / 6 files, the default) | **179** passed |
| `tools\e2e-v011.ps1` on kin_api37, S1 to S7, S10, S11 (the final run) | **62** passed |
| `android\tools\e2e-v011.ps1 -Avd kin_api29 -Only S1,S2,S3,S7` | **22** passed |
| `tools\e2e-1-1.ps1`, `e2e-1-3.ps1` on kin_api29 | **15** and **141** passed |
| S8, the stall hunt (two sessions) | 140 Wi-Fi cuts, **0 stalls**, 140 of 140 batches byte for byte |
| S9, 50 x 3.4 MB timings | 13 passed (below) |
| S12, S13, S14 (the silent peer, the backoff, the silent drop) | see "The stall" |

The first pass of 1.1 to 1.4 failed for script reasons (the `$1` bug and the 8.3 path bug below) and was thrown away; the numbers above are the second pass, with the fixes. `e2e-1-4.ps1` ran with the default burst, not the full 60 texts / 15 files (`-BurstTexts 60 -BurstFiles 15`), as in 1.4's own default run.

`release-smoke.ps1` was **not run**: the 0.1.1 release APK is ARM only and does not install on these x86_64 emulators (v0-1-1-android.md, section 3). It still needs an ARM AVD or the debug APK. The user's hotkey was never pressed, `check-dragdrop.ps1` was not run, no window opened on the PC.

### The v0.1.1 scenarios (`tools\e2e-v011.ps1`)

The milestone's entry is `tools\e2e-v011.ps1` (it runs `android\tools\e2e-v011.ps1` with every parameter). Default run: S1 to S7, S10, S11; the others are `-Only`.

| | What | Result (kin_api37) |
|---|---|---|
| S1 | phone → PC, 50 files through the share path | done, byte for byte, 1.7 s from the tap |
| S2 | PC → phone, 50 files | done, byte for byte, no `(1)` copies, nothing pending, 2.4 s |
| S3 | 8 files named `IMG.jpg` from 8 folders | `IMG.jpg`, `IMG (1)` to `IMG (7)`, 8 different contents |
| S4 | PC → phone 12 x 5 MB, the user cancels on the phone with several in flight | canceled on both sides, nothing pending, what is left complete and right |
| S5 | phone → PC 16 x 2 MB, user cancel | canceled on both sides, no `.part` |
| S6 | phone → PC 12 x 1 MB, the PC cancels | the phone ends canceled (`peer_canceled`), not failed |
| S7 | both directions, 10 x 3 MB, Wi-Fi off 6 s in the middle | done, byte for byte |
| S10 | (a) phone → PC 16 x 2 MB canceled on the phone; (b) **the PC forgets a half-sent 16-file upload** (24 h clock offset), the phone fails it (`not_found`), **user Retry** sends only the 9 files without a 200 under a new ID, all 16 right at the end; (c) PC → phone 12 x 5 MB canceled on the PC | all pass |
| S11 | the queue with a batch: 12 files shared while the PC is stopped (it starts afterwards); 12 files sent by the PC while the phone has no Wi-Fi | both arrive when the other side is back, byte for byte |
| S8 | stall hunt, see below | |
| S9 | timings | below |

A *user-canceled* upload has **no Retry** (only a failed one: `TransferEngine.retry`, by design), so the Retry check is S10b, built like 1.4's S7b but with a batch. A download can't be retried by the phone at all; the PC has to send it again.

### Timings (indicative only)

S9, 50 x 3.4 MB (170 MB), the emulator's own network path, three runs each, one long session on `kin_api37` while the PC was busy with a Unity editor opened by the user (CPU not saturated):

| | runs | peak files at once |
|---|---|---|
| PC → phone | 29.4 s, 27.9 s, 27.1 s (about 6 MB/s) | 4 |
| phone → PC | 42.0 s, 41.2 s, 40.9 s (about 4 MB/s) | 4 |

These are **pipe-bound** (the emulator's NAT, as the Android agent measured: 20 s and 30 s on a quiet PC). They say nothing about the 3 to 4 s target on a real phone on Wi-Fi, which still needs `tools/speedtest`. What the emulator does show: 50 small files (20 to 70 KB) take **1.7 s** phone → PC and **2.4 s** PC → phone including the tap. The per-file cost is gone, and four files are on the wire at once in both directions every time (`filesActive` peak 4, the PC's own count).

## The stall

**Question.** The Android agent saw one phone → PC batch with a Wi-Fi cut stall for 3.5 minutes (four files done, four connections idle until the PC closed them after 30 s, the PC still holding the old WebSocket), once in about 80 cuts with the new APK, never in 20 with the old one.

### 1. Wi-Fi cuts do not reproduce it, and why

`android\tools\e2e-v011.ps1 -Only S8` cuts `svc wifi disable / enable` at a random moment of a 10 x 3 MB batch (2 MB to 20 MB in, random 0 to 4 files in flight), for a random time, 35 cuts per direction per session, with the phone's logcat and log file and the PC's log kept per run (`-StallDir`).

| Session | Cuts | Off for | Recovery after Wi-Fi is back (median / p90 / max) | Stalls |
|---|---|---|---|---|
| 1 | 35 out + 35 in | 1.5 to 10 s, every 10th 20 s | out 9.2 / 11.7 / 12.2 s; in 8.4 / 10.4 / 12.1 s | **0** |
| 2 | 35 out + 35 in | 0.2 to 2.5 s | out 9.5 / 11.0 / 12.2 s; in 8.9 / 10.7 / 11.0 s | **0** |

"Recovery" is until the PC's record says done, including the rest of the bytes. In every run the phone saw `Wi-Fi lost`, closed the link (`closing with 1000 (Wi-Fi lost)`), reconnected **0.1 s** after `Wi-Fi available` (`available->connected` never above 0.1 s, no failed attempt) and the runners resumed. 

**An emulator Wi-Fi cut is a loud cut.** In all 140 runs the open sockets were *aborted* by the guest (`SocketException: Software caused connection abort` on the WebSocket and on the file calls, the instant Wi-Fi went off). That is not what a router or access point failing looks like: there the packets just stop. **So the Wi-Fi scenarios cannot test whether the phone notices a silent drop, and the one-off stall is most likely a silent drop.** Everything below uses silent drops.

### 2. Does the phone's ping/pong timeout fire?

Yes, but late with the default. S12 freezes the PC process (`NtSuspendProcess`: TCP stays up, nothing is read or answered) with the link idle or a batch running; a debug command sets the OkHttp WebSocket ping interval (`SET_PING_INTERVAL`).

| Ping interval | State | The phone's WebSocket ended after |
|---|---|---|
| 15 s | batch running | **27 s** (`SocketTimeoutException: sent ping but didn't receive pong within 15000ms`) |
| 15 s | idle | **27 s** (the same) |
| **240 s (the app's default)** | batch running | **476 s**, `sent ping but didn't receive pong within 240000ms (after 0 successful ping/pongs) ... hello accepted for 480017 ms` |

With the default 240 s the first ping goes out 240 s after the connection and the pong is waited for until the next one: **a silent drop is noticed 240 to 480 s later (about 8 minutes at worst).** In the 476 s run the upload's HTTP calls died after about 30 s each (`SocketTimeoutException: timeout`, `Read timed out`, four rounds) and **then nothing was logged for six minutes** until the WebSocket ended; after the thaw everything finished by itself within 6 s. So with a silent peer the batch waits, and the wait is set by the ping interval. The numbers 3.5 minutes and 237 s of the original stall fit the first ping at 240 s.

### 3. Does OkHttp's pool reuse a dead connection? Is it the 4-connection change?

No to both. Each `LinkSession` makes its own connection pool (v0.1.1 `LinkSession.httpEndpoint`), and a new session means a new pool, so nothing dead is reused across a reconnect; inside one session the PC's 30 s idle close is covered by the pool's 20 s keep-alive. And with the committed **0.1.0 app** (built from `git archive v0.1.0`, one file at a time) the silent-drop control (S14, API 29, the same PC build) fails the same way:

### 4. A silent drop of about a minute fails a phone → PC batch for good (P1, the reproduced symptom)

S14 roots the emulator (`adb root`, a google_apis image) and drops every packet between the phone and the PC with `iptables` for N seconds, with the Wi-Fi "up" (no `Wi-Fi lost`, no abort), in the middle of a 10 x 3 MB batch.

| Drop | New APK, kin_api37 | New APK, kin_api29 | **0.1.0 APK**, kin_api29 |
|---|---|---|---|
| 6 s, 30 s | done by itself, 5 to 6 s after the packets flow | | |
| 45 s | done by itself | done, 3 s after | done, 5 s after |
| 60 s | done, 5 s after | done, 0 s after | **failed** |
| 75 s | | **failed** | **failed** |
| 100 s | **failed** | **failed** | **failed** |

(PC → phone batches with 6, 30, 60 and 100 s drops all finished by themselves in 6 to 11 s: the PC notices the dead phone after 5 s without an `ack` and closes the WebSocket, which makes the phone's link end and its runner park correctly.)

"Failed" means: the phone log says `upload <id> failed: connection_lost`, the batch stays failed after the network is back, the WebSocket **never ended** (no `connection ended`), and nothing moves until the user presses Retry (an upload; for a download the PC has to send it again). The PC log shows exactly the original stall's signature: `closing the connection from 127.0.0.1:...: idle for 30 s` four times and `the connection broke during "f4"; 2899968 of 3000000 bytes are kept`. The timeline of a failing run (new APK, 100 s):

```
t+0     drop starts, 4 PUTs in flight
t+30 s  SocketTimeoutException: timeout        (the OkHttp write timeout, one attempt that did move bytes)
t+35 s  failed to connect to /10.0.2.2 ... after 5000ms   (the next POST / PUTs: connect timeout 5 s)
t+41 s  ... again, 4 more times, one every 5.5 s
t+60 s  upload <id> failed: connection_lost    (TransferPorts.maxStalledAttempts = 5 attempts with no byte moved)
        (the WebSocket is still "connected" for another 3 to 7 minutes)
```

The original stall fits: four files complete, four connections idle until the PC's 30 s idle close, a link that does not end, then silence. The Android agent's script saw "nothing moves" and its later Wi-Fi switch started a new connection and new attempts.

### 5. The backoff after a hole (P2, also reproduced)

S13 freezes the PC for N seconds starting the moment the Wi-Fi is back, during a batch, so the phone's first reconnects get no TLS answer (the race is 1.5 s per address).

| PC frozen for | The phone's waits between attempts | Batch done after the thaw |
|---|---|---|
| 10 s | 5 s | 6 s |
| 40 s | 5 s, 30 s | 12 s |
| 100 s | 5 s, 30 s, **120 s** | **80 s** |

So a PC that cannot answer for about 45 s after the phone's Wi-Fi returns leaves the phone asleep for 2 minutes (then 10 minutes), and nothing wakes it: a waiting transfer (`reconnectWaitMillis` 15 minutes) does not kick the loop, only a network change or a user action does. 5 + 30 + 120 s plus the attempts is about 3 minutes: the other way to a "3.5 minute stall". I could not make the PC miss its handshakes for that long on its own (no log line, no `too many connections`, no handshake timeout in 140 cuts), so I can't say that this is what happened in the one stall; it is a second way to the same symptom.

### What I conclude

- **Not reproduced by Wi-Fi cuts; reproduced by silent drops, with the exact signature, in 0.1.0 as well.** The one-off stall is a silent drop (or a very short one, such as the Wi-Fi stack enabling Wi-Fi again while it is still being disabled) meeting the two phone behaviours P1 and P2. It is **not caused by the 4-connection change**: the control APK fails at 60 s, the new one at 75 s, which is within the noise of where the drop falls among the attempts.
- Worth doing before the user's real-phone test: P1 below, which is what a user with a flaky Wi-Fi will meet (a notification "keeps dropping, try again" after a minute, with the app still showing connected).

## Product observations for a coder (nothing fixed by me)

**P1. A phone → PC (or PC → phone download) transfer gives up after about a minute of a silent drop although the link is only declared dead after 4 to 8 minutes; it then does not resume by itself.**
- *Repro:* `android\tools\e2e-v011.ps1 -Only S14 -S14Seconds 100 -S14Dirs in` (rooted emulator; a 10 x 3 MB share, `iptables -I OUTPUT 1 -d 10.0.2.2 -j DROP` and the same for `INPUT -s 10.0.2.2` for 100 s). Logs of the failing runs: phone logcat and PC log per run in the `-StallDir`.
- *Cause:* `UploadRunner.execute` (`UploadRunner.kt:110-137`) and `DownloadRunner.kt:106-128` count an attempt that moved no byte as stalled and fail the transfer with `connection_lost` after `TransferTimings.maxStalledAttempts = 5` (`TransferPorts.kt:159`). With the network silently gone each attempt ends in about 5.5 s (OkHttp connect timeout 5 s + `retryDelayMillis` 500 ms) and the first one takes 30 s (write timeout), so the transfer dies about 60 s in. The attempts do not wait for a new connection because `connectionFor` keeps handing out the **same still-"ready" `LinkSession`**: the WebSocket only ends at the OkHttp ping failure (`DefaultLinkEnvironment.kt:31`, `pingInterval(240 s)`: 240 to 480 s). A path that does notice a dead peer quickly exists (`LinkSession.sendAcked`: no ack in 5 s → `socket.cancel()` → the session ends → the runner parks and resumes on the next connection), and it is what makes PC → phone batches survive (the PC sends something and gives up after 5 s).
- *Not new:* 0.1.0 behaves the same (control).
- *Suggested fixes (any one):* when an attempt fails with no progress, **probe the link** (`connection.ping(5_000)`; on `NoReply` the session is cancelled and the runner parks) before counting it as stalled; or do not count a stalled attempt while the connection is `isReady` and has not been probed; or a shorter WebSocket ping interval while a transfer is active (S12: 15 s ends a silent link in 27 s). The user can press Retry on a failed upload today, so this is not data loss.

**P2. The reconnect backoff (5 s, 30 s, 2 min, 10 min) is not reset while a transfer is waiting for the link.**
- *Repro:* `-Only S13 -S13Freezes 100` (the PC process frozen for the first 100 s after the Wi-Fi is back): the batch ends 80 s after the thaw, the phone's waits were 5, 30 and 120 s.
- *Cause:* `LinkLoop.waitBeforeRetry` (`LinkLoop.kt`), woken only by `userAction()` and `networkChanged()`; a transfer that waits in `connectionFor(..., reconnectWaitMillis = 15 min)` does not call `userAction()` except for its first attempt (`kick = first`, `UploadRunner.kt:116`).
- *Suggested fix:* while a transfer waits for the link, cap the backoff at about 30 s, or kick the loop every 30 s from `connectionFor`.

I found **nothing wrong on the desktop side** in 140 cuts, about 50 batches under silent drops and the 50-file runs: no `409` storm, no `too many connections`, no leftover `.part`, a resume `POST` in the middle of four writers (the v0.1.1 change) never failed a file.

## Script fixes made (test code only)

| Where | Problem | Fix |
|---|---|---|
| `tools\e2e-1-2-scenarios.ps1:98`, `e2e-1-3-scenarios.ps1:154`, `e2e-1-4-scenarios.ps1:83,714` | The "no more `Set-Clipboard`" rewrite left `Set-QuietClipboardText $1` (a regex back-reference that was never replaced): every PC → phone text of 1.2 to 1.4 used the wrong (empty) text. The first pass of 1.3 and 1.4 failed on it (S10c to S10e of 1.3, S1 and S9 of 1.4) | `$Text` / `$auto` |
| `tools\kin-dev.ps1` (`Test-KinCommandLineHasDataDir`) | The data directory was compared as text. `[IO.Path]::GetFullPath` turns an 8.3 name in an existing part of a path (`$env:TEMP` here is `C:\Users\<NAME>~1\...`) into the long name, so `Start-DevKin` refused every e2e script ("the arguments must contain --data-dir") on a PC whose `TEMP` is short | compared as normalised paths (`Get-KinCommandLineDataDir`) |
| `android\tools\e2e-v011.ps1`, `bench-batch.ps1` | used the private build, recognised processes by path, ignored the lock, used `Set-Clipboard`-free files but did not save the clipboard | now the standard dev Kin (`kin-lock`, `kin-dev`, `Start-DevKin`, `Invoke-Kin`, `Stop-DevKin`), the clipboard saved and restored, the 8.3-safe work folder, an exe check (`tauri build`), `COMPUTERNAME=DESK-PC` |
| `android\tools\build-dev-pc.ps1` | redundant with D17 | **retired**: the file is a stub that says so and exits 1; `android\README.md`, `tools\README.md` updated (they also had `\t` and `\b` characters where `\tools\` and `\build` had been, from an earlier edit) |
| `android\tools\e2e-v011.ps1` S4, S10 | 12 x 1 MB finished before the phone's cancel arrived (the PC is faster than the emulator's pipe now); S10 planned a "Retry" of a user-canceled upload, which has none; S11 wrapped a hashtable in `@()` (`.Count` of 1) | S4, S10 12 x 5 MB; the Retry check is the 404 path (S10b) as in 1.4's S7b; `.Count` on the table |
| `android\tools\e2e-v011.ps1` | the "link is up" check read only the PC's `status.json`, which says `phone_connected` for a phone the PC has not noticed is gone | `Wait-Link`: the PC says connected **and** the phone's `STATE` says `link=Connected` |
| `e2e-v011.ps1`, `e2e-1-1.ps1`, `bench-batch.ps1` | `start.ps1` reuses any running emulator, so a run for `kin_api37` ran on a left-over `kin_api29` without noticing (the first S14 results of the second session were API 29 for this reason, found from the log line `API 29`) | the running AVD is compared with `-Avd` and the run stops if it differs |
| the same three | the adb server that `start.ps1` leaves behind sometimes showed the new emulator as "offline" | one `kill-server` / `start-server`, then look again |
| `e2e-v011.ps1` | a Kin left over from a killed run held the port; the new test Kin fell back to a random port, and after a restart the phone could not find it (mDNS does not work on the emulator) | the port the Kin listens on must be the one asked for, or the run stops with a message |
| `tools\kin-lock.ps1`, `tools\emulator\_common.ps1`, `tools\emulator\start.ps1` | Unity's Android tooling (the user opened the Unity editor and hub in the middle of this run) and our adb server fought over the same emulator ("device offline" every few seconds), and `Initialize-Adb` **stops any adb it did not start**, which would have stopped Unity's | the e2e/smoke/bench scripts use their own adb server port (`ANDROID_ADB_SERVER_PORT=5038`, set by `Enter-KinLock`); with that port set `Initialize-Adb` leaves other adb processes alone. `start.ps1 -WipeData` added (one use below) |

New scenarios in `android\tools\e2e-v011.ps1`: S8 (stall hunt), S9 (timings), S10 (cancel, 404 + Retry, PC cancel), S11 (queue), S12 (silent peer, frozen PC), S13 (backoff), S14 (silent drop with `iptables`). Parameters: `-S8Runs -S8CutMinMs -S8CutMaxMs -S8Dirs -S8Seed -S9Runs -S12Cases -S13Freezes -S14Seconds -S14Dirs -S14WaitSec -PingSeconds -StallDir`. Array parameters are strings (`-S14Seconds 45,75,100`) because `powershell -File` turns `45,75,100` into one number.

## For the reviewer

- **The PC was restarted at about 00:22 on 2026-10-17** (the installed Kin has a new process ID and start time, the Unity editor and hub started at the same moment, and my emulator and background runs died). I did not touch the installed Kin at any time: its process ID before was 25708, after 25796; every script run carried its own `--data-dir`, and `check-dev-beside-installed.ps1` passed twice. The restart is the user's. It cost a few hours of reruns.
- **The `kin_api37` emulator needed a factory reset** (`tools\emulator\start.ps1 -Avd kin_api37 -WipeData`, once): after it was killed hard by that restart, its adb went offline every few seconds for good. `kin_api29` was fine. Nothing of value was on the AVD (it is a test device the scripts reinstall on).
- **Orphans I made and removed:** twice I killed a script driver with `Stop-Process`, which skips the script's teardown and left three test Kins (`target\release\Kin.exe` with a `kin-e2e-v011-*` data directory) running. They held the test port and made one run fail (the port check above). I stopped them by process ID after checking the program file and the data directory, and deleted the throwaway folders of my runs in `%LOCALAPPDATA%\Temp` (28 of them). No process of anybody else was stopped.
- **`D:\kin-v011-android` (1.9 GB) was not deleted**: this environment refuses to remove that path. It holds only the Android agent's own output (the private desktop build, a source copy, benchmark outputs). `Remove-Item -Recurse D:\kin-v011-android` by hand when convenient. Nothing refers to it any more.
- The user's clipboard was saved before and restored after every run (the kept-backup rule of `clipboard-backup.ps1` took over after the killed runs: "keeping the earlier backup"). It now carries the invisible `ExcludeClipboardContentFromMonitorProcessing` mark, as v0-1-1-desktop.md says.
- The timings of S9 were taken with a Unity editor busy next to the emulator; the Android agent's numbers (30 s and 20 s) are the better indication of the emulator's pipe.
- `desktop\check-dragdrop.ps1` (`-AllowMouse`), `desktop\release-smoke` (ARM) and the real-phone speed test are not run, as above.
- **Not done:** the 0.1.0-vs-0.1.1 *speed* control (`bench-batch.ps1`, the Android agent's numbers stand), a Wi-Fi-cut run on a real router (the only thing that settles whether the one-off stall was a silent drop), API 29 runs of 1.2 and 1.4.
