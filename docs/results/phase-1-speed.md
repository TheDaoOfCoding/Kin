# Phase 1: transfer speed, Kin vs LocalSend (real phone, real Wi-Fi)

**Status: Kin measured in both directions. LocalSend measured PC to phone only, and with a scripted sender, not the LocalSend PC app. LocalSend phone to PC is not measured. Verdict on targets.md row 4: provisional (see "Verdict").**

## Setup

- **PC:** Windows 11, wired Ethernet (1 Gbps) to the router. Kin 0.1.0 installed and paired, normal mode. LocalSend 1.18.2 installed and running (not used as a sender, see below).
- **Phone:** OnePlus Nord 2T (CPH2401), Android 14, 5 GHz Wi-Fi, link rate 702/866 Mbps, strong signal. Kin 0.1.0 release build, paired. LocalSend installed. About 10 GB free before and after; storage 91% full.
- **A game was running on the PC the whole time** (Fortnite with EasyAntiCheat). It may skew the numbers a little and add Ethernet traffic. It affects Kin and LocalSend equally. Because of it, **no input of any kind was sent to the PC**: the PC side only ran `Kin.exe --send`, counter and file polling, and an outgoing script. All taps were phone taps through adb.
- **Files:** one 2 GiB (2,147,483,648 bytes) random file, and 50 random-content `.jpg` files (171,275,000 bytes in total, about 3.4 MB each). All names start with `kin-speedtest-`.
- MB/s means 10^6 bytes per second.

## Method

