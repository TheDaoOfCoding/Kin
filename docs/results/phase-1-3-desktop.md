# Phase 1.3 result: the Windows app (files in both directions)

Built and measured 2026-10-15 on Windows 11 Pro 10.0.26300, Ryzen 9 7900X, 31 GB RAM (the PC of [phase-1-1-desktop.md](phase-1-1-desktop.md)). Code is in `desktop/` (see [desktop/README.md](../../desktop/README.md)), the spec and vectors in `docs/protocol.md` and `protocol/`. This is the Windows half of milestone 1.3 ([phase-1-mvp.md](../phases/phase-1-mvp.md)); the Android half was built in parallel from the same spec and passes the same vectors ([phase-1-3-android.md](phase-1-3-android.md)). Nothing here is committed. **Reviewed the same day: the findings and their fixes are in [1.3 review fixes](#13-review-fixes) below, and the numbers in the rest of this file are from before them** (238 tests and 6,084 vector checks then; 269 and 6,196 now).

**Summary.** Files go both ways. The phone uploads to the PC (`POST /v1/transfers`, `PUT`, `DELETE`) and the PC offers files and the phone downloads them (`offer`, `GET` with `Range`, `complete`, `cancel`). `cargo test` passes **238 tests** (5 ignored on purpose), `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean, `bun protocol/check-vectors.ts` passes **6,084 checks**. The real `Kin.exe` was run against a fake phone by two scripts (**38 checks** and a **real drag and drop** onto the drop window), with the real clipboard saved and restored byte for byte. Loopback throughput for a 2 GB file is **about 1 GB/s in both directions**, with the app's memory at 24 MB while it received 6 GB. Idle memory is unchanged (2.8 MB private working set). Toast buttons **do** work for the unpackaged app (R10).

## 1.3 review fixes

The Opus review of this milestone blocked it on two findings and listed more. Everything below was fixed on 2026-10-15 in `desktop/`, `protocol/`, `docs/protocol.md` and `docs/windows.md`; the Android half was fixed by another agent. Still **nothing is committed**. After the fixes: `cargo test` passes **269 tests** (5 ignored on purpose; 239 before, 30 new), `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean, `bun protocol/check-vectors.ts` passes **6,196 checks** (6,084 before), `desktop/check-1-3.ps1` passes **39 checks** on the real `Kin.exe` (38 before, plus the Mark of the Web), and `tools/e2e-1-3.ps1` passes **142 checks on kin_api37** (the emulator with the real debug APK, `kin_api37`, about 4.5 minutes, the APK being the one the Android agent had built that morning, copied first so it could not change under the run; the real clipboard saved and restored in all formats).

### 1. Blocking: a rotated photo no longer hangs the transfer

**Before.** The reviewer measured a 12 MP JPEG with EXIF orientation 6 at over 400 s to reach the clipboard (upright: 35 ms). I re-measured with the unfixed `Kin.exe` (kept aside, run through `desktop/check-rotation.ps1`, a 4000 x 3000 JPEG with orientation 6, the fake phone): **the picture was not on the clipboard after 700 s** (the script gave up then), and the phone **never got its `200`**: the decode ran inside the `PUT` handler, the connection was closed by the PC's own 30 s idle rule while it ran, and the fake phone reported `peer closed connection without sending TLS close_notify`.

**After** (release build, the same script, a noisy 4000 x 3000 JPEG of 2.6 MB, the real clipboard saved and restored in all 7 formats every run; the phone waits 1 s after the `hello` by design, so "PUT answered" includes that second):

| EXIF orientation | `PUT` answered | picture on the real clipboard | picture | four corners (top left, top right, bottom right, bottom left) |
|---|---|---|---|---|
| 6 (a quarter turn clockwise) | 1.62 s (first run after start-up; 1.20 s in the other runs) | +0.08 s (1.70 s) | 3000 x 4000 | blue, red, green, yellow: **upright** |
| 3 (half a turn) | 1.22 s | +0.03 s (1.25 s) | 4000 x 3000 | yellow, blue, red, green: **upright** |
| 8 (a quarter turn back) | 1.20 s | +0.08 s (1.28 s) | 3000 x 4000 | green, yellow, blue, red: **upright** |
| 1 (no rotation, for comparison) | 1.21 s | +0.04 s (1.25 s) | 4000 x 3000 | red, green, yellow, blue |

In the unit test (`images.rs`, a 12 MP JPEG made with WIC's own encoder, decode only): **orientation 6 takes 45 ms, 3 takes 25 ms, 8 takes 45 ms, upright 30 ms in a release build** (30, 140, 134 and 141 ms in a debug build), against more than 400 s. The limit in the test is 2 s (release) or 30 s (debug).

| What | Where |
|---|---|
| The two WIC flip-rotators are gone. The picture is read **row by row, in order** (bands of about 4 MiB, the one access pattern a JPEG decoder is fast at; the WIC rotator asks for a scanline per pixel), and every pixel is written straight to its place in the **bottom-up** DIB by our own code: no rotator, no vertical flip, no second copy of the picture. All eight orientations as a pure function, tested for every pixel of every orientation | `images.rs:91` (`upright_position`), `images.rs:107` (`place_row`), `images.rs:271` (`decode`) |
| `images.rs` read uninitialised memory (a slice over `GlobalAlloc` memory that WIC then filled). The block is now allocated with `GMEM_ZEROINIT`, and nothing but our own row loop writes to it | `images.rs:181` (`Global::alloc`) |
| **All the arrival work moved off the `PUT` path.** The toast and the image to the clipboard run in a task of their own (`tokio::spawn`) after the transfer is marked done; the `PUT` answers `200` as soon as the file has its final name. Test: a clipboard that takes 2 s per image, and the `PUT` is answered in well under 1.5 s with the toast first and the image after | `transfers.rs:1156` (`upload_done`), `arrival.rs:50` (`files_received`), test `the_put_is_answered_before_the_toast_and_the_image_are_made` |
| **The whole `PUT` also runs as a task that outlives the request.** Hyper drops the handler when the phone's connection breaks, at whatever it is waiting for; before this fix that threw away the bytes the PC had received but not yet written (found by a flaky test, see the notes below). Now the task reads what arrived, writes it and records it, exactly as if the body had ended. A writer that a newer request has told to stop first takes what is arriving (30 ms a frame, 150 ms in all) | `transfers.rs:853` (`upload_put`), `transfers.rs:870` (`upload_put_task`), `transfers.rs:59` (`STOP_GRACE`); test `bytes_that_arrived_are_kept_when_the_request_is_dropped_before_the_body_ends` (fails without the detaching, checked) |
| The tests the review asked for: a generated 12 MP JPEG with orientation 6, 3 and 8 decoded within the limit, upright (corner colours), with a real file | `images.rs` `a_12_megapixel_photo_with_exif_rotation_is_decoded_upright_within_the_time_limit` |

### 2. Blocking: "a click on a toast never runs a program" is now true

**The deny-list is gone; there is an allow-list** (`links.rs:104` `OPENABLE_EXTENSIONS`, `links.rs:115` `may_open_file`, used at `links.rs:154` (`ShellOpener::open_path` refuses anything else), `sync.rs:624` (a click on anything else shows the file in its folder, selected), `arrival.rs:72` and `app.rs:290` (a file that isn't openable gets a toast with only **Show in folder**, a new action type `received_file_show_only`)). A single received file is opened by a click or by **Open** only if its **last** extension, compared ASCII case-insensitively, is one of these 32:

| Kind | Extensions |
|---|---|
| Pictures (9) | `jpg` `jpeg` `png` `gif` `webp` `heic` `heif` `bmp` `tiff` |
| Audio and video (13) | `mp3` `m4a` `aac` `flac` `wav` `ogg` `opus` `mp4` `mov` `mkv` `webm` `avi` `3gp` |
| Documents (10) | `pdf` `txt` `csv` `md` `docx` `xlsx` `pptx` `odt` `ods` `odp` |

Everything else (and a name with no extension, or with a stream `:`) gets **Show in folder**. Only the last extension counts: `x.pdf.exe` is not openable, `x.exe.pdf` is a PDF; trailing dots and spaces are ignored, as Windows ignores them. The unit tests cover every allowed type, the old deny-list, and the ones it missed: `.sh`, `.pyz`, `.py`, `.pyw`, `.pyc`, `.wsb`, `.theme`, `.ahk`, `.appinstaller`, `.website`, `.vsto`, `.jnlp`, `.bash`, `.zsh`, `.pl`, `.rb`, `.php`, the macro Office types, `.svg`, `.html`, `.zip`, `.apk`, double extensions, streams, non-ASCII look-alikes and trailing dots or spaces. End to end (`transfers_e2e.rs` `a_click_never_runs_a_received_program`): a click on a received `setup.exe`, `run.sh`, `tool.pyz` and `evil.pdf.exe` opens nothing and shows each in its folder; a click on `report.pdf` opens it.

**Mark of the Web on every received file.** `motw.rs` writes the `Zone.Identifier` stream (`[ZoneTransfer]`, `ZoneId=3`, `HostUrl=about:internet`) on the `.part` file just before the final move (`transfers.rs:1116` `finish_blocking`, `transfers.rs:1140`), so the file has it from the moment it has its final name (the stream moves with the file). **Verified three ways:** the unit tests (`motw.rs`: written, read back, goes with a rename); the integration test `every_received_file_carries_the_mark_of_the_web` (a photo, a 0-byte file and a `.ps1`, and the stream isn't a file of the folder); and the **real `Kin.exe`** in `check-1-3.ps1`, which reads `Get-Content -Stream Zone.Identifier` of a received file (new check, passes). **SmartScreen and Office Protected View now apply to received files** (documented in [windows.md](../windows.md#what-a-received-file-may-do) and one line of the security model in architecture.md). A volume with no alternate data streams (FAT, exFAT, some shares) can't carry it: the file is received anyway and the log says so (no name in it). Not run: a real Office file in Protected View, or SmartScreen on a real program.

### 3. The other desktop findings

| Finding | Fix, and where |
|---|---|
| **A resume adopted another transfer's `.part`** (a file at my path was "mine") | Names of unfinished transfers now count as taken **even when their file is missing** (`Transfers::claimed_names`, `transfers.rs:340`), used when a transfer is created (`transfers.rs:707`) and when a lost `.part` is created again (`transfers.rs:812`). A `.part` is only ever the one the record names. Test `a_resume_never_takes_over_another_transfers_part_file` (A's `.part` deleted, B arrives with the same name and gets ` (1)`, A resumes with a new `.part` under its own name, B's bytes untouched) |
| **The 24 h sweeper deleted any `*.part`** | It deletes **only paths a database record names** for a transfer that is over (`storage.rs:434` `leftover_part_files`, `transfers.rs:1463` `remove_leftover_parts`); a path that any record has as a finished file, or that an unfinished upload uses, is never touched. A dropped transfer clears its records' paths after it removes the files (`storage.rs:387`). Tests: `the_sweeper_deletes_only_paths_a_record_names` (a recorded leftover goes; an old unrecorded `movie.mp4.part` and a finished `backup.part` stay, also when a stale record points at the same path) and the updated `part_files_are_kept_a_day_and_then_cleaned_up` |
| **`insert_transfer` wiped old records when an ID was reused** | **A transfer ID is never reused.** The wipe is gone (`storage.rs:292`: a used ID is an error and changes nothing). A known finished ID answers its offsets for 24 h (`transfers.rs:268` `past_keeping`, `transfers.rs:621` `find_upload`) and then `404`; a canceled or failed ID is `404` at once; `POST` checks the database (`storage.rs:396`, `transfers.rs:718`). Tests `a_transfer_id_is_never_reused` (also after a restart), `inserting_a_used_transfer_id_changes_nothing`, and the `transfer-id-*` vectors |
| **The final move could be interrupted by the request** (the phone then re-sent the whole file) | Finalising runs in a task of its own (`transfers.rs:1056` `finalize_file`, `1069` `finalize_task`, `1116` `finish_blocking`), one at a time per file, with the database told the name **before** the move (`finalizing_name`, new column with a migration, `storage.rs:105`, `:378`) and the result **after** it. A retry waits for it and sees "complete"; a restart in the middle finds a finished move (`transfers.rs:558`). Tests: a request dropped during the finish (a test hook delays it) still ends, one file, one toast, a retry `POST` says complete and an empty `PUT` is `200`; `a_restart_in_the_middle_of_a_move_finds_the_file_final` |
| **`upload_put` answered `200` without checking stored bytes == size** | The `.part` file is checked to be exactly `size` bytes before the move (`transfers.rs:1116`), and the handler checks `done == size` and "final" before it answers (`transfers.rs:959`). Test: the file is cut short while the PC is finishing it, the answer is `500 internal`, and the next `POST` reports what is really stored |
| **Cancel racing a `PUT`; `upload_done` could run twice** | A transfer ends once: `end_if_active` (`transfers.rs:275`) is a compare-and-set, `dropped` makes a cancel happen once however many arrive (`transfers.rs:1183`), a cancel flips the status first so a `PUT` that gets its turn answers `404`, waits for the finish that is running (the `finalizer` lock) and then removes the parts; the finish of a canceled transfer is skipped. The cancel itself also runs as a task that outlives its request. A finished transfer that is canceled stays done (its files stay). Tests: **40 rounds** of a cancel (`DELETE` plus a `cancel` message) racing the last `PUT` (always one consistent end, one toast at most, no `.part`), 30 rounds of two files finishing together (one toast), a second cancel does nothing, a dropped `DELETE` is still carried out |
| **The cancel reason was logged raw** | `sanitize_reason` (`transfers.rs:470`): control characters and bidi controls removed, 32 characters at most; logged with `{:?}` (escaped) and stored cleaned (`peer` if nothing is left). Test with `ESC[31m`, CR/LF, NUL, U+202E and 500 characters |
| **`short()` of a UUIDv7 was the timestamp** | `short_transfer` (`core.rs:212`) takes the **last** 8 characters; used for every transfer ID in the logs (`transfers.rs`, `outgoing.rs`, `fileserver.rs`, `arrival.rs`). Device IDs keep `short`. Test with two IDs of the same moment |
| **Image detection trusted the MIME type; PNG format by extension** | `images::sniff` and `find_images` (`images.rs:236`, `:256`) ask the decoder, and the `PNG` clipboard format is written when the decoder says the container is PNG (`images.rs:271`). Tests: text named `fake.png` with an image type is not an image; a PNG named `scan.dat` with a wrong type is (and gets the PNG format) |
| **Blocking file I/O on async threads** | Moved to `spawn_blocking`: creating a transfer and its up to 10,000 `.part` files, the stored-bytes check, opening for a `PUT`, the free-space check, the finish, the removal of partial files, the clean-up, `GET`'s open and seek and the offer's source check, `send_paths`' look at the files, writing a long text's `.txt`, `status.json` (`transfers.rs:707`, `:791`, `:853`, `:1056`, `:482`, `:1402`; `fileserver.rs:259`; `outgoing.rs:127`, `:305`; `app.rs:231`) |
| **Paths over ~259 characters failed the final move** | `paths::long_path` (`paths.rs:33`, the `\\?\` form from 240 characters up) is used for `MoveFileExW` (`filenames.rs:192`), the free-space call, the imaging calls and ShellExecute / Explorer; `std::fs` adds the prefix itself. Tests: a move in a folder of 3 x 70 characters, an upload into a received folder deeper than 259 characters (final name, mark, a collision suffix) |
| **Collision loop of 10,000 tries** | 100 candidates (`filenames.rs:15`, `:103`), see the spec below. Tests: unit, the vectors in a real folder, and `a_hundred_names_taken_fail_the_file_with_internal_and_leave_nothing_behind` |

### 4. Spec and vectors (`docs/protocol.md` changelog items 40 to 44, `protocol/`)

- **Three malformed vectors fixed** in `http.json`: `get-expired-wins-over-source-changed`, `get-unknown-file-wins-over-source-changed` and `get-on-an-upload-id-is-not-found` had their explanation inside the `Range` header; it is in `note` now and the requests have no `Range`.
- **Caps.** At most **800 files in an offer**: a receiver rejects a bigger one with `error` `too_large` (`ref` = the offer's id) instead of an ack (`messages.rs:294`; vectors `offer-800-files-is-the-limit`, `offer-801-files-is-too-large`; the PC already split at 800, now one constant, `messages.rs:328`). Collision suffixes stop after **100 candidates** (the plain name and ` (1)` to ` (99)`, skipped ones counted); then the file fails with `internal` (vectors `create-new-99th-suffix-is-the-last-candidate`, `create-new-100-names-taken-fails-with-internal`, `create-new-100-names-known-fails-with-internal`, `move-part-100-names-taken-fails-with-internal`, `move-part-99th-suffix-is-the-last-candidate`).
- **A transfer ID is never reused** (known finished ID: offsets for 24 h, then `404`; canceled: `404` at once). Vectors: two new sequences `transfer-id-canceled-is-never-reused` and `transfer-id-finished-answers-for-a-day-then-404`, with a new step type `clock: { advanceHours }` that moves the PC's clock (explained in `http.json` and `protocol/README.md`; `transfer_vectors.rs` implements it by shifting the finish times).
- **The reviewer's rulings:** a `cancel` message on an upload is equivalent to `DELETE` (#11), and `offset` given twice is `400` (#12; vectors `put-offset-given-twice-is-bad-request` and `put-offset-given-twice-with-the-right-value-first-is-bad-request`).
- `check-vectors.ts` has the new cases in its model (the used-ID rule, the clock step, the offer limit, the candidate limit) and its 1.3 scope; it passes. **The Android harness needs to catch up** on: the two `offer-80x-*` cases, the five `create-new-*` / `move-part-*` limit cases, the `put-offset-given-twice-*` exchanges (server side only) and the `transfer-id-*` sequences (the `clock` step has no request).

### Notes and decisions

1. **Hyper drops the handler when the connection breaks.** Found by a flaky test (`a_full_disk_is_507...` failed about one run in ten under load): with more `await` points before the body is read, a request dropped at the wrong moment lost the 3,000 bytes it had already taken from the connection. The cure is structural (the work of a `PUT` and of a cancel runs as a task that finishes whatever the request does), not a retry in the test.
2. A **stopping writer takes what is arriving** (30 ms a frame, 150 ms in all) before it answers `409`, so "keeps what arrived" holds when the phone comes back on a new connection a few milliseconds after the old one died.
3. **DELETE of a finished transfer** answers `204` and the files stay, but the record stays `done` (it is not turned into `canceled`); the ID is gone from then on (`404`).
4. The finalising step has a **test hook** (`Hooks::finalize_delay_ms`, `transfers.rs:380`) that delays the move so a test can drop the request or change the file during it. It is zero outside tests.
5. The `transfer_files` table got a column (`finalizing_name`) with an `ALTER TABLE` migration for a database made before it (unit test `a_database_from_before_finalizing_name_gets_the_column_and_keeps_its_rows`).
6. `Kin.exe` is now 11,994,624 bytes (11.44 MiB; 11.23 MiB before) and the NSIS installer 3,597,202 bytes (**3.43 MiB**; 3.39 MiB before). Idle memory re-measured (`measure-idle.ps1`, 61 s after start): **2.8 MB private working set**, 17.2 MB working set, 29 threads, 221 handles, unchanged from 1.3.

### Not done, or not verified

- **A real Office file in Protected View, SmartScreen on a real program, and the toast's "Show in folder"-only button on screen**: the mark is verified on the real app and the rules by unit tests, the toast variant by the test of the event and the code path of the other button; I did not drive it with a real toast.
- **A paste into real apps** (as before), **a real phone, Windows 10, real Wi-Fi** (as before).
- The 12 MP measurement is on the PC of 1.1 (Ryzen 9 7900X); a slower PC will take longer, but the cost is linear in the pixels now, not in pixels times rows.

## Spec changes (step 1)

All in `docs/protocol.md` (the changelog, items 32 to 38), with vectors in `protocol/` and checks in `protocol/check-vectors.ts` (new groups `urls`, `text-rules`, `scope-1.3`). `protocol/README.md` has the 1.3 scope table you asked for.

| # | Change | Vectors |
|---|---|---|
| 32 | **`sensitive`**: optional boolean on `text`, default false. The PC sets it when the clipboard carried an opt-out format (the hotkey may still send it); the phone sets it on `EXTRA_IS_SENSITIVE`. The receiver marks the clipboard (on Windows also `ExcludeClipboardContentFromMonitorProcessing` and `CanIncludeInClipboardHistory` = 0, and, an addition, `CanUploadToCloudClipboard` = 0), shows "Text from <device>" with no preview and no "Open", and never stores or logs it (not even length or hash) | `messages.json`: 7 valid + 5 invalid `text-sensitive-*` cases, new `expect.sensitive`; `text-rules.json`: `sensitiveSender[]` (11), `receiver[]` (5), `windowsSensitiveFormats` |
| 33 | **Sender size threshold**: 1 MiB stays the receivers' hard limit; senders send a `text` as a message only up to 128 KiB of UTF-8 (wire form), longer goes as a `.txt` file; an automatic `clipboard` over 128 KiB isn't sent (`too_large`); a *sensitive* text over 128 KiB isn't sent at all (my addition: a file can't carry the mark); receivers keep "Couldn't copy" | `text-rules.json` `senderSize[]` (16 cases, including CRLF text measured after conversion and multibyte text at the boundary) |
| 34 | **Line ends**: LF only on the wire (CRLF → LF in one left-to-right pass, a lone CR stays); the Windows receiver turns an LF that isn't preceded by a CR into CRLF; loop-guard hashes are of the wire form | `lineEndings[]` (11, with the SHA-256 of the wire form) |
| 35 | **`isUrl` / openable URL** rule (trim with an explicit `White_Space` list, `http`/`https` + `://`, no whitespace or control character anywhere, non-empty host that isn't only dots or a non-empty `[IPv6]`, port 1–65535 in ASCII digits with a bare `:` rejected, at most 8 KiB). Receivers re-check | new `protocol/urls.json` (106 cases, including the 8 KiB boundary in bytes and characters) |
| 36 | A *pasted* pairing link needs no confirmation either (the paste field exists only on the first-run screen, where no paired PC exists to replace) | (a rule for the phone) |
| 37 | Clarified: the free-space check counts only the missing bytes (per file for a `PUT`, over all files for a `POST`, no reserve); uploads and offers are one ID space (`PUT`/`DELETE` on an offer ID, `GET` on an upload ID and a `POST` with an offer's ID are all `404`) | 23 new exchanges in `http.json` (`*-free-space-*`, `*-wins-over-*`, `*-an-offer-id-*`, `*-an-upload-id-*`, `get-range-from-one`) |
| 38 | Clarified, no rule changed: an error to a `PUT` is answered before the body is read and the connection may be closed (the phone treats the I/O error as a dropped connection and `POST`s again); one writer per file; a connection with no byte moving for 30 s is closed | (behaviour tests) |

## The 1.2 review fixes

| What | Where |
|---|---|
| `--dev-get-clipboard` / `--dev-set-clipboard` are honoured only when the running instance was started with a developer setting. In a normal run they are refused and the log says so ("refused --dev-get-clipboard: it works only when the app was started with a developer setting") | `cli.rs:45` (`Config::dev_hooks_enabled`: any of `--dev-bind`, `--dev-qr-address`, `--dev-port`, `--dev-print-pair-url`, `--dev-no-windows`, `--dev-received-dir`, `--dev-free-bytes`), `sync.rs:671`, `sync.rs:691`, `sync.rs:722` (`dev_hook_allowed`). Tests: `cli.rs` `the_developer_clipboard_hooks_need_a_developer_setting`, `tests/fake_phone.rs:1267` |
| `newest_link_toast()` fallback removed: a toast click that doesn't say which toast it was opens nothing | `app.rs:298` (`register_toast_clicks`); `sync.rs:610` (`toast_clicked`) now looks the id up and does nothing for an unknown one. Test: `a_toast_click_that_matches_no_toast_opens_nothing` |
| `tools/e2e-1-1.ps1 -Build`: the Gradle call used `cmd /c "gradlew.bat …"`, which doesn't see `Push-Location` and so couldn't find the wrapper | `tools/e2e-1-1.ps1:87-92`: `Push-Location $androidDir; & .\gradlew.bat -p $androidDir assembleDebug` (and `--stop`). **Tested:** the old form says "'gradlew.bat' is not recognized"; the new one, run from `C:\`, built `app-debug.apk` (38,496,585 bytes, 30 s). The rest of the e2e script (emulator) was not run |
| `sensitive`, the 128 KiB threshold and LF normalisation on the desktop side | `sync.rs` (`plan_send`, `deliver`, `receive_text`), `clipboard.rs` (`to_wire`, `write_text_sensitive`), `links.rs` (the new URL rule), `messages.rs` (`sensitive` validation) |

## What was built

| Area | What it does | Where |
|---|---|---|
| **Phone → PC** | `POST /v1/transfers` (new, or a resume with the same `fileId`s), `PUT …/files/{fileId}?offset=N`, `DELETE`, in the spec's order of checks, ASCII-digit parsing, the Content-Type rules (a `PUT` never looks at it), `413` for the 1 MiB JSON body and for a file over 1 TiB or 10,000 files, `507 disk_full` on the missing bytes only, `409 offset_mismatch` with the actual offset | `fileserver.rs`, `transfers.rs` |
| **Streaming** | The body of a `PUT` is cut into 1 MiB pieces with one write in flight while the next piece is read: at most two pieces in memory. `sync_data` and the database every 32 MiB; `fsync` before the final name; `MoveFileExW` **without** replace, so a name taken in the meantime (or that NTFS thinks is the same) gets the next suffix. The move retries for about a second when a virus scanner holds the file | `transfers.rs` (`Pipeline`, `finalize_file`), `filenames.rs` |
| **File names** | The File names section exactly (UTF-8 byte limits, bidi controls, reserved names, the cut rules) and create-new semantics; all 222 `cases[]` and 12 `createNew[]` vectors, with a fake create and with a real folder (a replacing rename fails the NTFS case-folding vector) | `filenames.rs`, `tests/transfer_vectors.rs` |
| **Resume** | A connection that breaks keeps what arrived. `POST` again reports it. A new request for a file an older one is still writing stops the older one (a phone that comes back before the PC notices the old connection is dead). After a restart a file counts as stored up to the smaller of the database and the disk (a `.part` that is short, long or gone is handled). `.part` files and the transfer are kept 24 h from the last progress, then removed (at start and hourly); a finished file is never touched, even one named `*.part` | `transfers.rs` (`recover_transfers`, `clean_up`) |
| **Storage** | `transfers` and `transfer_files` tables of architecture.md (status, bytes done, local path, plus the expiry and the source's modified time) | `storage.rs` |
| **Toast** | "<n> file(s) from <phone>"; **Open** and **Show in folder** buttons (one file) or **Open folder** (several); a click does the first button's job. A file Windows would run (`.exe`, scripts, shortcuts, installers, macro documents, …) is never opened: its folder opens with it selected | `arrival.rs`, `links.rs`, `app.rs` |
| **Images → clipboard** | On by default (`imagesToClipboard`). See the approach below | `images.rs`, `arrival.rs` |
| **PC → phone** | `--send`, the drop window and the tray's "Send files…" make an `offer` (`{fileId, name, size, mime}` + `totalSize`, at most 800 files per offer, MIME from the extension so the phone can tell photos and videos). `GET` streams from disk in 1 MiB pieces with one read ahead; `Range: bytes=N-` → `206`, `416` for anything else, `404` (unknown, canceled, finished, expired), `409 source_changed` (size or modified time changed, or the file is gone). Offers expire after 24 h and the phone then gets a `cancel` (reason `expired`) | `outgoing.rs`, `fileserver.rs` |
| **Drop window** | The tray's left click (or `--drop`). A target for files dragged from Explorer (real OLE drag and drop, Tauri's drag-drop event gives the paths), a click opens the Windows file dialog (`IFileOpenDialog`, several files), one row per transfer with a progress bar, speed and **Cancel**, the last finished ones, and the received folder. Plain DOM, destroyed on close. The tray tooltip follows the progress ("sending 45%") | `src/drop.ts`, `ui.rs`, `tray.rs` |
| **Explorer "Send to"** | `Kin.exe --send <paths…>` through the single-instance plugin (a relative path is resolved against the second launch's folder). `desktop/make-sendto-shortcut.ps1` creates the shortcut by hand (`-Folder` for a test folder, `-Remove`); **it was only run against a test folder, never against the real `SendTo`** (the installer does it in 1.5) | `cli.rs`, `app.rs` |
| **Folders** | Decision: **skipped with a message** ("A folder can't be sent yet. Send the files inside it instead"), the other dropped files are sent. Flattening loses structure and clashes names; sending structure needs a protocol addition | `outgoing.rs`, windows.md |
| **Long text** | A text over 128 KiB from the hotkey goes as `Text.txt` (text/plain, LF, no BOM) through the same offer path, from a temporary file of ours in `outgoing\` that is removed when the phone has it or the offer expires (or after a day, for a crash) | `sync.rs`, `outgoing.rs` |
| **Cancel** | From the drop window (a `cancel` message, reason `user`, goes to the phone and partial files go) and from the phone (`DELETE` or a `cancel` message), in both directions. A download under way is cut | `outgoing.rs`, `transfers.rs` |
| **The WebSocket never waits for a file** | All of it runs in the HTTP handlers and in tasks of its own; `complete` and `cancel` messages are handled in spawned tasks. Test: 5 pings during a 300 MiB upload, all answered, slowest well under 500 ms | `ws.rs`, `tests/transfers_e2e.rs` |

## The toast buttons (R10)

**Finding: they work for an app that isn't packaged. Toasts are not click-only.** Checked with the real `Kin.exe` on Windows 11: a toast for one received file shows **Open** and **Show in folder**; pressing "Show in folder" with the mouse reached the app (the log said `toast 6: showed the received file in its folder`) and Explorer opened the received folder with the file selected. What comes with it, and is written up in [windows.md](../windows.md#toast-buttons-r10):

- Clicks and button presses are reported **only while the toast is on screen** (the notification plugin keeps the handler in the running process). Once it has moved to the Action Center, pressing it does nothing. That was already the 1.2 finding for a click.
- An unpackaged app has no app ID of its own, so Windows shows **"Windows PowerShell"** as the toast's name and icon. Cosmetic: the installer of 1.5 can register an app ID, and the buttons don't depend on it.
- A second toast waits for the first to leave the screen (Windows stacks them but showed them one after the other here).
- I could not drive "Open" the same way (the toast's accessibility tree isn't reachable from UI Automation, so the button was clicked by pixel position, and one attempt found the toast gone and was skipped on purpose). "Open" and a click on the toast body use the same code as "Show in folder" up to the last call, and the recording-opener tests cover them.

## Images on the clipboard

An image from the phone goes on the clipboard so that Ctrl+V works in common apps. **Exactly one image in a transfer** goes there; several leave the clipboard alone. It is decoded with the **Windows Imaging Component** (no image crate: the installer grew by 0.15 MiB in total), straight into the one block of memory that becomes the clipboard data, and these formats are set:

| Format | Why |
|---|---|
| `CF_DIBV5` | 32-bit BGRA with alpha. Windows builds `CF_DIB` and `CF_BITMAP` from it, so Paint, Word, Outlook, Photoshop and anything that asks for a plain bitmap gets a bitmap, and an app that wants alpha gets alpha |
| `PNG` | the original PNG bytes (when the file is a PNG): Chrome, Edge, Firefox, Teams, Slack and Office prefer it |
| `CF_HDROP` | the path in the received folder: Explorer pastes the file, chat apps attach the original full-quality file |

- **EXIF rotation is applied** (all eight orientations): a phone photo is stored sideways with an orientation tag, and an app that reads a bitmap never looks at EXIF.
- **Memory is bounded:** 4 bytes a pixel, **at most 64 MiB of pixels (about 16 MP)**; a 12 MP phone photo fits, a 48 MP one is skipped and the log says why. A PNG over 64 MiB doesn't get the `PNG` format.
- **Verified with the real clipboard** (`check-1-3.ps1`, which saves every format first and restores them after): an 8 × 4 PNG with transparency pastes as 8 × 4 with the left half opaque red, the clipboard lists `CF_DIBV5`, `PNG` and `CF_HDROP`, `GetFileDropList()` returns the file and the `PNG` format has the file's size; a 40 × 20 JPEG with EXIF orientation 6 pastes as **20 × 40** with the green half on top and the yellow below.
- Not verified: a paste into a real app (Paint, Word, Chrome). I checked what the clipboard offers and read it back with .NET (`Clipboard.GetImage()`), which goes through the same `CF_DIB` conversion those apps use.

## Throughput

Release build, loopback, TLS 1.3 with a mutual-TLS client, the fake phone and the PC in one process (tests) or two processes (the real app), files written to and read from a temporary folder on the system disk. **Real Wi-Fi numbers come later**: on a Wi-Fi network the radio is the limit (roughly 50 to 150 MB/s), and this shows the PC side is not the bottleneck.

| Run | Result |
|---|---|
| **Phone → PC, 2 GB** (`large_upload_of_2_gb`, final build) | **1,038 MB/s** (2.1 s). An earlier run of the same test: 725 MB/s (3.0 s). The file was then verified byte for byte against the pattern (0.9 s) |
| Phone → PC, 2 GB cut after 1,000 MiB + 4,321 bytes and **resumed** (`…_that_breaks_and_resumes`) | The PC kept exactly 1,048,580,321 bytes; the other 1.10 GB at **1,045 MB/s** (earlier run 629 MB/s); the file is intact |
| **PC → phone, 2 GB** (`large_download_of_2_gb_and_a_resume_in_the_middle`) | **1,143 MB/s** (1.9 s), intact. Earlier run: 872 MB/s |
| PC → phone, **resumed** with `Range` from byte 1,048,580,321 | the remaining 1.10 GB at **1,308 MB/s** (earlier run 1,028 MB/s); `206` with the right `Content-Range`, intact |
| The real `Kin.exe` receiving 6 GB from the fake phone (two processes) | **644 MB/s** as the phone measured it; `status.json` showed up to **662 MB/s** |
| The same, memory | idle working set 17 MB / private bytes 4 MB; **peak 24 MB / 11 MB while receiving 6 GB**; 20 MB / 5 MB afterwards. Nothing is held in memory beyond two 1 MiB pieces |
| 100 MiB upload in the ordinary test suite (crosses three checkpoints) | passes in well under a second |
| 50 files (1 to 50 KB, repeated names) over one keep-alive connection | all 50 intact, with the suffixes `photo.jpg`, `photo (1).jpg`, … |

(The numbers vary between runs by up to 40% because the disk cache and the other programs on the PC differ; both runs of each are shown.)

## Tests

`cargo test` in `desktop/src-tauri`: **238 tests pass, 5 are ignored** (1.2: 158 and 1). `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean. `bun protocol/check-vectors.ts`: 6,084 checks (1.2: 5,265).

| File | Tests | What |
|---|---|---|
| `src/*.rs` (unit) | 110 (+2 ignored) | 1.2's 81, and the new: file names (the steps, collisions, the byte limit, create-new, the real folder never overwrites, the no-replace move on a name NTFS treats as the same, a `.part` held by a scanner for 300 ms), the URL rule (accepted, refused, the 8 KiB limit), risky file types, strict number parsing, speed, ranges, ids, the offset parameter, MIME types, the image limits, the V5 header, `DROPFILES`, EXIF orientations, settings, CLI (`--send`, the new flags, relative paths, dev-setting gating) |
| `tests/vectors.rs` | 13 | 1.1/1.2, plus the **1.3 message scope**: `offer-*`, `complete-*`, `cancel-*`, the duplicate offer, and `expect.sensitive` on every `deliver_text` case |
| `tests/transfer_vectors.rs` | 9 | **file-names.json** (all `cases[]`, all `createNew[]` with a fake and with a real folder), **urls.json**, **text-rules.json** (line ends, the threshold), and **http.json: every transfer exchange (≥100) and sequence** in-process against the router with a core in the state the context describes (transfers and offers seeded, free space, the 1 MiB limit, a real create failure, an interrupted upload or download) |
| `tests/fake_phone.rs` | 50 | 1.2's 44 and 6 new: the PC sets `sensitive` exactly when the clipboard carried an opt-out format and never sends it automatically (`sensitiveSender[]`), a sensitive text from the phone has no preview, no link and **nothing in the log or status.json** (`receiver[]`), wrong types of `sensitive`, an automatic clipboard over 128 KiB, CRLF/LF both ways without an echo, the dev hooks refused without a developer setting |
| `tests/transfers_e2e.rs` | 22 (+2 ignored) | real TLS: one file, several files with a 0-byte one in any order, a **50-file batch**, a file bigger than the checkpoint interval, **resume after a dropped connection** (the PC kept exactly what arrived), **takeover** of a connection the PC hasn't noticed is dead (by `POST` and by `PUT`), `DELETE` (complete files stay), **cancel from the PC and from the phone**, **disk full** (faked, only the missing bytes count, resumed `POST` re-checked), **a name taken while the `.part` is moved**, cleaning of hostile names, the WebSocket during a 300 MiB upload, **restart recovery** (and a `.part` that is short, long or gone), the 24 h clean-up (a finished `backup.part` survives), images to the clipboard (one yes, several no, off), a click never runs an `.exe`, an unknown toast id opens nothing. Ignored: the 2 GB runs |
| `tests/offers_e2e.rs` | 13 (+1 ignored) | real TLS: single, several and 0-byte offers and downloads and `complete`, **resume with `Range`**, `416` for every bad range, **cancel from each side** (also a download under way, which is cut), **`source_changed`** (time, size, gone), an **expired offer** (and the `cancel` the phone gets), a phone that answers `disk_full`, folders skipped, no phone, more files than fit in one offer, a text over 128 KiB as `Text.txt`, a sensitive one over it not sent. Ignored: the 2 GB run |
| `tests/server_limits.rs`, `tls_vectors.rs` | 9, 12 | unchanged |

**Mutation check:** with the final move changed to *replace* an existing file, the unit tests `the_real_folder_never_overwrites` and `the_move_does_not_replace_a_name_the_filesystem_treats_as_the_same` fail (and nothing else needed to).

### The real app: `desktop/check-1-3.ps1` (38 checks, all pass) and `check-dragdrop.ps1`

The release `Kin.exe` with `--dev-bind 127.0.0.1`, a throwaway profile and received folder, and the fake phone. The user's clipboard (HTML, text, Chromium's own formats: 7) was saved in every format and **put back identical** (checked byte for byte), every run. Checks: a 30 MB upload cut at 12,345,678 bytes and resumed, a real file, the hashes equal on both sides, no `.part` left, `status.json` lists the finished transfer; **`Kin.exe --send photo.jpg empty.txt album` from another folder** (relative paths, a 0-byte file, a folder skipped with a message), a download cut at 900,000 bytes and resumed, intact; a password-manager copy with `--send-clipboard` arrives with `sensitive=True` and the log has no trace of it; a 200 KiB text arrives as `Text.txt` with LF line ends; a text with LF from the phone is on the clipboard with CRLF; the PNG and the rotated JPEG on the real clipboard (above); `--dev-free-bytes 1000000` refuses a 5 MB upload with 507 and takes a 900 KB one. `check-dragdrop.ps1` does a real OLE drag and drop of a file onto the drop window (a data object with a file list, the pointer moved by the script): the phone was offered the file and downloaded it intact (8 runs in a row passed after the harness was fixed to close the "New device paired" window first).

The drop window was also looked at (screenshot): its dashed target, "Fake Phone is connected.", and a row "From Fake Phone: generated-1.bin — 856 MB of 7.5 GB · 630 MB/s" with a progress bar and **Cancel**; Cancel was pressed with the mouse during a 12 GB upload and the log said `the user canceled transfer … (in)`, the row became `canceled/user` at 7,449 MB, and the `.part` file was gone.

## Idle memory and installer

Release `Kin.exe`, tray only, no window ever opened, 60 s after launch, `desktop/measure-idle.ps1` (now with a throwaway received folder), three runs, measured as in 1.1:

| Run | Working set | **Private working set** | Private bytes | Threads | Handles |
|---|---|---|---|---|---|
| 1 | 17.1 MB | **2.8 MB** | 4.1 MB | 29 | 220 |
| 2 | 17.1 MB | **2.8 MB** | 4.1 MB | 29 | 220 |
| 3 | 17.1 MB | **2.8 MB** | 4.0 MB | 29 | 220 |

The same as 1.2 (2.7 to 2.9 MB, 29 threads, 220 handles) against the target of under 30 MB ([targets.md](../targets.md)). The drop window, when open, is a WebView2 like the pairing window and is destroyed on close.

| Size | |
|---|---|
| `Kin.exe` | 11,777,024 bytes (11.23 MiB; 1.2: 10.7 MiB) |
| NSIS installer `Kin_0.1.0_x64-setup.exe` | 3,553,718 bytes = **3.39 MiB** (1.2: 3.24 MiB), built not run |

## Dev hooks for the integration agent

All documented in [desktop/README.md](../../desktop/README.md#developer-and-integration-settings).

- **`Kin.exe --send <paths…>`** works headless (and through the single-instance plugin from a second launch, relative paths resolved against that launch's folder). `--drop` opens the drop window.
- **`status.json` → `transfers`**: `active[]` and `recent[]` (last 10) with `direction`, `status`, `files`, `filesDone`, `bytesDone`, `totalBytes`, `speedBps`, `error`, times and the first names, and `receivedFolder`. Rewritten once a second while a transfer is going.
- **`--dev-received-dir <path>`** (`KIN_DEV_RECEIVED_DIR`): the received-files folder, so a test never writes to the real Downloads. **Always pass it** (and `--data-dir`) in a test run.
- **`--dev-free-bytes <n>`** (`KIN_DEV_FREE_BYTES`): fakes the free space; the `507` check uses it.
- The developer clipboard commands work only with a developer setting (the 1.2 fix above).
- **`examples/fake_phone.rs`** uploads (`--upload`, `--upload-generated`, `--upload-cut`), downloads offers (`--save-dir`, `--download-cut`, `--no-download`, `--refuse-offer`), and prints one `uploaded …`/`downloaded … sha256=…` line per file.
- **`desktop/clipboard-backup.ps1`** (`Save-Clipboard` / `Restore-Clipboard`, every format), **`check-1-3.ps1`**, **`check-dragdrop.ps1`**, **`make-sendto-shortcut.ps1`**.

## Spec ambiguities and decisions

Nothing was changed silently; where the spec is silent, this is what the PC does (all easy to change):

1. **A sensitive text over 128 KiB** isn't sent at all (the spec says a longer `text` goes as a file; a file has no `sensitive` mark and would put a secret on the phone's disk). Written into protocol.md as part of item 33.
2. **`CanUploadToCloudClipboard` = 0 is written as well** for a sensitive text (the task named two formats): a password in the cloud clipboard is the thing the flag is meant to prevent.
3. **Which image goes to the clipboard.** Exactly one image in a transfer; several leave the clipboard alone (no way to hold them as pictures, and picking one would be a surprise). Spec: "images also go to the clipboard".
4. **Clicking a toast never runs a program.** A file with an extension Windows would execute is shown in its folder instead. Spec: "clicking opens the file".
5. **Folders are skipped**, not flattened (decision asked for; documented in windows.md).
6. **A 0-byte file** is created (`.part`) at `POST` and completes with its `PUT` (offset 0, empty body), like every other file; the `POST` answers offset 0 for it. A complete file at `POST` (offset = size) is already finalized; the phone's idempotent empty `PUT` gets `200`. (The Android agent noted the same gap.)
7. **When the PC answers an error to a `PUT` it may close the connection** without reading the body (item 38). A phone sending a big body then sees an I/O error, not the answer, and has to `POST` again. A test showed it: on Windows a client that keeps writing gets `WSAECONNABORTED`.
8. **Limits "up to the PC":** a file over 1 TiB, or more than 10,000 files in one transfer, is `413`. One `offer` carries at most 800 files (a long-name worst case is under 700 KB of JSON).
9. **One writer per file, newest request wins:** the older request is answered `409 offset_mismatch` with what it had delivered. The spec doesn't say what happens to a request that is still writing when a new one arrives.
10. **A finished received transfer stays known for 24 h** (also across a restart), so a repeated `POST` or an empty `PUT` after a lost `200` is answered instead of `404`. After a `DELETE` or a cancel it is forgotten (a repeated `DELETE` is `404`, as the spec says).
11. **`cancel` from the phone for an upload** drops partial files like `DELETE` does; `complete` for an upload (or for anything unknown) is acked and ignored.
12. **A query with `offset` twice** is `400`; the unit is `bytes` in `Range` case-insensitively; an `offset` over 2^53 − 1 is `400`.
13. **The free-space check for a `PUT`** counts that file's remaining bytes only (`Content-Length`); for a `POST` the missing bytes of all files; no reserve. A disk that really fills up mid-write (`StorageFull`) is `507`.
14. **`Text.txt`** is the name of a long text as a file (the spec says "a name ending in `.txt`"; Android uses `Kin text <date time>.txt`). A temporary file of ours holds it until the phone is done.
15. **The Downloads folder** is asked from Windows (`FOLDERID_Downloads`), not built from `%USERPROFILE%`, so a moved Downloads folder works. The default is `Downloads\Kin` as in windows.md.
16. **Developer-setting gating:** "started with the dev flags" means *any* `--dev-*` setting at the first launch.
17. **`status.json` can't always be replaced** while a script has it open (Windows refuses the rename); the app tries 5 times. A script should read it with a share mode that allows replacing, or tolerate one failed read.
18. **The loop guard's 10 s memory** now holds the hash of the *wire form* (item 34), so the PC clipboard (CRLF) and the phone's text (LF) are one text for it.

## Not done, or not verified

- **A real phone, Windows 10, mDNS, real Wi-Fi throughput, the installed app.** As in 1.1 and 1.2. The installer was built, not run, and the "Send to" shortcut was made only in a test folder.
- **A paste into real apps** (Paint, Word, Chrome, Slack). The clipboard formats and the read-back through .NET were checked, not the apps.
- **"Open" on a toast with the mouse** (see the R10 section); "Show in folder" was.
- **The toast stack with several files at once** (many toasts in a row), and how Focus Assist or "Do not disturb" hides them.
- **The tray tooltip text on screen** ("sending 45%"): the string and the refresh are tested, a tooltip can't be read from outside.
- **The 1.2 `-Build` fix inside the whole e2e script** (the emulator part was not run), only the Gradle call.
- **Antivirus holding a `.part` file for real** (the retry is tested with a file held exclusively for 300 ms).
- **A phone that sends `Expect: 100-continue`**, chunked bodies (refused with `400`, as the spec says), and several phones at once (ownership is vector-tested).
