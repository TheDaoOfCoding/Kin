# Milestone 2B.3 integration: media controls in both directions

Date: 2026-10-17. Integration agent (Claude Sonnet 5.5). **Not committed.**

## Verdict: BLOCKED, none of the integration scenarios ran

A game was running for the whole session, so every step that needs the dev Kin and the emulator was **skipped on purpose**. Nothing in this file is a pass or a fail of the product.

* From 12:36 to the end of this session (14:30 and later, 2 hours or more) **Fortnite** was running with its anti-cheat: `FortniteClient-Win64-Shipping` (window title "Fortnite"), `FortniteClient-Win64-Shipping_EAC_EOS`, `FortniteLauncher`, `FortniteBootstrapper` and the `EasyAntiCheat_EOS` service, plus the Epic launcher.
* `tools\check-2b3-desktop.ps1` (S0) stopped itself at `Assert-NoGameRunning` with "A game or an anti-cheat seems to be running ... Close the game and run this again". It wrote and touched nothing.
* The rule for this task says to check for a fullscreen game or EasyAntiCheat before anything visible and to skip the step if one is there. I polled every 30 s for about 2 hours (one background watcher plus foreground loops) and the game never stopped. I did **not** bypass the guard, and I did not edit `kin-dev.ps1` to weaken it: the user is playing, and the run would also put an emulator, a dev Kin and a media session on a PC that is busy with the game.
* One more thing for whoever runs this next: the guard also matches the Epic launcher's helper processes (`EpicGamesLauncher`, `EOSOverlayRenderer`, `EpicWebHelper`, `EpicOnlineServicesUserHelper`, all under `\Epic Games\`) and the idle `EasyAntiCheat_EOS` service. After Fortnite exits, those may still make `Assert-NoGameRunning` throw. Quit the Epic launcher too, or run with only the launcher left and decide whether that counts as "a game" (it does not, in my reading, but the shared guard says so).

**To run it:** close the game (and the Epic launcher), then, from the repo root, in this order, each once:

```
tools\check-2b3-desktop.ps1                       S0 (silent, about a minute)
tools\e2e-2b-3.ps1                                S1 .. S4 and S6 on kin_api37
tools\e2e-2b-3.ps1 -Avd kin_api29 -Only S1,S2     the short Android 10 pass (1 and 2)
tools\e2e-2b-2.ps1 -Only S1                       S5, pending since 2B.2
tools\e2e-1-3.ps1 -Only S1,S4                     S5, one photo each way
```

## What was done

### Builds (both sides changed since the last builds)

Rebuilt on this PC on 2026-10-17, in the foreground. The desktop build ran with `CARGO_BUILD_JOBS=2` (the page file had run out during a parallel build before); it did not run out this time.

| | |
|---|---|
| `desktop\build.ps1` | succeeded (`tsc && vite build`, `kin-native-host.exe`, then the release build and the NSIS bundle, `Compiling kin v0.1.1`, 3 min 30 s) |
| `desktop\src-tauri\target\release\Kin.exe` | 14,244,864 bytes, 2026-10-17 12:41:33, SHA-256 `1AC4B8F8C4486EDF9F5843A0EE722A9DC5243B8A38CC32F1329C6B161FD5AE9E` |
| `...\bundle\nsis\Kin_0.1.1_x64-setup.exe` | 4,279,368 bytes (4.08 MiB), 2026-10-17 12:41:33, SHA-256 `C03F23C6AEC7C21FEC0EF894B4529E7EFB8B1EC97EB30B5FFC82CCFDF4D481A8`. Not installed |
| `gradlew assembleDebug assembleRelease` (android) | BUILD SUCCESSFUL in 2 min 55 s (16 tasks executed) |
| `app-release.apk` (signed) | 4,792,782 bytes, 2026-10-17 12:44:31, SHA-256 `581611BF34EAF4800D9A8FC1FC96824CAA7038380CAD9F4F0DC716851E7718B6`. No keystore value was printed |
| `app-debug.apk` | 49,214,099 bytes, 2026-10-17 12:42:10, SHA-256 `D4F45138673D6DFB846004DF65A559E7A4CE245CB23BF5B48687EEF221C83DB5` |
| Debug build for the checks | `cargo build`, `cargo build --examples` and `cargo test --test media_surface_real --no-run` (debug, `-j 2`) all finished, so `check-2b3-desktop.ps1` has what it needs (`Kin.exe` debug, `media_test_session.exe`, `fake_phone.exe`, the `#[ignore]`d surface test) |

Both APK hashes differ from the Android results file (release 4,776,398 bytes, `D281B72...`; debug `9A65EAFE...`, same 49,214,099 bytes), as expected: the Android review round 1 fixes landed after that file's builds.

### `tools\e2e-2b-3.ps1` (written, parse-checked, **not run**)