- **Wire window:** the PC's Ethernet byte counters, sampled every 50 to 250 ms. It runs from the first interval at 3 MB/s or more to the end of the last one. (For phone to PC it counts all received bytes, so other traffic on the PC counts too; in every run the wire time matches the other two times, so it was small.)
- **Complete time:** from the start of the wire window until the receiver has every file at its final name and full size (the phone is polled with `adb shell stat` every 150 ms; the PC folder is polled for phone to PC).
- **Kin `status.json`:** `startedMs` to `finishedMs` of the transfer record (first start to last finish for the photo batches).
- **Phone CPU and I/O wait:** `adb shell top -b -n 1` every 3 to 5 s during the transfer.
- **Triggers:** Kin PC to phone: `Kin.exe --send <files>`. Kin phone to PC: the file(s) selected in the phone's file browser, then Share, then Kin in the share sheet (adb taps). The file was pushed to the phone over USB first. LocalSend: see below.
- **Scripted LocalSend sender.** The LocalSend protocol (v2) is open: `POST /api/localsend/v2/prepare-upload`, then `POST /api/localsend/v2/upload` per file, over HTTPS on port 53317 (the phone's LocalSend also asks for a client certificate; a throwaway self-signed one was used). A small Node script on the PC sends the files to the **phone's LocalSend app** with Quick Save on and the app in the foreground. Outgoing connection only; no listener was started on the PC. **So the LocalSend numbers below measure the phone's LocalSend receive path with a plain, scripted sender, not the LocalSend PC app.** The real PC app could be slower (Flutter, hashing) or the same. The script does not hash the files.
- Order of the runs, which matters (see "The Wi-Fi changes by itself"): Kin 2 GiB PC to phone r1 to r3, Kin phone to PC, Kin photos, LocalSend 2 GiB r1 to r3, LocalSend photos, then Kin r4, LocalSend r4, Kin r5, LocalSend r5.

## Results

### 2 GiB file

| Case | App | Run | Wire s | Wire MB/s | Complete s | Complete MB/s | App's own s (MB/s) |
|---|---|---|---|---|---|---|---|
| PC to phone | Kin | 1 | 62.1 | 34.6 | 63.2 | 34.0 | 64.9 (33.1) |
| | Kin | 2 | 73.9 | 29.0 | 76.9 | 27.9 | 77.5 (27.7) |
| | Kin | 3 | 105.2 | 20.4 | 107.6 | 20.0 | 111.6 (19.2) |
| | **Kin** | **median of 1 to 3** | **73.9** | **29.0** | **76.9** | **27.9** | **77.5 (27.7)** |
| | Kin | 4 (extra) | 29.7 | 72.3 | 30.0 | 71.6 | 30.2 (71.1) |
| | Kin | 5 (extra) | 29.8 | 72.1 | 29.9 | 71.9 | 30.7 (70.1) |
| | **Kin** | **median of 1 to 5** | **62.1** | **34.6** | **63.2** | **34.0** | **64.9 (33.1)** |
| PC to phone | LocalSend, **scripted sender** | 1 | 98.9 | 21.7 | 99.2 | 21.6 | sender 99.8 (21.5) |
| | LocalSend, scripted | 2 | 29.4 | 73.0 | 29.5 | 72.7 | sender 29.4 (73.1) |
| | LocalSend, scripted | 3 | 30.8 | 69.7 | 31.0 | 69.2 | sender 30.7 (70.0) |
| | **LocalSend, scripted** | **median of 1 to 3** | **30.8** | **69.7** | **31.0** | **69.2** | |
| | LocalSend, scripted | 4 (extra) | 29.2 | 73.5 | 29.4 | 73.0 | |
| | LocalSend, scripted | 5 (extra) | 30.0 | 71.5 | 30.2 | 71.2 | |
| | **LocalSend, scripted** | **median of 1 to 5** | **30.0** | **71.5** | **30.2** | **71.2** | |
| Phone to PC | Kin | 1 | 90.3 | 23.8 | 90.9 | 23.6 | 88.5 (24.3) |
| | Kin | 2 | 46.1 | 46.6 | 46.2 | 46.5 | 46.2 (46.5) |
| | Kin | 3 | 39.8 | 53.9 | 39.8 | 53.9 | 39.6 (54.2) |
| | **Kin** | **median of 1 to 3** | **46.1** | **46.6** | **46.2** | **46.5** | **46.2 (46.5)** |
| Phone to PC | LocalSend | not measured | | | | | needs the PC LocalSend to accept, which needs PC UI input (a game was running) |

### 50 photos (171.3 MB, about 3.4 MB each)

| Case | App | Run | Wire s | Wire MB/s | Complete s | Complete MB/s | App's own s (MB/s) |
|---|---|---|---|---|---|---|---|
| PC to phone | Kin | 1 | 9.3 | 18.4 | 9.4 | 18.2 | 10.2 (16.8) |
| | Kin | 2 | 6.4 | 26.7 | 6.5 | 26.4 | 6.6 (25.8) |
| | LocalSend, **scripted sender** | 1 | 3.6 | 47.8 | 3.7 | 45.9 | sender 3.0 (56.6) |
| | LocalSend, scripted | 2 | 2.8 | 61.7 | 2.8 | 60.3 | sender 2.8 (60.3) |
| Phone to PC | Kin | 1 | 13.6 | 12.6 | 13.7 | 12.5 | 9.5 (18.0) |
| | Kin | 2 | 7.1 | 24.2 | 7.1 | 24.1 | 7.1 (24.0) |
| Phone to PC | LocalSend | not measured | | | | | as above |

Two runs each: too few for a median, so both are shown. For Kin phone to PC run 1, the Kin record (9.5 s) starts later than the wire window (13.6 s): about 4 s passed between the tap on Kin in the share sheet and Kin's transfer starting (reading the 50 files and preparing them). Two earlier Kin PC to phone photo runs were not counted because the monitor was watching the wrong phone folder (Kin saves images to `Pictures/Kin`); Kin's own times for them, 8.0 s and 10.9 s, are in the same range.

## The Wi-Fi changes by itself

The runs fall into two groups, whichever app is used:

- **Slow:** Kin PC to phone r1 to r3 (34.6, 29.0, 20.4 MB/s) and LocalSend r1 (21.7), and Kin phone to PC r1 (23.8).
- **Fast:** LocalSend r2 to r5 (69.7 to 73.5), Kin r4 and r5 (72.3, 72.1), Kin phone to PC r2 and r3 (46.6, 53.9, probably a lower ceiling in that direction).

72 MB/s is about 580 Mbps, a plausible real rate for a 702/866 Mbps link. In the fast period, the pairs run back to back agree: Kin 72.3 and 72.1 against LocalSend 73.5 and 71.5. In the slow period, Kin (20 to 35) and LocalSend (21.7) overlap. The order of the runs (Kin's three slow runs first, LocalSend's one slow run first) is the only reason the first three medians differ (29.0 against 69.7): do not read that as a speed difference between the apps. The cause of the slow period is not known (the game's own network use, other devices, the router, the phone's radio; not recorded).

## Phone CPU, storage and Wi-Fi

`top` samples (the machine has 8 cores, so 800% in total):

- **CPU is not the limit.** The Kin process peaked at 57 to 90% of one core in every run (80% in the fast ones). The whole phone was 230 to 750% idle; the lowest idle was 145% in one sample of the slowest Kin run (r3), still over 18% of the machine. LocalSend's app was at the same level where it showed in the list (up to 97% of one core in r1 and r2).
- **Phone to PC (the phone only reads):** I/O wait stayed at about 0 to 30%, and the CPU was idle in the 23.8 MB/s run, so the slow phone to PC run was the **Wi-Fi uplink**, not storage.
- **PC to phone (the phone writes):** I/O wait was high in the slow Kin runs (up to 483% and 476% in r2 and r3) and in LocalSend r1 and r2 (197%, 172%), and low in most fast samples (16 to 30%), with some fast samples up to 187%. So writes stall in the slow period, but I/O wait also appears in the fast ones, and the slow LocalSend r1 had only moderate wait. The phone's storage is 91% full. This is not enough to say storage or Wi-Fi caused the slow runs. The `kswapd0` kernel thread showed up at 12% in the first run.
- **The ceiling is at least 72 MB/s PC to phone**, reached by both apps, so neither the phone's CPU nor its storage caps that direction in the fast period.
- `adb push` over USB ran at 20.3 MB/s (the 2 GiB file) and 32 MB/s (the photos): that is the USB and storage path, not Wi-Fi, so it does not rule storage in or out. A planned on-phone `dd` write test was not valid (the timing arithmetic overflowed on the phone shell) and is not reported.

## Verdict on targets.md row 4

Row 4: "A 2 GB file, 5 runs each way, median MB/s. Then LocalSend with the same file, devices and positions."

- **Not met as written:** LocalSend phone to PC was not measured, LocalSend was driven by a scripted sender, not its PC app, Kin phone to PC has 3 runs (not 5), and the runs are not interleaved well enough for the Wi-Fi swings above.
- **PC to phone, large file:** the data gives **no sign that Kin is slower than LocalSend's receive path**. In the same conditions (fast period, interleaved r4 and r5) the two are within about 3% (72.3 and 72.1 MB/s against 73.5 and 71.5). Medians of five: Kin 34.6, LocalSend 71.5, and that gap comes from the slow period falling on Kin's first three runs. Provisional read: **parity for a large file PC to phone**, not proven by the target's own method.
- **Phone to PC, large file:** Kin 46.6 MB/s median, no LocalSend comparison, so **no verdict**. Kin's phone to PC speed (24 to 54) is lower than PC to phone (up to 72), and the phone's Wi-Fi uplink is the likely cause.
- **Many photos (not row 4, but a speed point):** Kin was **slower** than the scripted LocalSend for the 50-photo batch PC to phone: 6.4 to 9.3 s against 2.8 to 3.6 s (2 to 3 times), and these runs were not interleaved. Per-file work in Kin (a record, a checksum and a gallery entry for each file) is a likely reason; the scripted sender does less work (no hashing). To be checked with the real LocalSend app and an interleaved order before treating it as a Kin problem.

## Not measured

- LocalSend phone to PC, both cases (2 GiB and photos). **Manual steps:** on the PC, open LocalSend, Settings, turn on Quick Save (everyone). On the phone, `adb` can then send with LocalSend's share-sheet entry: select `kin-speedtest-2g-in.bin` in the file browser (push it first with `adb push`), Share, LocalSend, pick the PC. Time it from the PC's Ethernet counters with `tools/speedtest/speedmon.ps1 -Dir in -PcDir <LocalSend's received folder>`. Then turn Quick Save off again. Needs a time when no game is running, or the owner's OK to click in LocalSend's window.
- The LocalSend PC app as a sender (PC to phone), with a human clicking send.
- A 5-run median in every cell, and runs in strictly alternating order (Kin, LocalSend, Kin, ...) with the phone in the same position.
- Why the Wi-Fi was slow in the first half hour (router or other devices were not looked at), and a clean storage-only write test of the phone.
- Quiet-PC numbers (no game).

## State left behind

- **Phone LocalSend settings:** changed for the test (Quick Save on, which switches "Quick Save for Favorites" off) and restored: Quick Save off, Favorites on, Require PIN off, as found. Verified by the switch states.
- **PC LocalSend settings:** not touched. LocalSend's `settings.json` was not read.
- **Clipboard:** saved before the Kin phone to PC photo runs (image formats on it) and restored afterwards, byte for byte (the helper confirmed it). A first call with the wrong argument emptied it for about a minute; the second call put it back.
- **Test files:** the 2 GiB file and the photos are deleted from the PC and from the phone (`Download`, `Download/Kin`, `Pictures`, `Pictures/Kin`: no `kin-speedtest*` left; the PC's `Downloads\Kin` has none either). The throwaway client certificate and key are deleted. The phone has about 10 GB free.
- **Kin and LocalSend:** running as found. My own monitor and Node processes have all exited.
- **Helper scripts:** moved to [`tools/speedtest/`](../../tools/speedtest/README.md). The small raw run results (JSON and `top` text) are in `%LOCALAPPDATA%\Temp\kin-speedtest\results` (about 160 KB).
