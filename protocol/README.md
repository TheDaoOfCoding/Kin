# protocol

Example messages and test vectors shared by both apps. The Android and Windows tests load these files, so the two sides stay compatible. The protocol itself is described in [docs/protocol.md](../docs/protocol.md). If a vector and the spec disagree, one of them is a bug. Fix it before building on either.

| File | What it checks | Spec section |
|---|---|---|
| [`device-id.json`](device-id.json) | Device ID, short ID, token encoding, SPKI from a key and from a certificate, the certificate profile | Encodings, Transport |
| [`pairing-urls.json`](pairing-urls.json) | Parsing the QR / link text, and the PC building it | Pairing > Parsing rules |
| [`messages.json`](messages.json) | WebSocket messages: what the receiver replies and does | Control channel |
| [`http.json`](http.json) | HTTP requests and responses, error bodies | Pairing, Files, Error codes |
| [`file-names.json`](file-names.json) | File name cleaning and collision handling | File names |
| [`urls.json`](urls.json) | Which text is an openable URL (`isUrl`, and what a receiver opens) | Text messages > Links |
| [`text-rules.json`](text-rules.json) | Line ends on the wire, the sender's 128 KiB threshold, the `sensitive` flag | Text messages |
| [`links.json`](links.json) | The `link` message's rules: which `url` is valid (checked as sent, no trimming), title and sourceApp cleaning, when a sender uses `link` or falls back to `text` (`hello.capabilities`) | Links: `link`, Versioning |
| [`notif.json`](notif.json) | Notification mirroring (changelog 56): `nid`, icon hashes and the icon rules, field cleaning, `notif.post` cases, one-time codes, toast names and click arguments, the PC and phone rate limits, the phone's coalescing, when a sender may send, the `notif.sync` diff | Control channel > Notifications |
| [`fixtures/`](fixtures/) | DER certificates (and one key) for the negative `tls[]` cases | Transport |
| [`check-vectors.ts`](check-vectors.ts) | A small reference implementation that proves the vectors and the spec agree | all of the above |

## What each milestone needs

The vectors are written for all of Phase 1, but each milestone ([phase-1-mvp.md](../docs/phases/phase-1-mvp.md)) only has to pass the part it implements. `*` at the end of a name means "every case whose name starts with this". The reference checker confirms that every name below matches a case, so this list can't drift.

**Milestone 1.1 (connection spike).** These arrays and cases, and nothing else yet:

| File | In scope for 1.1 |
|---|---|
| `device-id.json` | `keys[]`, `hashVectors[]`, `tokenVectors[]`, `shapeVectors[]` (all of it, including the certificate profile of `keys[].certificatePem`) |
| `pairing-urls.json` | `valid[]` and `invalid[]` (all of it) |
| `http.json` | `exchanges[]`: `pair-*`, `ws-upgrade-*`, `unknown-route*`, `unknown-version-path`, `wrong-method-on-known-route*`. `sequences[]`: `pair-rate-limit-per-token`, `pair-token-is-single-use`. `errors[]`. **`tls[]`** (all of it; the `thenHttp` follow-up `transfers-unauthorized-unpaired-certificate` of one case belongs to 1.3) |
| `messages.json` | `valid[]`: `hello-*`, `ack`, `ack-from-pc`, `error-*`, `ping-*`, `unpair-*`, `nesting-depth-64-*`, `brackets-inside-strings-do-not-count-as-nesting`. `invalid[]`: `first-message-*`, `hello-*`, `ack-*`, `error-*`, `unpair-missing-id`, `ping-*`, `type-*`, `unknown-type-is-checked-before-fields`, `frame-*`, `binary-frame*`, `text-frame-1-MiB-plus-1-byte`, `text-frame-multibyte-over-limit`, `text-frame-invalid-utf8`, `text-lone-*`, `text-high-surrogate-followed-by-non-low-escape`, `text-reversed-surrogate-pair-escape`, `lone-surrogate-escape-*`, `strict-*`, `nesting-*` (all of these are rejected before any message-specific field is read, so no `text` handler is needed). `sequences[]`: `second-hello-after-handshake-is-a-normal-message`, `invalid-hello-does-not-count-as-accepted`, `garbage-before-hello-keeps-the-connection-open`, `future-hello-gets-unsupported-version-not-a-timeout`, `duplicate-ping-shows-one-toast`. Plus **`timers[]`** and **`closeCodes[]`** (all of it) |

Note on the 1.1 `messages.json` scope: the sequence `garbage-before-hello-keeps-the-connection-open` ends with a valid `text` that has to be acked (`deliver_text`), and `invalid-hello-does-not-count-as-accepted` needs `text` to be a known type. So the 1.1 vectors only need a receiver that validates `text` and acks it. (The 1.1 apps did that and nothing more: delivering the text was 1.2.)

**Milestone 1.2 (text, links and the clipboard).** Both sides now handle `text` fully (clipboard, notification or toast, the `ack`, de-duplication), and the phone handles `clipboard`. The `text` and `clipboard` cases of `messages.json` come in, on top of the 1.1 scope above. Nothing in `http.json`, `device-id.json`, `pairing-urls.json` or `file-names.json` is new.

| File | In scope for 1.2 (in addition to 1.1) |
|---|---|
| `messages.json` | `valid[]`: `text-*` (this includes `text-frame-exactly-1-MiB`, `text-frame-multibyte-under-limit` and `text-valid-surrogate-pair-escape`), `clipboard-*`, `duplicate-keys-*`. `invalid[]`: `text-*`, `clipboard-*` (including `clipboard-from-phone-wrong-direction`: the PC answers `unsupported_type`). `sequences[]`: `handshake-then-text-then-duplicate`, `invalid-message-keeps-connection-open`, `rejected-message-id-is-not-remembered` |

The cases have a `to` role, and a receiver of that role has to answer them: `text-from-pc`, `text-extra-fields-ignored` and `clipboard-from-pc` are for the phone, the rest of the `text` cases are for the PC. `deliver_text` and `set_clipboard` are checked by category with a fake that records calls (the real clipboard is never touched). `check-vectors.ts` checks that every name above matches a case, like the 1.1 list.

**Milestone 1.3 (files in both directions).** The transfers, the file names, and the new text rules. Both sides run these on top of the 1.1 and 1.2 scope. The Windows app is the server for `http.json`, the phone is the client (`MockWebServer`). Both implement the file-name rules, and both sides send and receive the `offer`, `complete` and `cancel` messages in their own direction.

