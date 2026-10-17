# Phase 2B.2 desktop: phone notifications on the PC

Date: 2026-10-17, desktop coder (Sonnet 5.5). Spec: [phase-2b-2-design.md](../phases/phase-2b-2-design.md) (the contract), [protocol.md](../protocol.md) "Notifications" and changelog **56**, [windows.md](../windows.md) "Phone notifications (2B.2)". Spike: [phase-2b-2-spike-s1.md](phase-2b-2-spike-s1.md). Nothing is committed. The Android side (`android/`, `docs/android.md`) was built at the same time by another agent and was not touched.

## Result in one paragraph

The desktop side of 2B.2 is done and checked against the real Windows on this PC: **a real dev Kin registers its own identity under HKCU, the fake phone posts, Windows holds the toasts (tag, group, the reply box and buttons, the private form, a replacement in place, a removal), a click done the way the shell does it (`CoCreateInstance` of the registered class and `Activate`, no input injection) delivers a reply, a button and Copy to Kin and the phone, the one-time code is on the real clipboard with the sensitive formats and gone after 60 s, nothing of a notification is in the log or on disk, and quitting leaves no toast and no registry key** (checked again 100 s later: Windows' second key did not come back). **The one link nobody can script is still open: a person's click making the shell call the class** (R21). The text-only fallback is one switch away until then.

## What changed

### Protocol (first, so the Android coder could start) — `docs/protocol.md`, `protocol/`

* **Changelog 56** (not 55: 55 was the 2B.1 review's) with the `notif` capability, the seven messages, the ephemeral class, `nid`, limits, offline = dropped, sync, dismiss and action rules, sensitive/redacted handling, icon transfer, the four new error codes (`stale`, `action_failed`, `disabled`, `rate_limited`), validation step 6b (silent drop unless both sides list `notif`) and the 32 KiB rule.
* **Vectors:** `protocol/notif.json` (`nid[]`, `iconHash[]`, `icon[]` with two fixtures of exactly 32,768 and 32,769 bytes, `clean[]` (69), `post[]` (79), `code[]`, `toastIds[]`, `toastArguments[]` (40), `rate[]` for the PC and the phone, `coalesce[]`, `sendNotif[]`, `syncDiff[]`, and the phone's own `otpDetect[]` (69: every language of the keyword table, every negative of 4.10), `redactedText`, `skip[]` (47), `redactedText` (16) and `pickActions[]` (20)); 113 + 251 + 33 cases in `messages.json` of which 39 + 61 + 10 are `notif-*` (cases have `own` and `peer`: the capabilities of the receiver's own `hello` and of the sender's). `check-vectors.ts` has the reference models and a `scope-2b.2` group; `protocol/README.md` documents the format. **9,086 checks OK** (7,400 before).
* The Rust receiver (`messages.rs`, `mirror/wire.rs`) passes all of it, **and so does the TypeScript model**, written separately: they agreed at the first run on the messages and arrays that both run.

### The mirror (`desktop/src-tauri/src/mirror/`)

| File | What |
|---|---|
| `wire.rs` | validation, cleaning, cutting and the normalized post of the `notif.*` frames (pure) |
| `limits.rs` | bucket 20 / 1 per s, identical post within 2 s ignored, one `rate_limited` per 10 s, 5 actions per 10 s (millitokens, no rounding) |
| `args.rs` | the identifiers on a toast (`kin=1&d=&n=&a=[&i=]`), read back strictly |
| `toast_xml.rs` | post → XML (escaped in one place, a small node tree, cut to 4,800 bytes), and a toast read back from Windows' history |
| `surface.rs` | the toast layer behind a trait: `WinSurface` (WinRT, one MTA worker thread) and `FakeSurface` |
| `activator.rs` | the COM class (`INotificationActivationCallback`, its factory, registered from its own MTA thread) |
| `registration.rs` | HKCU keys: the installed Kin's fixed names (`app.kinrelay.desktop.phone`, `{6B0A1D3E-2F54-4E8A-9C71-3D5B8E2A4F90}`) only in a normal release run; every other instance `...dev-<key>.phone` and a CLSID from the key, **all keys removed again** including Windows' two |
| `icons.rs` | the icon cache by hash (WIC decode, never overwritten, 200 files / 8 MiB) |
| `otp_clip.rs` | Copy (sensitive formats) and the 60 s clear (change counter + text, one step) |
| `mod.rs` | the ledger (memory), what the core does with a message, a click, an X; the settings; start and stop |

Also: `messages.rs` (new actions, the receiver's own capabilities), `connections.rs` (`ConnCommand::Notif`, the phone's capabilities), `ws.rs` (the `hello` follows the switch, a `notif.action` waits 5 s and **never closes the connection**), `clipboard.rs` (`sequence_number`, `clear_if_text_unchanged`), `settings.rs`/`prefs.rs`/`ui.rs`/`build.rs`/`capabilities/settings.json` (the five settings and two commands), `cli.rs` (`--dev-toast-quiet`, `--dev-toast`, `--toast-activated`/`-Embedding` ignored), `app.rs` (start, exit, hourly prune, `--dev-set mirror*`), `desktop/src/settings.ts` (the "Phone notifications" section), `windows/hooks.nsh` (the uninstaller removes the CLSID and AppUserModelId keys in both contexts), `Cargo.toml` (`windows-core = "=0.62.2"` direct, five more `windows` features), `examples/fake_phone.rs` (`--notif*`), `examples/toast_probe.rs` and `examples/link_address.rs` (new), `tools/check-2b2-desktop.ps1` (new).

### The user's decisions, as built

1. **Banking, authenticator and password apps:** the phone sets `sensitive`; the PC shows the app and "Private notification", no reply box, no buttons, no code (tested in the XML and against Windows' history).
2. **Text-only fallback:** automatic when the keys or the COM class can't be registered, and the switch "Text only (no reply box, no buttons)" (`mirrorTextOnly`).
3. **X dismisses on the phone:** only `Dismissed(UserCanceled)` of the **current** generation; setting "Closing a phone notification here also dismisses it on the phone", on by default (`mirrorDismissToPhone`). Timed out, hidden, suppressed, replaced and phone-removed send nothing (tests).
4. **No Companion Device Manager workaround:** a `redacted` post shows "Code hidden by Android" (the design said "Content hidden by Android" in 3.3 and 5.3; decision 4 names the code).
5. **Calls and alarms** are the phone's to skip (the `skip[]` vectors pin it); the PC shows whatever arrives.
6. (Android: `<queries>`.)

### Carry-over from the 2B.1 review

* (a) The PC link toast puts **the address (with ⚠) first**, the title under it, cut to 100 characters, with U+2028, U+2029, U+0085 and CR/LF as spaces; a 300-character title can no longer push the address out (`phonelinks::toast_body`, `display_title`).
* (b) A **text** with `isUrl` is shown like a link: "Link from <phone>", the real address first with ⚠, Open and Copy. Nothing is opened by itself (it never was), **and a click on the body of a link with a warning no longer opens it** (`toast_action`): only the Open button does (this applies to `link` toasts too). Three existing tests that expected the old text toast were updated; two new tests.
* (c) `tools/e2e-2b-1.ps1` S6: reads what the PC shows from `links::link_address` through the new `examples/link_address.rs` (`display_host` is gone), and expects a link with a warning **not** to be opened (and logged as such). It parses; **it was not run** (it needs the emulator).
* (d) `phase-2b-1-integration.md`: a line saying B1/B2 are fixed in changelog 55. (e) `phase-2b-1-desktop.md`: 503 → 504.

## Deviations from the design and things it left open

| # | What | Why |
|---|---|---|
| 1 | The changelog item is **56**, not 55 | 55 was taken by the 2B.1 review |
| 2 | A `notif.icon` frame may be **48 KiB**, other `notif.*` 32 KiB | A 32,768-byte PNG is 43,691 base64url characters: one 32 KiB limit would have made the allowed icon sizes impossible (found by the vector, `icon[]` `valid-exactly-32768-bytes`) |
| 3 | For a **reply**, `notif.action.label` is the post's `reply.hint` ("" when none) | The design said "the label the PC was shown" and "title ≠ label → stale", but the PC is never shown the reply action's title. The phone compares it with the cleaned remote-input label. **The Android coder must read changelog 56 (h)** |
| 4 | The reply and button **arguments carry the index**: `a=reply&i=<n>` | The design's example had `a=reply` only; the index is needed after a restart of Kin |
| 5 | The toast XML is built from a **small node tree with one escaping function**, then `LoadXml`, instead of `XmlDocument` setters | Testable without Windows; equally safe (a hostile-string test, a 4,800-byte fit test). The history reader (`offered_from_xml`) is its inverse |
| 6 | A **missing `ack` for `notif.action` does not close the connection** | The general rule closes it; for a user's press that would drop the link. Decided with the design's "never re-sent" |
| 7 | `messages`, `lines`, `actions`: empty entries are dropped first, then the newest 5 / first 3 are kept; `code` is checked **as sent** (not cleaned); a reply `text` is trimmed (White_Space) and used trimmed | The design left the order open |
| 8 | Windows' `NotificationMirroring::Disabled` | The spike's correction (`Disallowed` does not exist) |
| 9 | The generation counter is kept although replacing raises no `Dismissed` | The spike says it is not needed for replacement; it still protects a late event of a superseded object |
| 10 | Extra settings `mirrorTextOnly` (decision 2) | |
| 11 | The "no hint" placeholder of the reply box is `Reply` + U+200B | So a toast read back from the history can tell "no hint" from a phone whose hint is really "Reply" |
| 12 | A repeat click is recognized by its **arguments alone** (3 s), not by its inputs | The COM route and the toast object's route may report different inputs for one click; a reply must never be sent twice |
| 13 | Release identity needs `autostart_is_real()` and not a build folder; otherwise the mirror **does not start** (settings say why) instead of using a fallback name | A debug build in the installed Kin's data directory must never write the installed Kin's CLSID (it would point it at a development exe) |

## What was measured

### Real Windows (`tools\check-2b2-desktop.ps1`, the last of five runs; all PASS)

A real debug dev Kin (own data directory, `--dev-toast-quiet`: every toast to the Action Center, no popup, no sound; no game or anti-cheat running, only the Steam launcher), `fake_phone --notif`, `toast_probe`. **26 checks passed**, including: the dev identity is never the installed Kin's and `app.kinrelay.desktop.phone` was not written; Windows held exactly the three toasts (a fourth removed by the phone, one updated in place) with the phone's group, an input box and 3 buttons on one, the private form on the sensitive one; a **reply, a button and Copy through the COM class** reached Kin and the phone with the right index, label and text; clicks that are not ours (the private toast's button, an unknown phone, a button never offered, nonsense arguments) sent nothing; `status.json` counted them (numbers only); the code was on the **real clipboard with `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` and `CanUploadToCloudClipboard`** and was gone 63 s later; the **clipboard was saved before and restored after (9 formats, back as it was)**; the dismissal reached the phone; no notification text, reply, code or `nid` in the Kin log or any file of the data directory; the icon cache held one file named by its hash; after quitting Windows held no toast for the identity and **none of the four registry keys existed, nor 100 s later**; the installed Kin (PID 25796) was never started, stopped or restarted; no process of the run was left. The four earlier runs failed for reasons in the script and in the fake phone (a native `reg.exe` error under `$ErrorActionPreference = 'Stop'`; an uncompressed 96 x 96 icon over the 48 KiB frame limit, which got `too_large`: the protocol working; the history and the status read before the phone had sent everything; the locked database read without sharing), all fixed; each left the machine clean (no process, no key, the installed Kin untouched) and the temp directories were removed.

What this proves of the spike's open list: **a registered class is found through `HKCU` and called with Kin running** (in the test process and from another process: `toast_probe activate` is a separate process), the arguments and the reply text arrive intact, and COM-started launches are accepted by the command line parser. **Not** proven: the shell calling the class for a person's click, the look of a toast, the X, Windows 10 (below).

### Tests

* `cargo test` (debug, once, after the last change): **580 passed, 0 failed, 8 ignored** (504 before: **+76**: 43 unit tests in `mirror/`, 1 in `phonelinks`, `tests/mirror_2b2.rs` 24, `tests/vectors.rs` 7, `tests/fake_phone.rs` 1). `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean. `bun protocol/check-vectors.ts`: **9,086 checks OK**. `bun run build` (tsc + vite): OK.
* `tests/mirror_2b2.rs` (real core, fake phone over real TLS, fake toast layer, a clock the test moves): the capability follows the switch and a new `hello` goes out; post, update, remove and no ack; a repeated id is processed again; popup and expiry by importance, `silent`, `sensitive` and code; the settings shape the toast (text-only, hide content, sound); a flood of 30 is cut at 20 with one `rate_limited` error; an identical post within 2 s is ignored; 30 toasts per phone; feature off = dropped without a word; a phone that did not list `notif` is not obeyed; status; **sync after a restart of Kin**; icons by hash and bad ones dropped silently; unpair; reply, button and Copy with the exact frames, the user told what came of it; Copy leaves a clipboard the user changed alone; every refused click; the same click twice and too many clicks; **no answer in 5 s is said, not retried, the connection stays**; a click **after a restart** is checked against the history; the X only for the exact signal, the right generation and when asked; the developer commands; **no notification text in the log, the data directory or `status.json`**; and **the switch turned on for real under a throwaway identity** (keys written, toast layer started, the COM class registered and **called through COM from the test**, switched off, every key gone).
* Not run: the e2e against the emulator (the integration agent's `e2e-2b-2.ps1`), the installer (not run, as always), Windows 10.

### Build

`desktop\build.ps1` (release, NSIS): `Kin_0.1.1_x64-setup.exe` **4,141,572 bytes** (3.95 MiB), SHA-256 `F4D446DDE07B758C98FB7225E211432EC7128B24FE0E9AB0FD760DD3EFB7EC88`, `Kin.exe` 13,642,752 bytes (unsigned, built after the last code change). Version stays 0.1.1 until 2B.6. Not installed.

## Manual checklist for the user's later real-click test (R21)

Run on the dev PC with a real phone at 0.2.0, **after** `tools\check-2b2-desktop.ps1` is green (or with the spike's `manual-check.ps1` for the 1-minute version):

1. Settings → Phone notifications → the switch **on**. Windows Settings → System → Notifications now lists **"Kin: phone notifications"** with Kin's icon (the first time a toast of the identity is shown).
2. Post a notification on the phone from a messaging app that has a reply (WhatsApp, Signal, Telegram, Google Messages). It appears on the PC with the app's icon (a circle), title, one or two lines, "<app> · <phone>", a reply box and **Send**.
3. Type a word and click **Send** while it is a popup; then with the toast in the **Action Center** (Win+N); then after **quitting and starting Kin again**; then with Kin **not running** at all (Kin should start, connect and send within ~10 s). The phone must send the reply. **If nothing happens on a click, the shell does not call the registry-form activator on this build: turn on "Text only" and record it in R21.**
4. **Copy**: a notification with a code (a bank/test SMS): click Copy, paste somewhere, wait a minute, paste again (empty); copy something else in between and check it is left alone. Check Windows' clipboard history (Win+V) does not keep the code.
5. The **X**: close a popup, a toast in the Action Center, and use "Clear all": the phone's notification should go for the first two (setting on); **record whether the Action Center ones do** (the known gap).
6. A banking/authenticator app the phone flagged: the PC shows only "Private notification". Turn on "Hide notification content": every one does.
7. Android 15+ with a code: "Code hidden by Android".
8. Look: wrapping of a long title, five buttons, the icon, the toast with no text but "Private notification", dark and light theme, Focus assist / presentation mode (the toast should be held back by Windows).
9. Windows 10 (if there is a machine).
10. Turn the switch **off**: the toasts and the "Kin: phone notifications" entry's keys go (`toast_probe registry`), and a click on an old toast does nothing.

## For the reviewer

* The risk I would look at first: `ws.rs` (the `Notif` command, the action timer, `Timer::ActionAck`/`End::Continue`), `mirror/mod.rs` (`mirror_activation_inner`: the checks before a press), `registration.rs` (the guard that keeps the installed Kin's names out of every other run), and `hooks.nsh`.
* The Android coder should read changelog 56 (h) and the 48 KiB icon frame. The Android agent already runs `notif.json` (`NotifVectorsTest`); the `otpDetect[]`, `skip[]` and `pickActions[]` models were written from the design and **checked against the Android code only by reading it** (I did not run Gradle while that agent was building): any difference is a finding for the integration run.
* Not done on purpose: no sparse MSIX, no polling of the Action Center, no Companion Device Manager, no change to the Phase 1 toasts or `register_toast_clicks`.

## Review round 1 fixes

Date: 2026-10-17, desktop coder. The Opus reviewer's round 1; its blocker was on Android (another agent). Still uncommitted. **No release build, no installer, no emulator and no `desktop\build.ps1` was run** (the integration agent was running the release `Kin.exe`): only debug `cargo test`, clippy, fmt and `check-vectors`.

### Desktop

| # | Finding | Fix |
|---|---|---|
| 1 | A reply over 2,000 characters was not checked on the PC (the phone answers `bad_request`) | `mirror_activation_inner`: refused on the PC with "That reply is too long (N characters; at most 2000 are sent). Shorten it and press Send again." Nothing is sent, the click is forgotten so the shortened reply can go at once. 2,000 exactly is sent |
| 2 | The action limit was skipped for a phone with no entry (`is_none_or`) | `mirror_send_action` gives the phone an entry first, so the 5-per-10-s limit counts from the first press (also after a restart of Kin, when there is no status and no ledger yet) |
| 3 | The `hello` for a change of the mirroring setting went to every connection | New `Connections::send_hello_to_capable("notif")`: only a peer whose latest `hello` lists `notif` gets it (a name change still goes to all) |
| 4 | `too_large` said "32 KiB" for icons too | `A <type> message can be at most <32 or 48> KiB.` |
| 5 | The post limiter was per phone, the spec said per connection | **Per phone kept (the safer: closing and re-opening the connection can't refill the bucket)**; protocol.md, windows.md, the design and the code comment now say per phone |
| 6 | A second, different reply within 3 s was dropped silently by the click de-dup | The de-dup remembers the reply text of the click. A repeat with the same text is still silent (the COM and the toast-object routes of one click); a **different non-empty text** is not sent and the user is told "A reply to this notification was sent a moment ago. Wait a few seconds, then send this one." A click that sent nothing (empty box, too long) is forgotten at once |
| 7 | `surface.show()` could block a runtime thread up to 10 s | `mirror_post` is `async` and `show` runs in `spawn_blocking`; it is awaited by the connection that delivered the post, so the posts and removes of one phone stay in order |
| 8 | After a restart a hint/label with U+2028/U+2029/U+FFFE read back from Windows' history with a space, so the phone answered `stale` | New `toast_xml::toast_label` (U+2028, U+2029, U+FFFE, U+FFFF become U+0020, the ends are trimmed). The toast text, the ledger and the `label` sent back all use that one form, so memory and history agree; a button whose label is empty in that form is not offered. **The phone must apply the same function to its own label before it compares** (spec below): this needs a small Android change (see "For the integration run") |
| 9 | Docs: what an uninstall leaves | `windows.md` (Uninstall): the toasts in the Action Center stay until they expire or Windows restarts, and the icon cache `%APPDATA%\Kin\notif-icons\` stays until the application data is deleted |

### Spec and docs

* `protocol.md`: (a) a **sensitive** post carries `app` and a `title` = the app name only; (b) `notif.action`: the sender may send the hint up to 60, the receiver cuts it to 30; (c) the **toast form** of a label (item 8) and the phone's comparison; (d) the post bucket is per phone; (e) changelog 56 (i) records all of it. `protocol/notif.json` has a new `toastLabel[]` (12 cases, both sides run it), `check-vectors.ts` a model of it, `protocol/README.md` the description, `tests/vectors.rs` the Rust runner: **9,123 checks OK** (9,086 before).
* The design (`phase-2b-2-design.md`): "Content hidden by Android" is now "Code hidden by Android" (3.3, 5.3); `NotificationMirroring` `Disallowed` is `Disabled` (5.3, T3); "changelog 55" is 56; 4.10 says "more than 10 characters closer"; 4.4 rule 10 says `getLockscreenVisibilityOverride()` is API 31; the post bucket is per phone.
* `TESTING.md`: on Android 10 and 11 the "Open" buttons from the PC are offered and reported sent, but Android silently blocks them.

### Tests

New: `tests/mirror_2b2.rs` +6 (a reply over 2,000 and exactly 2,000; a second different reply within 3 s; the action limit from the first press with no entry; the `hello` only to a peer that lists `notif`; the `too_large` text per type; a hint and a button label with U+2028/U+2029/U+FFFF that come back from the history with spaces, checked after a restart), `toast_xml` +1 (`toast_label`), `tests/vectors.rs` +1 (`toastLabel[]`). `cargo test` (debug): **588 passed, 0 failed, 8 ignored** (580 before); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean; `bun protocol/check-vectors.ts`: 9,123 checks OK.

### For the integration run

* **The release build and the installer must be rebuilt (`desktop\build.ps1`) after the integration agent has finished**: the `Kin.exe` and `Kin_0.1.1_x64-setup.exe` (and the SHA-256) in "Build" above are from before this round and do not have these fixes.
* **Android**: apply the toast form to the label comparison (`ActionHandler`: `clean(toastForm(own label), 30)` against `clean(toastForm(received label), 30)`, vectors `toastLabel[]`) and run `toastLabel[]` in `NotifVectorsTest`. Until then a hint with U+2028, U+2029, U+FFFE or U+FFFF gets `stale` from a phone that compares the raw text (before this round it worked until a restart of Kin, and then failed; it now fails in both cases, so the Android side must follow).
* One recovery note: `protocol/notif.json` was truncated to 0 bytes by a bad edit script of mine mid-round (it is not in git) and restored from the Android build's copy (`android\app\build\...\processDebugUnitTestJavaRes\out\notif.json`, made 09:00 with the same size and the same `check-vectors` result: 9,086 checks OK before my additions). Nothing else was lost.
* Not changed: the per-phone limiter has no reconnect test of its own (the limiter is a pure type; the behaviour is that `Phone` outlives a connection).
