# protocol

Example messages and test vectors shared by both apps. The Android and Windows tests load these files, so the two sides stay compatible. The protocol itself is described in [docs/protocol.md](../docs/protocol.md). If a vector and the spec disagree, one of them is a bug. Fix it before building on either.

| File | What it checks | Spec section |
|---|---|---|
| [`device-id.json`](device-id.json) | Device ID, short ID, token encoding, SPKI from a key and from a certificate, the certificate profile | Encodings, Transport |
| [`pairing-urls.json`](pairing-urls.json) | Parsing the QR / link text, and the PC building it | Pairing > Parsing rules |
| [`messages.json`](messages.json) | WebSocket messages: what the receiver replies and does | Control channel |
| [`http.json`](http.json) | HTTP requests and responses, error bodies | Pairing, Files, Error codes |
| [`file-names.json`](file-names.json) | File name cleaning and collision handling | File names |
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

**1.3** adds the transfers and file names: the `offer`, `complete` and `cancel` cases and `duplicate-offer-is-acked-not-restarted` of `messages.json`, every other exchange and sequence of `http.json` (`transfers-*`, `put-*`, `get-*`, `delete-*`, `internal-error`, `upload-*`, `download-*`, `repeat-with-different-files-keeps-the-stored-list`) and all of `file-names.json`.

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
- **`expect.action`:** what the receiver does, by category: `handshake_ok`, `mark_delivered`, `mark_failed`, `show_error_only`, `deliver_text`, `set_clipboard`, `start_download`, `mark_transfer_done`, `cancel_transfer`, `remove_peer_and_close`, `show_ping_toast`, `ignore_duplicate` (a repeat of a processed ID: acked, not processed again), `close_only`. Unit tests check the category with a fake that records calls, not the real clipboard or notifications.
- **`valid[]`** has at least one example of every message type, plus the edge cases that must be accepted: unknown extra fields, `null` for an optional field, IDs that aren't UUIDs, a frame of exactly 1 MiB.
- **`invalid[]`** has the rejected ones: a missing field, a wrong type, a wrong number format, an unknown type, a wrong direction, a message before `hello`, oversize, binary and bad UTF-8 frames.
- **`sequences[]`**: steps run in order on one connection that starts before hello. Each role has its own receiver state. They cover duplicate IDs, "an invalid message doesn't close the connection", and what can precede `hello`.
- **`timers[]`**: `state` and `elapsedSinceOpenMs` in, `expect.close` out. A socket with no accepted `hello` after 10 seconds is closed with 1002 (no error message first). Test it with a fake clock.
- **`closeCodes[]`**: what a client does when the connection ends. `closeCode` (`null` = network error, no code) or `upgradeStatus` (the HTTP status that refused the upgrade) in, `action` out: `reconnect_with_backoff`, `reconnect_on_user_action`, `stop_update_app` or `stop_pair_again`. Some cases also have `helloAcceptedForMs` (how long the peer's `hello` had been accepted when the connection ended; `null` if none ever was) and `resetsBackoff`: **the backoff resets only when a connection stayed open with an accepted `hello` for at least 60,000 ms**, counted from the accepted `hello` and not from the socket opening. A close right after an accepted `hello`, or a connection with no accepted `hello`, leaves the backoff alone. Test it with a fake monotonic clock.
- **`strict-*` and `nesting-*` cases** are `raw` texts that are invalid JSON for the protocol: unquoted literals (`yes`, `True`, `NaN`, `Infinity`), `01`, `+1`, `.5`, `0x10`, single quotes, unquoted keys, comments, trailing commas, and nesting deeper than 64. All of them give `bad_request` with no `ref`, even though their `id` is fine, and the connection stays open. They come in pairs: the same literal in a field the message knows and in one it doesn't. `nesting-depth-64-*` (valid) sits right next to `nesting-depth-65-*` (invalid): the top-level object counts as level 1.
- **Invalid UTF-8** (`text-frame-invalid-utf8`) can only be checked on the PC side. OkHttp decodes text frames with replacement and never shows the bad bytes (see the spec), so the Android tests skip that one case. The 1 MiB cases need the app's own size check on both sides, because OkHttp doesn't limit incoming messages.

## `http.json`

Each exchange has a `context` (the PC-side state the answer depends on), a `request` and a `response`.

- `context.client.deviceId` is the device ID taken from the TLS client certificate. It's always there: a client certificate is mandatory in the TLS handshake, so a request without one never reaches HTTP (see `tls[]`). `context.client.paired` says whether it's in `devices`. Other context keys describe the pairing token, the transfer (bytes stored per file, and `owner`: the device that created or received it) or the offer.
- `body` is a JSON value (a string for a non-JSON body). `bodyRaw` is JSON text sent exactly as written, for what a JSON value can't express (duplicate keys, lone surrogate escapes). `wrongContentType: true` marks a request whose `Content-Type` is deliberately not JSON: the media type must be `application/json` (ASCII case-insensitive) with no parameter or exactly `charset=utf-8`, anything else is `400`, after `401` and `413` (`PUT` ignores `Content-Type`, see the `put-ignores-*` and `put-without-content-type` exchanges). `context.server.createFails: true` means the PC can't create the file (not "already exists"): `500 internal`. `bodyBytes: N` is a binary body of `N` bytes (the content doesn't matter), and `Content-Length` must equal it.
- **Windows tests (server side):** set up the context, send the request to the handler, and check `status`, the headers listed, and the body. Ignore `message` text, which is only an example. The body of an error is always `{ error, message }`, plus `offset` for `offset_mismatch`.
- **Android tests (client side):** feed `response` to the client (for example with OkHttp `MockWebServer`) and check what it does, such as retrying a `409 offset_mismatch` from `body.offset`, or restarting from byte 0 when a Range request is answered with `200`.
- **`sequences[]`** run their steps in order against one evolving context. A step with `response: null` was cut off after `interruptedAfterBytes` bytes.
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

## `file-names.json`

`cases[]` of `{ name, input, existing, expected }`. `cleanFileName(input, existing)` must return `expected`. `existing` is the list of names already in the destination folder, and it's given as an input because the collision suffix depends on it. Names are compared case-insensitively.

**The 200 and 16 limits count UTF-8 bytes**, and a cut only falls on a code point boundary: take whole code points while the running UTF-8 size still fits. Never count or cut in UTF-16 units: Kotlin's `String.length` and `substring`, and JavaScript's `length`, count UTF-16 units. In Kotlin walk the code points (`codePoints()`) and add each one's UTF-8 size (1, 2, 3 or 4 bytes by code point range). In Rust walk `chars()` and add `char::len_utf8()`.

`createNew[]` is `{ name, operation?, input, existing, onDisk, createFails?, expected }`. `operation` is `create` (the default: the file is created for the first time) or `move_part` (`name.ext.part` is moved to its final name). Both follow the same rules, so the cases are the same shape: if the final name has appeared in the meantime, the move takes the next suffix, and "already exists" is never an error. For `move_part`, `existing` doesn't contain the transfer's own claim on the name. `existing` is what the app listed, and `onDisk` is what really exists when it creates the file (a file may have appeared since, or the filesystem may treat two names as one). Create with create-new semantics (`CREATE_NEW` / `O_EXCL`): if the create says "already exists", add the name to `existing` and try the next suffix. `expected` is the final name, or `{ "fails": "internal" }` when a create fails for any other reason. Test it with a temporary directory and the real create call, or with a fake that answers "already exists" for the names in `onDisk`.

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
