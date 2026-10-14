# Protocol v1

The PC runs an HTTPS server. The phone is always the client. Example messages and test vectors live in `protocol/` at the repo root (see [protocol/README.md](../protocol/README.md)), and both apps' tests use them. Where this document and a vector disagree, that's a bug in one of them. Fix it before building on either.

Words like MUST and MUST NOT are used only where two implementations could otherwise differ.

## Transport

- TLS 1.3 only, over TCP. HTTP/1.1 (ALPN `http/1.1`).
- Default port 47821. If it's taken, the PC uses a random port and advertises it.
- **Certificate profile.** Both sides generate a self-signed **X.509 v3** certificate, DER-encoded:
  - **Key:** ECDSA P-256 (`id-ecPublicKey` with the curve `prime256v1`, uncompressed point), so the SubjectPublicKeyInfo is 91 bytes. **Signature algorithm:** `ecdsa-with-SHA256` (the AlgorithmIdentifier carries no parameters, as in RFC 5758). In the TLS handshake the signature scheme is `ecdsa_secp256r1_sha256`.
  - **Serial:** a positive integer of at most 20 bytes.
  - **Validity** uses the RFC 5280 time encodings (UTCTime up to 2049, GeneralizedTime from 2050). Any dates are fine, because receivers ignore them.
  - **Extensions:** none are required, and **none may be critical** (non-critical ones are fine). **Subject and issuer** are free, for example `CN=Kin`. rustls-webpki also rejects issuer and subject unique IDs, and malformed or duplicated known extensions, even non-critical ones. A certificate with no extensions at all avoids every one of these, so write one.
  - **Why this is written down:** the PC verifies the CertificateVerify signature with rustls-webpki (`verify_tls13_signature`, see below), and webpki rejects v1 and v2 certificates, unknown critical extensions and bad encodings. A phone certificate outside this profile would fail the handshake even though nothing else is wrong. The certificates in `protocol/device-id.json` follow the profile and are the templates.
