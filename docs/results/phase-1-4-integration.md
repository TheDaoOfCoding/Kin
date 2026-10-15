# Phase 1.4 integration: queue, history, notifications on the real Windows app and the real debug APK

Run 2026-10-15 on Windows 11 Pro 10.0.26300 with `kin_api37` (Android 17) and `kin_api29` (Android 10), the release `Kin.exe` (built with `tauri build`; developer mode: `--dev-bind 127.0.0.1`, throwaway data and received folders) and the debug APK. Inputs: [phase-1-4-desktop.md](phase-1-4-desktop.md), [phase-1-4-android.md](phase-1-4-android.md) (both with their "1.4 review fixes"), [protocol.md](../protocol.md) (changelog 45 to 52, Queue). **Nothing is committed.** Script: `tools/e2e-1-4.ps1` (usage at the end). Screenshots: [phase-1-4-integration-screens/](phase-1-4-integration-screens/).

This is the second version of this document. The first described the run **before** the Opus review fixes. The review fixes changed what the two sides do when the PC cancels an upload while the phone is away (changelog 52), so scenarios S7 and S10 were rewritten; the rest of what changed is under "Since the first run".

**Summary.** The two halves work together. The queue, expiry, cancel, the no-ack re-queue, the 404 rules, sensitive text offline, the automatic clipboard offline, history parity and the notification grouping all pass end to end on both API levels: **179 checks on API 37 (full burst of 60 texts and 15 transfers), 178 on API 29 (default burst of 20 and 6), 0 failed**; `e2e-1-1`, `e2e-1-2` (twice), `e2e-1-3` still pass (16, 78 and 78, 142 checks). No product bug was found in this pass; what changed is test tooling (below). Everything is stopped: Kin, the emulator, adb, Gradle, cargo, bun.

## Scenarios

| # | Scenario | API 37 | API 29 | Notes |
|---|---|---|---|---|
| 1 | PC → phone with Wi-Fi off: text, file, text; Wi-Fi on; order | **PASS** | **PASS** | timings below |
| 2 | Phone → PC with `Kin.exe` stopped: text, then a shared file; `Kin.exe` started | **PASS** | **PASS** | text first, then the file (PC history times) |
| 3 | Queue survives restarts: `Kin.exe` restart (PC side), `am force-stop` and relaunch (phone side) | **PASS** | **PASS** | same ids, both delivered, hashes equal |
| 4 | Cancel a queued item on each side: it never arrives | **PASS** | **PASS** | no offer or text send in the PC log; nothing woke the canceled phone items after the reconnect |
| 5 | Expiry: PC `--dev-clock-offset 604801`; phone `EXPIRE --el ahead` | **PASS** | **PASS** | failed with `expired` on both, never sent. Phone: 30 minutes short of 7 days expires nothing, exactly 7 days expires both |
| 6 | No ack re-queues: a frozen receiver (6a PC → phone text, 6b PC → phone offer, 6c phone → PC text) | **PASS** | **PASS** | queued, not failed; delivered once; no `(1)` file; one `toast: Text from` |
| 7a | **The PC cancels a phone upload while the phone is offline** | **PASS** | **PASS** | **changed (item 52).** After the reconnect the PC sends its stored `cancel`; the phone ends the upload **`canceled` (`peer_canceled`)**, not `failed` (`not_found`); nothing is sent again, no new transfer |
| 7b | **The PC forgets a phone upload (24 h): the 404 path, user Retry** | **PASS** | **PASS** | **new.** see below |
| 8 | Sensitive text while the peer is offline, both directions | **PASS** | **PASS** | not queued, user told, never reaches the peer |
| 9 | Automatic clipboard while the phone is offline | **PASS** | **PASS** | nothing queued; nothing stale on the reconnect; a copy while connected is sent |
| 10 | History parity after 1 to 7 | **PASS** | **PASS** | **16 items on both sides, 16 identical** (before: 15, 14 identical); see below |
| 11 | Notifications: 60 texts, 15 transfers (script default 20 and 6) | **PASS** (60 and 15) | **PASS** (20 and 6) | below |
| 12 | Regressions `e2e-1-1`, `e2e-1-2` (twice), `e2e-1-3` | **PASS** (16, 78, 78, 142) | not run | |
| 13 | Logs and Export logs, both sides | **PASS** | **PASS** | below |

