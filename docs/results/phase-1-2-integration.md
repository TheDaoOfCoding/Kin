# Phase 1.2 result: integration (real Windows app + real Android app)

Run 2026-10-14 by the integration agent. The two halves of milestone 1.2 were built independently from [protocol.md](../protocol.md), by [the desktop agent](phase-1-2-desktop.md) and [the Android agent](phase-1-2-android.md). This is the first time they talked to each other with text and the clipboard. Nothing is committed.

**Setup:** the real release `Kin.exe` (unchanged since the desktop agent's build) in developer mode (`--dev-bind 127.0.0.1 --dev-qr-address 10.0.2.2`, loopback only, a throwaway data dir, never `%APPDATA%\Kin`), and the real debug APK (rebuilt once, with the one Android fix below), on `kin_api37` (Android 17) and `kin_api29` (Android 10). The PC is reached as `10.0.2.2:47821`. It used the **real Windows clipboard** (Kin watches it, `Set-Clipboard` / `Get-Clipboard` drive it): the user's text was saved first and put back at the end of every run, verified byte for byte by the script.

**Result in one line:** one cross-side bug (the phone acked a clipboard text it could not copy and said nothing), a flaky-looking race that is not a bug, and four things the spec should say (below). Every scenario passes on both emulators; `tools/e2e-1-2.ps1` automates 1 to 6, 8 (PC to phone), 10 and 11 and passes with 79 checks on API 37 and 78 on API 29.

## Known items

| Item | Result |
|---|---|
| `tools/e2e-1-1.ps1` opened the pairing link without `AUTO_CONFIRM` and stopped at the confirmation screen | Fixed, `tools/e2e-1-1.ps1:130`. `tools\e2e-1-1.ps1` passes on `kin_api37` (16 checks) |
| `protocol/README.md` pointed at the deleted `android-generated-cert.der` | Fixed, `protocol/README.md:108` now says `protocol/fixtures/android-client.{der,json}` (read by `CertificateBuilderTest`; the exporter writes to `android/app/build/fixture-export/`). The same stale path was in the `description` of `protocol/fixtures/android-client.json:2`, also fixed (a description, not data) |

## Scenarios

"Tap to ack" is the app's own number (the desktop's includes its 300 ms clipboard debounce); "wall" is measured by the script from the command to the PC's `status.json`. Loopback through the emulator's NAT, so the network part is small. Both emulators gave the same results unless a row says otherwise.