| File | In scope for 1.3 (in addition to 1.1 and 1.2) |
|---|---|
| `http.json` | `exchanges[]`: `transfers-*`, `put-*`, `get-*`, `delete-*` and `internal-error`. `sequences[]`: `upload-*`, `download-*`, `repeat-with-different-files-keeps-the-stored-list`, `delete-*` and `transfer-id-*` (a transfer ID is never reused; these have a `clock` step, see below). (That is every exchange and sequence that isn't in the 1.1 list: the `thenHttp` follow-up `transfers-unauthorized-unpaired-certificate` of `tls[]` is among them.) The `server.freeBytes`, `server.diskFull`, `server.tooLarge` and `server.createFails` contexts are PC-side hooks: a test fakes the free space, the size limit and a failing create |
| `file-names.json` | `cases[]` and `createNew[]` (both `operation`s: `create` and `move_part`), including the limit of 100 candidates: when the plain name and ` (1)` to ` (99)` are all taken the file fails with `internal` |
| `messages.json` | `valid[]` and `invalid[]`: `offer-*` (an offer of 800 files is accepted, 801 is rejected with `error` `too_large`), `complete-*` and `cancel-*`. `sequences[]`: `duplicate-offer-is-acked-not-restarted`. The new **`text-sensitive-*`** cases (the `sensitive` flag, with `expect.sensitive`) are among the `text-*` cases of the 1.2 scope, so the 1.2 apps fail them until the flag is implemented |
| `urls.json` | all of `cases[]`: `isOpenableUrl(text)` must equal `isUrl` |
| `text-rules.json` | all of it: `lineEndings[]`, `senderSize[]`, `sensitiveSender[]`, `receiver[]`, `windowsSensitiveFormats` |

`check-vectors.ts` checks that every name above matches a case (the `scope-1.3` group), like the 1.1 and 1.2 lists.

**Milestone 1.4 (queue, history, settings).** The queue has no message of its own, so the new vectors are rulings on behaviour that already existed on the wire, on top of the 1.1 to 1.3 scope. Both sides run them.

| File | In scope for 1.4 (in addition to 1.1 to 1.3) |
|---|---|
| `messages.json` | `sequences[]`: `offer-reusing-a-text-id-is-bad-request`, `offer-from-another-device-is-bad-request`, `offer-repeated-while-unfinished-is-acked-after-a-reconnect`, `same-pc-repeat-after-done-is-acked`, `same-pc-repeat-after-canceled-is-acked` (these two replace `offer-reused-after-it-is-over-is-bad-request`: changelog 52), `same-pc-repeat-with-different-files-is-bad-request` (**optional**: a receiver may skip the file-list check), `complete-re-sent-after-reconnect-is-acked-again`, `cancel-re-sent-after-reconnect-is-acked-again`, `queue-resend-after-reconnect-is-idempotent`, `queue-is-not-sent-before-hello`. They have two new kinds of step, see [`messages.json`](#messagesjson) |
| `http.json` | `sequences[]`: `transfer-id-window-ends-at-exactly-24-hours` and `transfer-id-window-is-not-extended-by-an-empty-put-or-a-post` (the done time + 24 h is the first moment the ID is `404`; nothing extends it), and `transfer-id-retry-after-404-sends-only-the-missing-files` (the user's Retry: the old ID first, then a new ID with only the files that never got a `200`). They match the `transfer-id-*` pattern of 1.3, so the 1.3 runners pick them up too |

`check-vectors.ts` checks that every name above matches a case (the `scope-1.4` group).

**v0.1.1 (several files of one transfer at once).** One new sequence in `http.json`, on top of everything above: `batch-files-of-one-transfer-are-independent-and-a-resume-leaves-the-others-alone` (changelog 53). Both sides run it like any other sequence. The sequential format can't interleave requests, so the concurrency itself (4 `PUT`s or 4 `GET`s of one transfer at once, a resume of one file while the others go on, a cancel in the middle, small files pipelined on one connection) is tested with real connections: the Windows app in `desktop/src-tauri/tests/parallel_files.rs`, the phone in its own tests. `check-vectors.ts` checks the sequence (the `scope-0.1.1` group).

**2B.1 (links).** The `link` message (changelog 54). Both sides run these on top of everything above; an app that doesn't implement `link` yet fails them, like any new scope.

| File | In scope for 2B.1 |
|---|---|
| `messages.json` | `valid[]`, `invalid[]` and `sequences[]`: `link-*`. A valid `link` case expects the ack with action `deliver_link` and `expect.link`: what the receiver shows and opens, which is `url` exactly as sent plus the **cleaned** `title` and `sourceApp` (left out when empty). An invalid one expects `error` `bad_request` with `ref` = the frame's id (when valid), the connection open, and the id not remembered (`link-rejected-id-is-not-remembered`). `link-sent-as-text-to-an-old-peer-is-a-text` is the receiver side of the fallback: it is an ordinary `text` |
| `links.json` | all of it: `urls[]`, `titles[]`, `sourceApps[]`, `sendAs[]`, `display[]`, `textFallback[]` (see below) |

`check-vectors.ts` checks that every name above matches a case (the `scope-2b.1` group).

**2B.2 (notifications).** The `notif.*` messages and the `notif` capability (changelog 56). Both sides run these on top of everything above.

| File | In scope for 2B.2 |
|---|---|
| `messages.json` | `valid[]`, `invalid[]` and `sequences[]`: `notif-*`. These cases have two extra keys, `own` and `peer` (arrays of capability strings): the capabilities of the **receiver's own** latest `hello` and of **the sender's**. A `notif.*` is dropped silently (reply `none`, action `notif_dropped`) unless both list `notif`. In a sequence the capabilities come from the `hello` steps (a `hello` from X sets X's own list and the other side's view of X). The new actions are `notif_status`, `notif_icon`, `notif_post`, `notif_remove`, `notif_sync`, `notif_dismiss`, `notif_action` and `notif_dropped`; `expect.notif` is what the receiver ends up with (see `notif.json`). Only `notif.action` is acked |
| `notif.json` | all of it: `nid[]`, `iconHash[]`, `icon[]`, `clean[]`, `post[]`, `code[]`, `toastIds[]`, `toastArguments[]`, `rate[]`, `coalesce[]`, `sendNotif[]`, `syncDiff[]`, `toastLabel[]` (see below); `otpDetect[]`, `redactedText`, `skip[]` and `pickActions[]` are the phone's own |

`check-vectors.ts` checks that every name above matches a case (the `scope-2b.2` group).

## Common format

Every file is one JSON object. It starts with `description`, `specVersion` (the protocol version, `1`) and sometimes `howToRead`, then one or more arrays of cases. Every case has a `name` that is unique inside its array, and some have a `note` explaining why the case exists.

- **All non-ASCII characters are written as `\uXXXX` escapes** (astral characters as surrogate pairs), so the files survive any editor or encoding mistake. A JSON parser gives you the real characters back. Lengths of IDs and text, in these files and in the spec, count Unicode code points. The exception is file names: their limits (200, and 16 for the extension) count UTF-8 bytes.
- Numbers are integers. Parse them as `Long` (Kotlin) and `u64` or `i64` (Rust), not as floating point.
- Unknown keys in a case are ignored, so cases can grow new fields.

## `device-id.json`

- **`keys[]`** has **test-only** P-256 keys. Each has a PKCS#8 PEM private key, a self-signed certificate over the same key, the DER SPKI as hex (91 bytes), the SHA-256 as hex, and the expected `deviceId` and `shortId`. Tests: derive the public key from the PEM and check the SPKI bytes and both IDs, and also read the SPKI out of `certificatePem` and check it's the same. (That's how each side learns the other's ID in real life.)
  - **Every `certificatePem` follows the certificate profile** ([protocol.md](../docs/protocol.md#transport)): X.509 v3, ECDSA P-256 key, `ecdsa-with-SHA256`, serial positive and at most 20 bytes, no critical extension. `certificateVariant` is `minimal` (no extensions at all, 311 or 312 bytes: the template for a hand-written DER) or `with-noncritical-extension` (key 4: one non-critical `subjectKeyIdentifier`, proving that non-critical extensions are accepted). The reference checker parses the DER of each one and fails if it leaves the profile.
- **`hashVectors[]`**: a raw 32-byte hash in, `deviceId` and `shortId` out. These catch encoding mistakes (padding, `+` and `/`, base32 alphabet) that real keys might not.
- **`tokenVectors[]`**: 16 raw bytes in, the 22-character `tk` out.
- **`shapeVectors[]`**: is this string a valid device ID or token (length and alphabet only).

> **The private keys in this file are public and for tests only.** They were generated for these vectors and must never be used by the apps or for anything real. Secret scanners will flag them. That's expected.

## `pairing-urls.json`

- **`valid[]`**: `url` in, `parsed` out: `{ version, addresses: [{host, port}], id, token, name }` (`name` is `null` when absent). If `canonical` is `true`, building the URL from `parsed` (what the PC writes into the QR code) must give exactly `url`.
- **`invalid[]`**: `url` in, `error` out: `{ category, param? }`. Categories: `not_pairing_url`, `unsupported_version`, `missing_param`, `invalid_param`, `duplicate_param`. `param` is there for the last three. Test the category and the parameter name, not any message text. The order the checks run in matters, and some cases exist only to pin it down (see their `note`).
- Don't parse with a URL library. Libraries disagree on custom schemes, on `+`, and on repeated parameters. The rules are small; implement them as written.
- Numbers in a URL are ASCII digits only. Cases such as `:+80`, `v=%D9%A1` and a fullwidth port exist because `parseInt` (Java) and `parse` (Rust) accept a leading `+`, and Java's also accepts other scripts' digits.
- `valid[]` includes `100.64.0.0/10` addresses: the parser accepts them even though the PC doesn't write them in Phase 1.

## `messages.json`

Each case has `from` (the sender role, `phone` or `pc`), `to` (the receiver), `state` (`before_hello` or `open`: has the receiver already seen the peer's `hello`), exactly one input, and `expect`.

- **Input:** `frame` (a JSON object; serialize it compactly), `raw` (a text frame, send exactly this string; it is also how a case holds duplicate keys or a lone surrogate escape such as `\ud800`, which a JSON value can't express), `rawHex` (a text frame with exactly these payload bytes), `binaryHex` (a binary frame), or `synthesize` (a frame too big to embed: take `base`, set `base[field]` to `fill` repeated `fillCount` times, serialize compactly in key order; the result is exactly `totalBytes` UTF-8 bytes).
- **`expect.reply`:** `ack` (the receiver sends `{ "type": "ack", "ref": <frame id> }`; `expect.ref` pins that id when it matters, as in the duplicate-key case), `error` (the receiver sends `{ "type": "error", "code": …, "ref": … }`, with `ref` left out when it is `null`), or `none`.
- **`expect.close`:** the WebSocket close code the receiver then uses, or `null` to leave the connection open.
- **`expect.action`:** what the receiver does, by category: `notif_status`, `notif_icon`, `notif_post`, `notif_remove`, `notif_sync`, `notif_dismiss`, `notif_action`, `notif_dropped` (changelog 56, see `notif.json`), `handshake_ok`, `mark_delivered`, `mark_failed`, `show_error_only`, `deliver_text`, `set_clipboard`, `start_download`, `mark_transfer_done`, `cancel_transfer`, `remove_peer_and_close`, `show_ping_toast`, `ignore_duplicate` (a repeat of a processed ID: acked, not processed again), `close_only`. Unit tests check the category with a fake that records calls, not the real clipboard or notifications.
- **`valid[]`** has at least one example of every message type, plus the edge cases that must be accepted: unknown extra fields, `null` for an optional field, IDs that aren't UUIDs, a frame of exactly 1 MiB.
- **`invalid[]`** has the rejected ones: a missing field, a wrong type, a wrong number format, an unknown type, a wrong direction, a message before `hello`, oversize, binary and bad UTF-8 frames.
- **`sequences[]`**: steps run in order on one connection that starts before hello. Each role has its own receiver state. They cover duplicate IDs, "an invalid message doesn't close the connection", and what can precede `hello`.
- **Events in `sequences[]`** (1.4). A step with an `event` and no `from` or `frame` is something that happens to the receiving app, not a frame. `{ "event": "reconnect" }`: a new connection of the same pair. Every receiver is `before_hello` again (so a frame before the `hello` is `bad_request` and the 1002 close), and **its memory of message IDs stays**, because it is stored in the database. With `"device": "second"` (or `"first"` again) the new connection is **from another paired device**: message IDs are remembered per paired device, an offer's transfer ID is remembered for all of them with the device that made it. `{ "event": "transfer_over", "transferId": "…", "how": "done" | "canceled" | "failed" }`: the receiving app ended the download of that transfer, so the ID is used up. Then **an `offer` with an ID that is already in the memory** is answered like this (changelog 52): an `ack` with action `ignore_duplicate` if the earlier offer is **from the same device, whatever the state of its download** (unfinished, done, canceled or failed: the first response is repeated and nothing is downloaded again; the app carries on, or sends its `complete` or `cancel` again); `error` `bad_request` with `ref` = the offer's `id` (nothing is started) if the ID was first used by another message (a `text`, say), if it is an offer of **another device**, or (**optional**) if the same device's repeat lists other `fileId`s or sizes than the first one. A runner that has no use for an event ignores it.
- **`timers[]`**: `state` and `elapsedSinceOpenMs` in, `expect.close` out. A socket with no accepted `hello` after 10 seconds is closed with 1002 (no error message first). Test it with a fake clock.
- **`expect.sensitive`** (on a `deliver_text` case): whether the receiver treats the text as sensitive, that is whether the optional `sensitive` field is `true`. Absent means `false`, so every `deliver_text` case also checks the default. `text-sensitive-*` cases cover `true`, `false`, `null` (absent), a repeated key (the last wins) and the invalid types (`"true"`, `1`, `0`, an array, an object: `bad_request`).
- **`closeCodes[]`**: what a client does when the connection ends. `closeCode` (`null` = network error, no code) or `upgradeStatus` (the HTTP status that refused the upgrade) in, `action` out: `reconnect_with_backoff`, `reconnect_on_user_action`, `stop_update_app` or `stop_pair_again`. Some cases also have `helloAcceptedForMs` (how long the peer's `hello` had been accepted when the connection ended; `null` if none ever was) and `resetsBackoff`: **the backoff resets only when a connection stayed open with an accepted `hello` for at least 60,000 ms**, counted from the accepted `hello` and not from the socket opening. A close right after an accepted `hello`, or a connection with no accepted `hello`, leaves the backoff alone. Test it with a fake monotonic clock.
- **`strict-*` and `nesting-*` cases** are `raw` texts that are invalid JSON for the protocol: unquoted literals (`yes`, `True`, `NaN`, `Infinity`), `01`, `+1`, `.5`, `0x10`, single quotes, unquoted keys, comments, trailing commas, and nesting deeper than 64. All of them give `bad_request` with no `ref`, even though their `id` is fine, and the connection stays open. They come in pairs: the same literal in a field the message knows and in one it doesn't. `nesting-depth-64-*` (valid) sits right next to `nesting-depth-65-*` (invalid): the top-level object counts as level 1.
- **Invalid UTF-8** (`text-frame-invalid-utf8`) can only be checked on the PC side. OkHttp decodes text frames with replacement and never shows the bad bytes (see the spec), so the Android tests skip that one case. The 1 MiB cases need the app's own size check on both sides, because OkHttp doesn't limit incoming messages.

## `http.json`

Each exchange has a `context` (the PC-side state the answer depends on), a `request` and a `response`.

- `context.client.deviceId` is the device ID taken from the TLS client certificate. It's always there: a client certificate is mandatory in the TLS handshake, so a request without one never reaches HTTP (see `tls[]`). `context.client.paired` says whether it's in `devices`. Other context keys describe the pairing token, the transfer (bytes stored per file, and `owner`: the device that created or received it) or the offer.
- `body` is a JSON value (a string for a non-JSON body). `bodyRaw` is JSON text sent exactly as written, for what a JSON value can't express (duplicate keys, lone surrogate escapes). `wrongContentType: true` marks a request whose `Content-Type` is deliberately not JSON: the media type must be `application/json` (ASCII case-insensitive) with no parameter or exactly `charset=utf-8`, anything else is `400`, after `401` and `413` (`PUT` ignores `Content-Type`, see the `put-ignores-*` and `put-without-content-type` exchanges). `context.server.createFails: true` means the PC can't create the file (not "already exists"): `500 internal`. `bodyBytes: N` is a binary body of `N` bytes (the content doesn't matter), and `Content-Length` must equal it.
- **`context.server`** holds PC-side hooks: `freeBytes` (the free space; the check counts only the missing bytes, with no reserve), `diskFull: true` (= `freeBytes` 0), `tooLarge: true` (the transfer is over the PC's size limit), `createFails: true` and `bug: true` (an internal error). The `*-wins-over-*` exchanges pin the order of checks.
- **Windows tests (server side):** set up the context, send the request to the handler, and check `status`, the headers listed, and the body. Ignore `message` text, which is only an example. The body of an error is always `{ error, message }`, plus `offset` for `offset_mismatch`.
- **Android tests (client side):** feed `response` to the client (for example with OkHttp `MockWebServer`) and check what it does, such as retrying a `409 offset_mismatch` from `body.offset`, or restarting from byte 0 when a Range request is answered with `200`.
- **`sequences[]`** run their steps in order against one evolving context. A step with `response: null` was cut off after `interruptedAfterBytes` bytes. **A step with `clock: { advanceHours: N }` and no request moves the PC's clock forward by `N` hours**: a finished upload is answered (its offsets, an empty `PUT`) for 24 hours after its last file arrived and is then `404` for good, and a canceled ID is `404` at once (the `transfer-id-*` sequences). The server-side test shifts the finish time of the finished transfers by that much.
- **`errors[]`** is the table of codes and statuses as data.
- **`tls[]`** is about the handshake, not HTTP. `verifier` says who checks (`pc` checks the phone's client certificate, `phone` checks the server's), and `expect` is `handshake_ok` or `handshake_fails`. The cases pin down four rules: a client certificate is mandatory, its issuer, dates and extensions aren't checked (any client chain is accepted), the leaf must still be inside the certificate profile (`certificateProfile`: a v1 certificate, an unknown critical extension or a key that isn't P-256 is refused), and the CertificateVerify signature is always verified. Test them with real TLS: a local server and client with test keys, one client with no certificate, one signing with the wrong key. `thenHttp` names an exchange that runs after a successful handshake. The three cases about the leaf certificate (`certificateProfile`) use the DER files in [`fixtures/`](#fixtures).
  - **TLS 1.3 timing (`failureSurfacesOn`).** A client finishes its handshake call *before* the server has checked the client certificate. For the `pc` cases that expect `handshake_fails` (`client_first_read`), a test that only checks that the client's handshake call fails will wrongly pass the case to "ok": **read from the client's connection after the handshake and expect an error or a close** (no HTTP response ever arrives). For the `phone` cases (`client_handshake`) the failure is inside the phone's own handshake call, because its `TrustManager` checks the server certificate there.
  - **`optional: true`** (`phone-rejects-an-invalid-certificateverify-signature`): the phone's check of the server's signature is done by Android's TLS stack (Conscrypt), which a custom `TrustManager` can't turn off. Writing that test (an attacker server with the real certificate but the wrong key) is optional on the phone. The same case on the PC side is not optional.

## `fixtures/`

DER files for the negative `tls[]` cases, because neither rcgen nor OpenSSL 3 can write an X.509 v1 certificate or an unknown critical extension. They are made by `bun protocol/fixtures/make-fixtures.ts` (a small DER writer plus the test keys), and `check-vectors.ts` confirms that each one leaves the certificate profile in exactly the way its name says and still carries a valid self-signature. A handshake with one of them therefore fails for that reason only.

| File | What is wrong with it | `tls[]` case | Key that signs the CertificateVerify |
|---|---|---|---|
| `client-v1.der` | X.509 v1: no `[0]` version field | `pc-rejects-a-v1-client-certificate` | `device-id.json` test key 2 |
| `client-unknown-critical-extension.der` | a critical extension nobody knows (`1.3.6.1.4.1.99999.1`) | `pc-rejects-a-client-certificate-with-an-unknown-critical-extension` | `device-id.json` test key 2 |
| `client-not-p256.der` | the key is ECDSA P-384, not P-256 | `pc-rejects-a-client-certificate-that-is-not-p256` | `client-not-p256.key.der` (PKCS#8 DER) |

> **The private keys here are public and for tests only**, like the ones in `device-id.json`. Running the script again writes different bytes (ECDSA signatures are randomized). Nothing depends on the exact bytes.

**Android fixture (milestone 1.1).** `fixtures/android-client.der` is a certificate exported from the Android app's own `CertificateBuilder` (the Android unit tests read it from here too: `CertificateBuilderTest` uses `protocol/fixtures/android-client.{der,json}`, and the exporter writes to `android/app/build/fixture-export/`), and `fixtures/android-client.json` holds its expected `deviceId`, `shortId` and SPKI. There is no private key: it lives in the app, so the real handshake with it is the emulator run (`tools/e2e-1-1.ps1`). `desktop/src-tauri/tests/tls_vectors.rs` (`android_generated_certificate_is_accepted`, which **fails** if the fixture is missing: it is part of the repository) checks it against the certificate profile, the verifier, rustls-webpki's parser and the device ID in the sidecar, and `check-vectors.ts` checks the profile, the self-signature and the sidecar. (If a throwaway `android-client.key.der`, PKCS#8 DER, is ever added, the test also runs a real handshake with it.)

## `notif.json`

One object with the arrays below (and `limits`, every limit of the spec in one place). Only `own`, `peer` and the `expect.notif` shape are new compared with `messages.json`.

- **`nid[]`**: a notification `key` (UTF-8) and its `nid`: the first 32 hex characters of SHA-256. It has the two examples of the design, an empty tag, a phone number, non-ASCII and astral tags, another user and a 1,000-character key.
- **`iconHash[]`**: a PNG as base64url (no padding) and its `hash` (first 32 hex of SHA-256 of the PNG bytes) and size.
- **`icon[]`**: a `notif.icon` frame made from `hash` and `png` (or `pngFile`, a path relative to `protocol/`, read and encoded as base64url: the two files of exactly 32,768 and 32,769 bytes are in `fixtures/`; `omit` removes a field). `expect` is `accepted`, `dropped` (silently, counted) or `bad_request`. The frame limit for a `notif.icon` is 48 KiB, so the 32,769-byte PNG gets as far as the size check.
- **`clean[]`**: `field`, `input`, `expected` (`null` = absent). The five steps of `link.title` with the limit of that field (`limits`: `messageSender`, `messageText`, `line`, `hint` and `label` are the inner fields). A U+2028 or U+2029 is **kept** by the protocol: the PC's toast layer replaces it with a space when it draws the text.
- **`post[]`**: cases in the format of `messages.json` (`from`, `to`, `state`, `own`, `peer`, `frame`, `raw` or `synthesize`, `expect`). `expect.notif` is the **normalized post**: every field present with its default (`group`, `silent`, `sensitive`, `redacted` false, `importance` 3), absent optional strings left out, empty arrays left out, strings cleaned, arrays cut and filtered as the spec says. An invalid one expects `error` `bad_request` (or `too_large`) with `ref` = the frame's id when valid.
- **`code[]`**: `input`, `valid`: the PC's check of `code` (4 to 10 of `0-9A-Z-`, checked as sent).
- **`toastIds[]`** and **`toastArguments[]`**: the tag (= the `nid`), the group (`kin-` + the phone's 8-character short ID), the `launch` and button `arguments` strings, and strict parsing of an `arguments` string (exact keys in the order `kin`, `d`, `n`, `a`, `i`; `i` only for `reply` and `btn`; at most 200 characters). Every refused string is a case.
- **`rate[]`**: `limiter` `pc` (bucket 20, one token per second, an identical post of one `nid` within 2 s of the last accepted one is ignored and costs nothing, at most one `rate_limited` error per 10 s: `errorsAt` lists the times) or `phone` (bucket 10, one token per 500 ms, 20 posts per 60 s per package over the window (t − 60 s, t], removes up to 50 per 10 s and exempt from the bucket). `events` are in time order (`t` in ms from 0, the bucket starts full), `expect` has one outcome per event: `accepted`, `ignored_duplicate`, `dropped_rate` for the PC; `sent`, `dropped_package`, `dropped_budget`, `remove_sent`, `remove_dropped` for the phone. Tokens are counted in thousandths so there is nothing to round.
- **`coalesce[]`**: the phone's per-`nid` sender. `events` (`t`, `op` `post` or `remove`, `nid`, `content` = a stand-in for the whole content) in time order, `expect.sent` what goes on the wire and when. A post goes out at once unless the last one for that `nid` went out less than 1 s ago, in which case the newest content waits and goes out when that second ends (dropped then if it equals what was last sent); identical content is never sent; a remove cancels a pending post, goes out at once, and makes the `nid` new again.
- **`sendNotif[]`**: the accepted `hello`s of a peer (their extra fields) and whether a sender may send a `notif.*` to it (`send` or `nothing`): the latest accepted `hello` decides.
- **`syncDiff[]`**: the `nid`s on screen, the `nids` of a `notif.sync`, and the `nid`s that are removed.
- **`toastLabel[]`**: `input` -> `expected`: a hint or a button label in its **toast form** (U+2028, U+2029, U+FFFE and U+FFFF become U+0020, the ends are trimmed, nothing else changes, and the function is idempotent). The PC puts a label in this form on the toast and sends it in `notif.action.label`; the phone puts its own label in the same form (then cleans and cuts it to 30) before it compares. Both sides run it.
- **The arrays that only the phone runs** (design 3.11; the PC never does). Their reference models in `check-vectors.ts` are written from design 4.4, 4.7, 4.9 and 4.10; **where the design is ambiguous the reading is the one the Android code and its tests settled on**, and it is written here so nobody has to guess:
  - **`otpDetect[]`**: `title?`, `text`, `bigText?`, `personHasContactUri?` (a saved contact is the sender) and `expect` (the code, or `null`). One positive for **every language of the keyword table** (digits of other scripts are folded to ASCII first), the negatives of design 4.10 (an order number, a coupon, a time, an amount, a phone number, three groups, a date, two equal candidates, a friend's message) and the shape rules (`NNN-NNN`, `NNNN NNNN`, 5 to 8 upper-case letters and digits with both, a year is a code only when it is alone). **"At least 10 characters closer" is read as *more than* 10**: the design's own example, `Verification code 123456 or 654321` (the second is exactly 10 farther), must give no code. Keywords of the scripts that are written without spaces (Chinese, Japanese, Korean, Thai) match inside a run of text, and digits right against such text are fine; every other keyword is a whole word (`otp`, `2fa`, `pin` always).
  - **`redactedText`**: `known` (the placeholder strings of the vector: English and three more; the phone has the whole Android set) and `cases` of `sdk`, `texts` (`null` = absent) and `expect`. A text is the placeholder when its trimmed, case-folded form equals a known one, on API 35 and later only; a sentence that merely contains it is not.
  - **`skip[]`**: `snapshot` (fields that override `skipDefaults`: the names of the phone's `NotifSnapshot`; `messages` and `lines` are counts here), `policy?` (`enabled`, `allowed`, `hiddenAllowed`; default on, `com.example.chat` allowed), `flagged?` (packages the catalog calls sensitive), `ownPackage?`, `debugChannel?`, and `expect`: the reason code of the first rule of design 4.4 that matches (`own`, `not_allowed`, `other_profile`, `ongoing`, `summary`, `local_only`, `media`, `category`, `importance`, `secret`, `empty`), or `send` or `send_sensitive`.
  - **`pickActions[]`**: `actions` (each `index`, `title`, `semantic` 1 = reply, `remoteInputs` of `{resultKey, label, freeForm, hasChoices}`, `hasIntent` default true, `isActivity` true, false or `null` = unknown on API 29 and 30, `contextual`) and `expect` = `{reply: {i, hint?} | null, actions: [{i, label}]}`: the first free-form input that is not an activity (the reply semantic wins), at most 3 other buttons with a cleaned title, no remote input of any kind and not an activity, indexes 0 to 31, a hint of at most 60 and a label of at most 30 code points.

## `file-names.json`

`cases[]` of `{ name, input, existing, expected }`. `cleanFileName(input, existing)` must return `expected`. `existing` is the list of names already in the destination folder, and it's given as an input because the collision suffix depends on it. Names are compared case-insensitively.

**The 200 and 16 limits count UTF-8 bytes**, and a cut only falls on a code point boundary: take whole code points while the running UTF-8 size still fits. Never count or cut in UTF-16 units: Kotlin's `String.length` and `substring`, and JavaScript's `length`, count UTF-16 units. In Kotlin walk the code points (`codePoints()`) and add each one's UTF-8 size (1, 2, 3 or 4 bytes by code point range). In Rust walk `chars()` and add `char::len_utf8()`.

`createNew[]` is `{ name, operation?, input, existing, onDisk, createFails?, expected }`. `operation` is `create` (the default: the file is created for the first time) or `move_part` (`name.ext.part` is moved to its final name). Both follow the same rules, so the cases are the same shape: if the final name has appeared in the meantime, the move takes the next suffix, and "already exists" is never an error. For `move_part`, `existing` doesn't contain the transfer's own claim on the name. `existing` is what the app listed, and `onDisk` is what really exists when it creates the file (a file may have appeared since, or the filesystem may treat two names as one). Create with create-new semantics (`CREATE_NEW` / `O_EXCL`): if the create says "already exists", add the name to `existing` and try the next suffix. `expected` is the final name, or `{ "fails": "internal" }` when a create fails for any other reason. Test it with a temporary directory and the real create call, or with a fake that answers "already exists" for the names in `onDisk`.

## `urls.json`

`cases[]` of `{ name, input, isUrl }` (or a `synthesize` case with a `prefix`, a `fill` repeated `fillCount` times and a `suffix`, for the 8 KiB boundary, with the size of the trimmed text in `trimmedBytes`). `isOpenableUrl(text)` must equal `isUrl`. A sender sets the `isUrl` field of `text` to that value; a receiver offers "Open" only when the field is `true` **and** the same function says yes for the text it received. The rule is in [protocol.md](../docs/protocol.md#links-isurl-and-openable-urls): trim, `http`/`https` + `://`, no whitespace or control character anywhere, an authority with no `@`, `\` or `%` (changelog 55: no user information, no backslash, no percent-encoding), a non-empty host that isn't only dots and has none of `[ ] < > ^ |` (or a non-empty `[IPv6]` literal), a port of 1 to 65535 in ASCII digits, at most 8 KiB. **Don't use a URL library for this rule** (Android's `Uri`, `java.net.URL` and Rust's `url` crate each accept something else), **and don't use the platform's `isWhitespace()`**: Kotlin's and Rust's differ on U+0085, U+00A0 and U+2007. The set is listed in the spec and in `howToRead`.

## `links.json`

- **`urls[]`**: `isLinkUrl(url)` must equal `valid`. It is the openable-URL rule of `urls.json` applied to the string **as it is sent, with no trimming** (a leading or trailing space, newline or U+3000 makes it invalid), and at most 8,192 bytes of UTF-8. A `synthesize` case is `prefix` + `fill` repeated `fillCount` times + `suffix`, and `bytes` is its UTF-8 size. `unchanged: true` marks URLs (YouTube `t=`, `list=`, a fragment, percent escapes in the path or query, an upper-case scheme) that the receiver must store and hand to the opener as exactly the same string: no normalizing, no dropping of parameters.
- **`titles[]`** and **`sourceApps[]`**: `cleanLinkText(input, max)` must equal `expected` (`null` = absent), with `max` 300 (title) or 64 (sourceApp) **code points**. Steps, in order: Bidi_Control characters become `_`; control characters (U+0000-U+001F, U+007F-U+009F) become a space; runs of U+0020 collapse to one and the ends are trimmed; cut to `max` whole code points and trim trailing spaces again; empty means absent. No-break space is not a control character and is kept. Don't count UTF-16 units.
- **`sendAs[]`**: `hellos` are the (partial) `hello` frames the peer sent on one connection, in order; the latest accepted one counts. `expected` is how a sender puts a link on the wire: `link` only if the peer's `capabilities` is an array with the string `"link"`, else `text`. A `capabilities` that is not an array counts as none, entries that aren't strings are skipped, and the `hello` is still accepted (`helloStillAccepted`). Android: `capabilities` of the Kotlin parser must not throw on `"link"`, `null` or an object.
- **`display[]`** (changelog 55): `{ name, url, display, warn, parseFails?, note? }`. `url` is valid as a link. `display` is the address a receiver shows next to a link's title: **the host a browser connects to** (WHATWG URL Standard: lower case, IDNA punycode, IPv4 in any spelling as dotted decimal, IPv6 compressed in brackets, trailing dot kept, the port only when it is not the scheme's default). `warn` says the address carries a warning (the apps put a warning sign in front of it, and the PC never opens such a link by itself): the host **as written** has a non-ASCII character, **or** the address shown has an `xn--` label, **or** it is an IP address (IPv4 in any form, or IPv6), **or** a browser refuses the host. For `parseFails: true` a browser refuses the URL (`new URL(url)` throws): the display is then the host as written, ASCII lower-cased, every character outside U+0021..U+007E as U+FFFD, no port, and `warn` is true. For every other case `display` equals `new URL(url).host` in a browser or in Bun (`check-vectors.ts` checks that). Unlike the accept/reject rule, **the display needs a WHATWG host parser with IDNA/UTS 46** (the `url` crate in Rust; on Android `java.net.IDN` is IDNA2003 and differs for `ß`, full-width forms and some others: the vectors win). **What is shown is the host only**: no userinfo exists any more, and the path, query and fragment are not part of it.
- **`textFallback[]`**: the `text` fields a sender uses for a link that goes as text: `text` = the `url`, `isUrl` = `true`, no title, the same `id` as the link. (An old phone or PC then still offers "Open".)

## `text-rules.json`

- **`lineEndings[]`**: `toWire(clipboardText)` equals `wire` (every CRLF becomes LF in one pass, a lone CR stays), `toWindowsClipboard(wire)` equals `windowsClipboard` (every LF not preceded by a CR becomes CRLF), and `wireSha256Hex` is the SHA-256 of the UTF-8 of the **wire** form: the loop guard hashes that. The Android side only has to pass `toWire` and the hash; the Windows conversion is for the PC.
- **`senderSize[]`**: `decideSend(kind, sensitive, wireBytes)` equals `expected` (`text`, `file`, `clipboard`, `too_large` or `not_sent`). The size is measured after the line-end conversion.
- **`sensitiveSender[]`** and **`receiver[]`**: category tests with fakes (a clipboard that offers the listed formats, or the `EXTRA_IS_SENSITIVE` extra; a notifier and a history that record calls). What matters: the PC marks a hotkey send `sensitive` when the clipboard carried an opt-out format, never sends such content automatically, and a receiver shows no preview, offers no "Open" and stores and logs nothing of a sensitive text.
- **`windowsSensitiveFormats`**: what a Windows receiver writes along with a sensitive text.

## Notes for the implementers

Things the libraries do differently from the spec. Each one is covered by vectors.

**Parsing JSON (both apps)**

- Parse into a **generic tree first** (`serde_json::Value` in Rust, kotlinx `JsonElement`), then validate field by field, in the order the spec gives. Don't deserialize straight into typed structs. For duplicate keys the last value wins (the `duplicate-keys-last-wins` cases), and derived serde structs and typed kotlinx decoding don't agree on that.
- **Rust, lone surrogates:** `serde_json` already rejects `"\ud800"`. Map that parse error to `bad_request` with no `ref`, like any other syntax error.
- **Kotlin, lone surrogates:** kotlinx accepts `"\ud800"` and hands you a string with an unpaired surrogate. After parsing, walk the tree (object keys too) and treat an unpaired UTF-16 surrogate in any string as a parse failure.
- **Kotlin, strict JSON first.** kotlinx `parseToJsonElement` very likely accepts unquoted literal values even with `isLenient = false`: `"isUrl":yes`, `"x":01`, `"x":NaN` (a bare run of characters that isn't a string is read as a literal and kept as text). Not verified here, so assume it does. **Validate the raw text with a strict RFC 8259 tokenizer or validator *before* you rely on kotlinx's tree** (a small hand-written recursive-descent checker is enough; it must reject unquoted literals, leading zeros, `+`, `.5`, `1.`, hex, single quotes, unquoted keys, comments, trailing commas, and anything after the one JSON value), and give any failure the usual `bad_request` with no `ref`. The `strict-*` cases are the test, in known fields and in unknown ones: kotlinx keeps unknown fields in the tree too, so checking only the fields you read isn't enough.
- **Nesting depth (both apps).** Count the depth yourself, on the raw text, **before** parsing: at most 64 levels, the top-level object is level 1, and a deeper text is invalid JSON (`bad_request`, no `ref`). A scanner that counts `[` `{` against `]` `}` while skipping string contents does it in one pass and can't overflow a stack (`nesting-depth-1000`). `serde_json` stops at 128 on its own, which is too late, and kotlinx has no limit that matters.
- **Kotlin, integers and booleans:** kotlinx may coerce a quoted `"10"`, or an exponent form such as `1e3`, into a `Long`, because `JsonPrimitive.longOrNull` and `.long` parse the primitive's text whatever its JSON type. `booleanOrNull` accepts the quoted string `"true"` in the same way. Validate from the primitive itself: require `!primitive.isString`, match `primitive.content` against `0|[1-9][0-9]*` (no sign, no fraction, no exponent), check the 2⁵³−1 limit, and only then convert. Never call `.long` or `.boolean` on a primitive that hasn't passed this. A string `"true"` for `isUrl` is `bad_request`.
- **Rust, integers:** use `serde_json::Number::as_u64()`. A number written with a fraction or exponent (`1.0`, `1e3`) is stored as a float, so `as_u64()` returns `None` for it, which is what the spec wants. Check the 2⁵³−1 limit as well.
- **Digits in URLs and headers:** `Integer.parseInt` and `Long.parseLong` (Java, Kotlin) and `str::parse::<u16>()` / `::<u64>()` (Rust) accept a leading `+`, and Java's also accept other scripts' digits. Match `[0-9]+` against the text first, then convert. Don't use a typed `Query<u64>` for `offset`.

**HTTP server (desktop app)**

- axum's default `Json` extractor answers `415` (a missing or non-JSON `Content-Type`), `422` (valid JSON of the wrong shape) or a `400`, each with a **plain-text** body. The spec wants `400 bad_request` with the JSON error body for all of them. Write your own extractor, or a rejection mapping, that takes the raw body, parses it into a `serde_json::Value`, and returns the protocol's error. The same goes for `Query` and `Path` rejections, and for unknown routes and methods (`404 not_found` with a JSON body, not axum's empty `405`, and for paired and unpaired devices alike: put the authorization check on the real endpoints, not around the fallback). Check `Content-Type` yourself, as the spec says (`application/json` ASCII case-insensitive, no parameter or exactly `charset=utf-8`): axum's `Json` extractor would also accept `application/vnd.api+json` and any parameter. `PUT …/files/…` must not look at `Content-Type` at all.
- **Moving `name.ext.part` to its final name must not overwrite.** Rust's `std::fs::rename` **replaces** an existing target on Windows. Use `MoveFileExW` **without** `MOVEFILE_REPLACE_EXISTING` (it fails with `ERROR_ALREADY_EXISTS`), or an equivalent no-replace rename, and treat that error, like `ERROR_FILE_EXISTS` from `CREATE_NEW`, as "already exists": take the next suffix and store the new final name. The `move-part-*` cases in `file-names.json` are the test; `move-part-case-folding-by-the-filesystem` is the one a replacing rename would fail.

**Android: the certificate**

- The phone has to produce a certificate that fits the [certificate profile](../docs/protocol.md#transport), because the PC's rustls-webpki refuses anything else (v1/v2 certificates, unknown critical extensions, bad encodings). Android has no public API for building an X.509 certificate, so there are two ways, and the choice is the 1.1 Android agent's:
  - **Hand-roll the DER** from the profile. It's one fixed structure: the `minimal` certificates in `device-id.json` (`keys[].certificatePem`, 311 or 312 bytes) are the template. Copy the layout, put in your own serial, name, times and SPKI, and sign the TBS bytes with `SHA256withECDSA` (the JCA signature is already DER-encoded, which is what the certificate wants). The profile's rules are small: version `[0] 2`, serial positive and at most 20 bytes with no redundant leading zero, `ecdsa-with-SHA256` with no parameters twice (inside the TBS and outside it), UTCTime up to 2049 and GeneralizedTime from 2050, SPKI of 91 bytes, no extensions.
  - **Use BouncyCastle `bcpkix`** (`X509v3CertificateBuilder` with a `JcaContentSignerBuilder("SHA256withECDSA")`). Mind the **~10 MB APK target** ([targets.md](../docs/targets.md)): `bcpkix` plus `bcprov` add several MB unless R8 shrinks them, and R8 needs keep rules for the provider classes it reaches by reflection. Measure the release APK before and after. Hand-rolling avoids the cost altogether.
  - Either way, **check the result with the same rules the reference checker uses** (`certProfileViolations` in `check-vectors.ts`: version, key type and curve, signature algorithm, serial length, no critical extension) and with `openssl x509 -noout -text`.
- **For 1.1: export one Android-generated certificate as a fixture** (a new `device-id.json` entry or a file next to it, PEM) and **make the Rust verifier tests accept it**. The `tls[]` vectors only prove the PC accepts the certificates that the *test keys* produce. The first real Android certificate is the test that matters, and a mistake here shows up as a handshake that fails with no HTTP response to read.

**TLS**

- The PC doesn't check the issuer, dates or extensions of a client certificate, but the leaf must be a parseable v3 certificate with a P-256 key (certificate profile), and it must still verify the CertificateVerify signature. In the rustls `ClientCertVerifier` that means implementing `verify_tls13_signature` properly (call the crypto provider's verification function, and never return an "assertion is valid" without checking), and making the client certificate mandatory. `verify_tls12_signature` can return an error, because TLS 1.2 isn't used. The `tls[]` vectors are the tests for this. A TLS 1.3 client's handshake call returns before the server has checked its certificate, so the failure appears on the client's first read: see `failureSurfacesOn` above.
- **Enforce P-256 in rustls yourself.** rustls maps `ecdsa_secp256r1_sha256` to the P-256 *and* the P-384 algorithm in its default list (`provider.signature_verification_algorithms`), so with that list a client with a P-384 certificate connects. In the `ClientCertVerifier`:
  1. `supported_verify_schemes()` returns only `SignatureScheme::ECDSA_NISTP256_SHA256`.
  2. `verify_tls13_signature` gets your own `WebPkiSupportedAlgorithms`: `all` is just `webpki::aws_lc_rs::ECDSA_P256_SHA256` (crate `rustls-webpki`, feature `aws-lc-rs`) and `mapping` has the one entry `(ECDSA_NISTP256_SHA256, [ECDSA_P256_SHA256])`. Both are `static` slices.
  3. `verify_client_cert` reads the leaf's SubjectPublicKeyInfo and requires `id-ecPublicKey` + `prime256v1` in exactly 91 bytes (the prefix `3059301306072a8648ce3d020106082a8648ce3d030107034200`, then `04` and the 64 bytes of the point), and returns an error otherwise. It never skips this, and the signature check never replaces it.
  4. Never return an assertion without having verified the TLS 1.3 CertificateVerify signature.
  5. **Take the device ID from the SPKI that the signature was verified against.** `verify_tls13_signature` parses the certificate with rustls-webpki. If the app reads the certificate with another parser (x509-parser, for the device ID), a certificate that the two read differently could give an ID that belongs to another key than the one that signed. So the ID comes from webpki's SPKI (`EndEntityCert::subject_public_key_info`), and the verifier refuses a certificate on which the two parsers disagree byte for byte (`tls::agreed_spki`, `tls::client_spki`).

  The reference is `desktop/src-tauri/src/tls.rs`, and `desktop/src-tauri/tests/tls_vectors.rs` runs every `tls[]` case against it.

**WebSocket**

- OkHttp decodes text frames with replacement and puts no limit on incoming message size. tungstenite reports invalid UTF-8 as an error without sending a 1007 close. So each app checks the size of what it receives itself, and enforces the 1 MiB limit with a 1009 close. See the spec, Control channel, Frames.

## Loading the files

**Android** (Gradle unit tests read them as classpath resources):

```kotlin
// android/app/build.gradle.kts   (path is relative to this file: android/app -> repo root / protocol)
android {
    sourceSets.getByName("test").resources.srcDir("../../protocol")
}
```

```kotlin
val text = javaClass.classLoader!!.getResource("pairing-urls.json")!!.readText(Charsets.UTF_8)
val root = Json.parseToJsonElement(text).jsonObject
```

(The directory also holds the `.ts` and `.md` files. They end up as harmless extra resources.)

**Rust** (paths relative to the crate, so tests run from any working directory):

```rust
let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("../../protocol/pairing-urls.json"); // desktop/src-tauri -> repo root / protocol
let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
```

Adjust the number of `..` to where `Cargo.toml` really is (for `desktop/Cargo.toml` it's `../protocol`).

## Checking the vectors

```
bun protocol/check-vectors.ts
```

This is a reference implementation of the device ID, the certificate profile (it walks the DER of every certificate in `device-id.json`: version, key type and curve, signature algorithm, serial, time encodings, critical extensions), the pairing URL rules, the file name rules, the WebSocket receiver (including strict JSON, the nesting limit, the 10-second rule, what a client does on each close code and when the backoff resets), a model of the HTTP endpoints (Content-Type rule, order of checks, ownership, create failures), and the TLS handshake rules, written from the spec text. It also confirms that every case name listed under "What each milestone needs" exists. It loads every file here and checks every case. Exit code 0 means every vector passes. It's not shipped in either app. It exists so the vectors and the spec can't drift apart.

## Changing a vector

1. Change `docs/protocol.md` first (or at the same time).
2. Update the vectors and, if the rule changed, `check-vectors.ts`.
3. Run `bun protocol/check-vectors.ts` until it passes.
4. Tell both app agents, since their tests read the same files.

The expected values in the vectors are written by hand from the spec, not produced by running an implementation. Keep it that way.
