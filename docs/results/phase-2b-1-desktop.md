# Phase 2B.1: desktop (the `link` message, the browser extension's way into Kin, links from the phone)

Agent: Sonnet 5.5, desktop coder. Date: 2026-10-17. Owns `desktop/` (including `desktop/native-host/`), `protocol/`, `docs/protocol.md`, `docs/windows.md`, `tools/`; `docs/extension.md` was edited only to fix contract gaps (listed below). **Nothing is committed.** `android/` and `extension/` were not touched.

## B1/B2 spec change (security fix after integration; the Android agent reads this first)

Date: 2026-10-17, desktop coder. Bugs: [phase-2b-1-integration.md](phase-2b-1-integration.md) B1 (`https://evil.example\@google.com` passed the rule, a browser goes to `evil.example`, the PC's toast said `google.com` and opened it by itself) and B2 (the PC showed homographs, IDN, escapes and odd IPs as written). Spec: [protocol.md](../protocol.md) "Links: `isUrl` and openable URLs" steps 4 and 5, the new section **"Showing the address of a link"**, changelog **55**. Still uncommitted; `android/` and `extension/` were not touched.

### What changed in the rule (both sides, `isUrl` of a `text` and `link.url`)

The **authority** (the text after `://` up to the first `/`, `?` or `#`) is now rejected if it contains:

1. **`@`**: no user information at all (the old "ignored up to the last `@`" is gone). `https://user:pw@example.com/`, `https://google.com@evil.example/`, `https://google.com:443@evil.example/`, `https://me@[::1]:9/` are **not openable** any more. An `@` after the authority (`/a@b`, `?email=a@b.c`, `#x@y`) is fine.
2. **`\`**: `https://evil.example\@google.com`, `https://evil.example\/x`, `https://example.com\`, `https://\example.com/`. A `\` in the path, query or fragment is fine (a browser reads it as `/`, the host is unchanged).
3. **`%`**: no percent-encoding in the host or the port (`https://goog%6Ce.com/`, `https://%67oogle.com/`, `http://[fe80::1%25eth0]/`). `%` in the path, query and fragment is fine.
4. A host that is **not a bracketed IPv6 literal** has none of **`[ ] < > ^ |`** (the rest of a browser's forbidden host characters is already covered by whitespace/controls, `/ ? #`, `:`, `@`, `\`).

Everything else that was valid stays valid: ports, `t=`/`list=`/`#` parts, IDN in Unicode or punycode, IPv4 in any spelling, IPv6, trailing dot, 8,192 bytes. **There is still no URL library for this accept/reject decision.**

### What changed in the vectors

| File | Change |
|---|---|
| `urls.json` | `userinfo-ignored` and `userinfo-then-ipv6` removed (now `userinfo-and-password`, `userinfo-then-ipv6` are `isUrl: false`); 37 new cases: the reject cases above (`backslash-*`, `userinfo-*`, `percent-*`, `*-in-the-host`) and valid ones (`@`/`\`/`%` after the authority, punycode, full-width, Cyrillic, IPv4 in hex/decimal, a host a browser refuses). `howToRead` says it |
| `links.json` `urls[]` | `userinfo-and-port` is now `valid: false`; the same 37 new cases |
| `messages.json` | `link-scheme-case-and-userinfo-are-kept` is **renamed `link-scheme-and-host-case-are-kept`** (its URL lost the `user:pw@`); **+3 invalid cases** `link-url-backslash-in-the-authority`, `link-url-userinfo-lookalike`, `link-url-percent-in-the-host` (`bad_request` with `ref`) |
| `links.json` `display[]` | **new, 46 cases** `{name, url, display, warn, parseFails?, note?}` (see below) |
| `check-vectors.ts` | the rule model changed; a `display` model on the runtime's WHATWG parser; every `display` value is also checked against `new URL(url).host` (except `parseFails`). **7,400 checks pass** (6,953 before) |

### `display[]`: what the phone must show

Per the spec section: `display` = **the host a browser connects to**: ASCII lower case, IDNA (UTS 46, **non-transitional**) punycode for non-ASCII, IPv4 in any spelling (`0x7f.1`, `2130706433`, `127.1`, `017700000001`, full-width digits) as dotted decimal, IPv6 compressed lower case in brackets (`[::ffff:127.0.0.1]` → `[::ffff:7f00:1]`), trailing dot kept, **the port only when it is not the scheme's default** (`:00443` on https shows nothing; `https://example.com:80/` shows `example.com:80`). `warn` is true when **any** of: the host **as written** has a non-ASCII character; the shown address has an `xn--` label; the shown address is an IP address (IPv4 any spelling, or IPv6; a plain `192.168.1.1:8080` warns too); a browser refuses the host (`parseFails: true`: `1.2.3.4.5`, `256.1.1.1`, `[abc]`, `xn--a.com`, U+0378): then `display` is the host as written, ASCII lower-cased, every character outside U+0021..U+007E as U+FFFD, no port.

**Where `LinkDomain` (Android, `links/LinkRules.kt`) differs from the vectors** (read from the code, not run):

1. **IP hosts**: it shows them as written (`0x7f.1`, `[::ffff:127.0.0.1]`) and never marks them. The vectors want the canonical form (`127.0.0.1`, `[::ffff:7f00:1]`) **and `warn`** for every IP address. Needs the WHATWG IPv4 parser ("ends in a number" check, hex/octal parts, 1 to 4 parts, out of range refused) and an IPv6 serializer with the `::` compression.
2. **IDN**: `java.net.IDN.toASCII` is IDNA2003. The vectors are UTS 46 non-transitional: `faß.de` → `xn--fa-hia.de` (IDNA2003 gives `fass.de`), `U+0378` is refused (it has `ALLOW_UNASSIGNED`), `xn--a.com` is refused. Suggestion (untested): `android.icu.text.IDNA.getUTS46Instance(NONTRANSITIONAL_TO_ASCII | CHECK_BIDI | CHECK_CONTEXTJ)` (API 24+, and the app's `minSdk` is what matters), with `errors` non-empty meaning "refused"; the vectors win over any library.
3. **`warn`** has to include a punycode label **as written** (`https://xn--bcher-kva.example/` warns; today it doesn't) and every IP address; it must not fire for plain ASCII hosts, default ports, or `@`/`\` after the authority.
4. The `@`, `\` and `%` triggers in `suspicious` can never fire for an accepted URL now (the rule refuses them); keep them as defence if you like.
5. The refused-host fallback today (`�` for anything outside U+0021..U+007E) is the same idea as the vectors; the vectors show no port there, and a refused host is `parseFails` for everything the WHATWG parser refuses (IP forms, bad punycode, unassigned characters), not only what `IDN` throws on.
6. Tests that **will fail until updated**: `LinkVectorsTest`, `MessageVectorsTest`, `TextAndUrlVectorsTest` and `UrlRulesTest` (cases flipped, new names; `minSdk` is 29, so `android.icu` is there), `LinkDisplayTest` (backslash/user-info/escape expectations: those URLs are now rejected, not shown with a mark), `LinkFlowTest` where a shared URL has `user@`. A shared text with user information is now **not a link** (stays a `text` with `isUrl` false).
7. The phone never opens a link by itself, so "a link with a warning is never opened automatically" is satisfied; the notification keeps its ⚠ in front of the address.

### Integration note (S6)

Re-running S6: of the 46 old `links.json` URL cases only `userinfo-and-port` flips (now rejected: the PC answers `BAD_URL`/`bad_request`, the phone shares it as a **text**, not a link). Of the 14 display cases, `x-backslash-authority` (+ the one with `/`), `x-userinfo-lookalike`, `x-userinfo-port-lookalike`, `x-colon-userinfo-in-host` and `x-escape-in-host` are rejected now (6 URLs); the other 8 (`x-idn-umlaut`, `x-punycode`, `x-homograph-cyrillic`, `x-homograph-mixed-script`, `x-fullwidth-host`, `x-upper-case-host`, `x-ip-octal`, `x-ipv6-mapped`) must show the `display`/`warn` of `display[]` on **both** sides: PC toast `⚠ xn--bcher-kva.example` etc., and the PC must not open the warned ones (all but `x-upper-case-host`) by itself, which the mocked opener can check (`x-punycode`, `x-ip-octal` and `x-ipv6-mapped` are new warnings; the phone doesn't mark them yet). For the extension: `extension/src/shared/validate.ts` `checkUrl` sends `new URL(x).href` (backslash becomes `/`, `%6C` is decoded): safe as it is, but it accepts `user@host`, which Kin now refuses with `BAD_URL` ("not a web link"): the extension owner may refuse userinfo itself for a clearer message.

### What the desktop does now

- `links.rs`: `is_openable_url` has the new rule (so `is_link_url`, the extension's `send` (`BAD_URL`), a text's `isUrl`, a received link and the image downloader (`ParsedUrl::parse`, redirects too) all refuse these URLs); **`link_address(url) -> LinkAddress { display, warn }`**: the `url` crate (WHATWG, with IDNA; no separate `idna` dependency) is given `scheme://authority/` only, `display` is its host (`host_str` for IPv6, `port()` when not the default), `warn` as above; the fallback for a refused host.
- `phonelinks.rs`: `decide(setting, state, warn)`: **a link with a warning is offered, never opened by itself**, whatever the setting or the state. The toast line under the title is `link_address(url).shown()` (`⚠ xn--80ak6aa92e.com`); Open and Copy as for the other offered links (Open works on purpose). The log says "not opened (the address has a warning; ...)", never the URL.
- New dependency: **`url = "2.5.8"`** (already in the lock through other crates).
- Tests: `links.rs` (the reject cases, `link_address`), `phonelinks.rs` (decision with a warning, toast bodies for the homograph/full-width/IDN/IP cases, B1), `tests/vectors.rs::links_json_display` (all 46 `display[]` cases against the PC), `tests/links_2b1.rs` (8 warned links from the phone: offered, never opened, Open works; the B1/B2 URLs refused from the phone and from the extension), `imagefetch` (user information refused). Extension sends of a warned address are accepted (it is a valid link); the warning is shown by the **receiver**.

Results of this change: `cargo test` (debug) **503 passed, 0 failed, 8 ignored**; `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean; `bun protocol/check-vectors.ts` **7,400 checks OK**. No e2e (integration re-runs S6). `desktop\build.ps1` rebuilt after the last change: `Kin_0.1.1_x64-setup.exe` **4,028,528 bytes**, SHA-256 `7FB3A4E3E6C3B982613BC4C0B72BFC71484295ED729764D6468E40451CD73AFC` (unsigned, not installed; the installer hash and size in "Installer" below are of the build before this fix).

## Summary

| Task | Result |
|---|---|
| 1. Protocol | **`link` message, `hello.capabilities`, vectors** (`protocol/links.json`, `link-*` in `messages.json`), changelog 54. Done first; the Android agent had it before any desktop code |
| 2. Native host integration | The pipe server inside Kin, the request API, the manifests and `HKCU` keys written by Kin at every start, the host in the installer, uninstall removes the keys |
| 3. Send paths | `text`, `link`, `page`, `image` (URL download by Kin, bytes fallback), queued when the phone is away |
| 4. Phone → PC links | Setting (on by default), open or toast with Open and Copy, **never over a fullscreen game / presentation / quiet time** |
| 5. Cleanup | `clipboard-backup.ps1`: `$ErrorActionPreference = 'Stop'` in `Write-ClipboardSnapshot` and `Save-Clipboard` |

## 1. Protocol (ready since the start of this task)

Spec: [protocol.md](../protocol.md#links-link) ("Links: `link`", "Versioning", changelog 54). Vectors: [protocol/links.json](../../protocol/links.json), the `link-*` cases of [messages.json](../../protocol/messages.json) (19 valid, 30 invalid, 4 sequences), `scope-2b.1` and `links` groups in `check-vectors.ts`. `bun protocol/check-vectors.ts`: **6,953 checks, all pass** (6,413 before).

Decisions in it (the Android agent implements exactly this):

- `{type:"link", id, url, title?, sourceApp?}`, both directions, acked, de-duplicated, queued (7 days) and retried like a `text`; never put on the clipboard by the receiver.
- **`url` is the openable-URL rule applied as sent, with no trimming** (a leading or trailing space is invalid; senders trim). http/https only, no whitespace or control character, 8,192 bytes at most. **Never rewritten** (YouTube `t=`, `list=`, fragments, percent escapes, user information pass through byte for byte).
- **`title`**: bidi controls → `_`, control characters (C0, DEL, C1) → a space, runs of spaces collapse, ends trimmed, cut to 300 **code points**, trailing spaces trimmed again, empty = absent. **`sourceApp`**: the same, 64.
- An invalid link (also a non-string `title`/`sourceApp`) is `error bad_request` with `ref`, the connection stays open, the id is not remembered. Senders refuse to send one.
- **Old peers.** A 0.1.x peer answers an unknown `link` with `error unsupported_type` and keeps the connection, but that `error` would fail a queued item for good. So `hello` gains the optional **`capabilities: string[]`** (`["link"]` from 0.2.0; a value that isn't an array counts as none, non-string entries are skipped, and **a bad `capabilities` never fails a `hello`**). **A sender sends a `link` only to a peer whose latest accepted `hello` of this connection lists `"link"`; to any other peer it sends the same item as a `text`** (same `id`, `text` = url, `isUrl` true, no title or sourceApp). The choice is made **when the message goes on the wire**, not when the item is made. The PC's `hello` lists `["link"]`.

## 2. The desktop code

### Where things are

| File | What |
|---|---|
| `src/messages.rs` | `link` is a receivable type; `Action::DeliverLink`; `Peer.capabilities`; `CAPABILITIES`; `parse_link` |
| `src/links.rs` | `is_link_url` (no trim), `clean_link_text`, `LogOnlyOpener` (`--dev-no-open`) |
| `src/ws.rs` | the `hello` has `capabilities`; **`wire_form`**: a `link` goes out as a `link` or as a `text` by the peer's capabilities; a received link is handled in a task of its own |
| `src/storage.rs`, `queuedb.rs`, `queue.rs`, `history.rs` | a link is a `text`-kind row of `transfers` with `is_link`, `link_title`, `link_source` columns of `transfer_texts` (added by `ensure_column`, so an old database works); `queue_link`, `queue_text_item`, `enqueue_offer_to`, `queue_target_for` (a named device); retry of a failed link; the history shows the title |
| `src/phonelinks.rs`, `src/quiet.rs` | phone → PC link handling and the fullscreen gate (`SHQueryUserNotificationState` behind a trait) |
| `src/nativehost/` | `mod.rs` (`HostKind`, pipe names, the user SID), `pipe.rs`, `framing.rs`, `api.rs`, `imagefetch.rs`, `registry.rs` |
| `src/outgoing.rs` | `send_temp_file` (the `.txt` of a long text and an image from the extension share one path) |
| `src/app.rs`, `ui.rs`, `build.rs`, `capabilities/settings.json` | the pipe is started, the keys registered, `link` toasts and their buttons (Open, Copy), the new setting command |
| `windows/hooks.nsh`, `windows/bundle-native-host.json`, `../build.ps1` | host in the installer, uninstall removes the keys |
| `examples/fake_phone.rs` | `capabilities` in its `hello`, `--old-phone`, `--send-link`, `--send-link-title`, prints `received link ...` |
| `native-host/smoke/run-real-kin.{ps1,ts}` | the extension in headless Chromium against this Kin and the real dev host (below) |

### The pipe

Exactly the requirements of [phase-2b-1-native-host.md](phase-2b-1-native-host.md): `D:P(A;;GA;;;<SID>)` (read back from the pipe in a test: `D:P(A;;FA;;;<SID>)`), `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE` on the first instance only (a taken name: Kin logs it and runs **without** the extension pipe, never with a fallback; tested with a squatter), byte mode, 64 KiB buffers, one request per connection, the next instance made before the client is served, each client on its own thread (a slow request does not block the next client), `FlushFileBuffers` + `DisconnectNamedPipe`, a request with a prefix over 64 MiB + 1 MiB dropped unread, a client that sends no whole request in 30 s cut off (`CancelIoEx`), a client that hangs up mid-request harmless (`ERROR_NO_DATA` handled: the instance is reused).

One thing the reference taught me the hard way: **do not make a spare instance ahead of time.** An instance that nobody waits on with `ConnectNamedPipe` still accepts a client, and that client then hangs until someone else connects to the first instance. A first version with a spare did exactly that (a test hung). The accept loop waits on one instance and makes the next at once; a host that finds the pipe busy in that gap waits for it (`WaitNamedPipe`), as the contract says, and the pipe tests do the same.

**Pipe names.** Installed Kin (`HostKind::for_config`: a release build, no developer setting, no `--data-dir` of its own, no instance key): `\\.\pipe\app.kinrelay.desktop.<SID>`. **Every other Kin** (debug build, any `--dev-*`, a `--data-dir`, any test): `\\.\pipe\app.kinrelay.dev.<SID>`, plus `.<suffix>` when `KIN_DEV_PIPE_SUFFIX` is set (so two test Kins can listen at once). The dev host build is pointed at it with `KIN_HOST_DEV_PIPE`. A test can never reach the installed Kin and the installed Kin never answers a test.

### The registry keys and manifests

**Decision: Kin writes them, at every start, as the signed-in user** (the elevated-installer `HKCU` problem of the host doc, the same reason the "Send to" shortcut is the app's). The manifests go to **`%APPDATA%\Kin\native-messaging\`** (written when different), with the **absolute** path of `kin-native-host.exe` next to the running `Kin.exe` (a moved install repairs itself). Keys: Chrome, Chromium, Edge, Brave, Vivaldi (`Software\<vendor>\NativeMessagingHosts\app.kinrelay.desktop`) → the Chromium manifest, Mozilla → the Firefox one; Opera shares the Chrome key (**not verified**). `allowed_origins` = the stable dev ID (`cbioefpddjpaionhljhffgbedncfchmn`) plus `registry::STORE_ORIGINS`, **a clearly marked TODO slot** for the store IDs (Edge reads only the first manifest, so they all go in this one file).

**Only the installed Kin does it** (`may_register_release`: release build, normal run, not from a build folder, host exe beside it). **Dev, debug and test runs never write browser keys** (and Kin never writes keys for `app.kinrelay.dev`: a test that needs its key writes `HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev` itself and removes it in a `finally`). The tests write below `HKCU\Software\KinRelayUnitTest\<uuid>\...` (the functions take the registry base), and delete that tree in a `finally`; `register(Dev, "Software", ...)` is refused.

**Uninstall** (`hooks.nsh`): removes exactly the six keys of the release host name, in this context and the signed-in user's (`RunAsUser` + `reg delete`), and the manifest folder; nothing else under the browsers' keys.

### Bundling

`desktop/build.ps1` builds `native-host` in release mode **with no features**, refuses a host that contains `KIN_HOST_DEV`, `app.kinrelay.dev`, `kin-host-test` or `mock-kin` or is under 100 KB, copies it to `src-tauri\binaries\` (git-ignored) and adds it to the bundle with `src-tauri\windows\bundle-native-host.json` (a config override as a **file**: Windows PowerShell 5.1 strips the quotes of a JSON text passed to a program, found the hard way). `tauri.conf.json` is unchanged, so a plain `cargo build` / `cargo test` never needs the host. The NSIS script has `File /a "/oname=kin-native-host.exe" ...` at `$INSTDIR` (next to `Kin.exe`) and the uninstaller deletes it. Kept standalone (not a Cargo workspace) as the host doc recommended.

### The send paths (`nativehost/api.rs`)

`status`, `start`, `open`, `send` (`text`, `link`, `page`, `image`), with the contract's codes. Every limit is checked again; `BAD_URL` for what the URL rule rejects, `BAD_REQUEST` for missing or wrong-type fields, `NOT_PAIRED` first after the rate limit. A text is never `sensitive`, goes in its wire form (LF), is queued like the hotkey's text (a `text` message; `isUrl` is Kin's decision), over 128 KiB it is `Text.txt`, over 4 MiB `TOO_LARGE`. A link/page is queued as a link with `sourceApp: "Browser"`. **An image** by URL: Kin's own downloader (below); a download failure the browser might get past is `IMAGE_NEEDS_BYTES`, a download over 64 MiB `TOO_LARGE`; by bytes: base64, at most 32 MiB raw, `NOT_AN_IMAGE` if the bytes aren't one. An image is a picture **by its magic bytes** (PNG, JPEG, GIF, WebP, BMP, TIFF, AVIF, HEIC; the contract also allowed "the decoder opens it": left out, see Decisions) and goes through the ordinary file transfer under a cleaned name with the extension of what it really is. Offline phone: `ok:true, queued:true, code DEVICE_OFFLINE_QUEUED`. Logs have counts and outcomes only; a test greps the log for a URL, a text, a title and file names.

**The downloader** (`imagefetch.rs`): no HTTP client crate was in the build, so it is a small HTTP/1.1 client on tokio and rustls (**one new dependency: `webpki-roots`**; `base64` is the `0.23` already in the lock). `GET`, **no cookies, no `Authorization`, user information dropped from the URL**, `Accept-Encoding: identity`, redirects up to 5 (relative, protocol-relative, `//` resolved; `mailto:`/`javascript:`/`data:`/`ftp:` refused; **https → http refused**, tested with a real local TLS server and a trusted throwaway certificate), 30 s in all, `Content-Length` over the cap `TooLarge` before the body, chunked and unframed bodies cut at 64 MiB, `Referer` without its fragment, a browser's `User-Agent`. A certificate the web's roots don't know fails the handshake and sends nothing.

### Phone → PC links

`phonelinks::decide(setting, state)`: open only if the setting is on **and** `SHQueryUserNotificationState` is not `QUNS_RUNNING_D3D_FULL_SCREEN`, `QUNS_BUSY`, `QUNS_PRESENTATION_MODE` or `QUNS_QUIET_TIME`; a failed call counts as free. Opened: toast **"Opened link from phone"** (title and the real host under it) with **Copy**; a click on it opens nothing. Not opened: **"Link from phone"** with **Open** and **Copy**. The history keeps it either way (a row like a text, with the title). Copy uses the history's clipboard path (the loop guard is told). The URL and the title are never logged. Settings window: "Open links from the phone automatically" (`openLinksAutomatically`, default true, saved at once); `--dev-set openLinksAutomatically false` for scripts. **`--dev-no-open`** (`KIN_DEV_NO_OPEN=1`) is new: nothing is opened (the log says it would have, with the length of a link), so a test run can receive links from a phone without starting a browser.

## Tests (all debug; one full run at the end, after the last change)

| Run | Result |
|---|---|
| `cargo test` (debug, `--no-fail-fast`) | **496 passed, 0 failed, 8 ignored** (the ignored are the real-clipboard tests and `fetches_a_real_https_image`) |
| `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` | clean |
| `bun protocol/check-vectors.ts` | **6,953 checks OK** |
| `bun run build` (desktop TS) | clean |
| `desktop\check-clipboard-backup.ps1` (real clipboard, saved and restored: every format, verified) | all checks passed after the `Stop` fix |
| `native-host\smoke\run-real-kin.ps1` (below) | **18/18** |

New Rust tests: `nativehost::pipe` (10: round trip, stop, a slow request does not block the next client, 24 clients at once, oversize prefix dropped, a client that hangs up, a handler that closes, **the DACL read back is exactly `D:P(A;;FA;;;<SID>)`**, a second first instance refused and later ones fine, a taken name means no server, remote access and the flags), `nativehost::registry` (manifests, the six keys and nothing else, the dev name refused on the real base, write/rewrite/repair/remove under a throwaway base with a neighbour key that survives, may_register_release false in tests), `nativehost::imagefetch` (headers sent, no cookies and no user information, statuses, redirects and the cap of 5, other schemes, https ↔ http with a real TLS server, `Content-Length`/chunked/unframed caps, exactly the cap, timeout, connection errors, magic bytes, file names), `nativehost` (names, `HostKind`, SID), `phonelinks`/`quiet` (the decision for every state, the host line), `links` (the rules), `cli` (`--dev-no-open`). **`tests/vectors.rs`** runs the `link-*` vectors and `links.json` against the real receiver and `ws::wire_form`; **`tests/links_2b1.rs`** (25 tests, real core + fake phone + real pipe): the PC's hello, status, open, protocol errors (bad JSON, ids, `v`, unknown types, repeated keys, host-added fields), the 30-per-10-s limit, text (LF, never sensitive, `isUrl`, `.txt`, 4 MiB), links to a capable and to an old phone and a link queued across a reconnect with the form chosen at send time, every refused URL, named and unknown devices, images by URL (named, referer, `IMAGE_NEEDS_BYTES`, `TOO_LARGE`, not an image) and by bytes (type ignored, `NOT_AN_IMAGE`, bad base64, 32 MiB + 1), the queue for an away phone, nothing user-sent in the log, a link from the phone (opened, Copy, setting off, **each of the four states holds it back and Open still works**, invalid refused, duplicates not opened twice, URL and title not in the log).

### The real run (`native-host\smoke\run-real-kin.ps1`)

Headless Chromium 1248, throwaway profile, **the real dev Kin** (debug build, own `--data-dir`, `--dev-bind 127.0.0.1`, `--dev-no-windows`, `--dev-no-open`, pipe `app.kinrelay.dev.<SID>.smoke<pid>`), **the real dev host**, **the fake phone** on the other side, the extension's test build. 18 steps, all passed: status through host and pipe; popup; a link arrives at the phone as a `link` with the URL exactly, the cleaned title and `sourceApp=Browser`; text; page with its title; bad links refused by the extension and, sent straight over the pipe, by Kin; image by URL (the phone's file has the right SHA-256); a sign-in image (Kin gets 403, the extension fetches it in the page, the bytes arrive intact); a 5 MiB image; a `data:` URL; the unreachable image; YouTube `t=83s`; the popup's Send this tab; a 200 KiB text as `Text.txt`; **the phone away: queued, popup offline, delivered when the phone is back**; **an old phone gets the link as a text with `isUrl`** and no `link`; **a link from the phone: opened (only logged), then with the setting off only offered; the URL and title are not in Kin's log; the history has the link with its title**; Kin quits: "isn't running", `KIN_NOT_RUNNING`. The first run (before a bug in my script was fixed: the fake phone was restarted with an invalid pairing URL) failed those same three steps and passed the rest; the numbers above are the second run. The registry key was removed (`registry key removed`), no process of mine was left, **the user's installed Kin was never started, stopped or asked** (it was running as PID 25796 before and after).

## Installer

`desktop\build.ps1` (version stays 0.1.1; the bump is 2B.6), built after the last code change: `src-tauri\target\release\bundle\nsis\Kin_0.1.1_x64-setup.exe`, **4,028,226 bytes (3.84 MiB)**, SHA-256 `D97DB8F4F32073DBF7CEBE432E3DC9F02E7BE3046725C64A8DE6DF4EFFD2B357` (3.60 MiB before: the host is 243 KiB, the rest is the new code and `webpki-roots`); `Kin.exe` 13.2 MB. Unsigned, per machine, **not installed**. The same 18-step real run passes with this release `Kin.exe` (own data directory, so the dev pipe) as with the debug one. The installer was **not run** (it needs administrator rights): what was checked is that `installer.nsi` has the host at `$INSTDIR\kin-native-host.exe` and its `Delete`, that the hooks compiled (`makensis` ran), and the release host's strings. Run `verify-install.ps1` after installing.

## Decisions

1. **Kin registers the host, the installer only lays the file down** (the host doc's recommendation; also the manifests are in the data folder, with an absolute path). Reason: elevated installer vs. the user's `HKCU`.
2. **One standalone host crate, bundled as a resource with a config override**, not a workspace and not `externalBin` (both would need the host file for every `cargo test`).
3. **A link is a text-kind row** (`is_link`) rather than a new `kind`: the queue, expiry, retry, the history and the metrics already work on text rows, and the wire form is chosen in one place (`ws::wire_form`). The success-rate numbers count a link as a text send.
4. **Image by magic bytes only** (no WIC fallback): deterministic, no COM, and SVG stays out. ICO and the like are refused from the extension.
5. **`open target main` = the "Send to phone" window**: Kin has no main window.
6. **`HostKind` follows `autostart_is_real`**: one rule for "is this the installed Kin" (Run key, SendTo, host keys, pipe name).
7. **`sourceApp` is `"Browser"`** for everything from the extension (the contract's choice).
8. **Toast text**: "Opened link from phone" / "Link from phone" (not "from <phone name>"), as the task says; the title and the real host are in the body.

## What changed in the contract (`docs/extension.md`; only gaps)

1. **Manifests**: written by Kin into `%APPDATA%\Kin\native-messaging\` at every start with an absolute `path` (it said the installer copies them into the install folder). **Registry keys**: written by Kin, only by the installed Kin; the installer only removes (it said the installer writes them). "Host build notes → Installer" says the same.
2. **Names**: a dev Kin is any Kin that is not the installed one (it said `--dev-bind`); `KIN_DEV_PIPE_SUFFIX` added.
3. **`open`**: `main` opens the "Send to phone" window, `pairing` starts a pairing.
4. **Images**: "decided by the magic bytes" (the decoder clause left out), SVG not accepted, TIFF added.

Still open in the contract (nothing changed): host-side time limits and `INTERNAL` for a Kin that hangs up are as in the host results ("Gaps in the contract" 2 to 4 there); the oversize-prefix answer has no `ref` (can't happen with this extension).

## What the Android agent must know

- **Everything is in protocol.md "Links: `link`" and "Versioning".** The vectors are `protocol/links.json` (`sendAs[]`, `textFallback[]` are for the phone as sender) and `link-*` in `messages.json` (a valid case expects `deliver_link` and `expect.link`: the URL as sent, cleaned title and sourceApp).
- **The desktop 2B.1 lists `"capabilities":["link"]`** in its `hello`. The installed 0.1.1 desktop does not: **send a `text` with `isUrl: true`** to it (same id, no title) — the PC then shows its normal text toast with a clickable link. Decide at send time, per connection, from the latest accepted `hello`.
- **What the PC does with your link**: acks it (not after opening), stores it in its history with the title, and opens it in the default browser (setting on, no fullscreen program) or shows a toast with Open and Copy. It never puts it on the PC's clipboard by itself. It refuses what isn't http(s) or has whitespace at either end: **trim before sending**.
- The desktop checks a link's `title`/`sourceApp` types: a non-string (not `null`) is `bad_request`.
- **`fake_phone`** (`desktop/src-tauri/examples/fake_phone.rs`) has `--send-link <url>`, `--send-link-title`, `--old-phone` if you need the PC side for an emulator run.

## What the integration agent must know

- **Run a dev Kin with `--dev-no-open`** (with `--data-dir`, `--dev-bind`, `--dev-no-windows`) when the emulator sends a link to the PC: otherwise the PC's default browser opens. Log lines to look for: `link <id> from device <8>: <n> characters, opened in the default browser` / `not opened (the setting is off; user state free), offered in a toast`, and `extension: send link (<n> characters): acked`.
- **`Kin.exe --data-dir <dir> --dev-set openLinksAutomatically false`** turns the setting off for a run (result in `dev-set-result.json`); `--dev-dump-view history` has `isLink` and `title` per item.
- **The extension against the emulator**: start the dev Kin with `KIN_DEV_PIPE_SUFFIX=<text>`, point the dev host build at it with `KIN_HOST_DEV_PIPE=\\.\pipe\app.kinrelay.dev.<SID>.<text>` (set in the browser's environment, as `smoke\run-real-kin.ts` does), register only `HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev` and remove it in a `finally`. `smoke\run-real-kin.ps1` is the template (it takes the scripts' lock, refuses to run beside a game, and builds everything).
- **The host cannot start a Kin in the dev setup** (no `KIN_HOST_DEV_KIN_EXE`): never set it to `Kin.exe` of a build folder, because the host starts it **with no arguments**, which is a launch on the installed Kin's data directory. The "Start Kin" path is tested in the host crate with its mock Kin.
- Not covered by anyone yet: a real Firefox, Edge, Brave, Vivaldi or Opera; the installer run (keys written at the first start, removed at uninstall); an elevated Kin (the host can't check an elevated process: it says `KIN_NOT_RUNNING`); a real fullscreen game (the gate is unit-tested with a fake state and the real call is exercised by a test).

## Cleanup list (not blocking)

- The history window shows a link as its title and URL with "click to copy"; there is no Open button in the history yet. The new settings switch and the history title were built (`bun run build`) but **not looked at in a real window** (no window was opened: the run was headless with `--dev-no-windows`).
- `status.json` has no field for the extension pipe or for the last extension send.
- `registry::STORE_ORIGINS` is empty (the TODO).
- `Kin.exe` with no arguments from the host while Kin runs but has no pipe (first instance refused): a no-op by the single-instance plugin (an empty command line is no command); checked by reading `cli::parse`, not run.
- The host reports its own version `0.1.0`; bump with the app at 0.2.0.
- Antivirus may flag the small unsigned host that starts another program (unchanged from the host results).

## Review cleanup

- **A locked or away user is treated like quiet time.** `UserState::NotPresent` (`QUNS_NOT_PRESENT`: session locked, screen saver on, switched away) used to count as free, so a link from the phone opened in the browser on a locked PC. `holds_back_windows()` now includes it, so `phonelinks::decide` gives `Offer` (toast with **Open** and **Copy**, history keeps the link; log reason "the PC is locked or the user is away"). Unit tests: the new `a_link_is_offered_not_opened_while_the_pc_is_locked_or_the_user_is_away`, and the `quiet` and `decide` tests were moved so `NotPresent` is on the held side. `docs/windows.md` says so.
- **The peer's capabilities are logged once.** The existing "phone connected" log line (written once per connection, when the first `hello` is accepted) now ends its parenthesis with `capabilities ["link"]`: at most 16 entries, each cut at 32 characters, no message content.
- `docs/extension.md`: the native-messaging paragraph said "six keys" for the Chromium manifest but listed five. It now says five Chromium keys (Opera has none of its own) plus one Firefox key.
- Checks: `cargo test` (debug) all green (270 unit, plus the integration suites), `cargo clippy --all-targets` and `cargo fmt --check` clean, `desktop\build.ps1` rebuilt the installer (`Kin_0.1.1_x64-setup.exe`). Nothing was launched, no installed Kin or user data touched.
