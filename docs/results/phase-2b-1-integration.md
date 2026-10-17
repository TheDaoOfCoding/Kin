# Phase 2B.1 integration: links and the browser extension, desktop and Android together

Agent: Sonnet 5.5, integration/test. Date: 2026-10-17. Entry point: [tools/e2e-2b-1.ps1](../../tools/e2e-2b-1.ps1) (+ [tools/e2e-2b-1-extension.ts](../../tools/e2e-2b-1-extension.ts) for S5). Nothing was committed and **no product code was changed**. The desktop and the Android side were built in parallel against the spec; this is the first time they talk to each other.

What ran: the real `Kin.exe` of `desktop\build.ps1` (release, 04:17 build, after the last desktop change) as a test Kin beside the installed one (own `--data-dir`, `--dev-bind 127.0.0.1`, `--dev-no-windows`, **`--dev-no-open`**, pipe `app.kinrelay.dev.<SID>.e2e2b1`), the real debug APK (03:59 build) and the throwaway `link-target` app on `kin_api37` (Android 17), the real dev `kin-native-host.exe` (driven over stdin/stdout, and by headless Chromium in S5), `fake_phone` and a `Kin.exe` built from the **v0.1.1 tag** (the "old PC", S4). Lean testing: each scenario once, one re-run of S4b after a script fix, one quick `kin_api29` pass of the link-open path.

## Result

| # | Scenario | Result |
|---|---|---|
| S1 | Pair; both hellos carry `capabilities: ["link"]` | **PASS** (6 checks) |
| S2 | PC → phone links (real host → real Kin → wire → phone): title + real domain, tap opens the owning app, Copy link, group of 3, lock-screen setting, YouTube `t=` unchanged | **PASS** (22 checks) |
| S3 | Phone → PC links via the share sheet: "Open links from phone automatically" on (opener asked) and off (offered, not opened), history | **PASS** (14 checks), two limits noted (the toast buttons) |
| S4 | Fallbacks: PC → 0.1.x phone (text with `isUrl`), 0.2 phone → 0.1.1 PC (text), links queued while the peer is away, delivered in the right form after reconnect | **PASS** (10 + 10 checks) |
| S5 | Extension end to end (headless Chromium, throwaway profile, dev host key only) → real Kin → the **emulator**: selection, link, page, YouTube `t=`, image by URL, image needing bytes (`IMAGE_NEEDS_BYTES`, 5 MiB too), `data:` image, unreachable image, bad links, 200 KiB text, popup send, phone offline → queued → delivered, Kin not running | **PASS** (16 steps + the registry key check) |
| S6 | URL edge cases on both sides (all 46 of `protocol/links.json` + 14 display cases) | **PASS on the rule and the bytes** (8 checks); **interop bugs in what the PC shows, see below** |
| S7 | v0.1.1 transfers: one file each way + a 10-file batch each way, SHA-256 equal | **PASS** (9 checks) |
| Api29 | `kin_api29`: S1 + the link-open path from the real PC (resolve check, owning app opens, "Open link" for an unowned link, no crash) | **PASS** (20 checks) |

The full run (`tools\e2e-2b-1.ps1`, no arguments, about 15 minutes) passed every check except one in S4b, which was a **script bug** (below); S4b was run again after the fix and passes. The installed Kin (PID 25796, the same process before and after) was never started, stopped or asked, the dev registry key was removed (checked after S5 and at the end), no process of the run is left (the script checks that itself now), the PC clipboard was saved in every format and restored.

## Bugs and interop findings (not fixed, for the reviewer)

### B1 (security-relevant, real): the PC shows a different host than the one a browser opens, for a backslash in the authority

