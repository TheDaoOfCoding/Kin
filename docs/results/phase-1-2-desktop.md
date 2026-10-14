# Phase 1.2 result: the Windows app (text, links and the clipboard)

Built and measured 2026-10-14 on Windows 11 Pro 10.0.26300, Ryzen 9 7900X, 31 GB RAM (the PC of [phase-1-1-desktop.md](phase-1-1-desktop.md)). Code is in `desktop/` (see [desktop/README.md](../../desktop/README.md)). This is the Windows half of milestone 1.2 ([phase-1-mvp.md](../phases/phase-1-mvp.md)); the Android half was built in parallel from the same spec. Nothing here is committed.

## What was built

| Area | What it does | Where |
|---|---|---|
| Automatic clipboard sync, PC → phone | On by default (tray check item "Sync clipboard automatically", or `autoClipboardSync` in `settings.json`). A hidden message-only window listens with `AddClipboardFormatListener`. Every change restarts a 300 ms timer, and the text is read once when it runs out and sent as a `clipboard` message. Text only. A text over 1 MiB as a message is skipped and logged (the `.txt` file is 1.3). Not stored anywhere (D10), never logged as text | `clipboard.rs`, `sync.rs` |
| Password managers | Not sent: content with `ExcludeClipboardContentFromMonitorProcessing`, or `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` with value 0 (and the older `Clipboard Viewer Ignore`, an addition). `status.json` says which format excluded it | `clipboard::exclusion` |
| Loop prevention | The text put on the clipboard for the phone is hashed first, and the change event it causes is recognised (for 10 s) and not sent back. A different text clears the memory, so copying the same text again later is a new copy | `clipboard::LoopGuard` |
| Hotkey | `Ctrl+Alt+Shift+K` by default, `hotkey` in `settings.json` to change it (read at start; empty turns it off; a bare key with no modifier is refused). Tauri `global-shortcut` plugin. Sends the clipboard text as a `text` message. The tray item "Send clipboard to phone" and `kin.exe --send-clipboard` run the same code | `app.rs`, `settings.rs`, `tray.rs` |
| Receiving `text` | Acked first, then put on the clipboard (with `\r\n` line ends) without being sent back, and a toast "Text from <phone>" with the first 100 characters. If `isUrl` is set and the text really is one `http`/`https` URL, a click on the toast opens it in the default browser. The scheme is checked when the toast is made and again before `ShellExecute` | `sync.rs`, `links.rs`, `app.rs` |
| Waiting for the `ack` | Every `text` and `clipboard` has a UUIDv7 `id`. No `ack` in 5 s: the connection is closed, the phone shows as offline. An `error` instead of an `ack` fails the send with its code. A send while the phone is offline fails at once: a toast for an explicit send, a line in the log and `status.json` for an automatic one | `sync.rs`, `ws.rs`, `connections.rs` |
| Arrival time | `tapToAckMs` and `rttMs` for the last `text` and the last `clipboard`, in `status.json` and in the log. For the automatic sync the tap is the first change of the burst, so the 300 ms debounce is in it | `sync.rs` |
| Messages | The receiver now handles `text` fully and `clipboard` for the phone role (`set_clipboard`), so all the `text` and `clipboard` vectors run against it | `messages.rs` |
| Server limits | Timeouts for the TLS handshake, the headers, a JSON body and an idle connection; 64 connections in three pools so strangers can't use up the paired phone's | `server.rs` |

### status.json in 1.2

```json
{
  "settings": { "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+K" },
  "hotkey": { "accelerator": "Ctrl+Alt+Shift+K", "registered": true, "error": null },
  "lastSent": {
    "text":      { "id": "…", "at": 1791998181481, "result": "acked", "chars": 16, "trigger": "hotkey", "tapToAckMs": 3.4, "rttMs": 3.0 },
    "clipboard": { "id": "…", "at": 1791998181399, "result": "acked", "chars": 16, "trigger": "clipboard", "tapToAckMs": 312.5, "rttMs": 1.6 }
  },
  "lastReceived": { "text": { "id": "…", "at": 1791998175368, "chars": 29, "isUrl": true, "openable": true, "clipboardSet": true } },
  "clipboardSync": { "enabled": true, "sent": 3, "skipped": 3, "lastSkip": { "at": 1791998175671, "reason": "echo_of_phone_text" } }
}
```