| # | Scenario | Result | Evidence |
|---|---|---|---|
| 1 | PC copy → phone clipboard within 1 s | **PASS** | `Set-Clipboard` → `clipboard` acked: **tap to ack 312 to 329 ms** (API 37: 318, 320, 325; API 29: 312, 322, 329), wall 526 to 544 ms, round trip 2.8 to 3.5 ms. The phone clipboard, read by the debug activity, is the text (exact, also with an emoji and `é`). Exactly one `clipboard` message per copy, though Windows raises several change events |
| 2 | Password-manager copy not sent | **PASS** | `--dev-clipboard-mark exclude` and `no-cloud` (by hand also `no-history`, `viewer-ignore`): `clipboardSync.sent` unchanged, `lastSkip.reason = password_manager:<format>`, the phone clipboard keeps the old text, and the secret is in no log |
| 3 | Hotkey path (`--send-clipboard`) | **PASS** | `text` acked, tap to ack **2.9 to 6.1 ms**; the phone clipboard is set and the notification "Text from EMPIRE" is posted. With a URL it has the **Open** action, with plain text it has no action. (The real key press `Ctrl+Alt+Shift+K` was verified by the desktop agent; the script uses the command, which runs the same code) |
| 4 | Phone "Send to PC" | **PASS** | All four paths, each time the PC clipboard has the text (`Get-Clipboard`, which is what Ctrl+V pastes), a `toast: Text from …` is logged, and the phone's tap to ack is recorded: **share intent 17 to 81 ms**, **PROCESS_TEXT 12 to 21 ms**, **clipboard alias 73 to 126 ms**, **notification action (a real tap on the button in the shade) 73 to 178 ms**. The adb command to the PC's `status.json` is 303 to 352 ms for share and PROCESS_TEXT (most of it is `am start`). The "done when" (select text, Send to PC, Ctrl+V) is met up to the paste: the script reads the clipboard Ctrl+V pastes, it does not type into an app |
| 5 | Loop prevention | **PASS** | PC side: after each of the four phone sends the PC did not send a `clipboard` back (`sent` 3 → 3, `lastSkip = echo_of_phone_text`, `not sent back` in the log). Phone side: after the PC set the phone clipboard, "send clipboard" was skipped (`send skipped (CLIPBOARD)`), nothing reached the PC, and the phone sent nothing by itself. Counts over a long manual session on API 37: 44 texts from the phone, 63 echo events suppressed on the PC (several per write), 33 `clipboard` received on the phone, 2 skipped sends |
| 6 | URL handling | **PASS** | Both ways, for `https://`, `HTTP://Example.ORG/…` (open) and `javascript:`, `file:///`, `intent://`, `ms-msdt:` (never): `isUrl` and the receiver's own check agree, the PC logs "click opens the link" only for http(s), and the phone's notification has the Open action only for http(s). I also compared the two `isUrl` rules on 19 edge cases (underscore host, `http://`, `http:///path`, `https:example.com`, spaces, trailing dot, IPv6, user-info, leading and trailing spaces): they agree on all (see open issue 6) |
| 7 | PC offline / phone offline | **PASS** (by hand) | PC killed: a phone send gives the "PC not reachable" notification (with the Wi-Fi/Kin hint) **4.3 s** after the send. PC restarted: the phone was in its 30 s backoff step and did not come back until I opened the app (then 1.9 s); see open issue 3. Phone app force-stopped: the PC's explicit send fails at once (`result: phone_offline`, toast "The phone isn't connected. The text was not sent.") and the automatic `clipboard` logs the same result in `status.json` and the log |
| 8 | Large text | **PASS**, with the Android limit below (by hand and, for PC → phone, in the script) | PC → phone: a `text` of exactly **1,048,493** characters (the largest frame) is acked (tap to ack 96 to 251 ms); one more is `too_large` (toast "That text is over 1 MiB…", nothing sent). A `clipboard` of 1,048,502 is sent, 1,048,503 is `too_large`. Phone → PC: a 1,040,000-character text sent with the clipboard alias arrives complete and is acked in 454 ms. **Phone side of "too long for now":** not reachable end to end, because Android won't take more than about 1 MB through binder into the app (clipboard, intent extra), so a text that long can't get in; the exact boundary and the message are in `TextFlowTest`. See bug 1 for the other Android limit |
| 9 | Unicode and multiline | **PASS** (by hand) | Emoji outside the BMP (`😀👍🏽`), CJK, Korean and `é ñ` in one text: identical both ways (36 code points on both sides). Phone → PC: LF → CRLF, CRLF stays CRLF (not `\r\r\n`), a bare CR stays, a trailing newline stays. PC → phone: unchanged (LF stays LF, CRLF stays CRLF). A phone LF text therefore comes back from the PC as CRLF if the user sends it again (design note 2) |
| 10 | Security: external pairing link | **PASS** | A link opened with `am start -a VIEW` and **without** the extra shows "Pair with this PC?" with the PC name and the short ID, which equals the PC's (`ETHU-PUAP` = `ETHUPUAP`); the screenshot is [phase-1-2-integration-screens/01-confirm-pairing.png](phase-1-2-integration-screens/01-confirm-pairing.png). Nothing is paired while it is up (4 s: `not_paired`, no device, the code is still valid; nothing about a pairing in the PC log), Cancel pairs nothing, and a later tap on **Confirm** pairs (connected in 2 s). With the extra it pairs by itself (the 1.1 flow) |
| 11 | Logs | **PASS** | PC log, phone log file, logcat and `status.json` searched for every text used (and three words of each), for the pairing token, `tk=` and `PRIVATE KEY`: no match. They do hold lengths and 8-digit hashes. Only the debug activity `DebugClipboardActivity` prints a text to logcat (`CMD CLIPBOARD`, by design, debug builds only); the script leaves those lines out |

## Bugs found and fixed