`https://evil.example\@google.com` passes the openable-URL rule on both sides (a backslash is neither whitespace nor a control character), so Kin accepts it, sends it and **opens it automatically by default**. A browser (WHATWG) treats `\` as the end of the authority and goes to **`evil.example`**.

| What | Shows / goes to |
|---|---|
| A browser (bun `new URL`) | `evil.example` |
| The phone's notification (line under the title) | `⚠ evil.example`, label "Open link" |
| **The PC's toast** (`display_host`, run on the URL) | **`google.com`**, no mark |

- Repro: S6 (`x-backslash-authority`, `x-backslash-authority-slash`); by hand: share `https://evil.example\@google.com` from the phone to Kin with "Open links from phone automatically" on (the default): the PC opens it in the default browser and the toast says `google.com` under the title.
- Where: `desktop/src-tauri/src/phonelinks.rs:38-47` (`display_host` splits the authority at the first `/ ? #` and then at the last `@`, so `evil.example\` is taken for user information); the rule it follows is protocol.md "Links: `isUrl` and openable URLs" step 4, which the spec also lets through. The phone avoids it by showing the browser's reading and marking the URL.
- Suggested fix (spec, then both apps): the rule rejects a `\` before the first `/`, `?` or `#` (the Android agent's suggestion); and/or the PC shows the WHATWG reading and marks odd addresses like the phone does. Until then the PC's "real domain" line can be spoofed, and with auto-open the user doesn't even get to look at it.

### B2 (same family, lower severity): the PC never marks or normalises odd hosts; the phone does

| URL | A browser goes to | PC toast shows | Phone shows |
|---|---|---|---|
| `https://аррӏе.com/` (Cyrillic) | `xn--80ak6aa92e.com` | `аррӏе.com` (looks like apple.com) | `⚠ xn--80ak6aa92e.com` |
| `https://paypaӏ.com/` (palochka for l) | `xn--paypa-iof.com` | `paypaӏ.com` | `⚠ xn--paypa-iof.com` |
| `https://ｇｏｏｇｌｅ.com/` (fullwidth) | `google.com` | `ｇｏｏｇｌｅ.com` | `⚠ google.com` |
| `https://bücher.example/` | `xn--bcher-kva.example` | `bücher.example` | `⚠ xn--bcher-kva.example` |
| `https://goog%6Ce.com/` | `google.com` | `goog%6ce.com` | `⚠ goog%6ce.com` |
| `https://0x7f.1/` | `127.0.0.1` | `0x7f.1` | `0x7f.1` |
| `https://[::ffff:127.0.0.1]/` | `[::ffff:7f00:1]` | `[::ffff:127.0.0.1]` | `[::ffff:127.0.0.1]` |

The PC shows the host as written (lower-cased ASCII only), so a homograph looks like the real site and opens automatically; the phone converts to what the browser will use and warns. Same cause and place as B1 (`display_host`). The last two rows are the same on both sides (neither shows the browser's reading, neither marks it): cosmetic. For user information (`https://google.com@evil.example/`, `https://google.com:443@evil.example/`, `https://example.com:8443@evil.example/`) both sides show `evil.example`; the phone adds `⚠`, the PC doesn't.

### Everything else matched

- **Rule parity** (all 46 `links.json` cases): the host/Kin accepts exactly the 13 valid ones and rejects the 33 invalid ones (`BAD_URL`); every accepted URL is on the phone **byte for byte** (YouTube `t=`/`list=`/`#`, percent escapes, userinfo+port, IPv6, IDN, trailing dot, `:080`, 8,192 bytes), nothing rejected reached it. Phone → PC: the phone makes a `link` of exactly the shares that are one web link after trimming (the 13 valid ones plus leading space / trailing space / leading newline / trailing U+3000 / leading space on an 8,192-byte URL: a sender trims, as the spec says) and the PC accepted and opened all of them with the right length (2 × 8,192 characters); everything else (the other 33, including 8,193 bytes) went as a **text with the exact bytes** (NBSP, NEL, tab, 0x01 and 0x7f inside checked in the phone database). 32 opens for 32 links, 0 for the rest.
- Capabilities both ways, old phone, old PC, queue: S4. The form is chosen when the message goes on the wire: the same queued link went to the old fake phone as a text and, after the next reconnect with a capable phone, as a link with its title. The 0.1.1 PC got the phone's link as a normal text (`isUrl`, no title), and the phone's own history keeps it as a link.
- The URL and title are not in the PC log (S3, S4a); the PC log shows lengths only.

### Smaller findings

1. **Hellos aren't logged by either side.** The capability exchange is proven by behaviour (the phone stored the PC's send as a link, the PC logged a link from the phone), not read from a hello. A one-line log of the peer's capabilities (no secrets) would make this checkable.
2. **The PC history window has no Open for a link** (known cleanup): the history action on a link item copies it (`--dev-history open` says "Copied to the clipboard."). The "Link from phone" toast's **Open** and **Copy** buttons cannot be clicked without OS input injection and the test Kin only logs toasts, so those two buttons are **not verified end to end** (unit-tested by the desktop agent); what is verified is the toast title and the decision (opened or offered), and that the opener was or wasn't asked.
3. The PC's history view returns a **preview** of a text (whitespace/control characters collapse to one space, long texts are cut at about 160 characters): fine as a preview; the script knows it. The stored text is exact on the phone and the opened URL had its full length on the PC.
4. The emulator image has **YouTube installed**: a `www.youtube.com` link gets the real label "Open in YouTube" (the `<queries>` host list works), instead of "Open link".
5. Android has no `cmd notification cancel_all`; the script clears the shade with its own "Clear all" button (the first runs failed because stale notifications grouped into a collapsed stack and the tap hit the summary: a test problem, not a product one).

## What S5 did and did not prove

The extension (test build, ID `cbioefpddjpaionhljhffgbedncfchmn`) in headless Chromium 1248 with a throwaway profile talked through the real dev host and the dev pipe (`KIN_HOST_DEV_PIPE`, never the release pipe) to the real test Kin that was paired with the emulator. Every landing was checked **on the phone**: the exact text/URL/title in the phone's database (`run-as sqlite3`), the notification (title, real domain), and the SHA-256 of image files in `Pictures/Kin`. The sign-in image was refused to Kin (403, no cookie; checked in the test site's request log), fetched in the page with the cookie and arrived intact (also 5 MiB). The "phone offline" step force-stops the app so Kin sees the socket close at once, queues (`queued`, popup "offline", `queuedCount` 1), and delivers after the app is started again (`queuedCount` 0).