### Timings (API 37; API 29 in brackets)

| | |
|---|---|
| `svc wifi enable` → the phone's `hello` on the PC | 4.7 s (5.2 s). About 3 s of it is the emulator bringing Wi-Fi up |
| `hello` → the 3 queued items sent / the last text acked | 170 ms (97 ms) |
| `hello` → the file offer done (the phone has the file) | 1,123 ms (414 ms) |
| `svc wifi enable` → queue empty / file offer done | 5.1 s / 6.9 s (5.4 s / 6.5 s) |
| S2 `Kin.exe` started + phone `CONNECT` → text / file at the PC | 7.1 s / 8.2 s (5.6 s / 6.0 s). Includes `Kin.exe` starting and, when the connect comes before Kin listens, one 5 s backoff step of the phone |
| S6b a frozen phone resumed → the offer done | 32.4 s (31.5 s). The phone had been closed by the PC twice, so its backoff was 30 s. Not seen on a normal reconnect |
| S11 60 queued texts, `svc wifi enable` → all acked | 7.7 s (20 texts on API 29: 5.6 s) |

The first run measured 96 to 103 ms and 352 to 431 ms for the two `hello` rows on API 37; the same script now measures 125 to 170 ms and 480 to 1,123 ms. The difference is within what the emulator does from run to run (the 1.4 done-when is "within seconds"); nothing in the PC's send path changed in a way that adds a second, and the review fix that the PC sends stored `cancel` messages before the queue adds one round trip only when there is one stored.

### Notes per scenario