| # | Where | What | Fix |
|---|---|---|---|
| 1 | `android/…/link/IncomingHandler.kt:24,33,37` | **The phone acks a `text` or `clipboard` it can't copy and says nothing.** The clipboard is a binder call of about 1 MB. On API 37 an ASCII text of 1,040,000 characters works and one of 1,048,502 fails with `TransactionTooLargeException` ("data parcel size 1048844"); on **API 29 the text goes into the parcel as UTF-16, so 500,000 characters work and 1,000,000 fail** ("data parcel size 2000220"), which puts the limit near 520,000 characters there. The spec allows 1 MiB, the PC sends it, the PC is told "acked", the notification for a `text` is shown, and Paste on the phone gives the old text. The log said `written=false` and nothing else | A failed write now posts "Couldn't copy the text from your PC. It may be too long for the phone's clipboard." (the same problem notification as a failed send), for both `text` and `clipboard`. Tests: `TextFlowTest.kt:223` (extended) and `:233` (new, for `clipboard`). Verified on both emulators by the script (S8) |
| 2 | `tools/e2e-1-1.ps1:130` | The pairing link had no `AUTO_CONFIRM` extra (known item) | Added |
| 3 | `protocol/README.md:108`, `protocol/fixtures/android-client.json:2` | Stale fixture path (known item) | Updated |

Not a bug, but it looked like one: in one API 29 run the phone clipboard, read 0.5 s after the hotkey `text` was acked, still held the older text. The ack goes out before the clipboard is written (the spec: ack "as soon as the message is valid and recorded"), so a read right after an ack can be early. The script now waits for the text (`Wait-PhoneClip`). I did not measure how long the write takes after the ack (the debug read needs about a second); a number for "on the phone's clipboard" rather than "acked" would need an in-app timestamp.

## Spec problems and decisions (the spec's behaviour is not changed)