Not covered: a real toolbar/menu click and `Alt+Shift+K` (headless; the same handlers are called, `activeTab` itself is browser behaviour), Firefox/Edge/Brave/Vivaldi/Opera, "Start Kin" (the host can't start a Kin in the dev setup and **`KIN_HOST_DEV_KIN_EXE` was never set**; the step checks that a send says `KIN_NOT_RUNNING`), Wi-Fi-off instead of a force-stop for the offline step, a real fullscreen game gate, the installer and the release keys, a real phone and the real lock screen (S2 checks `vis=PRIVATE`/`PUBLIC`, the "Link from DESK-PC" public version and the setting; the Android agent's screenshots show the locked phone).

## Edge-case table

Full table (46 vector cases and 14 display cases), generated by S6 (`%TEMP%\kin-e2e-2b-1-edge\edge-table.md`):

| case | valid (rule) | PC → phone | phone stored exact | phone → PC | PC opened |
|---|---|---|---|---|---|
| https-simple, http-simple, scheme-upper-case, youtube-with-timestamp-list-and-fragment, youtu-be-with-timestamp, query-and-fragment-only, userinfo-and-port, ipv6-literal-with-port, non-ascii-host, trailing-dot-host, port-with-leading-zero, percent-escapes-are-kept, exactly-8192-bytes (13) | yes | accepted | yes | link | yes |
| leading-space, trailing-space, leading-newline, trailing-ideographic-space, leading-space-on-an-8192-byte-url (5) | no (as sent) | rejected `BAD_URL` | no | **link** (the phone trims) | yes |
| 8193-bytes, javascript, javascript-with-slashes, file, data, ftp, intent, chrome, about, blob, mailto, kinrelay-scheme, no-slashes, single-slash, no-host, host-only-dots, empty-host-after-userinfo, port-zero, port-65536, port-empty, port-with-sign, space-inside, no-break-space-inside, next-line-inside, tab-inside, control-character-inside, delete-character-inside (27) | no | rejected `BAD_URL` | no | text (exact bytes) | no |
| empty (1) | no | rejected `BAD_URL` | no | nothing sent | no |