`result` is `acked`, `no_reply`, `phone_offline`, `no_phone_paired`, `error:<code>` or `too_large`. `lastSkip.reason` is `password_manager:<format>`, `echo_of_phone_text`, `not_text`, `empty`, `too_large` or `read_failed`. No clipboard text is in it, or in the log: ids, lengths, times and the first 8 hex digits of the text's SHA-256.

## The hotkey: `Ctrl+Alt+Shift+K`

Checked against what is commonly bound, and written up in [windows.md](../windows.md#hotkey):

- **Windows keeps `Win+<key>`** (`Win+V` clipboard history, `Win+Shift+S` snipping tool, `Win+K` casting, …), so no Win combination.
- **Two-modifier shortcuts are what apps use** (`Ctrl+Shift+K` is the web console in Firefox, and editors bind many `Ctrl+Alt+<letter>` commands). A global hotkey beats the app in front, so it would break them. Three modifiers is where far fewer commands live.
- **`Ctrl+Alt+<letter>` is `AltGr+<letter>` on many layouts**, which types characters (`€`, `@`, Polish `ł`, `ś`, …). A global `Ctrl+Alt+<letter>` takes that character away. With `Shift` added it stays clear of the common ones.
- **Checked on this PC:** the real app registered it (`hotkey.registered: true`), and a real key press (`Ctrl+Alt+Shift+K` sent with `keybd_event`) made the phone receive a `text` message. It can't be checked on every PC, which is why it is configurable, and a registration that fails is reported (log, toast once, `hotkey.error` in `status.json`) and the app carries on without it.

Not verified: how it behaves with the apps people have that bind `Ctrl+Alt+Shift+K` themselves. Nothing on this PC does.

## Dev hooks for the integration agent

All of it is in [desktop/README.md](../../desktop/README.md#developer-and-integration-settings). With `$kin = '…\desktop\src-tauri\target\release\Kin.exe'` and a throwaway profile `$data`:

```powershell
Start-Process $kin -ArgumentList '--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-print-pair-url', '--dev-no-windows'
& $kin --dev-start-pairing                      # the pairing URL is in $data\pair-url.txt

# PC -> phone, automatic. Use PowerShell: it is the real clipboard, as any app sets it.
Set-Clipboard -Value 'hello from the PC'        # the phone gets a `clipboard` message ~300 ms later
# PC -> phone, explicit: the hotkey, headless (the hotkey, the tray item and this run the same code)
& $kin --send-clipboard                         # the phone gets a `text` message
# The password-manager case, which PowerShell can't set:
& $kin --dev-set-clipboard 'secret' --dev-clipboard-mark exclude     # also: no-history, no-cloud, viewer-ignore. Not sent.
# Phone -> PC: read what is on the PC clipboard
Get-Clipboard -Raw                              # or: & $kin --dev-get-clipboard ; Get-Content $data\dev-clipboard.txt -Raw
# Results, with the ids and times:
(Get-Content $data\status.json -Raw | ConvertFrom-Json) | Select-Object lastSent, lastReceived, clipboardSync, hotkey
& $kin --quit
```

- **Which to use:** `Set-Clipboard`/`Get-Clipboard` for ordinary text (the whole real path, including the Windows change event, and nothing to be running); the `--dev-` hooks only for what PowerShell can't do (the opt-out formats) or to read the clipboard as Kin does.
- A `text` from the phone lands on the clipboard with `\r\n` line ends (see the ambiguities below), so `Get-Clipboard -Raw` shows `\r\n` where the phone sent `\n`.
- The fake phone: `cargo run --example fake_phone -- "<pairing URL>" --host 127.0.0.1 --seconds 60 --send-text "hi" --send-text-after 3` (also `--no-ack`). It prints `received clipboard id=… text=…` and `received text id=… isUrl=… text=…`.
- `desktop/check-1-2.ps1` runs all of this against the real app and the real clipboard, restores the clipboard and leaves nothing running.

## Tests

`cargo test` in `desktop/src-tauri`: **158 tests pass, 1 is ignored** (it touches the real clipboard, see below). `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean. `bun protocol/check-vectors.ts` passes (5,265 checks, 19 more than in 1.1).

| File | Tests | What |
|---|---|---|
| `src/*.rs` (unit) | 81 (+1 ignored) | 1.1's 45, and the new: the opt-out format detection (the three formats, value 0 against 1 and 2, no readable value, case-insensitive names, the old `Clipboard Viewer Ignore`), loop prevention (echo recognised, several events for one write, a real copy clears it, expiry, a failed write), the debounce with a clock that moves by hand (one change, a burst, a new burst, changes that keep coming), text helpers (hash, `\r\n`, the toast preview), URL rules (`isUrl`, which links open, 31 refusals including `file:`, `javascript:`, `ms-msdt:`, control characters), settings (defaults, wrong types, byte order mark, the default hotkey and 6 bad ones, bare keys), the CLI hooks, the SPKI agreement (c), the in-memory clipboard |
| `tests/vectors.rs` | 12 | 1.1's 11, run against the real code, and the **1.2 scope of `messages.json`**: 14 valid + 24 invalid `text-*`/`clipboard-*`/`duplicate-keys-*` cases (including the exactly-1-MiB and multibyte-under-limit frames, and `text-from-pc` and `clipboard-from-pc` against the phone-role receiver) and the 3 sequences. A test pins the scope names and that no `offer`/`complete`/`cancel` case is in it |
| `tests/tls_vectors.rs` | 12 | As in 1.1, and the Android certificate test now **fails** when the fixture is missing (d) |
| `tests/fake_phone.rs` | 44 | 1.1's 20, and 24 new: a PC clipboard change reaches the phone as `clipboard` and is acked (with the debounce, tap-to-ack and nothing of the text in `status.json`); a burst is one message; the three password-manager formats are not sent, and `…=1` is; a clipboard with no text; sync off and the setting saved; over 1 MiB skipped; offline fails quietly; `text` from the phone sets the clipboard, gets a toast, is not sent back, a repeat is acked and not repeated; a copy of the same text later is sent; `\r\n`; Unicode and the preview; links open from the toast only for `http`/`https` and only once (`file:`, `javascript:`, `ms-msdt:`, `isUrl` false, trailing words); bad `text` and a `clipboard` from the phone; **the hotkey path** (`send_clipboard_text`, what the hotkey, the tray and `--send-clipboard` call) with `isUrl`, with a password-manager clipboard, with nothing to send, with no phone paired, with the phone offline; no `ack` in 5 s (connection closed, phone offline, notice); an `error` instead of an `ack`; a late `ack`; the log and `status.json` never hold the clipboard text; and for (a) five wrong codes and an expired code |
| `tests/server_limits.rs` | 9 | (b): half a header, a slow body (`400`), an idle connection, a connection that keeps talking, a WebSocket that is not cut off for being idle, a TLS handshake that never finishes, strangers filling their own slots and the paired phone still connecting, the per-address limit, paired connections not counting against it |
| ignored: `real_clipboard_round_trip_and_password_manager_formats` | 1 | The Win32 clipboard for real: writes and reads text with a new line and characters outside the BMP, sees each of the four opt-out formats, gets a change event. Run with `cargo test real_clipboard -- --ignored --test-threads=1`. It saves the clipboard text first and puts it back, and does nothing if there is more than plain text on the clipboard. Run once here, passed, clipboard unchanged |

The tests that were written to fail without the fix were checked that way: with the header timeout, the body timeout, the idle watchdog, the unpaired-slot limit, the paired pool and the per-address limit each switched off, the matching test fails (and only that one); with the Android fixture hidden, the (d) test fails.

### The real app, with the real clipboard and the real hotkey

`desktop/check-1-2.ps1` (release `Kin.exe`, loopback only, a throwaway profile, the fake phone). **All 35 checks passed**: five wrong codes shut the code down and `pairing.active` leaves `status.json`; `Set-Clipboard` reaches the phone as `clipboard` (tap to ack **315.5 ms**, round trip 1.6 ms, and exactly one message per copy though Windows sends several change events); a password-manager copy is skipped and its text is not in the log; `--send-clipboard` and a real `Ctrl+Alt+Shift+K` press each send a `text`; a `text` from the phone arrives on the clipboard with `\r\n` and is not sent back; a link text is marked openable; the log never holds clipboard text; a phone that never acks is `no_reply` after 5 s and shows as offline, and the next send fails with the notice. The user's clipboard (183 characters of plain text) was saved and put back, and nothing was left running. The **5-minute expiry** was also run for real: a pairing was started, `pairing.active` was true, and `status.json` dropped it after exactly 300 s, with the log line "pairing: the code expired".

## Idle memory and installer

Release `Kin.exe`, tray only, no window ever opened, 60 s after launch, `desktop/measure-idle.ps1`, three runs with a throwaway profile and the developer network settings (loopback, no mDNS), measured as in 1.1:

| Run | Working set | **Private working set** | Private bytes | Threads | Handles |
|---|---|---|---|---|---|
| 1 | 17.4 MB | **2.9 MB** | 4.1 MB | 29 | 222 |
| 2 | 16.9 MB | **2.8 MB** | 4.0 MB | 29 | 220 |
| 3 | 16.9 MB | **2.7 MB** | 4.0 MB | 29 | 220 |

That is the same as 1.1 (2.7 MB, 28 threads, 216 handles) against the target of under 30 MB ([targets.md](../targets.md)): the clipboard listener window, the hotkey and the new code cost about 0.1 to 0.2 MB and one thread. Same caveats as 1.1: not the 1-hour figure, no phone connected, no mDNS daemon, and a toast costs one waiting thread until it goes away (the notification plugin watches it for clicks), which this run didn't have.

| Size | |
|---|---|
| `Kin.exe` | 11,179,520 bytes (10.7 MiB) |
| NSIS installer `Kin_0.1.0_x64-setup.exe` | 3,402,549 bytes = **3.24 MiB** (1.1: 3.17 MiB), built not run. `desktop/build.ps1` takes about 2 minutes after a change |

## Carry-forward fixes from the 1.1 review

| | What | Where | Test |
|---|---|---|---|
| (a) | The pairing window stops showing the QR code and `status.json` drops `pairing.active` when five wrong codes shut the token down, and when it expires. `pairing_view` now asks `Pairing` (`active_expiry`, new `is_invalidated`) instead of comparing its own clock, a new status `locked` says "Too many wrong attempts", the fifth failure emits `PairingChanged`, and the app wakes up at the token's expiry (`pairing_deadline`, `pairing_expired`) to tell the window and rewrite `status.json` | `core.rs:361-381` (`pairing_view`), `core.rs:394,417` (`pair`), `core.rs:339-347`, `pairing.rs:79`, `app.rs:147-156`, `src/pairing.ts` | `fake_phone.rs`: `five_wrong_guesses_stop_the_qr_code_and_clear_pairing_active`, `a_pairing_code_that_runs_out_clears_pairing_active_and_the_window`; the script, step 0 |
| (b) | Timeouts and reserved capacity. TLS handshake 10 s, headers 10 s (hyper), a JSON body 15 s (`400`, connection closed), 30 s without a byte on a connection that isn't a WebSocket (a watchdog around the connection that ends at the upgrade). Three pools: 16 handshaking, 24 unpaired (a stranger with a certificate of its own), 24 paired-only; a connection moves to the paired pool once its certificate is a paired device's. At most 8 handshaking or unpaired connections per address. The refusal is logged at most every 10 s. Everything is in `Limits`, so tests use short ones | `server.rs:56` (`Limits`), `server.rs:251` (body), `server.rs:397-495` (`Slots`, `Admission`, `settle`), `server.rs:505` (`Activity`), `server.rs:553-610` (`serve_connection`, the watchdog) | `tests/server_limits.rs` (9) |
| (c) | The device ID comes from the SPKI that webpki verified the signature against, and the verifier refuses a certificate on which webpki and x509-parser disagree byte for byte | `tls.rs:45-62` (`verify_client_cert`), `tls.rs:112` (`agreed_spki`), `tls.rs:121` (`client_spki`), `tls.rs:128` (`peer_device_id`) | `tls.rs` unit tests |
| (d) | The Android certificate test fails when `protocol/fixtures/android-client.der` is missing (renamed `android_generated_certificate_is_accepted`) | `tests/tls_vectors.rs:263-265` | checked by hiding the fixture |
| (e) | "Any host is safe because the phone pins the PC" is replaced: pinning protects against a spoofed host, not an attacker-made link, so the phone must confirm a pairing link that arrives from outside the app, with the PC name and the short ID. Changelog item 31 | `docs/protocol.md:93-94`, `docs/protocol.md:443` | n/a (the Android agent implements it) |
| (f) | The note that said a 1.1 receiver acks `text` is rewritten (it was a 1.1 note), and a "Milestone 1.2" scope table is added; `check-vectors.ts` checks the 1.2 names (`scope-1.2`) | `protocol/README.md:28-38`, `protocol/check-vectors.ts` | `bun protocol/check-vectors.ts` |

## Spec ambiguities and decisions

Nothing was changed silently. Where the spec doesn't say, this is what the PC does (all easy to change):

1. **Line ends.** protocol.md doesn't say. A `text` from the phone goes to the Windows clipboard with a bare `\n` turned into `\r\n` (older Windows apps show a bare-`\n` text as one line), and the loop guard hashes what was written. Text sent **to** the phone is not changed. If the integration test sends multi-line text and compares exactly, it will see `\r\n`.
2. **`isUrl` on send.** The spec's "the whole text, trimmed, is one absolute http(s) URL with no whitespace" is implemented as: the scheme (any case) is `http` or `https`, there is a host, and no Unicode whitespace or control character anywhere in the trimmed text. The text itself is sent untrimmed. The Android side may draw the line elsewhere (a different idea of a valid host); it only matters for whether "Open" shows, and **the receiver re-checks the text**: on the PC a link opens only if `isUrl` is set **and** the text really is one http(s) URL.
3. **The password-manager rule is for the automatic sync.** windows.md lists it under the automatic sync. The hotkey sends what is on the clipboard even if it carries an opt-out format, because the person pressed it. Say if the explicit send should skip them too.
4. **Extra opt-out format.** `Clipboard Viewer Ignore` (KeePass and others) is also skipped. It is not in the milestone text. And a value format (`CanInclude…`, `CanUpload…`) with no readable value counts as 0: when in doubt, don't sync.
5. **"Fails with a visible notice" while the phone is offline.** An explicit send (hotkey, tray, command) gets a toast. An automatic sync would be a toast on every copy while the phone sleeps, so it only gets a log line and `status.json` (`lastSent.clipboard.result: phone_offline`).
6. **A send that got no `ack` is dropped**, not re-sent after the phone reconnects (protocol.md says a sender re-sends with the same `id`). That is the queue, which is 1.4. The same for a clipboard text over 1 MiB, which waits for 1.3's `.txt` file.
7. **The `ack` doesn't wait** for the clipboard write or the toast (protocol.md: ack when valid and recorded). If the clipboard is busy for 200 ms the text is not on the clipboard, a toast says so, and the phone has already been told it arrived.
8. **A slow body is `400`**, because the error table has no timeout code.
9. **Hotkey syntax.** A bare key (no modifier) is refused, because it would be taken from every program.
10. **History.** The table in protocol.md says a received `text` is "stored in history". There is no history before 1.4, so nothing is stored; the text is on the clipboard and in the toast only.
11. **The 5 s `ack` timeout closes the connection** (architecture.md, and 1.1's rule for `ping`), for `text` and `clipboard` too. The automatic sync is serial: while one `clipboard` waits for its `ack` the next copy waits behind it (an `ack` takes a few ms on a good network).

## Not done, or not verified

- **A real toast.** Toasts were off in every run (`--dev-no-windows`: the toast is a log line). That a toast shows, and that **a click on it opens the link**, is not verified: the click handler is the plugin's `on_action`, which reports a click only while the toast is on screen, and `ShellOpener::open` (the `ShellExecute` call) is not exercised by any test, because it would open a browser on this PC. What is tested: the link and scheme rules (unit tests), that a link toast is made only for a real http(s) link, that `toast_clicked` opens it once and only through the opener (a recording one), and that the shell opener refuses everything else. As in 1.1, toasts from an unpackaged exe may carry PowerShell's name until the app is installed (phase-0-tauri.md).
- **The installed app, Windows 10, a real phone, mDNS**: as in 1.1. The installer was built, not run.
- **Hotkey conflicts with other programs** on other PCs.
- **The tray check item** ("Sync clipboard automatically") was built but not clicked: it is a native menu, and the setting it toggles is tested through `set_auto_clipboard_sync`.