* **S1.** The phone does not close the socket when Wi-Fi goes off, so the PC only finds out through a missing ack; the script forces that with `--ping` (5 s). Queued items sit in `status.json` (`queue`: kinds `text,file,text`, creation times in order, no text, no file name) and the tray says "3 queued". The PC sent text, offer, text in that order and the phone received them in that order. No `clipboard` message went out after the hello.
* **S6.** `NtSuspendProcess` on `Kin.exe` and `kill -STOP` on the phone's process through `run-as` freeze a receiver without ending the TCP connection. In 6a and 6c the frozen side processes the buffered frame after it is resumed and the repeat after the reconnect is answered by the de-duplication: one phone history row and one `text from` log line (6a), one PC history row and one toast (6c; the phone shows `attempts=2`, `done`).
* **S7a (rewritten).** Three files are uploaded; the phone's Wi-Fi goes off; `--cancel-transfers` on the PC cancels the upload (`canceled`/`user`, no `.part` left, the two finished files stay). The PC stores a `cancel` for the phone (changelog 52 (c)). When the phone is back the PC log shows "cancel message(s) are sent again after the hello" and "cancel message for <id>: acked", and the phone's row is `canceled` / `peer_canceled` (shown as "Canceled on <PC>"). If the phone's automatic resume reaches the PC first and gets its `404`, the row is `failed` / `not_found` for a moment and the cancel turns it into `canceled` (52 (e)); the script waits for that. Nothing was sent again: no new transfer on either side, no `r3` file, no `.part`. The files are 80 MB for the big one: on API 29 the 30 MB of the first version was fast enough to finish before the PC's cancel (a test race, not a product one: S7a failed once on API 29 for that reason and the file size was raised; see "Since the first run").
* **S7b (new): the 404 path without a cancel.** Under item 52 a cancel the PC *stores* always reaches the phone, so the old way of getting a 404 (cancel while away) no longer produces one. What still produces one is the PC forgetting the upload: an upload with no progress for 24 hours is dropped at start-up (`clean_up`, failed `expired`, its `.part` files removed), and no `cancel` is sent for it. The script uploads three files (two small, one 80 MB), takes the phone offline, quits `Kin.exe`, starts it with `--dev-clock-offset 90000` (25 hours: the sweep drops the upload; history says `failed`/`expired`), restarts it without the offset, and brings the phone back. The phone's automatic `POST` is answered `404` (PC log: "that ID was used before, answered 404"), the phone fails the transfer `failed` / `not_found` and sends nothing. `RETRY_TRANSFER` then makes a **new** transfer ID with **1 file** (the one without a `200`), which finishes (`done` on both sides); the old row stays `failed`; the PC folder has the three files once each, no `name (1)`, all byte-identical to the sources. This is the "404 → failed → user Retry" path of the done-when, kept end to end.
* **S8.** PC: `--dev-clipboard-mark exclude` then `--send-clipboard` → `result phone_offline`, `sensitive true`, `queued false`, no length; the toast says the text "is never queued, so it was not sent". Phone: `SEND_TEXT --ez sensitive true` answers `NotSent(...)`, the text history gets no row, a "Not sent: <PC> offline" notification is posted. Over an open connection a sensitive text is delivered and appears in no history.
* **S10 (changed).** Items are matched by ID: **16 of 16 have the same direction and status**, with no exception left. The S7a upload is `canceled` on both sides (`canceled/user` on the PC, `canceled/peer_canceled` on the phone); the S7b upload is `failed` on both (`failed/expired` on the PC, `failed/not_found` on the phone: the two sides give different reasons, by design, since the phone only learns of the PC's expiry through the `404`). The first run had one accepted mismatch (PC `canceled`, phone `failed`/`not_found`); item 52 (e) removed it. Nothing that is `done` on one side is missing on the other. No sensitive text and no automatic-clipboard text is in either history.
* **S11.** 60 queued texts all acked 7.7 s after Wi-Fi came back, and the phone shows **one** "60 texts from DESK-PC" notification (2 notifications of ours in all with the service). 15 single-file offers: 10 result notifications (the cap) in one group plus a summary, and the summary now says **"10 files from your PC"** (the first run showed 3 in a fast storm; the Android review fix 8 corrected it); 13 notifications of ours in all (Android's own limit is 50). See [11-transfer-storm.png](phase-1-4-integration-screens/11-transfer-storm.png) and [11-text-burst.png](phase-1-4-integration-screens/11-text-burst.png) (the shade, API 37).
* **S13.** No text content in the PC log, phone log, logcat or `status.json`; no token or key; **no file name in the PC log (0 of 30 names used) or in the phone's ordinary log lines (0 of 30)**. The only phone lines with names are the debug build's own command echoes (`debug: CMD TRANSFER ... names=`), which a release build does not have; the script leaves those out. PC export (`--dev-export-logs`): `README.txt`, `logs/kin.log`, `status.json`, `debug.json`; no IP address, PC or phone name, user name, file name, text, token or key. Phone export (Settings → Debug → Export logs): `state.txt`, `kin.log`; no file name, text, token or key. The "queue lines are in the logs" check needs S1 (PC) and S2 (phone) and is skipped, with a note, when `-Only` leaves either out.

## Since the first run

| What | Where |
|---|---|
| **The e2e parser.** The phone's `CMD TRANSFER` line now ends with `userRetry= started= owed= completeAcked=` (`TransferEngine.kt:484-486`); the regex anchored right after `retriedAs=` and would have matched nothing. It now allows trailing fields. The other debug-line parsers have no end anchor (`CMD TEXT`, `CMD RECEIVED`) or end in the free-text field by design (`CMD CLIPBOARD ... text="..."`) | `tools/e2e-1-3-scenarios.ps1:101` |
| **S7 split in two (a, b)**, S10 without an exception (see above) | `tools/e2e-1-4-scenarios.ps1` |
| **A failing scenario no longer aborts the run.** Each scenario is wrapped; a throw is recorded as a failed check, a frozen process is resumed, Wi-Fi is put back, `Kin.exe` is restarted if it was stopped, and the next scenario runs | `Scenario-Failed` in `tools/e2e-1-4-scenarios.ps1` |
| **S13's "queue lines" check is skipped** when S1 or S2 did not run | same file |
| **`e2e-1-2` S5** (the loop guard on the phone): the phone marks the PC's automatic clipboard as sensitive, and privacy fix 11 had silenced the `send skipped (CLIPBOARD)` line. **Ruling:** the line is logged again for a sensitive text, **without the fingerprint** (no length, no hash). `OutgoingSender.kt:120`; test `TextFlowTest.theEchoPathLogsTheSkipWithNoFingerprintForASensitiveTextAndAKeyedFingerprintForAnOrdinaryOne` (now also checks that the skip is logged and that nothing of the text is in it). The e2e assertion is unchanged |
| **A stale release build.** `cargo test --release` overwrites `target\release\Kin.exe` with a dev-server build; the e2e scripts and `check-1-4.ps1` now refuse an exe that is newer than the installer in `bundle\nsis` (`tauri build` writes the installer just after the exe); `-Build` rebuilds. Noted in `desktop/README.md` | `tools/e2e-1-1.ps1`, `desktop/check-1-4.ps1` |
| **A neutral PC name in every picture.** The e2e scripts run `Kin.exe` with `COMPUTERNAME=DESK-PC` (put back at the end), so a notification or pairing screenshot no longer shows the real computer name. `e2e-1-2` S8 reads the name from the same variable | `tools/e2e-1-1.ps1`, `tools/e2e-1-2-scenarios.ps1` |
| **Desktop success rate counts a retried send once**, like the phone; **export redaction** of phone names in any case, IPv6 addresses and a received folder outside `C:\Users` | see [phase-1-4-desktop.md](phase-1-4-desktop.md), "Changes made by the integration run" |
| **Screenshots with personal data replaced** (next section) | `docs/results/` |

Findings of the first run that the review fixes changed: the Android receiver acks a repeat of an offer from the same PC whatever the state of the download (item 52 (a)), so the first run's "An offer whose download is finished but whose `complete` was never acked is answered `bad_request`" no longer applies; the phone's backoff is reset when the network comes back (Android fix 9); the arrivals summary count is right (Android fix 8). S6b's 32 s is the frozen-phone case and is unchanged.

### Screenshots replaced

The committed pictures showed this person's Windows user name (`C:\Users\SOL 02\…`) and the PC name "EMPIRE". Every picture on both sides was looked at.

| Picture | Was | Now |
|---|---|---|
| `phase-1-4-desktop-screens/02-settings.png` | PC name `EMPIRE` (twice) and `C:\Users\SOL 02\AppData\Local\Temp\kin14-shots-…\received` | recaptured: `DESK-PC`, `C:\Kin-demo\received` |
| `phase-1-4-desktop-screens/04-drop.png` | the same received path | recaptured: `C:\Kin-demo\received` |
| `phase-1-4-desktop-screens/05-pairing.png` | `EMPIRE · 5ZVH-XBPY` | recaptured: `DESK-PC` |
| `phase-1-4-desktop-screens/01-history.png`, `03-debug.png` | nothing personal in the window, but the top rows showed pieces of the text of a window behind | the same pictures with the top 3 and bottom 2 pixel rows cut off (the new captures were cut 2 and 2). The debug window shows the Windows network name "Network 4" (a generic name, Public); it was kept |
| `phase-1-4-integration-screens/11-text-burst.png`, `11-transfer-storm.png` | "20 texts from EMPIRE", "Connected to EMPIRE", "File from EMPIRE …" | recaptured by the run: `DESK-PC` |
| `phase-1-2-integration-screens/01-confirm-pairing.png` | `EMPIRE` (tracked, committed before) | recaptured by the `e2e-1-2` run: `DESK-PC` |
| `phase-1-2-android-screens/10-api29-confirm-screen.png` | "This will replace EMPIRE." | the name pixelated (taken on a run that can't be repeated as it was) |
| `phase-1-4-android-screens/29-02-history.png` | "To / From EMPIRE" on eight rows | the name pixelated on those rows |
| `phase-1-4-android-screens/29-04-debug.png` | `EMPIRE` in the recent errors and the connection history | the text under "Recent errors" and "Connections" pixelated |

No picture shows an IP address (the emulator's `10.0.2.2` is not one), a user name or a path after this. The other pictures (`phase-1-2-android-screens`, `phase-1-3-android-screens`, the rest of `phase-1-4-android-screens`) show "Fake PC", the emulator's own names (`AndroidWifi`, `T-Mobile, 5G`, `sdk_gphone64_x86_64`) and nothing else. **Not done:** the text of the older result documents still says "EMPIRE" (e.g. `phase-1-1-integration.md`, `phase-1-2-integration.md`) and `phase-1-4-desktop.md` names `C:\Users\SOL02~1` in the 8.3 test; these are words, not pictures.

## Bugs fixed (first run, still true)

| # | Where | What | Test |
|---|---|---|---|
| 1 | `android/.../link/OutgoingSender.kt` (`send`, `sendSensitive`), `SendOutcome.NotSent` | A sensitive text was queued (stored while it waited), against ruling 1 and changelog 49. Now it goes out only over an open connection, never touches the store, and a missing connection or ack ends in `NotSent` and a "Not sent: <PC> offline" notification | `TextFlowTest`: `aSensitiveTextWithNoConnectionIsNotQueuedAndTheUserIsTold`, `aSensitiveTextThatGetsNoAckIsNotQueuedEither`. e2e S8 |
| 2 | `android/.../protocol/MessageReceiver.kt` (`onOffer`) and the link classes | An offer that reuses a remembered ID was acked again. Changelog 46 and ruling 3 say `bad_request` with `ref` = the offer ID, **except now the same PC repeating its offer (item 52)** | `MessageVectorsTest.sequences` runs the sequences of `messages.json`. e2e S6b |
| 3 | `OutboxFlusher.kt`, `TextOutbox.kt`, debug `DebugCommandReceiver.kt` | Not a bug: the `EXPIRE` hook could only look at the real clock; `--el ahead <ms>` judges the queue as if the clock were that far ahead (debug build only) | e2e S5 |

## Alignments between the two sides

| Ruling | Desktop | Android | Done |
|---|---|---|---|
| 1. Sensitive text never queued | yes (changelog 49) | yes (bug 1) | aligned |
| 2. "Sent" for an offer is the last attempt on an open connection; the 24 h runs from it | yes (`windows.md`, Queue) | not applicable | changelog 51 (b) |
| 3. Offer-ID reuse | `Dedup::offer_state`; item 52 (a)/(b) | as desktop after the review | aligned; vectors run on both sides |
| 4a. Expiry at exactly 7 days counts as expired | `queue.rs` `now >= expires` | `QueuePolicy.isExpired` `>=` | agree; changelog 51 (a) |
| 4c. User Retry makes a new row, the old stays failed with a pointer | `retry_of`, once | `retriedAs` | agree; changelog 51 (c). Seen in S7b |
| 4d. A text Retry uses a new message ID | yes | yes | agree |
| 5. `cancel` and `complete` are kept until acked and re-sent after every `hello`, before the queue | `pending_cancels` table, `flush_pending_cancels` | `OwedMessages` | agree; changelog 52 (c). Seen in S7a |
| 6. A PC `cancel` turns a phone upload that failed with `not_found` into `canceled` | n/a | `TransferStore` (`peer_canceled`) | changelog 52 (e). Seen in S7a, S10 |
| 7. A retried send counts once in the success rate | now yes (`sends_since`) | yes | aligned in this pass |

## Suites

| Suite | Result |
|---|---|
| `cargo test --no-fail-fast` (debug; **not** `--release`, which replaces `Kin.exe`) | **360 passed, 0 failed, 5 ignored** (356 before: +3 export, +1 success rate) |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `bun protocol/check-vectors.ts` | OK, **6,387 checks** |
| `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` | BUILD SUCCESSFUL: **444 tests, 0 failed, 1 skipped**, lint "No issues found". Release APK 3,782,631 bytes (3.61 MiB), debug 38,850,724 |
| `desktop/check-1-4.ps1` (real `Kin.exe` + fake phone) | **64 checks, 0 failed** |
| `tools/e2e-1-4.ps1` | **179** checks (API 37, `-BurstTexts 60 -BurstFiles 15`), **178** (API 29, defaults), 0 failed |
| `tools/e2e-1-1.ps1`, `e2e-1-2.ps1` (×2, the first with `-UpdateScreens`), `e2e-1-3.ps1` (API 37) | 16, 78 and 78, 142 checks, 0 failed |

Order of the runs: the first full `e2e-1-4` pass on API 37 (179) and API 29 passed except S7a on API 29 (the 30 MB upload finished before the cancel; the file was made 80 MB and the cancel's wait 1 s); both were then run again with the final scripts and passed, which are the numbers above.

## Script usage

```
tools\e2e-1-4.ps1                       # kin_api37
tools\e2e-1-4.ps1 -Avd kin_api29
tools\e2e-1-4.ps1 -Build                # tauri build Kin.exe and the debug APK first (needed after cargo test --release)
tools\e2e-1-4.ps1 -Only S1,S8           # some scenarios (the pairing still happens); S10 and S13 need what the others made
tools\e2e-1-4.ps1 -BurstTexts 60 -BurstFiles 15   # the full notification check (default 20 and 6)
tools\e2e-1-4.ps1 -UpdateScreens        # write the S11 pictures to docs\results (a run never overwrites a committed picture without it)
tools\e2e-1-4.ps1 -Only SX              # a probe: share while Kin.exe is stopped, start it at once
```

It is `tools\e2e-1-1.ps1 -Milestone 1.4` with the scenarios of `tools\e2e-1-4-scenarios.ps1`; exit code 1 if any check fails. A full run takes about 13 minutes (the 60/15 burst adds a minute). It uses the real PC clipboard (every format saved first, put back after Kin has quit), a throwaway data dir and received folder, and never touches autostart, SendTo or Downloads. `Kin.exe` runs with `COMPUTERNAME=DESK-PC`. A frozen process is always resumed (in a `finally` and after a failed scenario). To capture its output from a tool that reads a pipe, use `Start-Process -RedirectStandardOutput` as for 1.3 (the adb server it starts holds the pipe; `-File` paths with a space need their own quotes inside `-ArgumentList`).

## Open issues

* **Phone reconnect backoff after repeated "no ack" closes** reaches 30 s (S6b: 32 s to deliver). Fine for the dead-socket case it is; the Wi-Fi-back path (about 5 s) is what the done-when needs.
* The two sides give **different reasons** for an upload the PC forgot (`expired` on the PC, `not_found` on the phone: S7b/S10). Both say "failed".
* The debug window shows the Windows network's name ("Network 4") on screen (not in the export).
* **S7a depends on the emulator's speed** (the cancel has to land while the big file is still being sent); 80 MB is enough for both API levels measured here (15 MB/s on API 29), a faster machine would need more.
* Not run: a real phone, a real OEM phone, the idle memory after one hour, the tray menu click, the real `Run` key (all as in the 1.4 documents); `e2e-1-1/2/3` on API 29; the PC → phone queue over a real home network (mDNS, a real access point); **IPv6 in a real export** (the unit tests cover it, the loopback-only developer mode never produces an IPv6 address).
* The debug build's Export logs includes the `debug: CMD` echo lines (names of test files); a release build has none of them (the dex has no debug receiver).