- **Server certificate:** the PC's self-signed certificate. The phone ignores hostname and expiry and checks only that SHA-256(SPKI) matches the pinned PC device ID.
- **Client certificate: mandatory at the TLS level.** The PC asks every client for a certificate, and a handshake in which the client presents none **fails**. The connection is closed during the handshake and no HTTP response is sent. During the handshake the PC doesn't check the issuer, the dates or the extensions of the client certificate, and it doesn't need to know the device, because an unpaired phone has to be able to reach `POST /v1/pair`. But the leaf must be a parseable v3 certificate with a P-256 key (see the certificate profile).
- **TLS 1.3 timing:** the client finishes its side of the handshake *before* the server has checked the client certificate. So when the PC refuses a client (no certificate, a bad signature, a certificate outside the profile), the phone's handshake call still returns success, and the failure shows up as an alert or a closed connection on the phone's **first read** (or write). The phone treats that as a failed connection, never as "connected", and its tests read from the connection after the handshake instead of only checking that the handshake call returned.
- **Signatures MUST be verified.** Not checking the issuer doesn't mean skipping the TLS 1.3 CertificateVerify signature. That signature proves the peer holds the private key that belongs to the certificate it presented. Without the check, anyone could replay a paired phone's public certificate and be taken for that phone. With rustls this means implementing `verify_tls13_signature` in the custom certificate verifier so it really checks the signature (for example by calling the crypto provider's verification function), and never returning an "assertion is valid" without checking. The phone does the same for the server's signature: Android's TLS stack (Conscrypt) always checks it and a custom `TrustManager` can't turn that off, so a phone-side test for an invalid server signature is optional, but a rustls verifier on the PC side must do it itself. TLS 1.2 is never negotiated, so the TLS 1.2 verification method just returns an error.
- **Authorization** happens per request, after the handshake:
  - `POST /v1/pair` accepts any certificate *that passed the handshake* and needs a valid pairing token.
  - Every other endpoint needs a client certificate whose device ID is in `devices`. Otherwise it returns `401 unauthorized`. So `401 unauthorized` always means "valid certificate, but this device is unpaired (or was removed)". It never means "no certificate".
- A peer's device ID is always computed from the leaf certificate it presented in the TLS handshake (see [Encodings](#encodings)). It's never taken from a request or message body.
- JSON request and response bodies are sent with `Content-Type: application/json; charset=utf-8`.
- **Content-Type of a request body.** The media type must be `application/json`, compared ASCII case-insensitively, with either no parameter or exactly `charset=utf-8` (name and value also case-insensitive; spaces or tabs around the `;` are fine). Anything else is `400 bad_request`: a missing header, `text/plain`, `application/vnd.api+json`, `application/json; charset=utf-16`, a quoted `charset="utf-8"`, any other parameter. This check is part of "body invalid" in the order of checks (see [Files](#files-phone--pc) and [Pairing errors](#steps)): after `401` and the 1 MiB `413`, before the body is parsed. It applies to `POST /v1/pair` and `POST /v1/transfers`. `PUT …/files/…` carries raw bytes and **ignores `Content-Type`**.

## Discovery

The PC advertises itself on the local network with mDNS (DNS-SD):

- **Service type:** `_kinrelay._tcp`. The service port is the port the HTTPS server listens on (47821, or the random fallback from [Transport](#transport)).
- **TXT records:** `id` = the PC's **short ID** (8 characters, see [Encodings](#encodings)), and `v` = `1` (the protocol version).
- The phone matches the `id` TXT record against the short ID of the PC it is paired with. A match is only a hint about where to connect: the full device ID is checked in the TLS handshake, as always. A record with a different `v` isn't used.
- The phone never advertises itself. When it looks the PC up (only while reconnecting, for at most 5 seconds) is described in [architecture.md](architecture.md#finding-each-other).

## Encodings

- **base64url** means RFC 4648 §5 (alphabet `A–Z a–z 0–9 - _`) **without padding**. A `=` is never written and is invalid when read. It's used for device IDs and pairing tokens.
- **Device ID** = base64url(SHA-256(DER-encoded SubjectPublicKeyInfo)). The input is the SPKI exactly as it appears in the certificate (91 bytes for a P-256 key). The output is always **43** characters. Device IDs are compared as exact, case-sensitive strings. Nobody decodes them to compare.
- **Short ID** (the ID shown in the UI and the `id` TXT record in mDNS): take the same raw 32-byte SHA-256 hash and encode it with RFC 4648 §6 base32 (alphabet `A–Z 2–7`, uppercase, no padding). The short ID is the **first 8 characters** of that encoding. The UI may show it as `XXXX-XXXX`, but the stored and advertised value has no hyphen.
- **Pairing token** (`tk`): 16 bytes from a cryptographically secure random generator, written as base64url, which is always **22** characters. Tokens are compared as exact strings (in constant time), not decoded.
- **Validity check** for a device ID or token read from the wire or a QR code: the length above, and only characters from the base64url alphabet. Nothing else is checked.
- **Digits** outside JSON are **ASCII `0`–`9` only**. This covers the pairing URL's `v`, the IPv4 octets and the port, the `offset` query parameter and the `N` in `Range: bytes=N-`. No sign (`+5` and `-5` are both invalid), no spaces, no other Unicode digits (`٣`, `３`). Check the text against `[0-9]+` before converting it, and don't hand raw text to a library parser: Java's `Integer.parseInt` and `Long.parseLong` and Rust's `str::parse` accept a leading `+`, and Java's also accept non-ASCII digits. Where a rule says "no leading zeros", a lone `0` is fine and `00`, `007` are invalid. The `offset` value is also limited to 2⁵³−1.

## JSON conventions

These apply to WebSocket messages and to every HTTP JSON body.

- Text is UTF-8. Field names are camelCase and case-sensitive, exactly as written in this document. No Unicode normalization is applied to any value.
- **Required fields** must be present with the right type. A missing field, a wrong type, or an explicit `null` for a required field makes the message invalid.
- **Optional fields** are omitted when absent. Senders never write `null`. Receivers treat `null` for an optional field as absent.
- **Unknown fields** are ignored, on every message and body. This is how v1 gets new optional fields.
- **Integers** are JSON numbers with no fraction and no exponent (`1.0` and `1e3` are invalid), and are in the range 0 to 2⁵³−1 (so the TypeScript UI never loses precision). This covers `protocol`, `size`, `totalSize`, `offset`, `received` and `sentAt`. Kotlin reads them as `Long` and Rust as `u64`. Negative values are invalid.
- **Booleans** are JSON `true` / `false` only. `"true"`, `1` and `0` are invalid.
- **Timestamps** are Unix epoch milliseconds, UTC, as an integer. In v1 the only one is `ping.sentAt`. It's informational, and receivers never use it for logic, because the two clocks can differ.
- **Strings** must be valid Unicode. A sender whose platform string can hold an unpaired surrogate (for example Android clipboard text) replaces it with U+FFFD before serializing. When this document counts "characters" in a string, it means Unicode code points, not UTF-16 units or bytes.
  - **Lone surrogate escapes are invalid.** A string, or an object key, that contains an unpaired surrogate (`"\ud800"`, `"\udc00"`, or a high surrogate not followed by a low one) makes the whole text invalid JSON, the same as a syntax error: a WebSocket message gets `bad_request` with no `ref` (validation step 2), and an HTTP body gets `400 bad_request`. A correct pair (`"\ud83d\ude00"`) is fine. `serde_json` already rejects lone surrogates and kotlinx doesn't, so the Kotlin side checks the parsed strings (keys too) for unpaired surrogates.
- **Duplicate keys: the last value wins.** `{"a":1,"a":2}` is `{"a":2}`. Senders never write duplicates, but receivers must all treat them the same way. To get this on both sides, **parse into a generic tree first** (`serde_json::Value` in Rust, `JsonElement` in Kotlin), then validate the tree field by field. Don't deserialize straight into typed structs: depending on the library that rejects duplicates or picks the first value.
- **Strict JSON (RFC 8259).** Anything a lenient parser would let through but RFC 8259 doesn't is invalid JSON, in a known field and in an unknown one alike: unquoted literals (`"isUrl":yes`, `NaN`, `Infinity`, `True`), numbers such as `01`, `+1`, `.5`, `1.`, `0x10`, single-quoted strings, unquoted keys, comments and trailing commas. A WebSocket message that fails this gets `bad_request` with no `ref` (validation step 2), and an HTTP body gets `400 bad_request`. kotlinx.serialization's `parseToJsonElement` very likely accepts unquoted literals even in strict mode, so the Kotlin side validates the raw text itself before it looks at the tree (see [protocol/README.md](../protocol/README.md)).
- **Nesting depth: at most 64.** Arrays and objects may be nested 64 levels deep, counting the top-level object as level 1. Anything deeper is invalid JSON, the same as a syntax error (`bad_request` with no `ref`, or `400`), even in an unknown field. `serde_json` stops at 128 on its own and kotlinx has no limit that matters, so each side counts the depth itself.
- Wire form: senders write compact JSON (no insignificant whitespace). Receivers accept any valid JSON.

## Pairing

### QR code contents

```
kinrelay://pair?v=1&a=192.168.1.20:47821,10.0.0.5:47821&id=<pc-device-id>&tk=<token>&n=<pc-name>
```

| Field | Required | Meaning |
|---|---|---|
| `v` | yes | Protocol version. Must be `1` |
| `a` | yes | 1 to 3 IPv4 `host:port` entries, comma-separated. In Phase 1 the PC writes only private (RFC 1918) addresses and leaves out loopback, link-local, VPN (including `100.64.0.0/10`) and virtual adapters (Hyper-V, WSL). **The address of the adapter that holds the default route is never left out because of the adapter**, even if it looks virtual (a Hyper-V `vEthernet` switch can be the PC's only network). The phone's parser accepts a wider range, see the parsing rules |
| `id` | yes | PC device ID: 43 base64url characters |
| `tk` | yes | Pairing token: 22 base64url characters (16 random bytes). Single use, valid for 5 minutes |
| `n` | no | PC name, percent-encoded UTF-8 |

The PC writes the parameters in the order `v`, `a`, `id`, `tk`, `n`, and omits `n` if it has no name. It percent-encodes `n` as UTF-8, writing every byte except `A–Z a–z 0–9 - . _ ~` as `%XX` with uppercase hex (so a space is `%20`, never `+`). In `a` it leaves `:` and `,` unescaped. Names longer than 64 characters are cut to 64 before encoding.

In Phase 2 the QR switches to an `https://<domain>/pair#...` link so a normal camera can open the app. The phone app accepts both forms from day one.

### Parsing rules

Both apps parse the scanned text exactly like this. (The apps use their own small parser, not a URL library, because libraries disagree on custom schemes and on `+`.)

1. Trim leading and trailing ASCII whitespace (space, tab, CR, LF).
2. **Recognize the form.** Scheme and host are case-insensitive.
   - **App form:** `kinrelay://pair`, then optionally one `/`, then `?` and the parameters. Anything after a `#` is dropped. Any other host or path is `not_pairing_url`.
   - **Web form:** `https://<authority>/pair`, optionally with a trailing `/` (the path is otherwise case-sensitive). The parameters are the text after the first `#` (the fragment). A query string, if present, is ignored and never read. An empty authority, or a different path, is `not_pairing_url`.
   - Any other scheme (including `http://`) is `not_pairing_url`.
   - **For now, the web form accepts any host.** That's safe because the phone pins the PC by `id`, so a spoofed host can't pair with anything else. In Phase 2 the phone will accept only the Kin Relay domain.
3. **Split the parameters** on `&`, skipping empty pieces. Split each piece at its first `=`. A piece with no `=` has an empty value. Names are compared as written (case-sensitive, not decoded). **Parameter order doesn't matter. Unknown parameters are ignored**, and their values aren't even decoded.
4. **Percent-decoding.** A value is decoded as follows: `%XX` (hex in either case) becomes a byte, every other character is itself (a `+` stays a `+`), and the bytes must then be valid UTF-8. A malformed `%` sequence or invalid UTF-8 makes that parameter `invalid_param`.
5. **Check the known parameters in this order**, and stop at the first error: `v`, `a`, `id`, `tk`, `n`. For each one: it appears more than once → `duplicate_param`; it's missing (and required) → `missing_param`; its value is bad → `invalid_param`. The error names the parameter.
   - **`v`**: must be ASCII digits only. Exactly `1` is accepted. Any other digit string (`2`, `01`, `10`) → `unsupported_version` ("Update the app"). Anything else (empty, a sign such as `+1`, letters, non-ASCII digits such as `%D9%A1`) → `invalid_param`. `v` is checked first on purpose, so a QR from a future version says "update the app" even if its other parameters have changed shape.
   - **`a`**: split the decoded value on `,`. There must be 1 to 3 entries. **Any invalid entry, an empty entry, or more than 3 entries rejects the whole URL (`invalid_param`).** A QR comes from our own PC, so a bad entry means a bug or tampering. An entry is `IPv4:port`:
     - The IPv4 address is four decimal octets from 0 to 255, in ASCII digits only, with no leading zeros and no other characters (no `+`). IPv6, hostnames and brackets are invalid.
     - The address must be private (RFC 1918): `10.0.0.0/8`, `172.16.0.0/12` or `192.168.0.0/16`, **or in the shared address space `100.64.0.0/10`** (`100.64.0.0` to `100.127.255.255`, used by carrier-grade NAT and by Tailscale). Public, loopback (`127.x`), link-local (`169.254.x`) and unspecified addresses are invalid. The Android emulator's host alias `10.0.2.2` is in `10.0.0.0/8`, so it is valid.
     - The PC still doesn't *write* `100.64.0.0/10` addresses in Phase 1, because VPN adapters are excluded. The phone accepts them anyway: one bad entry rejects the whole URL, so a v1 phone that refused this range would block a later Tailscale feature until the app is updated, with no version bump to say so.
     - The port is decimal in ASCII digits only, 1 to 65535, with no leading zeros (no `+`).
     - Entries are kept in order. Duplicates aren't removed.
   - **`id`**: 43 base64url characters. **`tk`**: 22 base64url characters.
   - **`n`**: optional. After decoding, an empty value counts as absent. A name longer than 64 characters is cut to 64 (not an error).
6. **Result:** `{ version: 1, addresses: [{host, port}…], id, token, name or null }`.

Error categories, which the test vectors use: `not_pairing_url`, `unsupported_version`, `missing_param`, `invalid_param`, `duplicate_param`. The phone shows a message for each: "That's not a Kin Relay code", "Update the app", or "That code is damaged. Show a new one on your PC".

### Steps

1. The phone races the addresses in `a` and connects with TLS. It checks that the PC certificate's SHA-256(SPKI) equals `id`, and aborts if it doesn't.
2. The phone sends `POST /v1/pair`:

   ```json
   { "token": "<tk>", "name": "Pixel 8", "platform": "android", "model": "Google Pixel 8", "appVersion": "0.1.0", "protocol": 1 }
   ```

   | Field | Required | Notes |
   |---|---|---|
   | `token` | yes | String. The `tk` from the QR code |
   | `name` | yes | Device name, 1 or more characters. Longer than 64 is cut to 64, not rejected |
   | `platform` | yes | `android` or `windows`. Any other non-empty string is accepted and stored as is |
   | `protocol` | yes | Integer. `1` |
   | `model` | no | String, for display |
   | `appVersion` | no | String, for display |

   The body must **not** carry the phone's device ID, and if it does anyway (for example a `deviceId` field), the PC ignores it. **The PC takes the phone's device ID only from the TLS client certificate.**

3. The PC checks the request in this order and returns the first error (see the table below): body size (over 1 MiB is `413`), body valid (this includes the `Content-Type` rule of [Transport](#transport)), `protocol` is 1, token. (A missing client certificate never gets this far: the TLS handshake already failed, see [Transport](#transport).) It compares the token in constant time and checks that it hasn't expired or been used. It then saves the phone's device ID, computed from the TLS client certificate, into `devices`, together with `name`, `platform`, `model` and `appVersion`. The check and the "used" mark are one atomic step, so two requests with the same token can't both succeed. The token is consumed only when the record is saved.
4. The PC responds `200`:

   ```json
   { "name": "Desk PC", "deviceId": "<pc-device-id>", "protocol": 1 }
   ```

5. The phone checks that `deviceId` equals the pinned `id` and that `protocol` is 1 (otherwise it treats the result as `unsupported_version` and saves nothing). It then saves the PC as trusted and opens the WebSocket.

**Pairing errors:**

| Situation | Status | `error` |
|---|---|---|
| The body is over 1 MiB (1,048,576 bytes) | 413 | `too_large` |
| Body isn't a JSON object (or isn't sent with a JSON `Content-Type`, or isn't valid JSON), or a required field is missing or has the wrong type | 400 | `bad_request` |
| `protocol` isn't 1 | 426 | `unsupported_version` |
| The token isn't the active token (wrong text, wrong length, or no pairing in progress) | 401 | `bad_token` |
| The token is the active token but older than 5 minutes (or was invalidated, see below) | 401 | `token_expired` |
| The token was already used for a successful pairing, even if it has also expired by now | 401 | `token_used` |

- **Expiry:** a token expires 300 seconds after the PC created it (the moment it shows the QR). The PC remembers the last tokens it issued, for at least one hour after they expired or were used, so it can answer `token_expired` or `token_used` instead of `bad_token`. A token that was used **and** has expired answers `token_used`: the "used" fact is checked first.
- **Rate limit, per token:** every `bad_token` response counts as one failed attempt against the **currently active token** (the one on screen). The source IP isn't used. After **5** failed attempts within any 60 seconds, the PC invalidates that token. From then on the token answers `token_expired`, even if the right text is sent, and the user has to show a new QR code. The fifth failure itself still answers `bad_token`. A new token starts with a clean count. Responses other than `bad_token` aren't counted.
- Pairing an already-paired phone again (with a fresh token) just updates its record.

## Control channel: WebSocket

`GET /v1/ws` upgrades to a WebSocket. It needs a client certificate in `devices`, otherwise the upgrade is refused with `401 unauthorized` and the usual JSON error body. Each device has at most one WebSocket, and a new connection replaces the old one.

- **Frames:** every message is one **text** frame containing one UTF-8 JSON object. A binary frame closes the connection with code 1003. Invalid UTF-8 closes it with 1007. A message can be at most **1 MiB = 1,048,576 bytes** in UTF-8, counting the whole serialized JSON (so escaping counts, and senders measure the serialized frame, not the text). A longer message closes the connection with 1009, with no reply. Longer text is sent as a `.txt` file instead.
- **The libraries don't do the 1007 and 1009 checks for you**, so each app does them itself:
  - **Invalid UTF-8:** tungstenite (PC) returns a UTF-8 error to the caller and doesn't send a 1007 close, so when the PC sees that error it closes with 1007 itself. OkHttp (phone) never reports it: it decodes text frames with replacement, so a bad byte arrives as U+FFFD. That can't be told apart from a real U+FFFD (senders use it for lone surrogates), so on the phone the 1007 rule is best effort, and the phone must not close a connection just because a text contains U+FFFD. The invalid-UTF-8 vector is for the PC side.
  - **Size:** OkHttp puts no limit on the size of an incoming message, and tungstenite's default limits are far above 1 MiB. Each app enforces 1 MiB itself, measuring the UTF-8 size of the whole message (all fragments together), and closes with 1009 and no reply when it's over. On the PC, set the library's maximum message and frame size to 1 MiB so it stops reading early. On the phone, measure each received text (`utf8Size` / `toByteArray().size`) and close with 1009 if it's over. It's checked after OkHttp has buffered it, which is the best the library allows.
- **No extensions or subprotocols** are required.

### Close codes

| Code | Name | Used when |
|---|---|---|
| 1000 | normal | Clean shutdown: app quit, network lost, or after `unpair` |
| 4001 | `replaced` | The server closes the old socket of a device because a new one connected |
| 4002 | `unsupported_version` | Sent after an `unsupported_version` error message |
| 4003 | `unauthorized` | The server closes a live socket because the device was removed from `devices` and no `unpair` was exchanged |
| 1002 | protocol error | The first frame that is a JSON object with a string `type` isn't `hello` (an `error` is sent first), or no `hello` was accepted within 10 seconds of the socket opening. A first frame that isn't such an object gets an `error` and the connection stays open (see [Opening handshake](#opening-handshake)) |
| 1003 / 1007 / 1009 | standard | Binary frame / bad UTF-8 / frame over 1 MiB |

The close reason text is for humans only (at most 123 bytes). Code never matches on it.

What a client does after a close:

- **1000, a network error, no close code, and any code not listed in the table above** (1001, 1011, 1006, an unknown 4xxx, …): reconnect with the usual backoff. The same goes for 1002, 1003, 1007 and 1009 received from the peer: the backoff stops a persistent bug from turning into a fast loop.
- **4001:** don't reconnect on its own. Something else took over this device's connection. Reconnect only on the next user action or network change.
- **4002:** don't retry automatically. Show "Update the app on your phone/PC".
- **4003, or `401 unauthorized` on the upgrade:** the other side no longer trusts this device. Stop reconnecting and tell the user to pair again.

**When the backoff resets.** The backoff steps are in [architecture.md](architecture.md#finding-each-other). Opening a connection doesn't reset them, and neither does an accepted `hello` by itself. **Only a connection that stayed open, with the peer's `hello` accepted, for at least 60 seconds** (60,000 ms by a monotonic clock, counted from the moment the `hello` was accepted) resets the backoff to its first step, whatever way that connection ends. A connection that closes sooner, or one where no `hello` was ever accepted, leaves the backoff where it was, so a peer that accepts the `hello` and then closes at once can't turn the reconnect into a fast loop. (A network change or a user action still resets it, as architecture.md says.)

### One connection per device

When a paired device opens a WebSocket while one is already open, the server accepts the new connection and **closes the old one with 4001**. From then on only the new socket is used. Messages that were sent on the old socket and never acked aren't lost: their senders re-send them (same IDs) on the new connection.

### Opening handshake

1. Right after the WebSocket opens, **each side sends its own `hello` at once**, without waiting for the other's.
2. A side doesn't send anything else (such as queued items) until it has received the other's `hello` and accepted its `protocol`.
3. **Before a `hello` is accepted, the receiver rejects every other message.** The rule is: a frame that is a JSON object with a string `type` other than `hello` gets an `error` (`bad_request`) and the connection is closed with 1002. Messages can't overtake the peer's `hello`, so that is always a bug. A frame that is *not* such an object (not JSON, not an object, no `type`, a `type` that isn't a string) gets the usual `bad_request` `error` and the connection **stays open**. The 10-second limit still applies, and a `hello` that fails validation doesn't count as accepted.
4. If `hello.protocol` is a valid integer other than 1, the receiver sends `error` (`unsupported_version`, `ref` = the hello's `id` when it is a valid ID) and closes with 4002. This check comes before the other fields of the `hello` are looked at (see the validation order below). Both sides may do this at the same time. That's fine.
5. A `hello` is only special as the first message. A later one is handled the same way (version check, and the stored name is updated) and is still not acked.

### Messages

All messages are JSON objects with a `type`. Fields are *required* unless the table says optional.

| Type | Direction | Fields | Receiver does |
|---|---|---|---|
| `hello` | Both, first message | `id`, `protocol`, `name`, `platform`, `appVersion` | Checks the protocol version. On a mismatch, sends `error` (`unsupported_version`) and closes. Updates the peer's stored name |
| `ack` | Both | `ref` (the acknowledged `id`) | Marks the item delivered |
| `error` | Both | `ref` (optional), `code`, `message` | Marks the item failed and shows the error. No retry |
| `text` | Both | `id`, `text`, `isUrl` | Copies to the clipboard, notifies, stores in history. Links get an "Open" action |
| `clipboard` | PC → phone | `id`, `text` | Sets the clipboard silently. Not stored in history |
| `offer` | PC → phone | `id` (the transfer ID), `files[]` of `{fileId, name, size, mime}`, `totalSize` | Acks, then downloads each file (see below) |
| `complete` | Phone → PC | `id`, `transferId` | Marks the transfer done |
| `cancel` | Both | `id`, `transferId`, `reason` | Stops the transfer and deletes partial files |
| `unpair` | Both | `id` | Acks, removes the sender from `devices`, and closes the connection with 1000 |
| `ping` | Both | `id`, `sentAt` | Acks at once, then shows "Ping from <name>" |

Field types:

| Field | Type and rules |
|---|---|
| `id`, `ref`, `transferId`, `fileId` | Strings. See [Message IDs](#message-ids-and-acks) |
| `protocol` | Integer. `1` |
| `name` | String, 1 or more characters (a device name; longer than 64 is cut for display) |
| `platform` | String. `android` or `windows`; any other non-empty string is accepted |
| `appVersion` | String, such as `0.1.0` |
| `text` | String, 1 or more characters |
| `isUrl` | Boolean. The sender sets it to `true` only when the whole text, trimmed, is one absolute `http://` or `https://` URL with no whitespace. The receiver uses it only to offer "Open", and opens only `http` and `https` links |
| `files` | Array with at least 1 entry. Every `fileId` in it is different |
| `name` (in a file) | String. The sender's raw file name, cleaned by the receiver (see [File names](#file-names)). May be empty |
| `size`, `totalSize` | Integers, in bytes. `totalSize` is the sum of the `size` values. Receivers use the per-file sizes and don't reject a wrong `totalSize` |
| `mime` | String. `application/octet-stream` when the sender doesn't know. Receivers never trust it for security decisions |
| `reason` | String. The values senders use: `user` (the user canceled), `error` (a failure that won't go away, such as a file that's gone, changed or can't be saved), `expired` (the offer or transfer timed out). Receivers accept any string, including an empty one, and handle all of them the same |
| `sentAt` | Integer, Unix milliseconds |
| `code`, `message` (in `error`) | Strings. `code` is from the [error codes](#error-codes). An unknown code is still treated as a failure. `message` is human text in English and never used for logic |

- Use `text` for explicit sends (share sheet, text menu, hotkey) and `clipboard` for automatic sync.
- **Unknown message types** get an `error` with code `unsupported_type`. So does a known type sent in the wrong direction (for example `clipboard` from the phone).
- **Validation order**, so every implementation answers the same way:
  1. Size, UTF-8 and text frame (closes, no reply).
  2. The frame must be a JSON object (the text must be one valid JSON value as in [JSON conventions](#json-conventions): strict RFC 8259, nesting depth at most 64, no lone surrogate escapes), otherwise `bad_request`.
  3. `type` must be present and a string, otherwise `bad_request`.
  4. Before a `hello` is accepted, any `type` other than `hello` is `bad_request` **and the close with 1002** (see [Opening handshake](#opening-handshake)).
  5. An unknown or wrong-direction `type` gets `unsupported_type`.
  6. For `hello` only: **`protocol` is checked before every other field.** If it is a valid integer other than 1, the answer is `unsupported_version` (with the close 4002), whatever the other fields look like. A later protocol version may change the shape of `hello`, and the old side must still say "update the app" instead of `bad_request` followed by a timeout. If `protocol` is missing or isn't a valid integer, validation carries on to step 7, where it fails as `bad_request`.
  7. Required fields and types, otherwise `bad_request`.
  8. Duplicate check, then handle.

  Invalid frames (steps 2, 3, 5, 7) get an `error` and **the connection stays open**, except the 1002 of step 4 and the 4002 of step 6. In the `error`, **`ref` is the offending message's `id` when it is a valid ID** (a string of 1 to 64 characters, see [Message IDs](#message-ids-and-acks)), and is left out in every other case: no `id`, an `id` that isn't a string, or one that's empty or over 64 characters.
- Liveness uses WebSocket ping/pong frames, not JSON messages. `ping` below is a separate, application-level message for debugging.

### Message IDs and acks

- **These messages carry an `id` and get an `ack`: `text`, `clipboard`, `offer`, `complete`, `cancel`, `unpair`, `ping`.**
- `hello` has an `id` but isn't acked, and its ID isn't recorded for de-duplication.
- `ack` and `error` have no `id`. They point at another message with `ref`, and are never acked themselves. An `error` sent in place of an ack (for example `disk_full` in answer to an `offer`) ends that message's life: the sender doesn't retry.
- **ID format:** senders use a **UUIDv7, lowercase, hyphenated** (36 characters). Receivers accept **any non-empty string of up to 64 characters**, and handle it as an opaque value. The exception is a transfer ID or file ID, which also ends up in a URL path, so it must be 1 to 64 characters from `A–Z a–z 0–9 - _` (a UUIDv7 already is). An `offer` whose `id` doesn't fit gets `bad_request`.
- `offer.id` is the transfer ID. `complete` and `cancel` have their own message `id` and point at the transfer with `transferId`. (The transfer ID can't double as the message ID of `complete` and `cancel`: the receiver would take them for repeats of the `offer`, and an `ack` to them couldn't be matched.)
- **When to ack:** as soon as the message is valid and recorded. Not after the user sees it, and not after a download ends. An `offer` is acked before any file is downloaded. If the receiver can't accept the message, it sends an `error` with `ref` instead of the ack. Acks are due within 5 seconds (see [architecture.md](architecture.md#connection-lifecycle)).
- **Retries:** a sender keeps an item until it's acked (or failed with `error`, or canceled). It re-sends it with the **same `id`** after reconnecting, in the original order. It doesn't re-send on a connection that's still open, because the connection is reliable and in order. `ping` is never queued or re-sent. `unpair` isn't re-sent after the connection closes: the sender has already removed the peer locally.
- **De-duplication:** a receiver remembers the IDs of the messages it has processed for **at least 24 hours, and never fewer than the newest 1,000 IDs** (so an ID can be forgotten only when it's both older than 24 hours and beyond the newest 1,000). The IDs are stored in the database so they survive restarts, and are shared by all of a device's connections. A message with a remembered ID isn't processed again, but the receiver **repeats its first response** (the same `ack`, or the same `error`), so it stores the outcome together with the ID. A message rejected as invalid (validation steps 2 to 7 above) was never processed, so its ID isn't remembered.

### `ping`

A debug and latency tool. It's part of v1 and is separate from WebSocket ping/pong frames, which stay the liveness mechanism.

- `{ "type": "ping", "id": "…", "sentAt": 1760443200000 }`, from either side.
- **Receiver:** acks it immediately, before touching the UI, then shows a notification (Android) or toast (Windows) reading "Ping from <name>", where `<name>` is the peer's name from its `hello`. Pings aren't stored in history. A repeated `id` doesn't show a second toast.
- **Sender:** measures the round trip with a monotonic clock, from the moment it sends the `ping` until the matching `ack` arrives, and shows it (for example "Ping: 42 ms"). It doesn't use `sentAt` or the receiver's clock. With no ack in 5 seconds, it shows "No reply" and treats the connection as dead, as for any unacked message. With no connection, it says so and sends nothing.

### Unpair

`unpair` carries its own `id`. The sender removes the peer from its own `devices` first, then sends the message if it's connected. The receiver acks, removes the sender from `devices`, and closes with 1000. The sender closes too, after the ack or after 5 seconds. A device that's been removed gets `401 unauthorized` from then on.

## Files: phone → PC

IDs: the transfer ID and every `fileId` match `[A-Za-z0-9_-]{1,64}`, so they go in URLs without escaping. The phone picks them. All numbers are integers in bytes. A JSON request body is limited to 1 MiB, and a longer one gets `413 too_large`.

1. `POST /v1/transfers`

   ```json
   { "id": "<transfer-id>", "files": [ { "fileId": "f1", "name": "IMG_0042.jpg", "size": 3145728, "mime": "image/jpeg" } ] }
   ```

   All fields are required, and `files` has at least one entry. The response `200` gives, for every file in the same order, how many bytes the PC already has (0 for a new transfer, more when resuming, and equal to `size` when the file is already complete):

   ```json
   { "id": "<transfer-id>", "files": [ { "fileId": "f1", "offset": 0 } ] }
   ```

   Posting an ID that already exists **resumes** it, as long as the request lists **the same `fileId`s** as the stored transfer (as a set, in any order). It returns the current offsets, in the order of the request. Names, sizes and MIME types in a repeated request are ignored: the stored ones stay. If the `fileId`s differ (one more, one fewer, or a different ID), the PC keeps the stored list unchanged and answers `400 bad_request`. To send a different set of files, the phone uses a new transfer ID. The PC cleans each file name when it first creates the transfer, and stores the final name with the transfer, so a resume doesn't pick a new name (see [File names](#file-names)).

   **A resumed `POST` is checked again** for `413` and `507`, like a new one, so a disk that filled up in the meantime is noticed (the free-space check counts only the bytes the PC doesn't have yet). **If the phone's own source file changed** since the transfer was created (its size or modified time differs), the phone doesn't resume it: it sends `DELETE /v1/transfers/{id}` and starts over with a **new transfer ID**.

   Errors: `400 bad_request` (invalid body, or a repeated ID with different `fileId`s), `404 not_found` (the ID belongs to another device, see below), `507 disk_full`, `413 too_large`. What counts as too large is up to the PC. `500 internal` if the PC can't create a file (see [File names](#file-names)). **The phone treats a `500 internal` on a transfer as a failed transfer** (it shows the error) **and doesn't retry it automatically**; the user can retry from the history.

   **Ownership.** A transfer belongs to the device that created it (a `POST /v1/transfers`) or received it (an `offer` sent to it). Every request about a transfer ID from any *other* paired device, whatever the method (`PUT`, `GET`, `DELETE`), answers `404 not_found`, as if the ID didn't exist. A `POST` with an ID that another device owns answers `404` **instead of creating it** (an ID that doesn't exist yet would be created), so one device can neither take over nor probe another device's transfers. A `cancel` or `complete` message from a device that doesn't own the transfer is treated as one for an unknown transfer (acked and ignored).

2. `PUT /v1/transfers/{id}/files/{fileId}?offset=N`. The body is the raw bytes from offset N to the end, `Content-Length` = size − N, no chunked encoding. `offset` is required: ASCII digits only, no sign, no leading zeros (`0` is fine, `007` and `+5` are `400 bad_request`), and at most 2⁵³−1 (a larger number is `400 bad_request` too, not a `409`).
   - The PC accepts the upload only if N **equals the number of bytes it has durably written for that file**. Otherwise it answers `409 offset_mismatch { "error": "offset_mismatch", "message": "…", "offset": <actual> }` and the client retries from `actual`. This includes N greater than the file size and N less than the bytes already stored (the PC never overwrites).
   - `200 { "received": <size> }` means the file is complete. An empty body at N = size, or a 0-byte file with N = 0, is valid and gets this answer.
   - A `Content-Length` that isn't size − N, a missing or invalid `offset`, and a body that runs past the declared size are `400 bad_request`.
   - If the connection breaks mid-upload, the PC keeps the bytes that arrived (in the `.part` file), and that count is the offset it reports next time.
   - An unknown transfer or file ID is `404 not_found`. The PC accepts files in any order.

3. The phone sends files one at a time, in order. The transfer is done when every PUT has returned 200.

4. `DELETE /v1/transfers/{id}` cancels the transfer. **It's only for phone → PC transfers**, the ones created with `POST /v1/transfers`. It answers `204` with no body once the PC has dropped the partial files (files already completed stay). Its own check order is `401` not paired → `404` unknown transfer, or not owned by this device, or an ID that is only a PC → phone offer. A PC → phone offer is never canceled with `DELETE`: either side sends a `cancel` message (see below). A transfer the PC wants to cancel itself is canceled with a `cancel` message too. **A `DELETE` that is retried** (the `204` was lost) finds the transfer already gone and gets `404`: **the phone treats `404` on a `DELETE` as done.**

**Order of checks**, so every PC answers the same way. The first failing check decides the answer. "Not found" includes a transfer that belongs to another device.

- `POST /v1/transfers`: `401` not paired → `413` body over 1 MiB → `400` body invalid (not JSON, the `Content-Type` rule, a missing or wrong field) → `404` the ID exists and belongs to another device → `400` the ID exists with different `fileId`s → `413` too large → `507` not enough free space (the last two also for a resumed `POST`) → `500` the PC can't create a file.
- `PUT …/files/{fileId}`: `401` not paired → `404` unknown transfer (or not yours), then unknown file → `400` `offset` missing or invalid, or `Content-Length` missing (chunked) → `409` offset isn't the stored byte count → `400` `Content-Length` isn't size − N → `507` not enough free space → `500` the PC can't create or write the `.part` file. `Content-Type` isn't looked at.
- `GET …/files/{fileId}`: `401` not paired → `404` unknown, expired or not-yours transfer, then unknown file → `409` source changed → `416` bad `Range`.
- `DELETE /v1/transfers/{id}`: `401` not paired → `404` unknown (or not yours, or a PC → phone offer) → `204`.
- **Unknown routes and methods** (anything that isn't one of the endpoints above or `GET /v1/ws`, including a known path with a method it doesn't have, such as `GET /v1/pair`) answer `404 not_found` **whoever asks, paired or not**. Authorization applies only to the endpoints that exist, so there is no `401` for an unknown route and no `405` anywhere.
- `POST /v1/pair`: `413` body over 1 MiB → `400` body invalid (same rule as above) → `426` → `401` token errors, see [Pairing](#steps). It never answers `401 unauthorized`.

## Files: PC → phone

1. The PC sends an `offer` over the WebSocket. The phone acks it and accepts automatically, because the PC is paired. If the phone can't accept (for example the disk is full), it sends `error` (`disk_full`, `ref` = the offer's `id`) instead.
2. `GET /v1/transfers/{id}/files/{fileId}`, with `Range: bytes=N-` when resuming. This is the only range form supported. `N` is ASCII digits only, no sign and no leading zeros; anything else (`bytes=+5-`, `bytes=007-`, `bytes=-5`, `bytes=0-9`, a number over 2⁵³−1) is malformed.
   - Without `Range`: `200` with the whole file, `Content-Length` = size, `Content-Type: application/octet-stream`, `Accept-Ranges: bytes`. **The phone never sends `Range` for a 0-byte file** (there's nothing to resume, and any `Range` on it is unsatisfiable): it sends a plain `GET`, which answers `200` with `Content-Length: 0`.
   - With a valid `Range` and N < size (N = 0 included): `206 Partial Content` with `Content-Range: bytes N-(size-1)/size` and `Content-Length` = size − N.
   - If a request has a `Range` and the answer is `200`, the phone restarts the file from byte 0.
   - N is at least the file size, or the header is malformed: `416` with `Content-Range: bytes */size` and the usual JSON error body (`bad_request`). A phone that already has all the bytes skips the GET and sends `complete`.
   - `404 not_found`: the offer expired or was canceled, or the IDs are unknown.
   - `409 source_changed`: the file's size or modified time changed since the offer.
3. When every file is saved, the phone checks that each file has exactly `size` bytes and sends `complete` (with `transferId`).
4. **A file that fails for good fails the whole offer.** If a `GET` answers `404` or `409` (the offer expired or was canceled, or the file changed or is gone), the phone stops, deletes its partial files and sends a `cancel` with `reason` `error` for the whole transfer. Files that were already saved completely stay. A `cancel` for an offer the PC has already dropped is acked and ignored, so sending it is always safe. A dropped connection isn't final: the phone resumes with a `Range` request.
5. Offers expire after 24 hours, and then the PC stops serving them.

**Canceling.** The sender of the files is the PC for an `offer`, and the phone for a `POST /v1/transfers`.

- PC → phone transfer: either side cancels with a `cancel` message (`transferId`). The PC stops serving, and the phone stops downloading and deletes its partial files.
- Phone → PC transfer: the phone cancels with `DELETE`. The PC cancels with a `cancel` message, and the phone stops uploading.
- A `cancel` or `complete` for a transfer the receiver doesn't know, or that already ended, is acked and otherwise ignored.

## File names

The receiver cleans every file name before saving, with exactly these steps in this order. **Characters** means Unicode code points. **Lengths are in UTF-8 bytes** (see step 6).

1. **Path:** keep only what follows the last `/` or `\` (everything, if there is none). `a/b\c.txt` → `c.txt`, and `dir/` → empty.
2. **Forbidden characters:** replace each of `< > : " / \ | ? *`, each control character (U+0000–U+001F and U+007F–U+009F) and each **`Bidi_Control` character** with `_`. The bidi controls are U+061C, U+200E, U+200F, U+202A–U+202E and U+2066–U+2069. Without this a name such as `photo` + U+202E + `gpj.exe` would be shown as `photoexe.jpg` while really being an `.exe`; it becomes `photo_gpj.exe`. (`/` and `\` can't remain after step 1. They're listed for completeness. Other invisible characters, such as U+200B, aren't touched.)
3. **Trailing dots and spaces:** remove all trailing `.` and U+0020 characters, repeatedly, from the end.
4. **Empty:** if nothing is left, the name is `file`.
5. **Reserved names:** take the part of the name before its **first** `.` (the whole name if there is no dot) and remove trailing spaces from it. If it equals, ignoring ASCII case, one of `CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9`, or `COM` or `LPT` followed by `¹`, `²` or `³` (U+00B9, U+00B2, U+00B3), put `_` in front of the name. This matches Windows (see Microsoft's naming documentation), which reserves these names with any extension: `con.txt` → `_con.txt`, `NUL.tar.gz` → `_NUL.tar.gz`, `Com1` → `_Com1`, `LPT0.txt` → `_LPT0.txt`, `COM²` → `_COM²`. A name like `console.txt`, `COM10` or `.con` is fine.
6. **Collision and length limit,** together. Let `cleaned` be the result of step 5.
   - **Split** `cleaned` into `stem` and `ext`. The extension is the text from the **last** `.` to the end (the dot included), if that dot isn't the first character and the extension is **at most 16 UTF-8 bytes** including the dot. Otherwise `ext` is empty and the stem is the whole name. A leading dot doesn't start an extension, so `.bashrc` has no extension, and `.bashrc.bak` has `.bak`.
   - Try suffixes `s` = (none), ` (1)`, ` (2)`, … in turn. For each, the candidate is `stem'` + `s` + `ext`, where `stem'` is the stem cut to the **longest prefix of whole code points** whose UTF-8 size is at most `200 − bytes(s) − bytes(ext)`. A cut never lands inside a code point (so never inside a surrogate pair or a multi-byte character), but it may separate a combining mark or a joiner from its base.
   - **If the cut removed anything,** then for every candidate, after cutting: (a) remove the trailing dots and spaces from `stem'`; (b) if it is now empty, `stem'` is `file`; (c) if `stem'` is a reserved name (the step 5 test: ASCII case-insensitive, including `COM0`, `LPT0` and the superscript forms), put `_` in front of it. If the cut stem still contains a dot, step 5 has already looked at the same text before the first dot, so there's no second cut. An uncut stem is left exactly as it is: `a .txt` keeps its space before the extension. The result never has more than 200 bytes, because (a) to (c) only run after a cut, and the room that stripping frees is larger than the one byte that `_` or the four of `file` need.
   - The result is the first candidate whose name isn't in the `existing` set. Names are compared **case-insensitively** (Unicode default lowercase mapping, no normalization), because Windows is case-insensitive.
   - So the **200-byte limit is applied last**, to the final name including any suffix. The suffix goes **before the extension**: `photo.jpg` → `photo (1).jpg`. With no extension it goes at the end: `notes` → `notes (1)`.
   - **Why bytes, and why 200:** 200 *characters* of CJK text are 600 UTF-8 bytes, which breaks the 255-byte file-name limit of ext4 and f2fs on Android. MediaStore also adds a `.pending-<time>-` prefix while a file is being written, so there must be headroom below 255. 200 bytes also stays under NTFS's limit of 255 UTF-16 units for any text (at most 200 units for 200 bytes).

`existing` is the list of names already in the destination folder. It also includes names already claimed by files of the same transfer, and by transfers still in progress (a `name.ext.part` file claims `name.ext`). The cleaning runs once per file, when the transfer is created or accepted, and the final name is stored with the file and reused when resuming.

**Create new, never overwrite.** Files are created with create-new semantics: `CREATE_NEW` on Windows, `O_EXCL` on Unix. The same goes for the move from `name.ext.part` to its final name: it never replaces an existing file. The create call is the real collision check, because `existing` can be stale, and because NTFS may treat two names as equal that the app compared as different.

- If the create fails with "already exists", that's a collision: add the name to `existing` and try the next suffix. **This holds for the move from `.part` to the final name too**: when something has appeared under the final name since the transfer was created, the move takes the next suffix and the stored final name becomes that one. "Already exists" is never a `500` and never fails the transfer.
- **Implementer note (PC):** Rust's `std::fs::rename` **overwrites** an existing target on Windows, so it must not be used for this move. Use `MoveFileExW` **without** `MOVEFILE_REPLACE_EXISTING` (it fails with `ERROR_ALREADY_EXISTS`, which counts as "already exists"; `CREATE_NEW` reports `ERROR_FILE_EXISTS`, also "already exists"), or an equivalent no-replace rename.
- Any other create or move failure (permission, a name the filesystem refuses, an I/O error) fails the file with `internal`. On the PC the request that needed the file answers `500 internal`. On the phone the transfer is stopped with a `cancel` message with reason `error`.

**Android and scoped storage.** On Android the app only sees its *own* files in the Downloads collection, so `existing` can only contain files the app created, not other apps' files. MediaStore may also rename a file when it's inserted. The app therefore uses and stores **the final name MediaStore chose**, and shows that name. On Android the cleaned name is a request, and on Windows it is the name.

## Error codes

**Every non-2xx HTTP response has a JSON body** with `Content-Type: application/json; charset=utf-8`:

```json
{ "error": "<code>", "message": "<human text>" }
```

`message` is short human text in English. Code never matches on it. `offset_mismatch` adds the actual offset, `{ "error": "offset_mismatch", "message": "…", "offset": 1048576 }`. The same `error`/`message` pair is used in the WebSocket `error` message, where the field is called `code`. Unknown routes and methods get `404 not_found`, paired or not.

| Code | HTTP status | Meaning |
|---|---|---|
| `bad_request` | 400 (and 416) | Body, header or query isn't valid, including a JSON body that isn't sent as JSON. Also a WebSocket message that isn't valid |
| `unauthorized` | 401 | The client certificate is valid but its device ID isn't in `devices` (unpaired, or removed). Never returned by `POST /v1/pair`. A missing certificate fails the TLS handshake instead |
| `bad_token` / `token_expired` / `token_used` | 401 | Pairing failed |
| `unsupported_version` | 426 | Protocol versions don't match |
| `not_found` | 404 | Transfer or file not found (or the transfer belongs to another device), or unknown route |
| `offset_mismatch` | 409 | Upload offset is wrong; retry from the offset returned |
| `source_changed` | 409 | The file changed after it was offered |
| `too_large` | 413 | Over a size limit |
| `disk_full` | 507 | Not enough free space |
| `unsupported_type` | (WebSocket) | Unknown message type, or a type sent in the wrong direction |
| `internal` | 500 | A bug, or the PC couldn't create or move a file for a reason other than "already exists" (the phone doesn't retry that automatically). Check the logs |

## Versioning

- The version appears in the path (`/v1/`) and in `hello.protocol`.
- New optional fields can be added within v1. Receivers ignore fields they don't know.
- A breaking change means v2. In that case the apps show "Update the app on your phone/PC".

## Clarifications (2026-10-14)

Added so the Windows and Android implementations come out the same. No existing decision changed, except the ones marked **changed**. Each item says where it lives above.

1. **`ping` message added** (Messages, `ping`). Both directions, `id` and `sentAt`. The receiver acks and shows "Ping from <name>". The sender measures the round trip to the matching `ack` with its own monotonic clock. Not queued. Separate from WebSocket ping/pong.
2. **Encodings** (Encodings). base64url without padding for IDs and `tk`. Device ID is 43 characters: base64url(SHA-256(DER SPKI)). Short ID is RFC 4648 base32 (uppercase, no padding) of the raw 32-byte hash, first 8 characters. `tk` is 22 characters.
3. **HTTP error body** (Error codes). `{ "error", "message" }` on every non-2xx JSON response, `offset` added for `offset_mismatch`, `Content-Type: application/json; charset=utf-8`. Added code `bad_request` (400, 416, and for invalid WebSocket messages), because malformed input had no code.
4. **Message IDs and acks** (Message IDs and acks). The list of messages that carry `id` and get an `ack`. `hello` is not acked. `ack` and `error` use `ref`. UUIDv7 recommended, any non-empty string up to 64 characters accepted (transfer and file IDs limited to `[A-Za-z0-9_-]`). De-duplication memory is the larger of 24 hours and the newest 1,000 IDs, and a duplicate gets the same response again. Retries re-send the same `id` after a reconnect.
5. **`complete` and `cancel` fields — changed.** They now have their own message `id` plus `transferId`. The old table used the transfer ID as their `id`, which made them look like repeats of the `offer` and made their `ack.ref` ambiguous. `offer.id` is still the transfer ID. Cancel directions are spelled out (Files).
6. **WebSocket behavior** (Control channel). A first frame that is a JSON object with a string `type` other than `hello` gets `bad_request` and a 1002 close, and a `hello` is expected within 10 seconds (first frames that aren't such an object get `bad_request` and the connection stays open: see item 14). Close codes: 1000 normal, 4001 replaced, 4002 unsupported_version, 4003 unauthorized, plus the standard 1002/1003/1007/1009. The server closes the old socket with 4001 when a new one connects, and the old client doesn't auto-reconnect. Each side sends `hello` at once and sends nothing else until it has the peer's. Text frames only, UTF-8, 1 MiB limit measured on the serialized frame. Validation order, and invalid messages keep the connection open. A `hello` after the handshake is handled normally.
7. **Pairing** (Pairing). The 5-failures rule is per token, counted on `bad_token` against the active token, and an invalidated token then answers `token_expired`. Statuses and bodies for each error, with the check order. Required and optional fields of `POST /v1/pair`. The phone's device ID comes only from the TLS client certificate. The token is consumed atomically. The token's expiry is 300 seconds from creation.
8. **Pairing URL parsing** (Parsing rules). Order of parameters doesn't matter, unknown ones are ignored, no duplicates. Required `v`, `a`, `id`, `tk`, optional `n`. `a` is 1–3 IPv4 `host:port` entries, private (RFC 1918) or in `100.64.0.0/10` (**changed**, see item 12), and any invalid entry or a fourth entry rejects the URL. `v` other than 1 is "update the app", checked first. Web form `https://<any host>/pair#…` is accepted with any host for now, restricted in Phase 2. `10.0.2.2` is valid. Percent-decoding doesn't turn `+` into a space. The name is cut to 64 characters. The PC's canonical way of writing the URL.
9. **File names** (File names). Reserved names match case-insensitively and with any extension, and are checked after trailing dots and spaces are stripped. Step order is fixed. **Changed:** the 200 limit now counts UTF-8 bytes, not code points, and is applied last, after the collision suffix (the reasons are in the File names section). The extension is the last `.ext` of at most 16 bytes including the dot, and a leading dot isn't an extension. The collision suffix goes before the extension, comparison is case-insensitive, and the `existing` list includes names claimed by the same transfer and by `.part` files. A name is cleaned once and reused on resume.
10. **JSON typing** (JSON conventions). Required vs optional fields, `null` handling, unknown fields ignored, integers 0 to 2⁵³−1 with no fraction, strict booleans, timestamps in Unix milliseconds, code point counting, lone surrogates replaced by the sender. `isUrl` is defined. `totalSize` is informational. Field limits (`files` not empty, `text` not empty, unique `fileId`).
11. **HTTP file endpoint details** (Files). The PUT offset rule (equals bytes stored, never overwrite), `Content-Length` and 0-byte files, `DELETE` returns 204 or 404, `GET` status codes for no Range, valid Range, and an unsatisfiable Range (416), and a `200` to a `Range` request restarts the file. The order of checks for each endpoint, and `413 too_large` for a JSON body over 1 MiB.

### Review fixes (2026-10-14, second pass)

Added after the review of the first version of this list. Every item has vectors in `protocol/` and a check in `protocol/check-vectors.ts`.

12. **Pairing URL: `100.64.0.0/10` is accepted** (**changed**; Parsing rules). The shared address space (carrier-grade NAT, Tailscale) is valid in `a`. The PC still doesn't write these addresses in Phase 1. Reason: one bad entry rejects the whole URL, so rejecting the range in v1 phones would block a later Tailscale feature without a version bump.
13. **File names, second pass** (File names). (a) After a cut, trailing dots and spaces are removed from the cut stem, an empty stem becomes `file`, and a reserved stem gets `_`: the cut could produce `""`, `CON` or `CON.txt`. (b) **Changed:** lengths are in UTF-8 bytes (name at most 200, extension at most 16 including the dot), cut on code point boundaries. (c) Files are created with create-new semantics, "already exists" is a collision, any other failure is `internal`. (d) The twelve `Bidi_Control` characters are replaced by `_` in step 2. (e) `COM0`, `LPT0` and `COM`/`LPT` + `¹ ² ³` are reserved. (f) Android scoped-storage note: the final name is the one MediaStore chose. The two open points that used to end this section are resolved by (b) and (d).
14. **First frames before `hello`** (Control channel, Opening handshake, Validation order). The close table, the handshake text and the validation order now agree: 1002 only for a JSON object with a string `type` that isn't `hello`, or when no `hello` is accepted within 10 seconds. Anything else before `hello` that is invalid gets `bad_request` and stays open.
15. **`error.ref`** (Validation order). `ref` is the offending message's `id` when it is a valid ID (1 to 64 characters), left out otherwise. This is what the vectors already did.
16. **JSON edge cases** (JSON conventions). Duplicate keys: the last value wins (**changed**: they used to be "invalid, behavior unspecified"), and both apps parse into a generic tree before validating. Lone surrogate escapes make the text invalid (`bad_request`, no `ref`). `hello.protocol` is checked before the other `hello` fields, so a future `hello` gets `unsupported_version`.
17. **Digits** (Encodings). ASCII digits only for `v`, octets, port, `offset` and the Range `N`, no sign, no leading zeros (`v=01` stays `unsupported_version`). `offset` over 2⁵³−1 is `400`.
18. **TLS client certificates** (**changed**; Transport, Pairing errors, Error codes). A client certificate is mandatory in the handshake. `401 unauthorized` therefore means a valid certificate whose device ID isn't in `devices`, on every endpoint except `POST /v1/pair`, which never answers it. Both sides MUST verify the TLS 1.3 CertificateVerify signature even though any client chain is accepted.
19. **WebSocket libraries** (Control channel, Frames). The 1007 and 1009 checks aren't done by the libraries. What each app has to do is spelled out. Close codes that aren't in the table are handled like 1000.
20. **Transfers** (Files). `DELETE` has its own check order and is only for phone → PC transfers. A transfer belongs to the device that created or received it, and any other device gets `404 not_found`. A repeated `POST /v1/transfers` with the same `fileId`s resumes, with different ones it's `400` and the stored list stays. A file that fails with 404 or 409 on a download cancels the whole offer with reason `error`. A used token that has also expired answers `token_used`.
21. **Discovery** (Discovery). The mDNS contract is now in this document, copied from architecture.md.

### Review fixes (2026-10-14, third pass)

Added after the third review. Every rule change has vectors in `protocol/` and a check in `protocol/check-vectors.ts`. **Changed** marks a rule that moved.

22. **Certificate profile** (**changed**; Transport). Both sides write a self-signed X.509 v3 certificate with an ECDSA P-256 key (SPKI 91 bytes), `ecdsa-with-SHA256` (no parameters in the AlgorithmIdentifier), a positive serial of at most 20 bytes, RFC 5280 time encodings, and no critical extension. The TLS signature scheme is `ecdsa_secp256r1_sha256`. "The PC accepts any client certificate chain" now reads: the issuer, dates and extensions aren't checked, but the leaf must be a parseable v3 certificate with a P-256 key. Reason: rustls-webpki, used by `verify_tls13_signature`, rejects v1/v2 certificates, unknown critical extensions and bad encodings. The four certificates in `device-id.json` were regenerated to follow the profile (same keys, same device IDs): the earlier ones carried a critical `basicConstraints`.
23. **Content-Type of a request body** (**changed**; Transport). The media type must be `application/json` (ASCII case-insensitive) with no parameter or exactly `charset=utf-8`. Anything else, including another charset or another parameter, is `400 bad_request`. It's part of "body invalid", after `401` and `413`. `PUT` ignores `Content-Type`.
24. **`POST` with an ID owned by another device** (Files, Ownership). The answer is `404 not_found` *instead of creating the transfer*. The old text said "exactly as if the ID didn't exist", which for a `POST` would have meant creating it.
25. **Repeated `POST`** (Files). A resumed `POST` re-checks `413` and `507`. If the phone's own source file changed, the phone sends `DELETE` and uses a new transfer ID instead of resuming.
26. **Backoff reset** (Control channel, Close codes). Only a connection that stayed open with an accepted `hello` for at least 60 s resets the backoff. Opening a connection or an instantly closed one doesn't, which prevents a fast loop.
27. **Create and rename failures** (Files, File names). The phone treats `500 internal` on a transfer as a failed transfer, with no automatic retry. On the PC, "already exists" when moving `.part` to the final name takes the next suffix and is never a `500`. Implementer note: Rust's `std::fs::rename` overwrites, so use `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` or an equivalent.
28. **Strict JSON and nesting depth** (JSON conventions). Unquoted literals (`yes`, `NaN`), `01`, `+1` and the like are invalid JSON in known and unknown fields (kotlinx probably accepts them, so the Kotlin side validates the raw text first). Nesting deeper than 64 is invalid JSON. Both give `bad_request` with no `ref`, or `400`.
29. **Small rules** (Files, Error codes). The phone treats `404` on a retried `DELETE` as done. The phone never sends `Range` for a 0-byte file. An unknown route or method is `404` for everyone, paired or not (the vectors already behaved that way; now it's written down). `POST /v1/pair` with a body over 1 MiB is `413`, checked before the body is parsed.
30. **QR address list** (Pairing): the default-route address is never left out because its adapter looks virtual (`vEthernet`). **TLS 1.3 timing** (Transport): a refused client certificate shows up on the phone's first read, not in the handshake call, and the phone-side invalid-signature test is optional because Conscrypt always checks. **Typo:** item 13(d) said ten `Bidi_Control` characters, it's twelve.
