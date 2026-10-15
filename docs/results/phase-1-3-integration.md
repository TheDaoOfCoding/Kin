# Phase 1.3 result: integration (real Windows app + real Android app, files in both directions)

Run 2026-10-15 by the integration agent. The two halves of milestone 1.3 were built independently from [protocol.md](../protocol.md), by [the desktop agent](phase-1-3-desktop.md) and [the Android agent](phase-1-3-android.md). This is the first time they moved files to each other. Nothing is committed.

**Setup:** the real release `Kin.exe` (rebuilt twice here, see "Bugs fixed") in developer mode (`--dev-bind 127.0.0.1 --dev-qr-address 10.0.2.2`, loopback only, a throwaway data dir, **a throwaway `--dev-received-dir`**, never `%APPDATA%\Kin` or the real Downloads), and the real debug APK, on `kin_api37` (Android 17) and `kin_api29` (Android 10). The PC is reached as `10.0.2.2:47821`. The real Windows clipboard was used and **saved and restored in every format** by every run (the user's clipboard held HTML and Chromium's own formats: 7 formats, put back each time).

**Result in one line:** the two apps work together. **No case was found where they disagree about the protocol.** One real Android bug (a file name clash that MediaStore doesn't resolve failed the whole download) was fixed; the rest of the findings are in the test tooling (the 1.2 script was broken by a debug-line change, and an emulator quirk after boot) or are new automation hooks. `tools/e2e-1-3.ps1` automates S1 to S10 and S12 and passes with **142 checks on API 37 and 141 on API 29**; `e2e-1-1` (16) and `e2e-1-2` (78) still pass; every suite is green.

## Scenarios

"Emulator throughput" is the emulator's user-mode NAT and says nothing about real Wi-Fi (about 2 to 6 MB/s here). `kin_api37` and `kin_api29` gave the same results for every scenario (the one difference is in S10a).

| # | Scenario | Result | Evidence |
|---|---|---|---|
| 1 | Phone → PC, one photo | **PASS** | Shared through `DebugShareActivity` (the real `ShareActivity`, a `content://` URI with a read grant): 1600 × 1200 JPEG, in the received folder in 2.4 s, **SHA-256 equals the phone's `sha256sum`**, no `.part`, toast "1 file from …" logged, the file finished exactly once on the PC. The PC clipboard then has `CF_DIBV5`, `CF_HDROP` (and the `CF_DIB`, `CF_BITMAP` Windows derives): `Clipboard.GetImage()` is **1600 × 1200** and the file list is the received file. The clipboard of the user was restored afterwards in all 7 formats |
| 2 | Phone → PC, 50 photos | **PASS** | 50 JPEGs (13.9 MB), one transfer, 4.3 s: all 50 hashes match, names unchanged, no `.part` left, one toast "50 files from …", the phone says `done` with 50 files. (Several images: the PC leaves the clipboard alone, as the desktop agent decided) |
| 3 | Phone → PC, big file, Wi-Fi off and on | **PASS** | Scripted with 40 MB; also run by hand with **300 MB** (`-BigMb 300`). Wi-Fi off at a quarter: the PC kept 83,886,080 bytes, the phone logged the drop; Wi-Fi on: the transfer finished, the PC log says **"file 1 continues from byte 84,942,848 of 314,572,800"** (a little above what the status showed: the old request was still writing when the phone came back, and the newer request took its place, which is the desktop's decision 9) and the **hash matches**, no `.part` |
| 4 | PC → phone, one photo | **PASS** | `Kin.exe --send photo.jpg`: lands in MediaStore Images, `Pictures/Kin/`, **`IS_PENDING` cleared**, MediaStore's size equals the file's, **hash on the phone equals the PC's**, the PC received `complete` (transfer `done`, log "the phone has all the files of offer"), the phone showed "Photo from <PC>" |
| 5 | PC → phone, mixed batch | **PASS** | jpg → `Pictures/Kin/` (images), mp4 → `Movies/Kin/` (video), pdf and zip → `Download/Kin/` (downloads), all `pending=0`, all four hashes equal, "4 files from <PC>" notification, the PC got `complete` |
| 6 | PC → phone, big file, Wi-Fi off and on | **PASS** | 40 MB scripted, 300 MB by hand. After the drop the PC log says **"file 1 is served from byte 80,117,760 of 314,572,800"** (a `206`; the PC had handed over 81,788,928, the phone had saved less), hash on the phone equals the PC's, not pending, size right |
| 7 | Cancel from each side, both directions | **PASS** | 7a phone → PC cancelled on the phone (`DELETE`), 7b phone → PC cancelled on the PC (`--cancel-transfers`, new, see below), 7c PC → phone cancelled on the PC, 7d PC → phone cancelled on the phone (`cancel` message, reason `user`): in each, both sides say `canceled` (not `failed`; the phone's error text is `user` or `peer_canceled`), **no `.part` and no partial file on the PC, no pending entry left in MediaStore, the progress notification is gone**, and nothing is still active on the PC. (The PC's drop window was not driven: the script runs `--dev-no-windows`; the desktop agent pressed its Cancel by hand) |
| 8 | Errors | **PASS** except one case not run | `disk_full`: Kin restarted with `--dev-free-bytes 1000000`: a 3 MB share is `failed` / `disk_full` on the phone with a "doesn't have enough free space" notification, **no retry** (20 s later still one failed transfer, one refusal on the PC, no file created), and a 900 KB file is then taken and arrives intact. `source_changed`: a 40 MB offer is cut by Wi-Fi off, one byte is appended to the source, Wi-Fi on: the resumed `GET` is `409`, the phone **fails the transfer as `source_changed`, deletes its partial file, shows "changed on <PC>" and sends `cancel` reason `error`**, the PC records `canceled` / `error`. **Expired offer: not run**, no hook shortens the 24 hours (the desktop's `offers_e2e` covers it) |
| 9 | File names, both ways | **PASS** | Phone → PC (11 files in one transfer, then collisions in three more): `CON.txt` → `_CON.txt`, `rtl`+U+202E+`gpj.exe` → `rtl_gpj.exe`, an 83-character CJK name (249 bytes) → 65 characters + `.txt` (199 bytes), `tail.txt.`+space → `tail.txt`, `dir\evil.txt` → `evil.txt`, `we<ir>d"\|.txt` → `we_ir_d__.txt`, `q:a?b*.txt` → `q_a_b_.txt`, `Com1.tar.gz` → `_Com1.tar.gz`, `lpt¹.txt` → `_lpt¹.txt`, `.hidden` stays, a **0-byte file** arrives (a `POST`, then a `PUT` with `Content-Length: 0`); `dup.txt`, `dup.txt`, `DUP.TXT` sent separately → `dup.txt`, `dup (1).txt`, `DUP (2).TXT`. Contents equal for 10 files. PC → phone (files created on NTFS with the `\\?\` prefix, which is the only way to make `CON.txt`): `CON.txt` → `_CON.txt`, the bidi name → `rtl_fdp.txt`, a **100-character (300-byte) CJK name** → 65 characters + `.txt`, `tail2.txt.` → `tail2.txt`, `Com1.tar.gz` → `_Com1.tar.gz`, a name with a space, a 0-byte file (a plain `GET`, no `Range`), and `dup2.txt` sent twice → `dup2.txt`, `dup2 (1).txt`. All names are exactly what the `file-names.json` rules give, on both sides; hashes equal |
| 10 | Text spec changes | **PASS** (one API 29 limit) | **a. sensitive PC → phone:** `--dev-set-clipboard … --dev-clipboard-mark exclude` then the hotkey: sent with `sensitive`, acked, **no length in `status.json`**, the phone notification is "Text from <PC>" with no preview and no Open, the phone clipboard has the text and (API 33+) **is marked sensitive** (`sensitive=true` read by the debug activity; Android 10 has no such flag), the phone log has one line "sensitive text from … (its content is not logged)" and no length or hash. **b. sensitive phone → PC:** `SEND_TEXT --ez sensitive true`: arrives with `sensitive: true` and `chars` null, on the PC clipboard with **`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` and `CanUploadToCloudClipboard`**, a toast without a preview. **c. 200 KiB:** PC → phone, 204,600 bytes of CRLF text from the hotkey arrives as `Text.txt` in `Download/Kin`, `text/plain`, **byte for byte the LF form** (hash and size); phone → PC, a 198,000-byte text arrives as `Kin text <date>.txt`, byte for byte the LF form, and **is not put on the PC clipboard**. **d. LF/CRLF:** phone `a\r\nb\r\nc`, `a\nb\nc` and `p\r\r\nq` all reach the Windows clipboard as CRLF (`p\r\nq` for the last: the spec's one-pass rule); from the PC `x\r\ny\r\nz` goes as `x\ny\nz`, `p\r\r\nq` as `p\r\nq`, a lone CR stays; the **round trip** LF on the phone → CRLF on the PC → LF on the phone is `a\nb\nc` again. **e. openable URLs**, both ways: `http://example.com:8080/…` has Open; `http://.`, `http://example.com:/x`, `:0`, `:65536`, `https://[]/x` and an 8,219-character URL have **no Open** (no link toast on the PC, no Open action on the phone, `isUrl` false) |
| 11 | Regressions | **PASS** | `tools\e2e-1-1.ps1`: 16 checks. `tools\e2e-1-2.ps1`: 78 checks (79 before, see "Test tooling") |
| 12 | Logs | **PASS**, names noted | The PC log, the phone log, logcat and `status.json` were searched for every text used (the sensitive ones included, and three words of each), for the pairing token, `tk=` and `PRIVATE KEY`: **no match**. File names: **the PC log has none except** the toast line "Sending <name> to <phone>" of a one-file offer (the log repeats every toast text) and "<name> can't be read". The **phone log has all 8 names the script looked for**, because the debug-only `CMD MAKE_FILES` and `CMD TRANSFER … names=` result lines are written to the log file; in a release build the only names in the log are in the failure messages ("Kin can't read <name> any more") and the line "the system saved a file as '<name>'". Contents of files are in no log |

## Throughput on the emulator (not representative)

| Run | Result |
|---|---|
| Phone → PC, 1 photo (1.2 MB), share to done | 2.4 s |
| Phone → PC, 50 photos, 13.9 MB, one transfer | 4.3 s, 3.2 MB/s (2.9 s and 4.7 MB/s on API 29) |
| Phone → PC, 40 MB with a Wi-Fi drop (about 10 s of it is the drop) | 18.6 s overall, 2.2 MB/s (API 29: 13.7 s) |
| PC → phone, 40 MB with a Wi-Fi drop | 14.8 s overall, 2.7 MB/s (API 29: 12.9 s) |
| Phone → PC, **300 MB** with a drop | 71 s overall, 4.2 MB/s |
| PC → phone, **300 MB** with a drop | 46 s overall, 6.6 MB/s |

**Not measurable here, needs a real phone:** the **2 GB video** both ways, the **speed against LocalSend** on the same network, and the 50-photo batch with real camera photos (3 to 8 MB each; the emulator's 50 were 280 KB). The PC side of the 2 GB case was already measured by the desktop agent with the fake phone: **about 1 GB/s each way on loopback and 1.10 GB resumed after a cut at 1,000 MiB** ([phase-1-3-desktop.md](phase-1-3-desktop.md#throughput)). So the PC is not the bottleneck; the radio and the phone's storage are.

## Bugs fixed

| # | Where | What | Fix |
|---|---|---|---|
| 1 | `android/…/transfer/MediaStoreSink.kt:69` (`create`), `TransferPorts.kt:54` (`NameTakenException`), `DownloadRunner.kt:274` | **A file the app can't see makes the download fail.** After a reinstall (or with another app's file) `Download/Kin/Text.txt` can exist where the new install can't list it. MediaStore's unique index on the path then refuses the insert with a `SQLiteConstraintException` instead of renaming. That is not an `IOException`, so it escaped the name loop, the whole transfer **crashed** (`internal`, log "crashed: SQLiteConstraintException: UNIQUE constraint failed: files._data") and the phone sent `cancel` `error` for a file the PC had served correctly. Found when a run that died left `Text.txt` behind. The spec says "already exists" is a collision | `create` turns the exception into `NameTakenException`, the runner treats it as `AlreadyExists` and tries `Text (1).txt`. New test `DownloadFlowTest.aNameTakenByAFileTheAppCantSeeTriesTheNextSuffixInsteadOfFailingTheTransfer` (253 tests now). Spec: one sentence under "Android and scoped storage" and changelog item 39 |
| 2 | `tools/e2e-1-2-scenarios.ps1` (`Phone-Clip`) | The Android agent put `sensitive=<bool>` into the debug `CMD CLIPBOARD` line (between the hash and the text), so the 1.2 script's regular expression matched nothing and **every clipboard read of `tools\e2e-1-2.ps1` failed** | The expression accepts the new field and returns it (`.Sensitive`) |
| 3 | `tools/e2e-1-2-scenarios.ps1` (scenario 8) | The 1.2 "large text" scenario tested the old rules (a 400,000-character `clipboard`, a 1 MiB `text`), which the 128 KiB threshold of 1.3 replaced | Rewritten for the new rules: 128 KiB exactly is a message, 128 KiB + 1 is `too_large` for the automatic clipboard and a `.txt` file offer for the hotkey. 79 checks became 78 |
| 4 | `tools/e2e-1-2-scenarios.ps1` (`Save-PcClipboard`, `Restore-PcClipboard`) | The 1.2 script **refused to run with anything but plain text on the clipboard** (the user's held HTML), and `-DiscardClipboard` would lose it | Both milestones now use `desktop/clipboard-backup.ps1` (every format saved and put back). `-DiscardClipboard` is kept and does nothing |
| 5 | `tools/e2e-1-1.ps1` (pairing) | Right after an emulator boot the first connection to `10.0.2.2` now and then fails ("Couldn't reach the PC") although the PC never saw it (seen once in about 15 boots; a second failed start was bug 6). The run then fails every check | The same link is opened again, up to three times; the code is still valid |
| 6 | `tools/e2e-1-1.ps1` (after the boot) | A run that died while Wi-Fi was off left the emulator snapshot with Wi-Fi off, and the next run could not pair | `svc wifi enable` after every boot; the 1.3 clean-up also turns it on |
| 7 | desktop: `cli.rs:92,109,174`, `app.rs:443`, `outgoing.rs:285` | **Not a bug, a missing hook:** the PC's cancel could only be pressed in the drop window, which a script can't drive. New command `Kin.exe --cancel-transfers` = the Cancel button for every active transfer (test `cancel_transfers_is_a_command`; README row) | Added |
| 8 | desktop: `transfers.rs:709`, `fileserver.rs:267` | **Not a bug, missing evidence:** nothing in the PC log said where a resumed transfer carried on, so "resumes from the right offset" could be proved only by the hash. Two log lines, no names or contents: `upload <id>: file 1 continues from byte N of M` and `offer <id>: file 1 is served from byte N of M` | Added; the script checks them |
| 9 | android debug only: `DebugFiles.kt` (`fileName`), `DebugCommandReceiver.kt` (`textfile`) | The scenarios need names with bidi controls, CJK and trailing dots, and 200 KiB texts, which adb can't carry in an argument on Windows | `%uXXXX;` in names; `SEND_TEXT` / `SET_CLIPBOARD --es textfile <name>`. Release builds are untouched (`src/debug`). README |

The first bug is the only one in product code. The release APK is still 3,601,995 bytes (the `NameTakenException` is in it; nothing else changed there), and `classes.dex` has none of the debug hooks.

## Cross-side questions and decisions

**The Android agent's three questions**

1. **A reused offer ID is acked, then ignored for 24 h.** This is what the spec says (De-duplication: a remembered ID gets the first response again). The PC makes a fresh UUIDv7 for every offer and in 1.3 never re-sends an offer, so the two never meet. It matters for 1.4, when the queue re-sends an unacked offer with the same ID: the phone will then repeat its ack, which is right. **No change.**
2. **Does the PC finalise a file it already holds in full at `POST`?** Yes. A resumed `POST` finalises a file that is complete but never got its final name (`transfers.rs:529`), answers `offset = size` for it, and an **empty `PUT` at `N = size` answers `200 {received}`** with no write (the stored count is the size, `Content-Length` 0 equals size minus offset, the file is already finalised). The spec already says both ("equal to `size` when the file is already complete" and "an empty body at N = size … gets this answer"), so the phone sending an empty `PUT` for every complete file is correct and **no spec text was needed**. Seen end to end: a 0-byte file (S9, both ways). The "complete but the `200` was lost" case can't be provoked without a hook; it is covered by the `upload-lost-final-response` sequence on both sides.
3. **Sequences that start with a `PUT` and no `POST`.** The phone never does that (it always `POST`s first, also after a drop), and the PC answers `404` for an unknown transfer, which is what those sequences check on the desktop side. **No change.**

**The desktop agent's 18 decisions.** Checked against the phone's behaviour; all compatible, none had to change.

- *#7 an error to a `PUT` may close the connection:* the phone treats any `IOException` on a `PUT` as a dropped connection and `POST`s again (`UploadRunner.kt:126`), and the `POST` repeats the `413` and `507` checks, so the outcome is the same. Not provoked end to end (a `507` on a `PUT` can't happen when the `POST` passed with the same fake free space); covered by `UploadFlowTest`.
- *#9 one writer per file, newest wins:* **exercised live** by S3 (the resumed request took over from the old one and carried on at the PC's byte count; hash equal).
- *#6, #10 0-byte files and finished transfers kept for 24 h:* the 0-byte file works both ways (S9).
- *#8 limits (1 TiB, 10,000 files, 800 per offer):* compatible: the phone limits itself to 1,000 files per upload and has no limit on an offer. Not run live.
- *#1 a sensitive text over 128 KiB is never sent:* both sides implement it and the vectors agree (`senderSize[]`); not run live (it needs a 128 KiB secret).
- *#14 `Text.txt` against `Kin text <date>.txt`:* both are "a name ending in `.txt`"; the receiver cleans either. Different but harmless; **no change**.
- *#3, #4 one image to the clipboard, toasts never run a program:* S1 (one image: on the clipboard) and S2 (50 images: left alone).
- *#17 `status.json` can't always be replaced:* the script reads it with a retry (an unreadable read is a `$null` and is tried again).

**Where the spec was silent and the two sides disagreed:** none found. The decisions I made are tooling and one clarification, all listed above for the reviewer: (1) a clash MediaStore can't resolve is "already exists" (bug 1, spec item 39), (2) `Kin.exe --cancel-transfers`, (3) the two resume log lines, (4) the debug-only name and text-file hooks on the phone.

## Spec text changes

Only `docs/protocol.md`: one sentence in "Android and scoped storage" and changelog **item 39** (bug 1). `bun protocol/check-vectors.ts` still passes (no vector changed).

## Suites after the changes

| Suite | Result |
|---|---|
| `cargo test` (`desktop/src-tauri`) | **239 passed** (111 unit, 50 fake phone, 22 `transfers_e2e`, 13 `vectors`, 13 `offers_e2e`, 12 `tls_vectors`, 9 `transfer_vectors`, 9 `server_limits`), 0 failed, 5 ignored (the 2 GB runs, the real-clipboard test). One new test (`cancel_transfers_is_a_command`) |
| `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` | clean |
| `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` | BUILD SUCCESSFUL, **253 tests, 0 failed, 1 skipped** (the opt-in fake PC; 252 before, plus the new name-clash test), lint passes, release APK **3,601,995 bytes** |
| `bun protocol/check-vectors.ts` | OK, **6,084 checks** |
| `tools\e2e-1-1.ps1` | OK, 16 checks (`kin_api37`) |
| `tools\e2e-1-2.ps1` | OK, 78 checks (`kin_api37`) |
| `tools\e2e-1-3.ps1` | OK, **142 checks** (`kin_api37`), **141 checks** (`kin_api29`; the one fewer is the Android 13+ sensitive-clipboard check); about 3.5 minutes with a 40 MB big file. Also run with `-BigMb 300` for S3 and S6 (30 checks) |

## `tools/e2e-1-3.ps1`

```
powershell -ExecutionPolicy Bypass -File tools\e2e-1-3.ps1                  # kin_api37
powershell -ExecutionPolicy Bypass -File tools\e2e-1-3.ps1 -Avd kin_api29
powershell -ExecutionPolicy Bypass -File tools\e2e-1-3.ps1 -Build          # builds Kin.exe and the debug APK first
powershell -ExecutionPolicy Bypass -File tools\e2e-1-3.ps1 -BigMb 300      # a bigger "big file" for S3, S6 and S8 (default 40)
powershell -ExecutionPolicy Bypass -File tools\e2e-1-3.ps1 -Only S3,S9     # only these scenarios; the pairing still happens
powershell -ExecutionPolicy Bypass -File tools\e2e-1-3.ps1 -KeepRunning    # leaves the emulator and Kin up
```

It is `tools\e2e-1-1.ps1 -Milestone 1.3` (setup, pairing, teardown) plus `tools/e2e-1-3-scenarios.ps1`: **S1** one photo and **S2** 50 photos phone → PC, **S3** a big file with a Wi-Fi drop, **S4** one photo, **S5** a mixed batch and **S6** a big file with a drop PC → phone, **S7** cancel (four combinations), **S8** `disk_full` and `source_changed`, **S9** names both ways, **S10** the text rules, **S12** the logs. It checks hashes with `sha256sum` on the phone and .NET on the PC, and exits 1 on any failed check.

- Received files go to `<data dir>\received`; it never touches the real Downloads. PC-side source files are in `%LOCALAPPDATA%\Temp\kin-e2e13-*` and are removed at the end (with the `\\?\` prefix, since `CON.txt` can't be deleted any other way).
- **Clipboard:** every format is saved first and put back after Kin has quit (checked as "PC clipboard restored (every format)").
- **Phone clean-up:** `DELETE_RECEIVED`, `files/debug-share` and the transfer cache are removed before the emulator stops, and before an install it also deletes what an earlier install left in `Pictures/Kin`, `Movies/Kin` and `Download/Kin` (bug 1).
- Wi-Fi is switched back on in the clean-up; a notification reset (`am force-stop`, then a foreground start) runs before S10, because Android allows 50 notifications per app (open issue 2).
- **Capturing the output:** the adb server the script starts keeps the output pipe of a calling tool open until it is killed. Send the output to a file (`Start-Process -RedirectStandardOutput`) and wait for the process, not for the pipe. Without `-KeepRunning` the script kills the adb server itself.

## Open issues

1. **Not run, needs a real phone or more time:** 2 GB both ways on a phone, the LocalSend speed comparison, real camera photos, OEM MediaStore and gallery apps, mDNS, Windows 10, the drop window's Cancel and the toasts' buttons in this integration (the script uses `--dev-no-windows`; the desktop agent drove both by hand), an **expired offer** (no hook), a **`507` on a `PUT`** (needs a disk that fills between `POST` and `PUT`), a sensitive text over 128 KiB (vectors only).
2. **Android's limit of 50 notifications per app** is probably real for users. Every text from the PC and every finished transfer posts a notification with its own tag. On API 29, late in a long run, a text from the PC showed **no notification** (2 failures in 2 long runs, never on API 37; the script now force-stops the app before S10 and then it doesn't happen). I did not prove the cause (a count of 50 un-dismissed ones is the usual one). For 1.4: replace or group them, or cancel the old ones.
3. **The first connection after an emulator boot can fail** (bug 5). It was seen once, on the emulator right after boot, and a retry works, but the first-run pairing of a real phone has the same shape (one connection attempt, one error message). Worth a look at whether the phone should retry once before saying "Couldn't reach the PC".
4. **The PC says `phone_connected` for a while after the phone's process is killed** (1.1 open issue 1, seen again: `am force-stop` leaves it for several seconds). A send right then gets no ack and says `phone_offline`.
5. **File names in logs:** the PC log has a name in the "Sending <name>" toast line, the phone log has names in failure messages and in the debug result lines. Decide in 1.4 (the history screen) whether the log line should drop them; the desktop README no longer says "never".
6. **The 1.2 script lost one check** (79 → 78) because its large-text scenario was rewritten for the 128 KiB rule; the 1 MiB receivers' limit has no sender any more, so there is nothing to send above 128 KiB as a message from either app (only the unit tests and vectors reach the 1 MiB limit).
7. **`MediaStore`'s `SIZE` column** read 0 for a few seconds on an entry whose pending flag had just been cleared (seen on a `.txt` while an orphan file was in the way); the file itself and its hash were right. The script waits up to 20 s. It is the same effect the Android agent saw for a fake `.mp4`.
8. **Unit-test temp folders:** `%TEMP%\kin-identity*` (hundreds of them) are left by `IdentityStoreTest` of the Android agent's suite. Not from these scripts; mentioned because they are in the same temp folder.