1. **The 1 MiB limit is higher than Android's clipboard.** protocol.md allows a `text` or `clipboard` frame of 1 MiB; an Android phone cannot set its clipboard to more than about 1 MB (API 37, ASCII) or about 520,000 characters (API 29). Suggest: say so in the table next to `clipboard` ("a receiver may be unable to copy a long text and says so"), and let the file route of 1.3 take texts over a few hundred thousand characters. The apps now behave (bug 1); the limit stays a decision for the spec.
2. **Line ends are not specified.** The PC turns a bare `\n` into `\r\n` when it puts a phone text on the clipboard and sends the clipboard text to the phone unchanged (the desktop agent's reading). The result is not a round trip: phone `a\nb` → PC `a\r\nb` → (copy again) → phone `a\r\nb`. The spec should say who normalises. Not a bug for people, but a byte-exact test must expect it.
3. **`isUrl` is loosely specified** ("one absolute http or https URL with no whitespace"). Both sides implement it the same way (see scenario 6) and both accept `http://.` and `https://example.com:99999/` as URLs. The receiver re-checks and opens only http(s), so nothing unsafe, but the browser gets an invalid link. One sentence on host and port syntax would remove the guess.
4. **"Stored in history"** in the `text` row of the message table: there is no history before 1.4, so both apps only keep the de-duplication memory. (Known from both reports.)

## Open issues

1. **Not verified end to end:** a real toast and the click on it that opens the link (`--dev-no-windows` turns toasts into log lines), a real `Ctrl+V` into an app (the script reads the clipboard that Ctrl+V pastes), a real `Ctrl+Alt+Shift+K` press in the integration run (the desktop agent did it), the phone side of "too long for now" (see 8), a real phone, mDNS, Windows 10, the quick-settings tile click (the Android agent did it; the script uses the alias and the real notification button), OEM battery managers.
2. **Phone → PC at exactly the limit** (1,048,493 characters) was not driven end to end: nothing can put that much text on the phone. The 1,040,000-character case and `TextFlowTest`'s boundary tests stand in for it.
3. **After a PC restart the phone can take up to the next backoff step (5 s, then 30 s) to come back**, so a "send to PC" right then says "PC not reachable" although the PC is up again a moment later. It asks the loop to try now and waits 4 s, so this only happens when the PC is down for a while. 1.4's queue is the answer.
4. **The stale "connected" on the PC after the phone loses Wi-Fi** (1.1 open issue 1) is still there; I did not touch it.
5. **uiautomator is flaky on the emulators** ("could not get idle state", "null root node"); the script retries the taps, and an adb server fight ("daemon not running", "device offline", the known second adb on this PC) showed up in one run and the scenarios carried on. The notification-button scenario takes about 40 s when the dumps fail.
6. **The script's check for text in the logs** looks for each whole text and for three words of it; it can't prove the absence of every transformation of a text (a base64 of it, say). The logs hold only lengths and hashes by construction (both agents' tests check this too).
7. **Big texts are slower:** a 1,000,000-character `clipboard` took 664 to 688 ms tap to ack on API 37 (a 1,040,000-character text from the phone: 454 ms). Inside "under 1 s", but the target is meant for ordinary clipboard text.

## Tests after the changes

| Suite | Result |
|---|---|
| `cargo test` (`desktop/src-tauri`) | **158 passed** (81 unit, 44 fake phone, 9 server limits, 12 `tls_vectors`, 12 `vectors`), 0 failed, 1 ignored (the real-clipboard test). No desktop code changed in this run |
| `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` | clean |
| `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` | BUILD SUCCESSFUL, **181 tests, 0 failed, 1 skipped** (the opt-in fake PC; 180 before this run, plus the new `aFailingClipboardWriteOfAClipboardMessageTellsTheUser`), lint "No issues found", release APK 3,486,675 bytes |
| `bun protocol/check-vectors.ts` | OK, 5,265 checks |
| `tools\e2e-1-1.ps1` | OK on `kin_api37` (16 checks) |
| `tools\e2e-1-2.ps1` | OK on `kin_api37` (79 checks) and `kin_api29` (78 checks), exit code 0 |

## `tools/e2e-1-2.ps1`

```
powershell -ExecutionPolicy Bypass -File tools\e2e-1-2.ps1                 # kin_api37
powershell -ExecutionPolicy Bypass -File tools\e2e-1-2.ps1 -Avd kin_api29
powershell -ExecutionPolicy Bypass -File tools\e2e-1-2.ps1 -Build          # builds Kin.exe and the debug APK first
powershell -ExecutionPolicy Bypass -File tools\e2e-1-2.ps1 -KeepRunning    # leaves the emulator and Kin up
powershell -ExecutionPolicy Bypass -File tools\e2e-1-2.ps1 -DiscardClipboard   # the PC clipboard holds an image or files: don't refuse
```

It is `tools\e2e-1-1.ps1 -Milestone 1.2` (the 1.1 script keeps the emulator, install, pairing and teardown, and gets hook points; `-Milestone` defaults to `1.1`, which behaves as before), with the scenarios in `tools/e2e-1-2-scenarios.ps1` (dot-sourced; `Run-Milestone12` at the `EXTEND HERE` marker, `Test-ConfirmationScreen` before the pairing, `Test-ConfirmByTap` after the unpair). 1.1's checks run too, so a run is 79 checks, in about 3 minutes.

- **The real clipboard:** `Save-PcClipboard` saves the plain text before anything touches it (and writes a copy to `%TEMP%\kin-e2e-clipboard-backup.txt`), and refuses to run if the clipboard holds more than plain text (`-DiscardClipboard` overrides). `Restore-PcClipboard` puts it back **after Kin has quit**, so the restored text is not synced to the phone, and checks it byte for byte ("PC clipboard restored" is a check, so a failed restore fails the run and prints where the backup is).
- **Never touches the network beyond loopback, never `%APPDATA%\Kin`**; refuses to start if a `Kin.exe` is running; the teardown (also on failure) quits Kin, runs `stop.ps1 -All`, kills the adb server and removes the data dir (kept, with its path printed, when a check failed).
- **Prints "Arrival times"** at the end (tap to ack of PC → phone `clipboard`, the hotkey `text`, the four phone paths, the largest `text`).
- The screenshot of scenario 10 goes to `docs/results/phase-1-2-integration-screens/` (`-ScreenshotDir` changes it).
- Not in the script (done by hand, results above): 7 (PC killed and restarted), 9 (Unicode and line ends), the phone → PC large text, the `isUrl` edge-case comparison, and the API 29 scenarios 7 and 9.