Written to `tools/e2e-2b-3.ps1` and described in `tools/README.md`. It parses without errors; no line of it has been executed, so it **will need a first-run shake-down** (the likeliest places: the exact text of `MEDIA_STATE` fields, the slider's node in `uiautomator`, `cmd statusbar expand-settings` on the image, the `dumpsys media_session` text). It follows the 2B.2 script: lock, `kin-dev.ps1`, a headless emulator with no audio, a copy of the release `Kin.exe` as the test Kin on its own data directory and port 49371, `--dev-media-aumid` naming only the PC's own test session. Scenarios, all with only its own sessions:

| Id | What it does |
|---|---|
| setup | starts the PC test session (`media_test_session serve`, hidden window, `Paused`, nonce title), the test Kin (mode `test`), the emulator, installs the debug APK, pairs; the phone's switches are off by default and **nothing** is sent; grants listener access, turns share and show on |
| S1 | silent `DebugMediaSession` on the phone; flyout read back by its own AUMID (title, artist, 200,000 ms, thumbnail, position, status 5 = Paused, buttons); `playing` on the phone is `Paused` in the flyout; track change; `control <dev Kin's AUMID> toggle|next|prev|seek 120000` reach the phone's test session; `Kin.exe --dev-media volume 40` moves the emulator's music volume; share off = exactly one `media.clear` and nothing sent; share on again; `disallow_listener` while sharing clears the PC card (review fix 2); allow again |
| S2 | `kin-pc-media` in `dumpsys media_session` with the PC test title and state, `MediaStyle` notification on `kin_pc_media`, no media foreground service, a carousel screenshot on API 37 (shade on API 29); `MEDIA_PC_CMD play|pause|next|prev|seek 90000` reach the PC test session's press log; the fake PC volume (starts at 50, `level 30`, `delta 5` = one or more 2 % steps, `media.volume` back, real master volume unchanged); the Home slider dragged to about 85 %; the PC session stopped: card gone, slider still there and still works (fix 4); a Wi-Fi flap and a restart of the test Kin with the PC session stopped meanwhile leave no stale card (fix 8) |
| S3 | PC show off, PC share off, phone show off, phone share off: each sends nothing (counts), cards and flyout session go; no `media.cmd` out while the phone does not share; 200 phone states in ~3 s reach the PC as 3..40; 120 PC changes reach the phone as a handful; 40 phone commands in a burst: at most 20 reach the PC test session; `fake_phone` as `--old-phone`, as a phone that never sent a status, and as a phone with `show:false` are sent no `media.*` |
| S4 | log, every file of the data directory, `status.json`, `--dev-dump-view media`, the phone's Kin log (whole run, flushed before every command), the phone's app data (one `tar`): none of the test titles, artists, album, nonce, AUMID or `Kin test artist`; no picture file on the PC; the phone's system log is only counted as a note |
| S6 | real master volume read before and after (`volume-read`, no `Set*` exists in that tool); the installed Kin's PIDs; no process of the run left |

It never injects OS input on the PC (the emulator's `input tap/swipe` are on the emulator), never touches the clipboard, never sends a command to a session it did not create, and **cannot** write the real volume: the test Kin has the fake endpoint and the script never passes `--dev-media-real-volume`.

### Read-only facts taken while blocked

* The real master volume, read only (`media_test_session volume-read`, no `Set*` call exists in it): `VOLUME level=100 muted=false steps=51`. This is the baseline for S6 and equals the value in the desktop results file.
* The installed Kin (`C:\Program Files\Kin\Kin.exe`) has PID **25796** (the same as in the Android results file); I started, stopped and touched nothing of it. No process of mine is left (no dev Kin, fake phone, test session, emulator or adb server was started).

## Scenario table

| # | Scenario | Result |
|---|---|---|
| 0 | `check-2b3-desktop.ps1` (22 checks, the B1 refusal, the `#[ignore]`d surface test) | **not run** (guard: game and anti-cheat running) |
| 1 | Phone to PC | **not run** |
| 2 | PC to phone | **not run** |
| 2b | API 29 short pass (1 and 2) | **not run** |
| 3 | Gates and rate limits | **not run** |
| 4 | Privacy grep | **not run** (the unit-test greps of both coders pass; the integration grep is pending) |
| 5 | `e2e-2b-2.ps1 -Only S1`, one file each way | **not run** |
| 6 | Guard | partly: real volume baseline read once (above), installed Kin PID 25796 unchanged for the whole session, nothing of mine left running. The "after" half needs a run |

## Bugs found

None: nothing was run. (No product file was edited.)

## What I'd look at first on the first run

These come from reading the coders' notes, not from a failure:

* **Review fix 8 (fast reconnect)** is hard to see from outside because the spec already clears the card on a connection close; the script checks the visible result (no stale card after a Wi-Fi flap or a PC restart with no PC session meanwhile), not the internal path.
* **`media.cmd volume` from `MEDIA_PC_CMD`** goes through the phone's `VolumeProvider`, which exists only while "Use the phone's volume keys for the PC" is on; the script turns it on for the volume steps and off again, and uses the Home slider for the slider checks.
* **Android 10:** the access status can lag the listener binding by up to a minute (Android results, item 5); the script waits 70 s there.
* **S3's burst numbers** (3..40 states, at most 20 `button next`) are bounds from the design (250 ms trailing, bucket 10, 20 commands per 10 s), not measured values; adjust them after a first run if a bound is off for a good reason.
