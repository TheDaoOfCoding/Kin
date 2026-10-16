# Phase 1 results

Started 2026-10-15 in milestone 1.5 (the packaging), by the desktop agent. This document collects every target of [targets.md](../targets.md) and the exit criteria of [phase-1-mvp.md](../phases/phase-1-mvp.md) in one place. **A row is filled in only where it could be measured without a real phone.** The rest say **needs real phone** and are filled in from the tests in [TESTING.md](../TESTING.md): the 1.5 agents can build and check everything on the emulator and on the development PC, and nothing else.

How to read the status column: **met** (measured, inside the target), **not met**, **measured here, real phone still needed** (the PC half is done), **needs real phone** (nothing to report yet), **not measured** (could have been, was not).

## Reference devices

| Device | Model | OS / skin | Notes |
|---|---|---|---|
| PC | The development PC: AMD Ryzen 9 7900X, 31 GB RAM | Windows 11 Pro 10.0.26300 | Ethernet, network profile **Public** (the Windows 11 default for a new network; the reason for D12) |
| Phone 1 | **needs real phone** | | |
| Phone 2 | **needs real phone** | | |
| Phone 3 | **needs real phone** | | |
| Router | **needs real phone** (a real LAN: nothing on the development PC's network runs a phone) | | Wi-Fi standard, band |
| Emulators (what the agents tested on) | `kin_api37` (Android 17) and `kin_api29` (Android 10), x86_64 | Google APIs images (no Play Store) | Loopback through the emulator's NAT: they say nothing about Wi-Fi speed, Doze, OEM killers or battery |

## The targets

| # | Target (targets.md) | Result | Status | Where |
|---|---|---|---|---|
| 1 | **Reliability**: at least 99% of sends succeed on the first try on a normal home network; every failure logged with an error code | The count is built in (debug screens on both sides). On the emulator every scenario of the e2e runs passes and every failure carries a code. The 99% needs the real network | needs real phone | [phase-1-4-integration.md](phase-1-4-integration.md); both Debug screens |
| 2 | **Arrival time**: text and clipboard under 1 s, a 3 MB photo under 3 s | **PC side, loopback + the emulator's NAT:** a text by the hotkey **2.9 to 6.1 ms** tap to ack; the automatic clipboard **312 to 329 ms** (300 ms of that is the debounce); a 1,000,000-character clipboard 664 to 688 ms. A 3 MB photo: not timed on a network that means anything (the emulator gave 2 to 6 MB/s) | measured here, real phone still needed | [phase-1-2-integration.md](phase-1-2-integration.md), [phase-1-3-integration.md](phase-1-3-integration.md) |
| 3 | **Phone battery**: under 1% per day idle and connected | Nothing: a battery needs a real phone over 24 h | needs real phone | TESTING.md part 7 |
| 4 | **Transfer speed**: large files at least as fast as LocalSend on the same network | **PC side, loopback:** a 2 GB file **1,038 MB/s** into the PC (2.1 s) and **1,143 MB/s** out of it (1.9 s); a resume from the middle at 1,045 and 1,308 MB/s; 6 GB from a fake phone into the real `Kin.exe` at 644 MB/s. The PC is nowhere near the limit. **Real phone, 2 GiB, 5 GHz Wi-Fi (game running on the PC):** Kin PC to phone swings with the Wi-Fi, 20 to 35 MB/s in a slow stretch and 72 MB/s in a fast one (medians 29.0 of 3 runs, 34.6 of 5); phone to PC median 46.6 (23.8 to 53.9). LocalSend's phone receive path with a **scripted** sender: 71.5 median of 5 (one slow run at 21.7). In the same fast stretch the two agree (Kin 72.3 and 72.1, LocalSend 73.5 and 71.5). Phone CPU is mostly idle, so it is not the cap. Kin is slower on a 50-photo batch (6.4 to 9.3 s against 2.8 to 3.6 s). **No LocalSend phone to PC number and not the LocalSend PC app** | partly measured: parity PC to phone, no verdict phone to PC | [phase-1-speed.md](phase-1-speed.md) | [phase-1-3-desktop.md](phase-1-3-desktop.md#throughput) |
| 5 | **PC footprint**: idle under 30 MB of RAM, near 0% CPU | **Private working set 2.8 to 3.0 MB** (tray only, 60 s after launch, `desktop/measure-idle.ps1`; 3.0 MB in 1.4, the 1.5 code did not change it). After every window was opened and closed: **5.1 to 5.4 MB**, one process, **no WebView2 left over**. (The "Working set" column of Task Manager shows 17.3 MB: it counts shared pages, and the target is the private one.) Not measured: **one hour idle** and **CPU over 10 minutes** | **met** for the 60 s number; the 1 h and CPU parts **not measured** | [phase-1-4-desktop.md](phase-1-4-desktop.md#idle-memory-and-size), [phase-1-5-desktop.md](phase-1-5-desktop.md) |
| 6 | **App size**: APK and installer each about 10 MB or less | **Installer 3,764,755 bytes (3.59 MiB)**, per-machine NSIS, unsigned. `Kin.exe` 12.0 MB installed. **APK: 4,662,856 bytes (4.45 MiB)**, signed release, R8-minified and resource-shrunk (`android\app\build\outputs\apk\release\app-release.apk`) | **met** (both) | `desktop/build.ps1`; [phase-1-5-desktop.md](phase-1-5-desktop.md); the Android 1.5 results |
| 7 | **Setup time**: under 1 minute from opening the installer to "Paired" | Needs a person who has not seen the app, a stopwatch, a real UAC prompt and a phone | needs real phone | TESTING.md part 4 |

## The 1.1 soak (the go/no-go gate of [phase-1-mvp.md](../phases/phase-1-mvp.md))

At least 3 days on each test phone, during normal use. **Not started.** Nothing here can stand in for it: the question is whether the OS keeps the service alive and what it costs the battery.

| Measure | Phone 1 | Phone 2 | Phone 3 | Gate |
|---|---|---|---|---|
| Battery used by the app per day | needs real phone | needs real phone | needs real phone | under 1% |
| Disconnects per day | needs real phone | needs real phone | needs real phone | (recorded) |
| Times the OS killed the service | needs real phone | needs real phone | needs real phone | most phones survive, with the battery exemption |
| PC to phone ping after the screen was off for over an hour | needs real phone | needs real phone | needs real phone | under 3 s |

The permission cases (denied, revoked later, older Android) were run on the emulators; on a real Android 17 phone they are repeated at the first pairing (TESTING.md part 4).

## The test matrix (targets.md)

Six payloads, nine situations, both directions: the grid is in [TESTING.md](../TESTING.md#6-the-test-matrix). **Needs real phone**, all of it. What the emulator runs covered instead (and so is not repeated here): every payload in situation 1, the Wi-Fi drop with a resume (5), the queue after Wi-Fi was off, the app force-stopped (8, as far as `am force-stop` is the same as swiping it away), and a restart of Kin on the PC (7, without the Windows start).

## What is not verified until the first real install

The installer cannot be run during development (it needs administrator rights), so these are checked by `desktop/verify-install.ps1` and by hand at the first install; see [phase-1-5-desktop.md](phase-1-5-desktop.md):

- the **UAC prompt** and **SmartScreen** on an unsigned installer;
- the **firewall rules** (TCP for the app, and the UDP rule for mDNS that was added to the plan) and mDNS on a real network (it has never run on the development PC);
- the **Start Menu shortcut's app user model ID** on a real Windows: "Kin" with the app's icon on a toast, and the toast buttons;
- the **upgrade** path (the old uninstaller runs first, the data and the pairing are kept) and the **uninstall** (what `verify-uninstall.ps1` checks).

## The friends test (2 weeks, 3 to 5 people)

**Not started.** Collect: what failed, which features were used most, whether they kept using it. Use the log export (TESTING.md part 8).

## Exit criteria

- [ ] Every target in targets.md is met on the reference devices, or there is a written reason why not: **open** (rows 1 to 4 and 7, and the 1 h and CPU parts of row 5, need real phones and time)
- [ ] No known bug that loses or corrupts data: **no such bug is known** after the 1.1 to 1.4 reviews and the e2e runs; the real phones are the next test
- [ ] At least some friends are still using it after 2 weeks without being reminded: **open**
- [ ] The Phase 2 order is decided based on what the friends test showed: **open**