| display case | URL | a browser goes to | PC toast | phone |
|---|---|---|---|---|
| x-backslash-authority (+ with `/`) | `https://evil.example\@google.com` | evil.example | **google.com** | ⚠ evil.example |
| x-userinfo-lookalike | `https://google.com@evil.example/` | evil.example | evil.example | ⚠ evil.example |
| x-userinfo-port-lookalike | `https://google.com:443@evil.example/` | evil.example | evil.example | ⚠ evil.example |
| x-colon-userinfo-in-host | `https://example.com:8443@evil.example/` | evil.example | evil.example | ⚠ evil.example |
| x-escape-in-host | `https://goog%6Ce.com/` | google.com | goog%6ce.com | ⚠ goog%6ce.com |
| x-idn-umlaut | `https://bücher.example/` | xn--bcher-kva.example | bücher.example | ⚠ xn--bcher-kva.example |
| x-punycode | `https://xn--bcher-kva.example/` | xn--bcher-kva.example | xn--bcher-kva.example | xn--bcher-kva.example |
| x-homograph-cyrillic | `https://аррӏе.com/` | xn--80ak6aa92e.com | аррӏе.com | ⚠ xn--80ak6aa92e.com |
| x-homograph-mixed-script | `https://paypaӏ.com/` | xn--paypa-iof.com | paypaӏ.com | ⚠ xn--paypa-iof.com |
| x-fullwidth-host | `https://ｇｏｏｇｌｅ.com/` | google.com | ｇｏｏｇｌｅ.com | ⚠ google.com |
| x-upper-case-host | `https://EXAMPLE.COM/Path` | example.com | example.com | example.com |
| x-ip-octal | `https://0x7f.1/` | 127.0.0.1 | 0x7f.1 | 0x7f.1 |
| x-ipv6-mapped | `https://[::ffff:127.0.0.1]/` | [::ffff:7f00:1] | [::ffff:127.0.0.1] | [::ffff:127.0.0.1] |

All 14 display URLs pass the rule, were accepted by the host/Kin, stored exactly on the phone, and (phone → PC) became links that the PC opened automatically (mocked opener).

## Script and test-infrastructure changes

New: `tools/e2e-2b-1.ps1`, `tools/e2e-2b-1-extension.ts`, a `tools/README.md` section, `docs/results/phase-2b-1-integration-screens/` (11 pictures: phone notifications, link target, grouped stack, spoofing title, popups, API 29). No existing script or product file was changed. Things worth knowing (all found by running):

- **The old PC** is built from `git archive v0.1.1 desktop` (a tarball extracted to `%LOCALAPPDATA%\Temp\kin-e2e2b1-oldpc`, `cargo build --bin kin` with its own `CARGO_TARGET_DIR`, 1 m 16 s). `-Build` does this when it is missing; the main tree was never checked out. Its copy must be called `Kin.exe`: `kin-dev.ps1` finds a test Kin's processes by that name, and my first copy (`Kin-v0.1.1.exe`) was never seen, never quit, and four of them were left running by the first runs (stopped by PID after checking their paths; the script now fails if anything of its run is left).
- **Windows PowerShell 5.1 drops embedded double quotes** in arguments to `adb.exe`, which split a shared URL with a space; the URL and title now go to the phone as files read by a small script on the phone (this is what lets the control-character and NBSP cases through unchanged).
- The phone's database is read with `run-as sqlite3` (debug build, the system image's sqlite3) with `hex()` for exact bytes; the PC's with `--dev-dump-view history`.
- S6 reads "what the PC shows" by compiling the real `display_host` out of `phonelinks.rs` (regex cut, `rustc`), and "what a browser does" with bun's WHATWG `URL`.
- A shared lock, `COMPUTERNAME=DESK-PC` for the run, a copy of `Kin.exe` for the run, `autoClipboardSync` off in the test Kin's throwaway settings.

Re-run: `powershell -ExecutionPolicy Bypass -File tools\e2e-2b-1.ps1` (add `-Build` after a code change, `-Avd kin_api29` for the Android 10 pass; capture the output to a file as the other e2e scripts' notes say). Exit code 0 only if everything passed.
